//! Vector-drawing model, mirroring LightBurn's `.lbrn2` shape representation.
//!
//! A drawing is a flat list of [`Shape`]s. Each shape has a `CutIndex` (the cut
//! layer it belongs to) and an [`XForm`] (2D affine placement), exactly like
//! LightBurn. Geometry is one of:
//!
//! - `Rect`  — `W`, `H`, `Cr` (corner radius),
//! - `Ellipse` — `Rx`, `Ry`,
//! - `Path` — a `VertList` of [`Vertex`] (each with optional bezier handles
//!   `c0`/`c1`) and a `PrimList` of [`Prim`] (`L`ine or `B`ezier), the same
//!   vertex+primitive scheme LightBurn uses.

use serde::de::{self, Deserializer};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

/// A 2D affine transform, serialized like LightBurn's `<XForm>a b c d e f`.
///
/// Maps a local point `(x, y)` to `(a*x + c*y + e, b*x + d*y + f)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct XForm {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl XForm {
    pub const IDENTITY: XForm = XForm { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn translate(tx: f64, ty: f64) -> Self {
        XForm { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: tx, f: ty }
    }

    pub fn scale(sx: f64, sy: f64) -> Self {
        XForm { a: sx, b: 0.0, c: 0.0, d: sy, e: 0.0, f: 0.0 }
    }

    pub fn rotate(radians: f64) -> Self {
        let (s, c) = radians.sin_cos();
        XForm { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    /// Apply the transform to a local point.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    /// Compose: the returned transform applies `inner` first, then `self`
    /// (i.e. the matrix product `self * inner`).
    pub fn compose(&self, inner: &XForm) -> XForm {
        XForm {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    /// A world transform that applies `linear` about the pivot `(px, py)`.
    pub fn about(linear: XForm, px: f64, py: f64) -> XForm {
        XForm::translate(px, py)
            .compose(&linear)
            .compose(&XForm::translate(-px, -py))
    }
}

impl Serialize for XForm {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&format!(
            "{} {} {} {} {} {}",
            self.a, self.b, self.c, self.d, self.e, self.f
        ))
    }
}

impl<'de> Deserialize<'de> for XForm {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let n: Vec<f64> = s.split_whitespace().filter_map(|t| t.parse().ok()).collect();
        if n.len() != 6 {
            return Err(de::Error::custom("XForm must have 6 whitespace-separated numbers"));
        }
        Ok(XForm { a: n[0], b: n[1], c: n[2], d: n[3], e: n[4], f: n[5] })
    }
}

/// A path vertex. `c0` is the incoming bezier handle, `c1` the outgoing one;
/// `None` means a sharp corner (LightBurn's degenerate `c0x1c1x1`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vertex {
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c0: Option<(f64, f64)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c1: Option<(f64, f64)>,
}

impl Vertex {
    pub fn corner(x: f64, y: f64) -> Self {
        Vertex { x, y, c0: None, c1: None }
    }
}

/// A path primitive linking two vertices by index, as in LightBurn's PrimList.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "P")]
pub enum Prim {
    #[serde(rename = "L")]
    Line { a: usize, b: usize },
    #[serde(rename = "B")]
    Bezier { a: usize, b: usize },
}

/// Shape geometry, tagged by `Type` exactly like LightBurn's `<Shape Type=...>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "Type")]
pub enum ShapeKind {
    Rect {
        #[serde(rename = "W")]
        w: f64,
        #[serde(rename = "H")]
        h: f64,
        #[serde(rename = "Cr", default)]
        corner_radius: f64,
    },
    Ellipse {
        #[serde(rename = "Rx")]
        rx: f64,
        #[serde(rename = "Ry")]
        ry: f64,
    },
    Path {
        #[serde(rename = "VertList")]
        verts: Vec<Vertex>,
        #[serde(rename = "PrimList")]
        prims: Vec<Prim>,
        #[serde(default)]
        closed: bool,
    },
    /// A group of child shapes, as in LightBurn's `<Shape Type="Group">`.
    Group {
        #[serde(rename = "Children", default)]
        children: Vec<Shape>,
    },
}

/// A drawing element: geometry + its cut layer + its placement transform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    #[serde(rename = "CutIndex")]
    pub cut_index: u32,
    #[serde(rename = "XForm")]
    pub xform: XForm,
    #[serde(flatten)]
    pub kind: ShapeKind,
}

impl Shape {
    pub fn rect(cut_index: u32, x: f64, y: f64, w: f64, h: f64) -> Self {
        Shape {
            cut_index,
            xform: XForm::translate(x, y),
            kind: ShapeKind::Rect { w, h, corner_radius: 0.0 },
        }
    }

