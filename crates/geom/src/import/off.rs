//! OFF (Object File Format), ASCII: `[ST][C][N][4]OFF`, the vertex, face
//! and edge counts, the vertices (extra values such as colors or normals
//! after the three coordinates are skipped), then the faces as
//! `n i1 .. in`, counter-clockwise seen from outside.

use super::MeshBuilder;
use crate::primitives::TriMesh;

pub(super) fn read(data: &[u8], name: &str, warnings: &mut Vec<String>) -> Option<TriMesh> {
    let text = String::from_utf8_lossy(data);
    // Comments run from `#` to the end of the line.
    let mut lines = text.lines().map(|l| l.split('#').next().unwrap_or("")).filter(|l| !l.trim().is_empty());
    let fail = |warnings: &mut Vec<String>, why: &str| {
        warnings.push(format!("Can't read OFF file '{name}': {why}"));
        None
    };
    let Some(header) = lines.next() else { return fail(warnings, "empty file") };
    let mut words = header.split_whitespace();
    let keyword = words.next().unwrap_or("");
    if !keyword.ends_with("OFF") {
        return fail(warnings, "no OFF header");
    }
    if words.clone().next() == Some("BINARY") {
        return fail(warnings, "binary OFF is not supported");
    }
    // The counts may follow the keyword on its line.
    let mut counts: Vec<usize> = words.filter_map(|w| w.parse().ok()).collect();
    if counts.is_empty() {
        let Some(line) = lines.next() else { return fail(warnings, "no counts") };
        counts = line.split_whitespace().filter_map(|w| w.parse().ok()).collect();
    }
    let (Some(&vertices), Some(&faces)) = (counts.first(), counts.get(1)) else {
        return fail(warnings, "bad counts");
    };
    let homogeneous = keyword.trim_end_matches("OFF").contains('4');
    let mut mesh = MeshBuilder::default();
    let mut ids = Vec::with_capacity(vertices);
    for _ in 0..vertices {
        let Some(line) = lines.next() else { return fail(warnings, "missing vertices") };
        let v: Vec<f64> = line.split_whitespace().map_while(|w| w.parse().ok()).collect();
        if v.len() < 3 {
            return fail(warnings, &format!("bad vertex '{}'", line.trim()));
        }
        let w = if homogeneous && v.len() > 3 && v[3] != 0.0 { v[3] } else { 1.0 };
        ids.push(mesh.vertex([v[0] / w, v[1] / w, v[2] / w]));
    }
    for _ in 0..faces {
        let Some(line) = lines.next() else { return fail(warnings, "missing faces") };
        let mut values = line.split_whitespace().map(|w| w.parse::<usize>());
        let Some(Ok(n)) = values.next() else { return fail(warnings, &format!("bad face '{}'", line.trim())) };
        let corners: Option<Vec<u32>> = values.take(n).map(|i| i.ok().and_then(|i| ids.get(i).copied())).collect();
        match corners {
            Some(corners) if corners.len() == n => mesh.polygon(&corners),
            _ => return fail(warnings, &format!("bad face '{}'", line.trim())),
        }
    }
    mesh.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_with_quads_and_comments() {
        let text = "OFF # a cube\n8 6 12\n0 0 0\n1 0 0\n1 1 0\n0 1 0\n0 0 1\n1 0 1\n1 1 1\n0 1 1\n\
                    4 0 3 2 1\n4 4 5 6 7\n4 0 1 5 4\n4 1 2 6 5\n4 2 3 7 6\n4 3 0 4 7 255 0 0\n";
        let mesh = read(text.as_bytes(), "c.off", &mut Vec::new()).unwrap();
        assert_eq!(mesh.positions.len(), 8);
        assert_eq!(mesh.triangles.len(), 12);
    }

    #[test]
    fn counts_on_the_header_line_and_colored_vertices() {
        let text = "COFF 4 4 6\n0 0 0 1 0 0 1\n1 0 0 1 0 0 1\n0 1 0 1 0 0 1\n0 0 1 1 0 0 1\n3 0 2 1\n3 0 1 3\n3 0 3 2\n3 1 2 3\n";
        let mesh = read(text.as_bytes(), "t.off", &mut Vec::new()).unwrap();
        assert_eq!(mesh.positions[3], [0.0, 0.0, 1.0]);
        assert_eq!(mesh.triangles.len(), 4);
    }

    #[test]
    fn bad_index() {
        let mut warnings = Vec::new();
        assert!(read(b"OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 7\n", "b.off", &mut warnings).is_none());
        assert_eq!(warnings.len(), 1);
    }
}
