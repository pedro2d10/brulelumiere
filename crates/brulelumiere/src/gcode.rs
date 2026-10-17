//! GRBL G-code generation and a path/time preview, inspired by LightBurn's
//! job preview and GRBL device output.
//!
//! Shapes are flattened to world-space polylines, grouped by cut layer, and
//! emitted in the layer list's order (the cut priority). Each polyline is a
//! rapid (`G0`) to its start, laser on (`M4`/`M3` + `S`), cutting moves (`G1`)
//! at the layer feed, then laser off (`M5`); the whole layer repeats for its
//! pass count. The travelled and cut lengths give a duration estimate.

use crate::model::Project;
use crate::shapes::XForm;

#[derive(Debug, Clone)]
pub struct GcodeOptions {
    /// Rapid (travel) feed rate, mm/min.
    pub travel_feed: f64,
    /// `S` value corresponding to 100 % power.
    pub power_scale: f64,
    /// Use `M4` (dynamic power) rather than `M3` (constant).
    pub dynamic_power: bool,
}

impl Default for GcodeOptions {
    fn default() -> Self {
        Self { travel_feed: 6000.0, power_scale: 1000.0, dynamic_power: true }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Job {
    pub gcode: String,
    pub seconds: f64,
    pub cut_mm: f64,
    pub travel_mm: f64,
}

fn dist(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn fmt(v: f64) -> String {
    let s = format!("{v:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".to_string() } else { s.to_string() }
}

/// Generate a GRBL job from the project.
pub fn generate(project: &Project, opts: &GcodeOptions) -> Job {
    let mut g = String::new();
    g.push_str("; BrûleLumière — export GRBL\n");
    g.push_str("G21 ; unités en mm\nG90 ; positionnement absolu\nM5 ; laser éteint\n");

    let mut cut_mm = 0.0;
    let mut travel_mm = 0.0;
    let mut seconds = 0.0;
    let mut pos = (0.0, 0.0);
    let on = if opts.dynamic_power { "M4" } else { "M3" };

    // Emit layers in list (priority) order.
    for layer in &project.layers {
        // Collect this layer's polylines.
        let mut polylines: Vec<(Vec<(f64, f64)>, bool)> = Vec::new();
        for shape in &project.shapes {
            for (pts, closed, cut_index) in shape.flatten(64, &XForm::IDENTITY) {
                if cut_index == layer.index && pts.len() >= 2 {
                    polylines.push((pts, closed));
                }
            }
        }
        if polylines.is_empty() {
            continue;
        }

        let feed = (layer.speed * 60.0).max(1.0); // mm/s -> mm/min
        let spindle = (layer.max_power / 100.0 * opts.power_scale).round();
        g.push_str(&format!(
            "\n; Calque {} ({}) — {} passe(s), {} mm/s, {}%\n",
            layer.name, layer.kind.label(), layer.passes, layer.speed, layer.max_power
        ));

        for _pass in 0..layer.passes.max(1) {
            for (pts, closed) in &polylines {
                let start = pts[0];
                // Rapid to the start.
                travel_mm += dist(pos, start);
                seconds += dist(pos, start) / (opts.travel_feed / 60.0);
                g.push_str("M5\n");
                g.push_str(&format!("G0 X{} Y{}\n", fmt(start.0), fmt(start.1)));
                g.push_str(&format!("{on} S{}\n", fmt(spindle)));

                let mut iter = pts.iter().copied();
                let mut prev = iter.next().unwrap();
                for p in iter {
                    g.push_str(&format!("G1 X{} Y{} F{}\n", fmt(p.0), fmt(p.1), fmt(feed)));
                    cut_mm += dist(prev, p);
                    seconds += dist(prev, p) / layer.speed.max(0.1);
                    prev = p;
                }
                if *closed {
                    g.push_str(&format!("G1 X{} Y{} F{}\n", fmt(start.0), fmt(start.1), fmt(feed)));
                    cut_mm += dist(prev, start);
                    seconds += dist(prev, start) / layer.speed.max(0.1);
                    prev = start;
                }
                pos = prev;
                g.push_str("M5\n");
            }
        }
    }

    g.push_str("\nM5 ; laser éteint\nG0 X0 Y0 ; retour origine\nM2 ; fin de programme\n");
    Job { gcode: g, seconds, cut_mm, travel_mm }
}

/// Format a duration in seconds as `m min s s`.
pub fn fmt_duration(seconds: f64) -> String {
    let total = seconds.round() as u64;
    let m = total / 60;
    let s = total % 60;
    if m > 0 {
        format!("{m} min {s} s")
    } else {
        format!("{s} s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::Shape;

    #[test]
    fn rectangle_perimeter_and_commands() {
        let mut p = Project::default();
        p.shapes.clear();
        // 10x20 rectangle, closed -> perimeter 60 mm.
        p.shapes.push(Shape::rect(0, 0.0, 0.0, 10.0, 20.0));
        let job = generate(&p, &GcodeOptions::default());
        assert!(job.gcode.contains("G1"));
        assert!(job.gcode.contains("M4"));
        assert!(job.gcode.contains("M5"));
        assert!((job.cut_mm - 60.0).abs() < 1e-6, "cut_mm = {}", job.cut_mm);
        assert!(job.seconds > 0.0);
    }

    #[test]
    fn passes_multiply_cut_length() {
        let mut p = Project::default();
        p.shapes.clear();
        p.layers = vec![crate::model::CutLayer::new(0)];
        p.layers[0].passes = 3;
        p.shapes.push(Shape::rect(0, 0.0, 0.0, 10.0, 10.0)); // perimeter 40
        let job = generate(&p, &GcodeOptions::default());
        assert!((job.cut_mm - 120.0).abs() < 1e-6, "cut_mm = {}", job.cut_mm);
    }

    #[test]
    fn empty_layer_is_skipped() {
        let p = Project::default(); // no shapes
        let job = generate(&p, &GcodeOptions::default());
        assert!(!job.gcode.contains("G1"));
        assert_eq!(job.cut_mm, 0.0);
    }
}
