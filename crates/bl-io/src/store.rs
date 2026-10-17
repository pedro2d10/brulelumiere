use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::backup::{self, Category};
use crate::error::{Error, Result};
use crate::merge::merge_defaults;

/// A JSON-document configuration store modeled on LightBurn's prefs I/O.
///
/// One file holds one JSON object (like LightBurn's 194-key `prefs.ini`,
/// which is JSON despite the extension). On top of raw read/write it layers:
///
/// - a **configurable base dir** (LightBurn's `--prefsdir`),
/// - a **defaults layer** merged *under* user values (`defaultprefs.ini`),
/// - **atomic save** via write-temp-then-rename, so a crash never leaves a
///   half-written file,
/// - **rotating categorized backups** under `backup/<category>/`,
/// - a **version stamp** with a newer-than-this-build guard.
pub struct ConfigStore {
    base_dir: PathBuf,
    file_name: String,
    category: Category,
    defaults: Value,
    version_key: String,
    app_version: u64,
    max_backups: usize,
    /// In-memory document: defaults merged with whatever was loaded.
    doc: Value,
}

impl ConfigStore {
    /// Create a store rooted at `base_dir`, using `prefs.json` as the file,
    /// the `Prefs` backup category, an empty defaults layer, app version 1,
    /// and 10 retained backups. Tune with the `with_*` builders.
    ///
    /// Nothing touches disk until you call [`ConfigStore::load`] or
    /// [`ConfigStore::save`].
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            file_name: "prefs.json".to_string(),
            category: Category::Prefs,
            defaults: Value::Object(Map::new()),
            version_key: "__saveVersion".to_string(),
            app_version: 1,
            max_backups: 10,
            doc: Value::Object(Map::new()),
        }
    }

    pub fn with_file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = name.into();
        self
    }

    pub fn with_category(mut self, category: Category) -> Self {
        self.category = category;
        self
    }

    /// Supply the defaults layer. Must be a JSON object.
    pub fn with_defaults(mut self, defaults: Value) -> Self {
        self.defaults = defaults;
        self
    }

    /// Set this build's save version and the key it is stored under.
    /// On load, a file whose stored version is greater is rejected.
    pub fn with_version(mut self, app_version: u64, version_key: impl Into<String>) -> Self {
        self.app_version = app_version;
        self.version_key = version_key.into();
        self
    }

    pub fn with_max_backups(mut self, max_backups: usize) -> Self {
        self.max_backups = max_backups;
        self
    }

    /// Full path of the live config file.
    pub fn path(&self) -> PathBuf {
        self.base_dir.join(&self.file_name)
    }

    fn backup_root(&self) -> PathBuf {
        self.base_dir.join("backup")
    }

    /// Load the file, merging it over the defaults. If the file is missing,
    /// the document becomes the defaults (first run). Returns `NewerVersion`
    /// if the file was written by a newer build.
    pub fn load(&mut self) -> Result<()> {
        let path = self.path();

        if !path.exists() {
            self.doc = self.defaults.clone();
            return Ok(());
        }

        let bytes = fs::read(&path).map_err(|e| Error::Io { path: path.clone(), source: e })?;
        let loaded: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Parse { path: path.clone(), source: e })?;

        if !loaded.is_object() {
            return Err(Error::NotAnObject { path });
        }

        // Forward-compat guard: refuse files from a newer build.
        if let Some(file_version) = loaded.get(&self.version_key).and_then(Value::as_u64) {
            if file_version > self.app_version {
                return Err(Error::NewerVersion {
                    path,
                    file_version,
                    app_version: self.app_version,
                });
            }
        }

        self.doc = merge_defaults(&self.defaults, &loaded);
        Ok(())
    }

    /// Back up the current file, stamp the version, and atomically persist the
    /// in-memory document. Returns the path of the backup written, if any.
    pub fn save(&mut self) -> Result<Option<PathBuf>> {
        let path = self.path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::Io { path: parent.to_path_buf(), source: e })?;
        }

        // Back up the existing file before we overwrite it.
        let backup = backup::rotate(&path, &self.backup_root(), self.category, self.max_backups)?;

        // Stamp this build's save version into the document.
        if let Value::Object(map) = &mut self.doc {
            map.insert(self.version_key.clone(), Value::from(self.app_version));
        }

        let mut text = serde_json::to_string_pretty(&self.doc)
            .map_err(|e| Error::Parse { path: path.clone(), source: e })?;
        text.push('\n');

        atomic_write(&path, text.as_bytes())?;
        Ok(backup)
    }

    /// Borrow the whole document.
    pub fn document(&self) -> &Value {
        &self.doc
    }

    /// Replace the entire in-memory document. Useful when the caller keeps a
    /// typed model and serializes it back wholesale before [`save`](Self::save).
    /// The version stamp is (re)applied on the next save.
    pub fn set_document(&mut self, doc: Value) {
        self.doc = doc;
    }

    /// Read a top-level key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.doc.get(key)
    }

    /// Set a top-level key, returning the previous value if any.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<Value>) -> Option<Value> {
        let map = match &mut self.doc {
            Value::Object(map) => map,
            other => {
                *other = Value::Object(Map::new());
                other.as_object_mut().expect("just set to object")
            }
        };
        map.insert(key.into(), value.into())
    }
}

/// Write `bytes` to `path` without ever leaving a partially written target:
/// write to a sibling temp file, fsync it, then rename over `path`. The rename
/// is atomic on POSIX, so readers see either the old file or the whole new one.
///
/// This is the role Qt's `QSaveFile` plays inside LightBurn.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("prefs.json");
    let tmp = dir.join(format!(".{file_name}.{}.tmp", std::process::id()));

    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|e| Error::Io { path: tmp.clone(), source: e })?;
        f.write_all(bytes)
            .map_err(|e| Error::Io { path: tmp.clone(), source: e })?;
        f.sync_all()
            .map_err(|e| Error::Io { path: tmp.clone(), source: e })?;
    }

    fs::rename(&tmp, path).map_err(|e| {
        // Leave no stray temp file behind on failure.
        let _ = fs::remove_file(&tmp);
        Error::Io { path: path.to_path_buf(), source: e }
    })?;

    // Best-effort directory fsync so the rename itself is durable.
    if let Ok(dir_handle) = File::open(dir) {
        let _ = dir_handle.sync_all();
    }
    Ok(())
}
