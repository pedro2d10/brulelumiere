//! Minimal built-in vector (stroke) font, so text can be turned into vector
//! paths — LightBurn's `Type="Text"` shapes are vectorized to geometry too.
//!
//! Glyphs are defined as polyline strokes on a grid 4 wide × 6 tall with the
//! origin at the bottom-left (y up). [`vectorize`] scales them to a cap height
//! of `size` mm, lays characters left to right, and returns world polylines
//! (y down, to match the canvas).

/// Grid dimensions the glyph strokes are authored on.
const GW: f64 = 4.0;
const GH: f64 = 6.0;

/// Strokes for one glyph: each inner vec is a connected polyline of grid points.
fn glyph(c: char) -> Vec<Vec<(f64, f64)>> {
    // Helper shorthands for grid points.
    match c.to_ascii_uppercase() {
        'A' => vec![vec![(0, 0), (0, 4), (2, 6), (4, 4), (4, 0)], vec![(0, 2), (4, 2)]],
        'B' => vec![vec![(0, 0), (0, 6), (3, 6), (4, 5), (3, 3), (0, 3)], vec![(3, 3), (4, 1), (3, 0), (0, 0)]],
        'C' => vec![vec![(4, 5), (3, 6), (1, 6), (0, 5), (0, 1), (1, 0), (3, 0), (4, 1)]],
        'D' => vec![vec![(0, 0), (0, 6), (3, 6), (4, 5), (4, 1), (3, 0), (0, 0)]],
        'E' => vec![vec![(4, 6), (0, 6), (0, 0), (4, 0)], vec![(0, 3), (3, 3)]],
        'F' => vec![vec![(4, 6), (0, 6), (0, 0)], vec![(0, 3), (3, 3)]],
        'G' => vec![vec![(4, 5), (3, 6), (1, 6), (0, 5), (0, 1), (1, 0), (3, 0), (4, 1), (4, 3), (2, 3)]],
        'H' => vec![vec![(0, 0), (0, 6)], vec![(4, 0), (4, 6)], vec![(0, 3), (4, 3)]],
        'I' => vec![vec![(1, 6), (3, 6)], vec![(2, 6), (2, 0)], vec![(1, 0), (3, 0)]],
        'J' => vec![vec![(4, 6), (4, 1), (3, 0), (1, 0), (0, 1)]],
        'K' => vec![vec![(0, 0), (0, 6)], vec![(4, 6), (0, 3), (4, 0)]],
        'L' => vec![vec![(0, 6), (0, 0), (4, 0)]],
        'M' => vec![vec![(0, 0), (0, 6), (2, 3), (4, 6), (4, 0)]],
        'N' => vec![vec![(0, 0), (0, 6), (4, 0), (4, 6)]],
        'O' => vec![vec![(1, 0), (0, 1), (0, 5), (1, 6), (3, 6), (4, 5), (4, 1), (3, 0), (1, 0)]],
        'P' => vec![vec![(0, 0), (0, 6), (3, 6), (4, 5), (3, 3), (0, 3)]],
        'Q' => vec![vec![(1, 0), (0, 1), (0, 5), (1, 6), (3, 6), (4, 5), (4, 1), (3, 0), (1, 0)], vec![(2, 2), (4, 0)]],
        'R' => vec![vec![(0, 0), (0, 6), (3, 6), (4, 5), (3, 3), (0, 3)], vec![(2, 3), (4, 0)]],
        'S' => vec![vec![(4, 5), (3, 6), (1, 6), (0, 5), (1, 3), (3, 3), (4, 1), (3, 0), (1, 0), (0, 1)]],
        'T' => vec![vec![(0, 6), (4, 6)], vec![(2, 6), (2, 0)]],
        'U' => vec![vec![(0, 6), (0, 1), (1, 0), (3, 0), (4, 1), (4, 6)]],
        'V' => vec![vec![(0, 6), (2, 0), (4, 6)]],
        'W' => vec![vec![(0, 6), (1, 0), (2, 3), (3, 0), (4, 6)]],
        'X' => vec![vec![(0, 0), (4, 6)], vec![(0, 6), (4, 0)]],
        'Y' => vec![vec![(0, 6), (2, 3), (4, 6)], vec![(2, 3), (2, 0)]],
        'Z' => vec![vec![(0, 6), (4, 6), (0, 0), (4, 0)]],
        '0' => vec![vec![(1, 0), (0, 1), (0, 5), (1, 6), (3, 6), (4, 5), (4, 1), (3, 0), (1, 0)], vec![(0, 0), (4, 6)]],
        '1' => vec![vec![(1, 5), (2, 6), (2, 0)], vec![(1, 0), (3, 0)]],
        '2' => vec![vec![(0, 5), (1, 6), (3, 6), (4, 5), (4, 4), (0, 0), (4, 0)]],
        '3' => vec![vec![(0, 6), (4, 6), (2, 3), (4, 2), (4, 1), (3, 0), (1, 0), (0, 1)]],
        '4' => vec![vec![(3, 0), (3, 6), (0, 2), (4, 2)]],
        '5' => vec![vec![(4, 6), (0, 6), (0, 3), (3, 3), (4, 2), (4, 1), (3, 0), (1, 0), (0, 1)]],
        '6' => vec![vec![(4, 5), (3, 6), (1, 6), (0, 5), (0, 1), (1, 0), (3, 0), (4, 1), (4, 2), (3, 3), (0, 3)]],
        '7' => vec![vec![(0, 6), (4, 6), (1, 0)]],
        '8' => vec![vec![(1, 3), (0, 4), (0, 5), (1, 6), (3, 6), (4, 5), (4, 4), (3, 3), (1, 3), (0, 2), (0, 1), (1, 0), (3, 0), (4, 1), (4, 2), (3, 3)]],
        '9' => vec![vec![(0, 1), (1, 0), (3, 0), (4, 1), (4, 5), (3, 6), (1, 6), (0, 5), (0, 4), (1, 3), (4, 3)]],
        '-' => vec![vec![(1, 3), (3, 3)]],
        '.' => vec![vec![(2, 0), (2, 1)]],
        ' ' => vec![],
        _ => vec![vec![(0, 0), (4, 0), (4, 6), (0, 6), (0, 0)]], // tofu box for unknown
    }
    .into_iter()
    .map(|stroke| stroke.into_iter().map(|(x, y)| (x as f64, y as f64)).collect())
    .collect()
}

