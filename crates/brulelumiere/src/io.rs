//! Import/export, modeled on LightBurn's file I/O.
//!
//! - **`.lbrn2`** — read/write LightBurn's XML: `<LightBurnProject
//!   AppVersion="1.5.00" FormatVersion="1" …>`, a `<CutSetting>` per cut layer,
//!   and `<Shape>` elements carrying `<XForm>` + the packed `<VertList>` /
//!   `<PrimList>` encoding (`V x y[c0x..c0y..c1x..c1y..]`, `L a b` / `B a b`).
//!   Rect/Ellipse are written as paths so the geometry re-imports cleanly.
//! - **SVG** — import `rect`, `circle`, `ellipse`, `line`, `polyline`,
//!   `polygon`, and `path` (M/L/H/V/C/Z, absolute + relative).
//! - **DXF** — import `LINE`, `LWPOLYLINE`, `CIRCLE` (the entities LightBurn's
//!   `LWPOLYLINE` / `NURBS to Bezier` importer centers on).

use crate::model::{CutLayer, LayerType, Project};
use crate::shapes::{Prim, Shape, ShapeKind, Vertex, XForm};

const SAMPLES: usize = 64;

// ======================================================================
// .lbrn2 export
// ======================================================================

/// Serialize a project to LightBurn `.lbrn2` XML.
pub fn to_lbrn2(project: &Project) -> String {
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(
        "<LightBurnProject AppVersion=\"1.5.00\" FormatVersion=\"1\" MaterialHeight=\"0\" \
         MirrorX=\"False\" MirrorY=\"False\">\n",
    );

    for layer in &project.layers {
        out.push_str(&format!(
            "  <CutSetting type=\"{}\">\n    <index Value=\"{}\"/>\n    <name Value=\"{}\"/>\n    \
             <maxPower Value=\"{}\"/>\n    <minPower Value=\"{}\"/>\n    <speed Value=\"{}\"/>\n    \
             <numPasses Value=\"{}\"/>\n  </CutSetting>\n",
            layer.kind.label(),
            layer.index,
            xml_escape(&layer.name),
            layer.max_power,
            layer.min_power,
            layer.speed,
            layer.passes,
        ));
    }

    for shape in &project.shapes {
        write_shape(&mut out, shape, 1);
    }

    out.push_str("</LightBurnProject>\n");
    out
}

fn write_shape(out: &mut String, shape: &Shape, depth: usize) {
    let pad = "  ".repeat(depth);
    match &shape.kind {
        ShapeKind::Group { children } => {
            out.push_str(&format!(
                "{pad}<Shape Type=\"Group\" CutIndex=\"{}\">\n",
                shape.cut_index
            ));
            out.push_str(&format!("{pad}  <XForm>{}</XForm>\n", fmt_xform(&shape.xform)));
            for c in children {
                write_shape(out, c, depth + 1);
            }
            out.push_str(&format!("{pad}</Shape>\n"));
        }
        _ => {
            let Some((xform, verts, prims, _closed)) = shape.as_path_parts(SAMPLES) else {
                return;
            };
            out.push_str(&format!(
                "{pad}<Shape Type=\"Path\" CutIndex=\"{}\">\n",
                shape.cut_index
            ));
            out.push_str(&format!("{pad}  <XForm>{}</XForm>\n", fmt_xform(&xform)));
            out.push_str(&format!("{pad}  <VertList>{}</VertList>\n", fmt_vertlist(&verts)));
            out.push_str(&format!("{pad}  <PrimList>{}</PrimList>\n", fmt_primlist(&prims)));
            out.push_str(&format!("{pad}</Shape>\n"));
        }
    }
}

fn fmt_xform(x: &XForm) -> String {
    format!("{} {} {} {} {} {}", x.a, x.b, x.c, x.d, x.e, x.f)
}

fn fmt_vertlist(verts: &[Vertex]) -> String {
    let mut s = String::new();
    for v in verts {
        s.push_str(&format!("V{} {}", num(v.x), num(v.y)));
        if let Some((cx, cy)) = v.c0 {
            s.push_str(&format!("c0x{}c0y{}", num(cx), num(cy)));
        }
        if let Some((cx, cy)) = v.c1 {
            s.push_str(&format!("c1x{}c1y{}", num(cx), num(cy)));
        }
    }
    s
}

