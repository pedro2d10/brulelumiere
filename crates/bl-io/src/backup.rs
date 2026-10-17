use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

/// Backup categories, mirroring LightBurn's `~/.config/LightBurn/backup/`
/// tree, which has `prefs/`, `machine_settings/` and `legacy/` subdirs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Prefs,
    MachineSettings,
    Legacy,
}

impl Category {
    /// Subdirectory name under `backup/`.
    pub fn dir_name(self) -> &'static str {
        match self {
            Category::Prefs => "prefs",
            Category::MachineSettings => "machine_settings",
            Category::Legacy => "legacy",
        }
    }
}

/// Copy `source` into `backup/<category>/` with a timestamped name, then prune
/// that category to the `max_backups` most recent entries.
///
/// Returns the path of the backup that was written, or `None` if `source`
/// did not exist (nothing to back up) or `max_backups` is 0.
///
/// LightBurn surfaces this as "Backup Created: <path>" in its Backup Browser.
pub fn rotate(
    source: &Path,
    backup_root: &Path,
    category: Category,
    max_backups: usize,
) -> Result<Option<PathBuf>> {
    if max_backups == 0 || !source.exists() {
        return Ok(None);
    }

    let dir = backup_root.join(category.dir_name());
    fs::create_dir_all(&dir).map_err(|e| Error::Io { path: dir.clone(), source: e })?;

    let stem = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("prefs");
    let ext = source.extension().and_then(|s| s.to_str()).unwrap_or("json");

    // Millisecond stamp keeps names lexicographically sortable == chronological,
    // and a monotonically increasing counter breaks ties within the same ms.
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut backup_path = dir.join(format!("{stem}_{millis:013}.{ext}"));
    let mut dedup = 0u32;
    while backup_path.exists() {
        dedup += 1;
        backup_path = dir.join(format!("{stem}_{millis:013}_{dedup}.{ext}"));
    }

    fs::copy(source, &backup_path)
        .map_err(|e| Error::Io { path: backup_path.clone(), source: e })?;

    prune(&dir, stem, ext, max_backups)?;
    Ok(Some(backup_path))
}

/// Delete oldest backups for this stem until at most `max_backups` remain.
fn prune(dir: &Path, stem: &str, ext: &str, max_backups: usize) -> Result<()> {
    let prefix = format!("{stem}_");
    let suffix = format!(".{ext}");

    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| Error::Io { path: dir.to_path_buf(), source: e })?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&prefix) && n.ends_with(&suffix))
                .unwrap_or(false)
        })
        .collect();

    // Names sort chronologically thanks to the zero-padded millis stamp.
    entries.sort();

    if entries.len() > max_backups {
        for old in &entries[..entries.len() - max_backups] {
            fs::remove_file(old).map_err(|e| Error::Io { path: old.clone(), source: e })?;
        }
    }
    Ok(())
}

/// List existing backups for `stem` in `category`, newest last.
/// Powers a "Backup Browser"-style list.
pub fn list(backup_root: &Path, category: Category, stem: &str, ext: &str) -> Result<Vec<PathBuf>> {
    let dir = backup_root.join(category.dir_name());
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let prefix = format!("{stem}_");
    let suffix = format!(".{ext}");
    let mut entries: Vec<PathBuf> = fs::read_dir(&dir)
        .map_err(|e| Error::Io { path: dir.clone(), source: e })?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&prefix) && n.ends_with(&suffix))
                .unwrap_or(false)
        })
        .collect();
    entries.sort();
    Ok(entries)
}