/// Vectorize `text` at cap height `size` (mm), returning one polyline per
/// stroke, positioned with the first character's baseline-left at the origin.
pub fn vectorize(text: &str, size: f64) -> Vec<Vec<(f64, f64)>> {
    let sx = size / GH; // keep aspect: scale both axes by cap-height factor
    let advance = (GW + 1.0) * sx; // glyph width + 1 grid unit of spacing
    let mut out = Vec::new();
    let mut pen_x = 0.0;
    for ch in text.chars() {
        for stroke in glyph(ch) {
            let poly: Vec<(f64, f64)> = stroke
                .iter()
                .map(|&(gx, gy)| (pen_x + gx * sx, (GH - gy) * sx)) // flip y (canvas y-down)
                .collect();
            if poly.len() >= 2 {
                out.push(poly);
            }
        }
        pen_x += advance;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectorizes_letters_into_strokes() {
        let strokes = vectorize("AB", 10.0);
        assert!(!strokes.is_empty());
        // Every point stays within the expected height band [0, size].
        for s in &strokes {
            for &(_, y) in s {
                assert!((0.0..=10.0001).contains(&y));
            }
        }
    }

    #[test]
    fn space_advances_without_strokes() {
        // "A A" has strokes only from the two A's, none from the space.
        let one = vectorize("AA", 10.0).len();
        let two = vectorize("A A", 10.0).len();
        assert_eq!(one, two);
        // But the second A is further right when a space precedes it.
        let a_a = vectorize("A A", 10.0);
        let max_x = a_a.iter().flatten().map(|&(x, _)| x).fold(0.0_f64, f64::max);
        assert!(max_x > 10.0);
    }
}