    pub fn ellipse(cut_index: u32, cx: f64, cy: f64, rx: f64, ry: f64) -> Self {
        Shape {
            cut_index,
            xform: XForm::translate(cx, cy),
            kind: ShapeKind::Ellipse { rx, ry },
        }
    }

    /// An open or closed polyline through `points` (model coordinates).
    pub fn polyline(cut_index: u32, points: &[(f64, f64)], closed: bool) -> Self {
        let verts: Vec<Vertex> = points.iter().map(|&(x, y)| Vertex::corner(x, y)).collect();
        let mut prims = Vec::new();
        for i in 0..verts.len().saturating_sub(1) {
            prims.push(Prim::Line { a: i, b: i + 1 });
        }
        if closed && verts.len() > 2 {
            prims.push(Prim::Line { a: verts.len() - 1, b: 0 });
        }
        Shape { cut_index, xform: XForm::IDENTITY, kind: ShapeKind::Path { verts, prims, closed } }
    }

    /// Flatten the shape into one world-space polyline (after its XForm),
    /// sampling ellipses and beziers. Returns the points and whether the
    /// outline is closed. Groups have no outline of their own (see
    /// [`Shape::flatten`]).
    pub fn outline(&self, samples: usize) -> (Vec<(f64, f64)>, bool) {
        let (local, closed) = self.local_outline(samples);
        (local.into_iter().map(|(px, py)| self.xform.apply(px, py)).collect(), closed)
    }

    /// Outline in the shape's own local coordinates (before its XForm).
    fn local_outline(&self, samples: usize) -> (Vec<(f64, f64)>, bool) {
        match &self.kind {
            ShapeKind::Rect { w, h, .. } => {
                (vec![(0.0, 0.0), (*w, 0.0), (*w, *h), (0.0, *h)], true)
            }
            ShapeKind::Ellipse { rx, ry } => {
                let n = samples.max(12);
                let pts = (0..n)
                    .map(|i| {
                        let t = i as f64 / n as f64 * std::f64::consts::TAU;
                        (rx * t.cos(), ry * t.sin())
                    })
                    .collect();
                (pts, true)
            }
            ShapeKind::Path { verts, prims, closed } => {
                let mut out = Vec::new();
                for prim in prims {
                    match *prim {
                        Prim::Line { a, b } => {
                            if let (Some(va), Some(vb)) = (verts.get(a), verts.get(b)) {
                                if out.is_empty() {
                                    out.push((va.x, va.y));
                                }
                                out.push((vb.x, vb.y));
                            }
                        }
                        Prim::Bezier { a, b } => {
                            if let (Some(va), Some(vb)) = (verts.get(a), verts.get(b)) {
                                let p0 = (va.x, va.y);
                                let p1 = va.c1.unwrap_or(p0);
                                let p3 = (vb.x, vb.y);
                                let p2 = vb.c0.unwrap_or(p3);
                                let n = samples.max(8);
                                let start = if out.is_empty() { 0 } else { 1 };
                                for i in start..=n {
                                    let t = i as f64 / n as f64;
                                    out.push(cubic(p0, p1, p2, p3, t));
                                }
                            }
                        }
                    }
                }
                (out, *closed)
            }
            ShapeKind::Group { .. } => (Vec::new(), false),
        }
    }

    /// Flatten this shape (recursing into groups) into world-space polylines,
    /// each tagged with the cut-layer index to draw it in. `world` is the
    /// transform inherited from any enclosing groups.
    pub fn flatten(&self, samples: usize, world: &XForm) -> Vec<(Vec<(f64, f64)>, bool, u32)> {
        let eff = world.compose(&self.xform);
        match &self.kind {
            ShapeKind::Group { children } => children
                .iter()
                .flat_map(|c| c.flatten(samples, &eff))
                .collect(),
            _ => {
                let (local, closed) = self.local_outline(samples);
                let pts = local.into_iter().map(|(x, y)| eff.apply(x, y)).collect();
                vec![(pts, closed, self.cut_index)]
            }
        }
    }

    /// Reduce this shape to LightBurn path parts `(xform, verts, prims, closed)`
    /// in local coordinates. Rect/Ellipse are synthesized into vertices and
    /// line primitives; Path is returned as-is. Groups return `None`.
    pub fn as_path_parts(&self, samples: usize) -> Option<(XForm, Vec<Vertex>, Vec<Prim>, bool)> {
        match &self.kind {
            ShapeKind::Group { .. } => None,
            ShapeKind::Path { verts, prims, closed } => {
                Some((self.xform, verts.clone(), prims.clone(), *closed))
            }
            _ => {
                let (local, closed) = self.local_outline(samples);
                let verts: Vec<Vertex> = local.iter().map(|&(x, y)| Vertex::corner(x, y)).collect();
                let mut prims = Vec::new();
                for i in 0..verts.len().saturating_sub(1) {
                    prims.push(Prim::Line { a: i, b: i + 1 });
                }
                if closed && verts.len() > 2 {
                    prims.push(Prim::Line { a: verts.len() - 1, b: 0 });
                }
                Some((self.xform, verts, prims, closed))
            }
        }
    }

