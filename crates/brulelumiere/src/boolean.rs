//! Boolean operations on closed shapes (union / intersection / difference),
//! as in LightBurn's boolean tools. Shapes are flattened to world polygons,
//! combined with `geo`'s robust clipper, and the resulting rings are returned
//! as new closed paths.

use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon};

use crate::shapes::{Shape, XForm};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoolOp {
    Union,
    Intersection,
    Difference,
}

impl BoolOp {
    pub fn label(self) -> &'static str {
        match self {
            BoolOp::Union => "Union",
            BoolOp::Intersection => "Intersection",
            BoolOp::Difference => "Différence",
        }
    }
}

/// Convert a shape's first closed world outline into a `geo` polygon.
fn to_polygon(shape: &Shape) -> Option<Polygon<f64>> {
    let flat = shape.flatten(96, &XForm::IDENTITY);
    let (pts, closed, _) = flat.into_iter().next()?;
    if !closed || pts.len() < 3 {
        return None;
    }
    let ring: Vec<Coord<f64>> = pts.into_iter().map(|(x, y)| Coord { x, y }).collect();
    Some(Polygon::new(LineString::new(ring), vec![]))
}

/// Convert a resulting `geo` multipolygon back into closed path shapes (one per
/// ring — exteriors and holes alike, which is what a laser cuts).
fn from_multipolygon(mp: &MultiPolygon<f64>, cut_index: u32) -> Vec<Shape> {
    let mut shapes = Vec::new();
    for poly in mp {
        let mut rings = vec![poly.exterior()];
        rings.extend(poly.interiors());
        for ring in rings {
            let pts: Vec<(f64, f64)> = ring.coords().map(|c| (c.x, c.y)).collect();
            // geo closes rings (last == first); drop the duplicate.
            let pts = if pts.len() > 1 && pts.first() == pts.last() {
                pts[..pts.len() - 1].to_vec()
            } else {
                pts
            };
            if pts.len() >= 3 {
                shapes.push(Shape::polyline(cut_index, &pts, true));
            }
        }
    }
    shapes
}

/// Apply `op` to two shapes, returning the resulting shapes (empty if either
/// shape is not a usable closed polygon).
pub fn apply(a: &Shape, b: &Shape, op: BoolOp, cut_index: u32) -> Vec<Shape> {
    let (Some(pa), Some(pb)) = (to_polygon(a), to_polygon(b)) else {
        return Vec::new();
    };
    let result: MultiPolygon<f64> = match op {
        BoolOp::Union => pa.union(&pb),
        BoolOp::Intersection => pa.intersection(&pb),
        BoolOp::Difference => pa.difference(&pb),
    };
    from_multipolygon(&result, cut_index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_of_overlapping_squares_is_one_polygon() {
        let a = Shape::rect(0, 0.0, 0.0, 10.0, 10.0);
        let b = Shape::rect(0, 5.0, 5.0, 10.0, 10.0);
        let out = apply(&a, &b, BoolOp::Union, 0);
        assert_eq!(out.len(), 1, "overlapping union should merge into one ring");
    }

    #[test]
    fn intersection_of_overlapping_squares_is_the_overlap() {
        let a = Shape::rect(0, 0.0, 0.0, 10.0, 10.0);
        let b = Shape::rect(0, 5.0, 5.0, 10.0, 10.0);
        let out = apply(&a, &b, BoolOp::Intersection, 0);
        assert_eq!(out.len(), 1);
        let bb = out[0].world_bbox(16).unwrap();
        // Overlap is the 5x5 square from (5,5) to (10,10).
        assert!((bb.0 - 5.0).abs() < 1e-6 && (bb.2 - 10.0).abs() < 1e-6);
    }

    #[test]
    fn difference_removes_overlap() {
        let a = Shape::rect(0, 0.0, 0.0, 10.0, 10.0);
        let b = Shape::rect(0, 5.0, 0.0, 10.0, 10.0);
        let out = apply(&a, &b, BoolOp::Difference, 0);
        assert_eq!(out.len(), 1);
        let bb = out[0].world_bbox(16).unwrap();
        // a minus b leaves the left half: x in [0,5].
        assert!((bb.2 - 5.0).abs() < 1e-6);
    }
}