fn fmt_primlist(prims: &[Prim]) -> String {
    let mut s = String::new();
    for p in prims {
        match *p {
            Prim::Line { a, b } => s.push_str(&format!("L{a} {b}")),
            Prim::Bezier { a, b } => s.push_str(&format!("B{a} {b}")),
        }
    }
    s
}

fn num(v: f64) -> String {
    // Trim trailing zeros for compactness.
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-" { "0".to_string() } else { s.to_string() }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ======================================================================
// .lbrn2 import
// ======================================================================

pub fn from_lbrn2(xml: &str) -> Result<Project, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("XML invalide : {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "LightBurnProject" {
        return Err("racine <LightBurnProject> absente".into());
    }

    let mut project = Project { layers: Vec::new(), devices: Vec::new(), shapes: Vec::new(), ..Default::default() };

    for node in root.children().filter(|n| n.is_element()) {
        match node.tag_name().name() {
            "CutSetting" => {
                let kind = match node.attribute("type") {
                    Some("Scan") => LayerType::Scan,
                    Some("Image") => LayerType::Image,
                    Some("Tool") => LayerType::Tool,
                    _ => LayerType::Cut,
                };
                let child_val = |name: &str| -> Option<String> {
                    node.children()
                        .find(|c| c.has_tag_name(name))
                        .and_then(|c| c.attribute("Value").map(|s| s.to_string()))
                };
                let index = child_val("index").and_then(|s| s.parse().ok()).unwrap_or(project.layers.len() as u32);
                let name = child_val("name").unwrap_or_else(|| format!("C{index:02}"));
                let speed = child_val("speed").and_then(|s| s.parse().ok()).unwrap_or(100.0);
                let max_power = child_val("maxPower").and_then(|s| s.parse().ok()).unwrap_or(20.0);
                let min_power = child_val("minPower").and_then(|s| s.parse().ok()).unwrap_or(10.0);
                let passes = child_val("numPasses").and_then(|s| s.parse().ok()).unwrap_or(1);
                project.layers.push(CutLayer { index, name, kind, speed, max_power, min_power, passes });
            }
            "Shape" => {
                if let Some(shape) = parse_shape(node) {
                    project.shapes.push(shape);
                }
            }
            _ => {}
        }
    }

    if project.layers.is_empty() {
        project.layers = Project::default().layers;
    }
    Ok(project)
}

fn parse_shape(node: roxmltree::Node) -> Option<Shape> {
    let cut_index = node.attribute("CutIndex").and_then(|s| s.parse().ok()).unwrap_or(0);
    let xform = node
        .children()
        .find(|c| c.has_tag_name("XForm"))
        .and_then(|c| c.text())
        .map(parse_xform)
        .unwrap_or(XForm::IDENTITY);

    match node.attribute("Type") {
        Some("Group") => {
            let children: Vec<Shape> = node
                .children()
                .filter(|c| c.has_tag_name("Shape"))
                .filter_map(parse_shape)
                .collect();
            Some(Shape { cut_index, xform, kind: ShapeKind::Group { children } })
        }
        _ => {
            let verts = node
                .children()
                .find(|c| c.has_tag_name("VertList"))
                .and_then(|c| c.text())
                .map(parse_vertlist)
                .unwrap_or_default();
            let prims = node
                .children()
                .find(|c| c.has_tag_name("PrimList"))
                .and_then(|c| c.text())
                .map(parse_primlist)
                .unwrap_or_default();
            let closed = prims.iter().any(|p| matches!(p, Prim::Line { b, .. } | Prim::Bezier { b, .. } if *b == 0));
            Some(Shape { cut_index, xform, kind: ShapeKind::Path { verts, prims, closed } })
        }
    }
}

fn parse_xform(s: &str) -> XForm {
    let n: Vec<f64> = s.split_whitespace().filter_map(|t| t.parse().ok()).collect();
    if n.len() == 6 {
        XForm { a: n[0], b: n[1], c: n[2], d: n[3], e: n[4], f: n[5] }
    } else {
        XForm::IDENTITY
    }
}

