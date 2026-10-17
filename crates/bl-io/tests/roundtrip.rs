use std::fs;
use std::path::PathBuf;

use bl_io::{Category, ConfigStore, Error};
use serde_json::json;

/// Unique scratch dir per test, under the system temp dir.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bl-io-test-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn first_run_uses_defaults_then_persists() {
    let dir = scratch("firstrun");
    let defaults = json!({ "AutoSaveSeconds": 120, "units": 0 });

    let mut store = ConfigStore::new(&dir)
        .with_defaults(defaults)
        .with_version(1, "__saveVersion");

    // No file yet → defaults.
    store.load().unwrap();
    assert_eq!(store.get("AutoSaveSeconds").unwrap(), 120);

    store.set("AutoSaveSeconds", 300);
    let backup = store.save().unwrap();
    assert!(backup.is_none(), "nothing existed yet, so no backup");
    assert!(store.path().exists());

    // Reload from disk: user value wins, default survives, version stamped.
    let mut reopened = ConfigStore::new(&dir)
        .with_defaults(json!({ "AutoSaveSeconds": 120, "units": 0, "NewKey": true }))
        .with_version(1, "__saveVersion");
    reopened.load().unwrap();
    assert_eq!(reopened.get("AutoSaveSeconds").unwrap(), 300);
    assert_eq!(reopened.get("units").unwrap(), 0);
    assert_eq!(reopened.get("NewKey").unwrap(), true); // added default
    assert_eq!(reopened.get("__saveVersion").unwrap(), 1);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn backups_rotate_and_cap() {
    let dir = scratch("backups");
    let mut store = ConfigStore::new(&dir)
        .with_category(Category::Prefs)
        .with_max_backups(3);

    for i in 0..6 {
        store.set("iteration", i);
        store.save().unwrap();
    }

    // First save had no prior file; 5 subsequent saves produced backups,
    // capped at 3.
    let backups = bl_io::list_backups(
        &dir.join("backup"),
        Category::Prefs,
        "prefs",
        "json",
    )
    .unwrap();
    assert_eq!(backups.len(), 3, "capped to max_backups");

    // Live file holds the latest value.
    let mut reopened = ConfigStore::new(&dir);
    reopened.load().unwrap();
    assert_eq!(reopened.get("iteration").unwrap(), 5);

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn newer_version_is_rejected() {
    let dir = scratch("version");

    // Write a file stamped with version 9.
    let mut writer = ConfigStore::new(&dir).with_version(9, "__saveVersion");
    writer.set("hello", "world");
    writer.save().unwrap();

    // An older build (version 2) must refuse it.
    let mut old_build = ConfigStore::new(&dir).with_version(2, "__saveVersion");
    match old_build.load() {
        Err(Error::NewerVersion { file_version, app_version, .. }) => {
            assert_eq!(file_version, 9);
            assert_eq!(app_version, 2);
        }
        other => panic!("expected NewerVersion, got {other:?}"),
    }

    fs::remove_dir_all(&dir).ok();
}

#[test]
fn no_temp_file_left_behind_after_save() {
    let dir = scratch("notemp");
    let mut store = ConfigStore::new(&dir);
    store.set("x", 1);
    store.save().unwrap();

    let leftovers: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "atomic write left a temp file: {leftovers:?}");

    fs::remove_dir_all(&dir).ok();
}
