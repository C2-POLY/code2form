//! `surface()`: a height map from a PNG image or a DAT text matrix, meshed
//! as the reference implementation (2021.01) does.
//!
//! Row `i`, column `j` of the matrix is the height at (j, i); a PNG's top
//! row is the last one (y = height - 1), and a pixel's height is its
//! luminance scaled to 0..100. Each grid cell gets four triangles meeting
//! at its center (raised to the mean of its corners), the sides drop to a
//! floor at `min(0, lowest - 1)`, and the floor closes the solid.

use std::collections::HashMap;
use std::path::Path;

use super::png;
use crate::primitives::TriMesh;

/// The mesh of `surface(file, center, invert)`; `name` is the file as the
/// CSG names it, for messages.
pub(crate) fn surface(path: &Path, name: &str, center: bool, invert: bool, warnings: &mut Vec<String>) -> Option<TriMesh> {
    let Ok(data) = std::fs::read(path) else {
        warnings.push(format!("The file '{name}' couldn't be opened."));
        return None;
    };
    let heights = if data.starts_with(png::SIGNATURE) {
        match png::decode(&data) {
            Ok(image) => from_image(&image, invert),
            Err(_) => {
                warnings.push(format!("Can't read PNG image '{name}'"));
                return None;
            }
        }
    } else {
        read_dat(&String::from_utf8_lossy(&data), name, warnings)
    };
    mesh(&heights, center)
}

/// Heights by (line, column).
type Heights = HashMap<(usize, usize), f64>;

fn from_image(image: &png::Image, invert: bool) -> Heights {
    let mut heights = Heights::with_capacity(image.width * image.height);
    for y in 0..image.height {
        for x in 0..image.width {
            let p = &image.rgb[(y * image.width + x) * 3..][..3];
            let pixel = 0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64;
            // The reference release (2021.01) inverts as 1 - value, not
            // 255 - value.
            let z = 100.0 / 255.0 * if invert { 1.0 - pixel } else { pixel };
            heights.insert((image.height - 1 - y, x), z);
        }
    }
    heights
}

/// The DAT format: lines of numbers separated by spaces or tabs; empty
/// lines and lines starting with `#` are skipped. Reading stops at the
/// first word that is not a number.
fn read_dat(text: &str, name: &str, warnings: &mut Vec<String>) -> Heights {
    let mut heights = Heights::new();
    let rows: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).collect();
    for (line, row) in rows.iter().enumerate() {
        for (column, word) in row.split([' ', '\t']).filter(|w| !w.is_empty()).enumerate() {
            match word.parse::<f64>() {
                Ok(v) => {
                    heights.insert((line, column), v);
                }
                Err(_) => {
                    // The reference warns unless this is the last line.
                    if line + 1 < rows.len() {
                        warnings.push(format!(
                            "Illegal value in '{name}': bad lexical cast: source type value could not be interpreted as target"
                        ));
                    }
                    return heights;
                }
            }
        }
    }
    heights
}

