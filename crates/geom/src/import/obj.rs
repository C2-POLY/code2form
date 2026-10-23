//! Wavefront OBJ, as Blender, model sites and scanners write it: vertices
//! `v x y z [w]`, faces `f a b c …` counter-clockwise seen from outside,
//! each corner `v`, `v/vt`, `v//vn` or `v/vt/vn`, counted from 1, or from
//! the end when negative. Every object and group is read into one mesh;
//! texture coordinates, normals, lines and materials are skipped.

use super::MeshBuilder;
use crate::primitives::TriMesh;

pub(super) fn read(data: &[u8], name: &str, warnings: &mut Vec<String>) -> Option<TriMesh> {
    let text = String::from_utf8_lossy(data);
    let fail = |warnings: &mut Vec<String>, line: usize, why: &str| {
        warnings.push(format!("Can't read OBJ file '{name}', line {line}: {why}"));
        None
    };
    let mut mesh = MeshBuilder::default();
    let mut vertices: Vec<u32> = Vec::new();
    let mut faces = 0;
    // A line ending in `\` goes on on the next one.
    let mut logical = String::new();
    let mut first_line = 0;
    for (number, line) in text.lines().enumerate() {
        if logical.is_empty() {
            first_line = number + 1;
        }
        let line = line.split('#').next().unwrap_or("");
        if let Some(start) = line.trim_end().strip_suffix('\\') {
            logical.push_str(start);
            logical.push(' ');
            continue;
        }
        logical.push_str(line);
        let statement = std::mem::take(&mut logical);
        let mut words = statement.split_whitespace();
        match words.next() {
            Some("v") => {
                let v: Vec<f64> = words.map_while(|w| w.parse().ok()).collect();
                if v.len() < 3 || v.iter().any(|c| !c.is_finite()) {
                    return fail(warnings, first_line, &format!("bad vertex '{}'", statement.trim()));
                }
                // A fourth value is a weight; three more are a color.
                let w = if v.len() == 4 && v[3] != 0.0 { v[3] } else { 1.0 };
                vertices.push(mesh.vertex([v[0] / w, v[1] / w, v[2] / w]));
            }
            Some("f") => {
                let mut corners = Vec::new();
                for corner in words {
                    let index = corner.split('/').next().and_then(|i| i.parse::<i64>().ok());
                    let at = match index {
                        Some(i) if i > 0 => usize::try_from(i - 1).ok(),
                        Some(i) if i < 0 => vertices.len().checked_sub(i.unsigned_abs() as usize),
                        _ => None,
                    };
                    match at.and_then(|at| vertices.get(at)) {
                        Some(&id) => corners.push(id),
                        None => return fail(warnings, first_line, &format!("bad face corner '{corner}'")),
                    }
                }
                mesh.polygon(&corners);
                faces += 1;
            }
            _ => {}
        }
    }
    if faces == 0 {
        warnings.push(format!("OBJ file '{name}' has no faces"));
    }
    mesh.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUBE: &str = "# a cube\nmtllib cube.mtl\no Cube\n\
        v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nv 0 0 1\nv 1 0 1\nv 1 1 1\nv 0 1 1\n\
        vt 0 0\nvn 0 0 1\nusemtl red\ns off\n\
        f 1 4 3 2\nf 5/1 6/1 7/1 8/1\nf 1//1 2//1 6//1 5//1\nf 2/1/1 3/1/1 7/1/1 6/1/1\nf 3 4 8 7\nf -4 -1 -5 -8\n";

    #[test]
    fn a_cube_of_quads_in_every_corner_form() {
        let mesh = read(CUBE.as_bytes(), "cube.obj", &mut Vec::new()).unwrap();
        assert_eq!(mesh.positions.len(), 8);
        assert_eq!(mesh.triangles.len(), 12);
        // Negative indices count from the last vertex: -8 is the first.
        assert!(mesh.triangles.iter().any(|t| t.contains(&0) && t.contains(&4) && t.contains(&7)));
    }

    #[test]
    fn a_closed_cube_has_its_volume() {
        let program = c2f_eval::Program::load("import(\"cube.obj\");", std::path::Path::new("t.scad"), &[]).unwrap();
        let dir = std::env::temp_dir().join(format!("c2f-obj-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cube.obj"), CUBE).unwrap();
        let rendered = crate::render_in(&c2f_eval::evaluate(&program).nodes, &dir);
        assert!((rendered.stats.volume - 1.0).abs() < 1e-9, "{:?} {:?}", rendered.stats, rendered.warnings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_continued_line_and_a_weight() {
        let text = "v 0 0 0\nv 2 0 0 2\nv 0 1 0\nv 0 0 1\nf 1 3 \\\n 2\nf 1 2 4\nf 1 4 3\nf 2 3 4\n";
        let mesh = read(text.as_bytes(), "t.obj", &mut Vec::new()).unwrap();
        assert_eq!(mesh.positions[1], [1.0, 0.0, 0.0]);
        assert_eq!(mesh.triangles.len(), 4);
    }

    #[test]
    fn a_corner_past_the_vertices() {
        let mut warnings = Vec::new();
        assert!(read(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 9\n", "b.obj", &mut warnings).is_none());
        assert!(warnings[0].contains("line 4"), "{warnings:?}");
    }
}