/// Parse LightBurn's packed VertList, e.g. `V50 100c0x1c1x1V50 0...`.
fn parse_vertlist(s: &str) -> Vec<Vertex> {
    let mut verts = Vec::new();
    // Each vertex record starts at a 'V'.
    for rec in s.split('V').filter(|r| !r.is_empty()) {
        // "x y[c0x..c0y..c1x..c1y..]"
        let marker = rec.find(|c: char| c == 'c').unwrap_or(rec.len());
        let (xy, handles) = rec.split_at(marker);
        let mut it = xy.split_whitespace();
        let (Some(x), Some(y)) = (it.next().and_then(|t| t.parse().ok()), it.next().and_then(|t| t.parse().ok())) else {
            continue;
        };
        let c0x = extract(handles, "c0x");
        let c0y = extract(handles, "c0y");
        let c1x = extract(handles, "c1x");
        let c1y = extract(handles, "c1y");
        verts.push(Vertex {
            x,
            y,
            c0: match (c0x, c0y) {
                (Some(a), Some(b)) => Some((a, b)),
                _ => None,
            },
            c1: match (c1x, c1y) {
                (Some(a), Some(b)) => Some((a, b)),
                _ => None,
            },
        });
    }
    verts
}

/// Read the number following `key` in `s` (e.g. the value after `c0x`).
fn extract(s: &str, key: &str) -> Option<f64> {
    let start = s.find(key)? + key.len();
    let rest = &s[start..];
    let end = rest.find(|c: char| c == 'c').unwrap_or(rest.len());
    rest[..end].trim().parse().ok()
}

/// Parse LightBurn's packed PrimList, e.g. `L0 1B1 2`.
fn parse_primlist(s: &str) -> Vec<Prim> {
    let mut prims = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let kind = bytes[i] as char;
        if kind != 'L' && kind != 'B' {
            i += 1;
            continue;
        }
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != b'L' && bytes[i] != b'B' {
            i += 1;
        }
        let token = &s[start..i];
        let mut it = token.split_whitespace();
        if let (Some(a), Some(b)) =
            (it.next().and_then(|t| t.parse().ok()), it.next().and_then(|t| t.parse().ok()))
        {
            prims.push(if kind == 'L' { Prim::Line { a, b } } else { Prim::Bezier { a, b } });
        }
    }
    prims
}

// ======================================================================
// SVG import
// ======================================================================

pub fn import_svg(svg: &str) -> Result<Vec<Shape>, String> {
    let doc = roxmltree::Document::parse(svg).map_err(|e| format!("SVG invalide : {e}"))?;
    let mut shapes = Vec::new();
    for n in doc.descendants().filter(|n| n.is_element()) {
        let f = |name: &str| n.attribute(name).and_then(|s| s.parse::<f64>().ok());
        match n.tag_name().name() {
            "rect" => {
                if let (Some(x), Some(y), Some(w), Some(h)) = (f("x").or(Some(0.0)), f("y").or(Some(0.0)), f("width"), f("height")) {
                    shapes.push(Shape::rect(0, x, y, w, h));
                }
            }
            "circle" => {
                if let (Some(cx), Some(cy), Some(r)) = (f("cx"), f("cy"), f("r")) {
                    shapes.push(Shape::ellipse(0, cx, cy, r, r));
                }
            }
            "ellipse" => {
                if let (Some(cx), Some(cy), Some(rx), Some(ry)) = (f("cx"), f("cy"), f("rx"), f("ry")) {
                    shapes.push(Shape::ellipse(0, cx, cy, rx, ry));
                }
            }
            "line" => {
                if let (Some(x1), Some(y1), Some(x2), Some(y2)) = (f("x1"), f("y1"), f("x2"), f("y2")) {
                    shapes.push(Shape::polyline(0, &[(x1, y1), (x2, y2)], false));
                }
            }
            "polyline" | "polygon" => {
                if let Some(points) = n.attribute("points") {
                    let pts = parse_points(points);
                    if pts.len() >= 2 {
                        shapes.push(Shape::polyline(0, &pts, n.has_tag_name("polygon")));
                    }
                }
            }
            "path" => {
                if let Some(d) = n.attribute("d") {
                    shapes.extend(parse_svg_path(d));
                }
            }
            _ => {}
        }
    }
    if shapes.is_empty() {
        return Err("aucune forme reconnue dans le SVG".into());
    }
    Ok(shapes)
}

