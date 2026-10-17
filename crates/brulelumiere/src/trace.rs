//! Image vectorization — the engine behind LightBurn's "trace image" / camera
//! capture: load a raster, threshold it to ink/background, extract the ink
//! region boundaries as closed contours (marching squares), stitch the cell
//! segments into loops, scale to millimetres and simplify.
//!
//! The result is a set of closed polylines in model mm, ready to drop onto the
//! canvas as cut paths.

use std::collections::HashMap;

/// Options controlling the trace.
#[derive(Clone, Copy)]
pub struct TraceOptions {
    /// Luma threshold 0–255: pixels darker than this are "ink".
    pub threshold: u8,
    /// Treat light pixels as ink instead (for white-on-dark art).
    pub invert: bool,
    /// Width the traced result should span on the canvas, in mm.
    pub target_width_mm: f64,
    /// Douglas–Peucker tolerance in mm (0 = keep every point).
    pub simplify_mm: f64,
}

impl Default for TraceOptions {
    fn default() -> Self {
        Self { threshold: 128, invert: false, target_width_mm: 80.0, simplify_mm: 0.3 }
    }
}

/// Decode `path`, trace it, and return closed loops in model mm.
pub fn trace_file(path: &str, opts: &TraceOptions) -> Result<Vec<Vec<(f64, f64)>>, String> {
    let img = image::open(path).map_err(|e| format!("Image illisible : {e}"))?;
    let gray = img.to_luma8();
    let (w, h) = (gray.width() as usize, gray.height() as usize);
    if w < 2 || h < 2 {
        return Err("Image trop petite".into());
    }
    let ink: Vec<bool> = gray
        .pixels()
        .map(|p| {
            let dark = p.0[0] < opts.threshold;
            if opts.invert {
                !dark
            } else {
                dark
            }
        })
        .collect();
    Ok(trace_bitmap(w, h, &ink, opts))
}

/// Trace a boolean ink bitmap (row-major, `w*h`) into closed loops in mm.
pub fn trace_bitmap(w: usize, h: usize, ink: &[bool], opts: &TraceOptions) -> Vec<Vec<(f64, f64)>> {
    // Marching squares over the cell grid. A cell's four corners sample the
    // ink field (with an implicit `false` border so edge blobs close cleanly).
    let sample = |x: i64, y: i64| -> bool {
        if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
            false
        } else {
            ink[y as usize * w + x as usize]
        }
    };

    // Segments between edge-midpoints, keyed by quantized endpoints so adjacent
    // cells share exact vertices. Coordinates are multiples of 0.5 → key = *2.
    let key = |p: (f64, f64)| -> (i64, i64) { ((p.0 * 2.0).round() as i64, (p.1 * 2.0).round() as i64) };
    let mut adj: HashMap<(i64, i64), Vec<(i64, i64)>> = HashMap::new();
    let mut coord: HashMap<(i64, i64), (f64, f64)> = HashMap::new();
    let mut connect = |a: (f64, f64), b: (f64, f64)| {
        let (ka, kb) = (key(a), key(b));
        coord.insert(ka, a);
        coord.insert(kb, b);
        adj.entry(ka).or_default().push(kb);
        adj.entry(kb).or_default().push(ka);
    };

    // Iterate cells; corners at (x,y). Include a one-cell border so shapes that
    // touch the image edge still produce closed loops.
    for y in -1..h as i64 {
        for x in -1..w as i64 {
            let tl = sample(x, y);
            let tr = sample(x + 1, y);
            let br = sample(x + 1, y + 1);
            let bl = sample(x, y + 1);
            let case = (tl as u8) << 3 | (tr as u8) << 2 | (br as u8) << 1 | (bl as u8);

            let fx = x as f64;
            let fy = y as f64;
            let top = (fx + 0.5, fy);
            let right = (fx + 1.0, fy + 0.5);
            let bottom = (fx + 0.5, fy + 1.0);
            let left = (fx, fy + 0.5);

            // Standard marching-squares segment table (saddles 5/10 resolved one way).
            match case {
                1 | 14 => connect(left, bottom),
                2 | 13 => connect(bottom, right),
                3 | 12 => connect(left, right),
                4 | 11 => connect(top, right),
                6 | 9 => connect(top, bottom),
                7 | 8 => connect(top, left),
                5 => {
                    connect(top, left);
                    connect(bottom, right);
                }
                10 => {
                    connect(top, right);
                    connect(left, bottom);
                }
                _ => {} // 0 and 15: no boundary
            }
        }
    }

    // Stitch segments into loops by walking the adjacency graph.
    let mut loops: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut used: HashMap<((i64, i64), (i64, i64)), bool> = HashMap::new();
    let edge_key = |a: (i64, i64), b: (i64, i64)| if a <= b { (a, b) } else { (b, a) };

    let keys: Vec<(i64, i64)> = adj.keys().copied().collect();
    for &start in &keys {
        loop {
            // Find an unused edge leaving `start`.
            let next = adj.get(&start).and_then(|ns| {
                ns.iter().copied().find(|&n| !*used.get(&edge_key(start, n)).unwrap_or(&false))
            });
            let Some(mut prev) = Some(start) else { break };
            let Some(mut cur) = next else { break };
            used.insert(edge_key(prev, cur), true);

            let mut path = vec![coord[&prev], coord[&cur]];
            while cur != start {
                let step = adj.get(&cur).and_then(|ns| {
                    ns.iter()
                        .copied()
                        .find(|&n| n != prev && !*used.get(&edge_key(cur, n)).unwrap_or(&false))
                });
                match step {
                    Some(n) => {
                        used.insert(edge_key(cur, n), true);
                        prev = cur;
                        cur = n;
                        path.push(coord[&cur]);
                    }
                    None => break,
                }
            }
            if path.len() >= 4 {
                // Drop the duplicated closing point if present.
                if path.first() == path.last() {
                    path.pop();
                }
                loops.push(path);
            }
        }
    }

    // Scale pixel space → mm so the whole trace spans `target_width_mm`.
    let scale = if w > 1 { opts.target_width_mm / w as f64 } else { 1.0 };
    loops
        .into_iter()
        .map(|lp| {
            let scaled: Vec<(f64, f64)> = lp.iter().map(|&(x, y)| (x * scale, y * scale)).collect();
            if opts.simplify_mm > 0.0 {
                simplify(&scaled, opts.simplify_mm)
            } else {
                scaled
            }
        })
        .filter(|lp| lp.len() >= 3)
        .collect()
}