    /// World-space bounding box `(min_x, min_y, max_x, max_y)`, or `None` if empty.
    pub fn world_bbox(&self, samples: usize) -> Option<(f64, f64, f64, f64)> {
        let mut bb: Option<(f64, f64, f64, f64)> = None;
        for (pts, _, _) in self.flatten(samples, &XForm::IDENTITY) {
            for (x, y) in pts {
                bb = Some(match bb {
                    None => (x, y, x, y),
                    Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
                });
            }
        }
        bb
    }
}

fn cubic(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64), p3: (f64, f64), t: f64) -> (f64, f64) {
    let u = 1.0 - t;
    let w0 = u * u * u;
    let w1 = 3.0 * u * u * t;
    let w2 = 3.0 * u * t * t;
    let w3 = t * t * t;
    (
        w0 * p0.0 + w1 * p1.0 + w2 * p2.0 + w3 * p3.0,
        w0 * p0.1 + w1 * p1.1 + w2 * p2.1 + w3 * p3.1,
    )
}

/// LightBurn-flavored layer palette: color is implied by the cut-layer index,
/// cycling through a fixed set (black, blue, red, green, …).
pub const LAYER_PALETTE: [(u8, u8, u8); 10] = [
    (0, 0, 0),
    (0, 0, 255),
    (255, 0, 0),
    (0, 160, 0),
    (200, 120, 0),
    (160, 0, 200),
    (0, 170, 200),
    (200, 0, 120),
    (120, 120, 0),
    (90, 90, 90),
];

pub fn layer_rgb(index: u32) -> (u8, u8, u8) {
    LAYER_PALETTE[index as usize % LAYER_PALETTE.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xform_string_roundtrip() {
        let x = XForm::translate(50.0, 25.0);
        let v = serde_json::to_value(x).unwrap();
        assert_eq!(v, serde_json::json!("1 0 0 1 50 25"));
        let back: XForm = serde_json::from_value(v).unwrap();
        assert_eq!(x, back);
    }

    #[test]
    fn rect_serializes_like_lightburn() {
        let s = Shape::rect(2, 10.0, 20.0, 30.0, 40.0);
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["Type"], "Rect");
        assert_eq!(v["CutIndex"], 2);
        assert_eq!(v["W"], 30.0);
        assert_eq!(v["XForm"], "1 0 0 1 10 20");
        let back: Shape = serde_json::from_value(v).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn group_serializes_like_lightburn_and_flattens() {
        let group = Shape {
            cut_index: 0,
            xform: XForm::translate(5.0, 5.0),
            kind: ShapeKind::Group {
                children: vec![Shape::rect(0, 0.0, 0.0, 10.0, 10.0)],
            },
        };
        let v = serde_json::to_value(&group).unwrap();
        assert_eq!(v["Type"], "Group");
        assert_eq!(v["Children"][0]["Type"], "Rect");
        let back: Shape = serde_json::from_value(v).unwrap();
        assert_eq!(group, back);
        // The child rect, placed at (0,0) then offset by the group's (5,5).
        let bb = group.world_bbox(16).unwrap();
        assert!((bb.0 - 5.0).abs() < 1e-9 && (bb.2 - 15.0).abs() < 1e-9);
    }

    #[test]
    fn scale_about_center_doubles_size() {
        let r = Shape::rect(0, 0.0, 0.0, 10.0, 10.0);
        let (minx, miny, maxx, maxy) = r.world_bbox(16).unwrap();
        let (cx, cy) = ((minx + maxx) / 2.0, (miny + maxy) / 2.0);
        let mut scaled = r.clone();
        scaled.xform = XForm::about(XForm::scale(2.0, 2.0), cx, cy).compose(&scaled.xform);
        let bb = scaled.world_bbox(16).unwrap();
        assert!((bb.2 - bb.0 - 20.0).abs() < 1e-9); // width doubled
        // Center preserved.
        assert!(((bb.0 + bb.2) / 2.0 - cx).abs() < 1e-9);
    }

    #[test]
    fn polyline_builds_line_prims_and_outlines() {
        let s = Shape::polyline(0, &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], true);
        if let ShapeKind::Path { prims, .. } = &s.kind {
            assert_eq!(prims.len(), 3); // 2 segments + closing segment
        } else {
            panic!("expected path");
        }
        let (pts, closed) = s.outline(8);
        assert!(closed);
        assert_eq!(pts.first().copied(), Some((0.0, 0.0)));
    }
}