fn parse_points(s: &str) -> Vec<(f64, f64)> {
    let nums: Vec<f64> = s
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    nums.chunks_exact(2).map(|c| (c[0], c[1])).collect()
}

/// Parse an SVG path `d`, supporting M/m L/l H/h V/v C/c Z/z.
fn parse_svg_path(d: &str) -> Vec<Shape> {
    let tokens = tokenize_path(d);
    let mut shapes = Vec::new();
    let mut verts: Vec<Vertex> = Vec::new();
    let mut prims: Vec<Prim> = Vec::new();
    let mut cur = (0.0, 0.0);
    let mut start = (0.0, 0.0);
    let mut closed = false;
    let mut i = 0;

    let flush = |verts: &mut Vec<Vertex>, prims: &mut Vec<Prim>, closed: &mut bool, shapes: &mut Vec<Shape>| {
        if verts.len() >= 2 {
            shapes.push(Shape {
                cut_index: 0,
                xform: XForm::IDENTITY,
                kind: ShapeKind::Path { verts: std::mem::take(verts), prims: std::mem::take(prims), closed: *closed },
            });
        } else {
            verts.clear();
            prims.clear();
        }
        *closed = false;
    };

    while i < tokens.len() {
        let Token::Cmd(cmd) = tokens[i] else {
            i += 1;
            continue;
        };
        i += 1;
        let rel = cmd.is_ascii_lowercase();
        let mut nums: Vec<f64> = Vec::new();
        while i < tokens.len() {
            if let Token::Num(n) = tokens[i] {
                nums.push(n);
                i += 1;
            } else {
                break;
            }
        }
        match cmd.to_ascii_uppercase() {
            'M' => {
                flush(&mut verts, &mut prims, &mut closed, &mut shapes);
                for (k, pair) in nums.chunks_exact(2).enumerate() {
                    let p = if rel { (cur.0 + pair[0], cur.1 + pair[1]) } else { (pair[0], pair[1]) };
                    cur = p;
                    if k == 0 {
                        start = p;
                        verts.push(Vertex::corner(p.0, p.1));
                    } else {
                        let a = verts.len() - 1;
                        verts.push(Vertex::corner(p.0, p.1));
                        prims.push(Prim::Line { a, b: verts.len() - 1 });
                    }
                }
            }
            'L' => {
                for pair in nums.chunks_exact(2) {
                    let p = if rel { (cur.0 + pair[0], cur.1 + pair[1]) } else { (pair[0], pair[1]) };
                    cur = p;
                    let a = verts.len() - 1;
                    verts.push(Vertex::corner(p.0, p.1));
                    prims.push(Prim::Line { a, b: verts.len() - 1 });
                }
            }
            'H' => {
                for &n in &nums {
                    let p = if rel { (cur.0 + n, cur.1) } else { (n, cur.1) };
                    cur = p;
                    let a = verts.len() - 1;
                    verts.push(Vertex::corner(p.0, p.1));
                    prims.push(Prim::Line { a, b: verts.len() - 1 });
                }
            }
            'V' => {
                for &n in &nums {
                    let p = if rel { (cur.0, cur.1 + n) } else { (cur.0, n) };
                    cur = p;
                    let a = verts.len() - 1;
                    verts.push(Vertex::corner(p.0, p.1));
                    prims.push(Prim::Line { a, b: verts.len() - 1 });
                }
            }
            'C' => {
                for seg in nums.chunks_exact(6) {
                    let c1 = if rel { (cur.0 + seg[0], cur.1 + seg[1]) } else { (seg[0], seg[1]) };
                    let c2 = if rel { (cur.0 + seg[2], cur.1 + seg[3]) } else { (seg[2], seg[3]) };
                    let end = if rel { (cur.0 + seg[4], cur.1 + seg[5]) } else { (seg[4], seg[5]) };
                    let a = verts.len() - 1;
                    verts[a].c1 = Some(c1);
                    let mut v = Vertex::corner(end.0, end.1);
                    v.c0 = Some(c2);
                    verts.push(v);
                    prims.push(Prim::Bezier { a, b: verts.len() - 1 });
                    cur = end;
                }
            }
            'Z' => {
                closed = true;
                if let Some(first) = verts.first() {
                    let _ = first;
                    let a = verts.len() - 1;
                    prims.push(Prim::Line { a, b: 0 });
                }
                cur = start;
            }
            _ => {}
        }
    }
    flush(&mut verts, &mut prims, &mut closed, &mut shapes);
    shapes
}

