//! BrûleLumière — a standalone laser-project editor inspired by LightBurn.
//!
//! Vector drawing, cut layers, G-code, GRBL machine control and image tracing,
//! persisted through the `bl-io` config store (atomic save, rotating
//! backups, version guard).

mod app;
mod boolean;
mod gcode;
mod io;
mod machine;
mod model;
mod shapes;
mod text;
mod trace;

use std::path::PathBuf;

use app::BruleLumiereApp;

/// XDG config base: `$XDG_CONFIG_HOME/brulelumiere` or `$HOME/.config/brulelumiere`.
fn config_dir() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()) {
        return PathBuf::from(xdg).join("brulelumiere");
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join("brulelumiere")
}

fn main() -> eframe::Result<()> {
    let base_dir = config_dir();

    let native_options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([820.0, 560.0])
            .with_title("BrûleLumière — éditeur laser"),
        ..Default::default()
    };

    eframe::run_native(
        "BruleLumiere",
        native_options,
        Box::new(move |_cc| Ok(Box::new(BruleLumiereApp::new(base_dir)))),
    )
}
