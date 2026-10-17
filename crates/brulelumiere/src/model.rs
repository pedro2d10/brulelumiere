//! The laser-project data model, inspired by LightBurn's `prefs.ini`.
//!
//! Field names are serialized to match LightBurn's own JSON keys (`CutList`,
//! `DeviceList`, `maxPower`, `units`, `AutoSaveSeconds`) so the documents look
//! familiar. Serde ignores unknown keys (such as the store's `__saveVersion`),
//! so a round-trip through [`bl_io::ConfigStore`] is lossless for us.

use serde::{Deserialize, Serialize};

use crate::shapes::Shape;

/// Measurement units. LightBurn stores this as `units: 0` (mm) / `1` (inch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "u8", into = "u8")]
pub enum Units {
    Millimeters,
    Inches,
}

impl Units {
    pub const ALL: [Units; 2] = [Units::Millimeters, Units::Inches];
    pub fn label(self) -> &'static str {
        match self {
            Units::Millimeters => "mm",
            Units::Inches => "pouces",
        }
    }
}

impl From<u8> for Units {
    fn from(v: u8) -> Self {
        match v {
            1 => Units::Inches,
            _ => Units::Millimeters,
        }
    }
}
impl From<Units> for u8 {
    fn from(u: Units) -> u8 {
        match u {
            Units::Millimeters => 0,
            Units::Inches => 1,
        }
    }
}

/// Operation type of a cut layer, matching LightBurn's `"type"` strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerType {
    Cut,
    Scan,
    Image,
    Tool,
}

impl LayerType {
    pub const ALL: [LayerType; 4] =
        [LayerType::Cut, LayerType::Scan, LayerType::Image, LayerType::Tool];
    pub fn label(self) -> &'static str {
        match self {
            LayerType::Cut => "Cut",
            LayerType::Scan => "Scan",
            LayerType::Image => "Image",
            LayerType::Tool => "Tool",
        }
    }
}

/// A single cut layer — an entry of LightBurn's `CutList`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CutLayer {
    pub index: u32,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: LayerType,
    /// mm/s.
    pub speed: f64,
    #[serde(rename = "maxPower")]
    pub max_power: f64,
    #[serde(rename = "minPower", default = "default_min_power")]
    pub min_power: f64,
    #[serde(rename = "numPasses", default = "default_passes")]
    pub passes: u32,
}

fn default_min_power() -> f64 {
    10.0
}
fn default_passes() -> u32 {
    1
}

impl CutLayer {
    pub fn new(index: u32) -> Self {
        Self {
            index,
            name: format!("C{index:02}"),
            kind: LayerType::Cut,
            speed: 100.0,
            max_power: 20.0,
            min_power: default_min_power(),
            passes: default_passes(),
        }
    }

    /// Palette color implied by the layer's index (as in LightBurn).
    pub fn rgb(&self) -> (u8, u8, u8) {
        crate::shapes::layer_rgb(self.index)
    }
}

/// Where the job origin sits on the bed — LightBurn's device origin corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Origin {
    FrontLeft,
    FrontRight,
    RearLeft,
    RearRight,
}

impl Origin {
    pub const ALL: [Origin; 4] =
        [Origin::FrontLeft, Origin::FrontRight, Origin::RearLeft, Origin::RearRight];
    pub fn label(self) -> &'static str {
        match self {
            Origin::FrontLeft => "Avant-gauche",
            Origin::FrontRight => "Avant-droit",
            Origin::RearLeft => "Arrière-gauche",
            Origin::RearRight => "Arrière-droit",
        }
    }
}

/// A laser machine — an entry of LightBurn's `DeviceList`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    #[serde(rename = "width")]
    pub width_mm: f64,
    #[serde(rename = "height")]
    pub height_mm: f64,
    pub origin: Origin,
}

impl Device {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), width_mm: 400.0, height_mm: 400.0, origin: Origin::FrontLeft }
    }
}

/// The whole project document persisted through the config store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub units: Units,
    #[serde(rename = "AutoSaveSeconds")]
    pub auto_save_seconds: u32,
    #[serde(rename = "CutList")]
    pub layers: Vec<CutLayer>,
    #[serde(rename = "DeviceList")]
    pub devices: Vec<Device>,
    /// The vector drawing: a flat list of shapes, as in a LightBurn project.
    #[serde(rename = "Shapes", default)]
    pub shapes: Vec<Shape>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            units: Units::Millimeters,
            auto_save_seconds: 120,
            layers: vec![CutLayer::new(0), CutLayer::new(1)],
            devices: vec![Device::new("Ma machine")],
            shapes: Vec::new(),
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_roundtrips_through_json() {
        let p = Project::default();
        let v = serde_json::to_value(&p).unwrap();
        // Keys look like LightBurn's.
        assert!(v.get("CutList").is_some());
        assert!(v.get("DeviceList").is_some());
        assert_eq!(v.get("units").unwrap(), 0);
        let back: Project = serde_json::from_value(v).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn project_with_shapes_roundtrips() {
        use crate::shapes::Shape;
        let mut p = Project::default();
        p.shapes.push(Shape::rect(0, 10.0, 10.0, 40.0, 25.0));
        p.shapes.push(Shape::ellipse(1, 80.0, 40.0, 20.0, 15.0));
        p.shapes.push(Shape::polyline(0, &[(10.0, 60.0), (60.0, 60.0), (35.0, 90.0)], true));
        let v = serde_json::to_value(&p).unwrap();
        assert!(v.get("Shapes").is_some());
        assert_eq!(v["Shapes"][0]["Type"], "Rect");
        let back: Project = serde_json::from_value(v).unwrap();
        assert_eq!(p, back);
    }

    /// Not a test: writes a sample drawing to the real config dir so the app
    /// opens with something on the canvas. Run with:
    /// `cargo test -p brulelumiere -- --ignored seed_sample_project`
    #[test]
    #[ignore]
    fn seed_sample_project() {
        use crate::shapes::Shape;
        use bl_io::ConfigStore;

        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var_os("HOME").unwrap()).join(".config")
            })
            .join("brulelumiere");

        let mut p = Project::default();
        p.shapes.push(Shape::rect(0, 10.0, 10.0, 60.0, 40.0));
        p.shapes.push(Shape::rect(0, 20.0, 20.0, 40.0, 20.0));
        p.shapes.push(Shape::ellipse(1, 120.0, 40.0, 30.0, 20.0));
        p.shapes
            .push(Shape::polyline(2, &[(10.0, 80.0), (70.0, 80.0), (40.0, 120.0)], true));
        p.shapes
            .push(Shape::polyline(1, &[(90.0, 80.0), (110.0, 110.0), (140.0, 85.0), (160.0, 120.0)], false));

        let defaults = serde_json::to_value(Project::default()).unwrap();
        let mut store = ConfigStore::new(base)
            .with_file_name("project.json")
            .with_version(1, "__saveVersion")
            .with_defaults(defaults);
        store.set_document(serde_json::to_value(&p).unwrap());
        let backup = store.save().unwrap();
        eprintln!("seeded {} (backup: {:?})", store.path().display(), backup);
    }

    #[test]
    fn unknown_keys_are_ignored() {
        // The store adds __saveVersion; deserialization must tolerate it.
        let mut v = serde_json::to_value(Project::default()).unwrap();
        v.as_object_mut().unwrap().insert("__saveVersion".into(), 7.into());
        let back: Project = serde_json::from_value(v).unwrap();
        assert_eq!(back, Project::default());
    }
}