#[derive(Clone, Copy)]
enum Token {
    Cmd(char),
    Num(f64),
}

fn tokenize_path(d: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut num = String::new();
    let flush_num = |num: &mut String, tokens: &mut Vec<Token>| {
        if !num.is_empty() {
            if let Ok(n) = num.parse::<f64>() {
                tokens.push(Token::Num(n));
            }
            num.clear();
        }
    };
    for c in d.chars() {
        match c {
            'a'..='z' | 'A'..='Z' => {
                flush_num(&mut num, &mut tokens);
                tokens.push(Token::Cmd(c));
            }
            '-' => {
                // A '-' starts a new number unless it follows 'e' (exponent).
                if !num.is_empty() && !num.ends_with('e') && !num.ends_with('E') {
                    flush_num(&mut num, &mut tokens);
                }
                num.push(c);
            }
            '.' => {
                if num.contains('.') {
                    flush_num(&mut num, &mut tokens);
                }
                num.push(c);
            }
            ',' | ' ' | '\t' | '\n' | '\r' => flush_num(&mut num, &mut tokens),
            _ => num.push(c),
        }
    }
    flush_num(&mut num, &mut tokens);
    tokens
}

// ======================================================================
// DXF import
// ======================================================================

pub fn import_dxf(dxf: &str) -> Result<Vec<Shape>, String> {
    // DXF is a stream of (group-code, value) line pairs.
    let mut lines = dxf.lines();
    let mut pairs: Vec<(i32, String)> = Vec::new();
    while let (Some(code), Some(val)) = (lines.next(), lines.next()) {
        if let Ok(code) = code.trim().parse::<i32>() {
            pairs.push((code, val.trim().to_string()));
        }
    }

    let mut shapes = Vec::new();
    let mut i = 0;
    while i < pairs.len() {
        if pairs[i].0 == 0 {
            let entity = pairs[i].1.clone();
            // Collect this entity's codes until the next 0.
            let mut j = i + 1;
            let mut codes: Vec<(i32, String)> = Vec::new();
            while j < pairs.len() && pairs[j].0 != 0 {
                codes.push(pairs[j].clone());
                j += 1;
            }
            match entity.as_str() {
                "LINE" => {
                    let g = |c: i32| codes.iter().find(|(k, _)| *k == c).and_then(|(_, v)| v.parse::<f64>().ok());
                    if let (Some(x1), Some(y1), Some(x2), Some(y2)) = (g(10), g(20), g(11), g(21)) {
                        shapes.push(Shape::polyline(0, &[(x1, y1), (x2, y2)], false));
                    }
                }
                "CIRCLE" => {
                    let g = |c: i32| codes.iter().find(|(k, _)| *k == c).and_then(|(_, v)| v.parse::<f64>().ok());
                    if let (Some(cx), Some(cy), Some(r)) = (g(10), g(20), g(40)) {
                        shapes.push(Shape::ellipse(0, cx, cy, r, r));
                    }
                }
                "LWPOLYLINE" => {
                    let closed = codes.iter().find(|(k, _)| *k == 70).and_then(|(_, v)| v.parse::<i32>().ok()).map(|f| f & 1 == 1).unwrap_or(false);
                    let xs: Vec<f64> = codes.iter().filter(|(k, _)| *k == 10).filter_map(|(_, v)| v.parse().ok()).collect();
                    let ys: Vec<f64> = codes.iter().filter(|(k, _)| *k == 20).filter_map(|(_, v)| v.parse().ok()).collect();
                    let pts: Vec<(f64, f64)> = xs.into_iter().zip(ys).collect();
                    if pts.len() >= 2 {
                        shapes.push(Shape::polyline(0, &pts, closed));
                    }
                }
                _ => {}
            }
            i = j;
        } else {
            i += 1;
        }
    }
    if shapes.is_empty() {
        return Err("aucune entité DXF reconnue (LINE/LWPOLYLINE/CIRCLE)".into());
    }
    Ok(shapes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::ShapeKind;

    #[test]
    fn lbrn2_roundtrips_paths_and_groups() {
        let mut p = Project::default();
        p.shapes.push(Shape::rect(0, 10.0, 10.0, 40.0, 25.0));
        p.shapes.push(Shape::polyline(1, &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], true));
        p.shapes.push(Shape {
            cut_index: 0,
            xform: XForm::translate(5.0, 5.0),
            kind: ShapeKind::Group { children: vec![Shape::ellipse(1, 0.0, 0.0, 8.0, 6.0)] },
        });

        let xml = to_lbrn2(&p);
        assert!(xml.contains("<LightBurnProject AppVersion=\"1.5.00\""));
        assert!(xml.contains("Type=\"Group\""));
        assert!(xml.contains("<PrimList>"));

        let back = from_lbrn2(&xml).unwrap();
        // 3 top-level shapes survive; the group keeps its child.
        assert_eq!(back.shapes.len(), 3);
        let groups = back.shapes.iter().filter(|s| matches!(s.kind, ShapeKind::Group { .. })).count();
        assert_eq!(groups, 1);
    }

    #[test]
    fn vertlist_with_handles_roundtrips() {
        let verts = vec![
            Vertex { x: 0.0, y: 0.0, c0: None, c1: Some((3.0, 1.0)) },
            Vertex { x: 10.0, y: 0.0, c0: Some((7.0, 1.0)), c1: None },
        ];
        let s = fmt_vertlist(&verts);
        let back = parse_vertlist(&s);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].c1, Some((3.0, 1.0)));
        assert_eq!(back[1].c0, Some((7.0, 1.0)));
    }

    #[test]
    fn primlist_roundtrips() {
        let prims = vec![Prim::Line { a: 0, b: 1 }, Prim::Bezier { a: 1, b: 2 }, Prim::Line { a: 2, b: 0 }];
        let back = parse_primlist(&fmt_primlist(&prims));
        assert_eq!(back, prims);
    }

    #[test]
    fn imports_svg_primitives() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg">
            <rect x="1" y="2" width="30" height="20"/>
            <circle cx="50" cy="50" r="10"/>
            <line x1="0" y1="0" x2="10" y2="10"/>
            <polygon points="0,0 10,0 5,8"/>
            <path d="M0 0 L10 0 C13 3 13 7 10 10 Z"/>
        </svg>"#;
        let shapes = import_svg(svg).unwrap();
        assert_eq!(shapes.len(), 5);
        assert!(matches!(shapes[0].kind, ShapeKind::Rect { .. }));
        assert!(matches!(shapes[1].kind, ShapeKind::Ellipse { .. }));
        // The path has a cubic, so it must contain a Bezier primitive.
        if let ShapeKind::Path { prims, .. } = &shapes[4].kind {
            assert!(prims.iter().any(|p| matches!(p, Prim::Bezier { .. })));
        } else {
            panic!("path not parsed");
        }
    }

    #[test]
    fn imports_dxf_entities() {
        let dxf = "0\nSECTION\n2\nENTITIES\n0\nLINE\n10\n0.0\n20\n0.0\n11\n10.0\n21\n5.0\n0\nCIRCLE\n10\n20.0\n20\n20.0\n40\n4.0\n0\nLWPOLYLINE\n90\n3\n70\n1\n10\n0.0\n20\n0.0\n10\n10.0\n20\n0.0\n10\n5.0\n20\n8.0\n0\nENDSEC\n0\nEOF\n";
        let shapes = import_dxf(dxf).unwrap();
        assert_eq!(shapes.len(), 3);
        assert!(matches!(shapes[1].kind, ShapeKind::Ellipse { .. }));
    }
}
