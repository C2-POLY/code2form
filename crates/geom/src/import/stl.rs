//! STL, ASCII or binary. As in the reference implementation, a file is
//! binary when its size is exactly what its triangle count (bytes 80..84)
//! calls for, and ASCII when it otherwise starts with `solid`.

use super::MeshBuilder;
use crate::primitives::TriMesh;

pub(super) fn read(data: &[u8], name: &str, warnings: &mut Vec<String>) -> Option<TriMesh> {
    let mut mesh = MeshBuilder::default();
    if is_binary(data) {
        for facet in data[84..].chunks_exact(50) {
            let f = |i: usize| f32::from_le_bytes([facet[i], facet[i + 1], facet[i + 2], facet[i + 3]]) as f64;
            // 12 bytes of normal, then the corners.
            let corner = |k: usize| [f(12 + 12 * k), f(16 + 12 * k), f(20 + 12 * k)];
            mesh.corners([corner(0), corner(1), corner(2)]);
        }
    } else if data.starts_with(b"solid") {
        ascii(&String::from_utf8_lossy(data), name, &mut mesh, warnings);
    } else {
        warnings.push(format!("'{name}' is neither an ASCII nor a binary STL file"));
        return None;
    }
    mesh.finish()
}

fn is_binary(data: &[u8]) -> bool {
    data.len() >= 84 && {
        let count = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as u64;
        data.len() as u64 == 84 + 50 * count
    }
}

fn ascii(text: &str, name: &str, mesh: &mut MeshBuilder, warnings: &mut Vec<String>) {
    let mut corners = [[0.0; 3]; 3];
    // Corners read in the current `outer loop`; 3+ once the facet is done
    // or broken.
    let mut count = 0;
    for (number, line) in text.lines().enumerate().skip(1) {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("outer") => count = 0,
            Some("vertex") => {
                // The first three words; more are ignored.
                let values: Vec<f64> = words.take(3).map_while(|w| w.parse().ok()).collect();
                if values.len() < 3 {
                    warnings.push(format!("STL line {}, can't parse vertex line '{}' importing file '{name}'", number + 1, line.trim()));
                    count = 10;
                    continue;
                }
                if count >= 3 {
                    if count == 3 {
                        warnings.push(format!("STL line {}, extra vertex line '{}' importing file '{name}'", number + 1, line.trim()));
                    }
                    continue;
                }
                corners[count] = [values[0], values[1], values[2]];
                count += 1;
                if count == 3 {
                    mesh.corners(corners);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TETRA: [[[f32; 3]; 3]; 4] = [
        [[0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]],
        [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        [[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    ];

    #[test]
    fn ascii_and_binary_agree() {
        let mut text = String::from("solid t\n");
        let mut binary = vec![0u8; 80];
        binary.extend(4u32.to_le_bytes());
        for tri in TETRA {
            text.push_str(" facet normal 0 0 0\n  outer loop\n");
            binary.extend([0u8; 12]);
            for p in tri {
                text.push_str(&format!("   vertex {} {} {}\n", p[0], p[1], p[2]));
                binary.extend(p.iter().flat_map(|c| c.to_le_bytes()));
            }
            text.push_str("  endloop\n endfacet\n");
            binary.extend([0u8; 2]);
        }
        text.push_str("endsolid t\n");
        let mut warnings = Vec::new();
        let a = read(text.as_bytes(), "t.stl", &mut warnings).unwrap();
        let b = read(&binary, "t.stl", &mut warnings).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(a.positions.len(), 4);
        assert_eq!(a.positions, b.positions);
        assert_eq!(a.triangles, b.triangles);
    }

    #[test]
    fn binary_wins_over_a_solid_header() {
        // A binary file may begin with "solid"; its size decides.
        let mut binary = b"solid but binary".to_vec();
        binary.resize(80, b' ');
        binary.extend(1u32.to_le_bytes());
        binary.extend([0u8; 12]);
        for p in TETRA[3] {
            binary.extend(p.iter().flat_map(|c| c.to_le_bytes()));
        }
        binary.extend([0u8; 2]);
        let mesh = read(&binary, "t.stl", &mut Vec::new()).unwrap();
        assert_eq!(mesh.triangles.len(), 1);
    }
}
