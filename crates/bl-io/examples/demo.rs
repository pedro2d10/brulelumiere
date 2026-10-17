use bl_io::{Category, ConfigStore};
use serde_json::json;

fn main() {
    let dir = std::env::temp_dir().join("bl-io-demo");
    std::fs::create_dir_all(&dir).unwrap();
    let mut store = ConfigStore::new(&dir)
        .with_category(Category::Prefs)
        .with_version(2, "__saveVersion")
        .with_max_backups(5)
        .with_defaults(json!({ "AutoSaveSeconds": 120, "units": 0, "CutList": [] }));
    store.load().unwrap();
    store.set("AutoSaveSeconds", 300);
    store.set("DefaultFontName", "Arial");
    let backup = store.save().unwrap();
    println!("wrote: {}", store.path().display());
    println!("backup: {:?}", backup);
}
