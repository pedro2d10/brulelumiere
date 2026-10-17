//! `bl-io` — a JSON-document configuration store modeled on how
//! LightBurn performs preferences I/O.
//!
//! ## What LightBurn does (the model)
//!
//! LightBurn keeps its application bundle in `~/.local/share/LightBurn/`
//! (read-only program code) and its user data in `~/.config/LightBurn/`:
//!
//! - `prefs.ini` — a single JSON object (~194 keys), *despite the `.ini`
//!   extension*, holding scalars and nested arrays like `CutList` and
//!   `DeviceList`.
//! - `backup/` — with `prefs/`, `machine_settings/` and `legacy/` subdirs,
//!   driven by its `PrefsBackupHandler` / `MachineBackupHandler` and surfaced
//!   in a Backup Browser.
//! - a configurable base dir (`--prefsdir`), a `defaultprefs.ini` defaults
//!   layer, crash-safe autosave, and a "saved with a newer version" guard.
//!
//! ## What this crate provides
//!
//! The same mechanism, as a reusable store:
//!
//! ```no_run
//! use bl_io::{ConfigStore, Category};
//! use serde_json::json;
//!
//! let mut store = ConfigStore::new("/home/pierre/.config/MyApp")
//!     .with_file_name("prefs.json")
//!     .with_category(Category::Prefs)
//!     .with_version(2, "__saveVersion")
//!     .with_max_backups(10)
//!     .with_defaults(json!({
//!         "AutoSaveSeconds": 120,
//!         "units": 0,
//!         "CutList": [],
//!     }));
//!
//! store.load().unwrap();                 // merges file over defaults
//! store.set("AutoSaveSeconds", 300);
//! let backup = store.save().unwrap();    // backs up, then atomic-writes
//! println!("previous version archived at {:?}", backup);
//! ```

mod backup;
mod error;
mod merge;
mod store;

pub use backup::{list as list_backups, rotate as rotate_backup, Category};
pub use error::{Error, Result};
pub use merge::merge_defaults;
pub use store::ConfigStore;