fn mesh(heights: &Heights, center: bool) -> Option<TriMesh> {
    let lines = heights.keys().map(|&(l, _)| l + 1).max().unwrap_or(0);
    let columns = heights.keys().map(|&(_, c)| c + 1).max().unwrap_or(0);
    if lines < 2 || columns < 2 {
        // No cell: the reference's mesh is flat, with no volume.
        return None;
    }
    let floor = heights.values().fold(0.0f64, |m, &v| m.min(v - 1.0));
    let h = |l: usize, c: usize| heights.get(&(l, c)).copied().unwrap_or(0.0);
    let (ox, oy) = if center { (-((columns - 1) as f64) / 2.0, -((lines - 1) as f64) / 2.0) } else { (0.0, 0.0) };
    let mut positions = Vec::with_capacity(lines * columns * 2 + (lines - 1) * (columns - 1));
    // Grid points, then cell centers, then the floor along the border.
    for l in 0..lines {
        for c in 0..columns {
            positions.push([ox + c as f64, oy + l as f64, h(l, c)]);
        }
    }
    let top = |l: usize, c: usize| (l * columns + c) as u32;
    let centers = positions.len();
    for l in 1..lines {
        for c in 1..columns {
            let mean = (h(l - 1, c - 1) + h(l - 1, c) + h(l, c - 1) + h(l, c)) / 4.0;
            positions.push([ox + c as f64 - 0.5, oy + l as f64 - 0.5, mean]);
        }
    }
    let center_of = |l: usize, c: usize| (centers + (l - 1) * (columns - 1) + c - 1) as u32;
    // The border, counter-clockwise seen from above, from (0, 0).
    let mut border: Vec<(usize, usize)> = Vec::new();
    border.extend((0..columns - 1).map(|c| (0, c)));
    border.extend((0..lines - 1).map(|l| (l, columns - 1)));
    border.extend((1..columns).rev().map(|c| (lines - 1, c)));
    border.extend((1..lines).rev().map(|l| (l, 0)));
    let floor_start = positions.len();
    let mut floor_index = HashMap::new();
    for (k, &(l, c)) in border.iter().enumerate() {
        positions.push([ox + c as f64, oy + l as f64, floor]);
        floor_index.insert((l, c), (floor_start + k) as u32);
    }
    let mut triangles = Vec::new();
    for l in 1..lines {
        for c in 1..columns {
            let (v1, v2, v3, v4, vx) = (top(l - 1, c - 1), top(l - 1, c), top(l, c - 1), top(l, c), center_of(l, c));
            triangles.extend([[v1, v2, vx], [v2, v4, vx], [v4, v3, vx], [v3, v1, vx]]);
        }
    }
    // Side walls: for each border edge (a -> b, counter-clockwise from
    // above), the quad down to the floor, facing out.
    for k in 0..border.len() {
        let (a, b) = (border[k], border[(k + 1) % border.len()]);
        let (ta, tb) = (top(a.0, a.1), top(b.0, b.1));
        let (fa, fb) = (floor_index[&a], floor_index[&b]);
        triangles.push([fa, fb, tb]);
        triangles.push([fa, tb, ta]);
    }
    // The floor, one strip per line of cells, zipped between the strip's
    // lower and upper border points (so no triangle is degenerate), and
    // facing down.
    let (last_line, last_column) = (lines - 1, columns - 1);
    let chain = |l: usize| -> Vec<u32> {
        if l == 0 || l == last_line {
            (0..columns).map(|c| floor_index[&(l, c)]).collect()
        } else {
            vec![floor_index[&(l, 0)], floor_index[&(l, last_column)]]
        }
    };
    let x_of = |i: u32| positions[i as usize][0];
    for l in 0..last_line {
        let (lower, upper) = (chain(l), chain(l + 1));
        let (mut i, mut j) = (0, 0);
        while i + 1 < lower.len() || j + 1 < upper.len() {
            if j + 1 >= upper.len() || (i + 1 < lower.len() && x_of(lower[i + 1]) <= x_of(upper[j + 1])) {
                triangles.push([lower[i], upper[j], lower[i + 1]]);
                i += 1;
            } else {
                triangles.push([lower[i], upper[j], upper[j + 1]]);
                j += 1;
            }
        }
    }
    Some(TriMesh { positions, triangles })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(mesh: &TriMesh) -> f64 {
        mesh.triangles
            .iter()
            .map(|&[a, b, c]| {
                let (p, q, r) = (mesh.positions[a as usize], mesh.positions[b as usize], mesh.positions[c as usize]);
                p[0] * (q[1] * r[2] - q[2] * r[1]) - p[1] * (q[0] * r[2] - q[2] * r[0]) + p[2] * (q[0] * r[1] - q[1] * r[0])
            })
            .sum::<f64>()
            / 6.0
    }

    fn dat(text: &str) -> Heights {
        read_dat(text, "t.dat", &mut Vec::new())
    }

    #[test]
    fn volumes_match_reference() {
        // From exports of the same files by the reference release (2021.01).
        let cases = [
            ("5 5 5\n5 5 5\n", 10.0),
            ("# comment\n-3 0 2\n1 -1 0\n\n4 2 1\n", 17.5),
            ("3 4\n5 6\n", 4.5),
            ("1 2 3 4\n5 6\n7 8 9\n", 22.5),
        ];
        for (text, expected) in cases {
            let mesh = mesh(&dat(text), false).unwrap();
            assert!((volume(&mesh) - expected).abs() < 1e-9, "{text:?}: {}", volume(&mesh));
        }
    }

    #[test]
    fn closed_and_centered() {
        let mesh = mesh(&dat("-3 0 2\n1 -1 0\n4 2 1\n"), true).unwrap();
        let mut edges = HashMap::new();
        for t in &mesh.triangles {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((a, b)).or_insert(0) += 1;
            }
        }
        for (&(a, b), &count) in &edges {
            assert_eq!(count, 1);
            assert_eq!(edges.get(&(b, a)), Some(&1), "edge {a}-{b} has no twin");
        }
        let xs = mesh.positions.iter().map(|p| p[0]);
        assert_eq!(xs.clone().fold(f64::INFINITY, f64::min), -1.0);
        assert_eq!(mesh.positions.iter().map(|p| p[2]).fold(f64::INFINITY, f64::min), -4.0);
    }

    #[test]
    fn png_heights() {
        let image = png::decode(&png::tests::encode(3, 2, 8, 0, &[vec![0, 128, 255], vec![10, 20, 30]], None)).unwrap();
        let heights = from_image(&image, false);
        // The top row of the image is the last line.
        assert!((heights[&(1, 2)] - 100.0).abs() < 1e-9);
        assert!((heights[&(0, 0)] - 1000.0 / 255.0).abs() < 1e-9);
        let inverted = from_image(&image, true);
        assert!((inverted[&(1, 0)] - 100.0 / 255.0).abs() < 1e-9);
        let m = mesh(&heights, false).unwrap();
        // The export rounds heights to 6 digits.
        assert!((volume(&m) - 59.941192).abs() < 1e-4, "{}", volume(&m));
    }
}