/// Ramer–Douglas–Peucker simplification of a closed loop.
fn simplify(pts: &[(f64, f64)], tol: f64) -> Vec<(f64, f64)> {
    if pts.len() < 4 {
        return pts.to_vec();
    }
    // Treat the loop as open between its two farthest-apart points to avoid
    // collapsing the whole ring; simplest robust choice is index 0 and len/2.
    let n = pts.len();
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n / 2] = true;
    rdp(pts, 0, n / 2, tol, &mut keep);
    rdp(pts, n / 2, n - 1, tol, &mut keep);
    keep[n - 1] = true;
    let out: Vec<(f64, f64)> = (0..n).filter(|&i| keep[i]).map(|i| pts[i]).collect();
    out
}

fn rdp(pts: &[(f64, f64)], first: usize, last: usize, tol: f64, keep: &mut [bool]) {
    if last <= first + 1 {
        return;
    }
    let (a, b) = (pts[first], pts[last]);
    let mut max_d = 0.0;
    let mut idx = first;
    for i in (first + 1)..last {
        let d = perp_dist(pts[i], a, b);
        if d > max_d {
            max_d = d;
            idx = i;
        }
    }
    if max_d > tol {
        keep[idx] = true;
        rdp(pts, first, idx, tol, keep);
        rdp(pts, idx, last, tol, keep);
    }
}

fn perp_dist(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-9 {
        let (ex, ey) = (p.0 - a.0, p.1 - a.1);
        return (ex * ex + ey * ey).sqrt();
    }
    ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traces_a_solid_square_into_one_loop() {
        // 10x10 bitmap with a 6x6 filled square in the middle.
        let (w, h) = (10, 10);
        let mut ink = vec![false; w * h];
        for y in 2..8 {
            for x in 2..8 {
                ink[y * w + x] = true;
            }
        }
        let opts = TraceOptions { target_width_mm: 10.0, simplify_mm: 0.0, ..Default::default() };
        let loops = trace_bitmap(w, h, &ink, &opts);
        assert_eq!(loops.len(), 1, "a single blob should trace to one loop");
        // The loop should enclose the square's boundary (perimeter ~ 24 cell edges).
        assert!(loops[0].len() >= 4);
    }

    #[test]
    fn empty_bitmap_traces_nothing() {
        let loops = trace_bitmap(8, 8, &vec![false; 64], &TraceOptions::default());
        assert!(loops.is_empty());
    }

    #[test]
    fn two_blobs_trace_two_loops() {
        let (w, h) = (12, 6);
        let mut ink = vec![false; w * h];
        for y in 2..4 {
            for x in 1..3 {
                ink[y * w + x] = true;
            }
            for x in 9..11 {
                ink[y * w + x] = true;
            }
        }
        let loops = trace_bitmap(w, h, &ink, &TraceOptions { simplify_mm: 0.0, ..Default::default() });
        assert_eq!(loops.len(), 2);
    }

    #[test]
    fn trace_file_reads_and_vectorizes_a_png() {
        // Write a real PNG (white field, black square) and trace it end-to-end.
        let path = std::env::temp_dir().join(format!(
            "rea-trace-{}-{}.png",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let mut img = image::GrayImage::from_pixel(20, 20, image::Luma([255u8]));
        for y in 5..15 {
            for x in 5..15 {
                img.put_pixel(x, y, image::Luma([0u8]));
            }
        }
        img.save(&path).expect("save PNG");
        let opts = TraceOptions { threshold: 128, invert: false, target_width_mm: 20.0, simplify_mm: 0.0 };
        let loops = trace_file(path.to_str().unwrap(), &opts).expect("trace");
        std::fs::remove_file(&path).ok();
        assert_eq!(loops.len(), 1, "the black square should trace to one loop");
        // Traced at 20 mm wide over 20 px → ~1 mm/px, square spans roughly 5..15 mm.
        let minx = loops[0].iter().map(|p| p.0).fold(f64::MAX, f64::min);
        assert!(minx >= 4.0 && minx <= 6.0, "left edge near 5 mm, got {minx}");
    }

    #[test]
    fn simplify_reduces_collinear_points() {
        // A straight run of points should collapse to its endpoints.
        let line: Vec<(f64, f64)> = (0..=10).map(|i| (i as f64, 0.0)).collect();
        let out = simplify(&line, 0.1);
        assert!(out.len() < line.len());
    }
}
