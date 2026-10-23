//! Mending an imported mesh that is not a closed solid, as many files
//! from the web or from other programs are: corners a hair apart, faces
//! listed twice, faces turned the wrong way, small holes. Each is fixed
//! the way a slicer's repair does, so that the mesh can be cut and
//! combined like any solid.

use std::collections::{BTreeMap, HashMap};

use super::triangulate_face;
use crate::primitives::TriMesh;

/// What [`repair`] changed, for the warning.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Repairs {
    pub merged: usize,
    pub dropped: usize,
    pub flipped: usize,
    pub holes: usize,
}

impl Repairs {
    /// What was done, in words: `2 holes filled, 40 faces turned`.
    pub fn describe(&self) -> String {
        let mut parts = Vec::new();
        let mut say = |n: usize, one: &str, many: &str| {
            if n > 0 {
                parts.push(format!("{n} {}", if n == 1 { one } else { many }));
            }
        };
        say(self.holes, "hole filled", "holes filled");
        say(self.flipped, "face turned", "faces turned");
        say(self.dropped, "face removed", "faces removed");
        say(self.merged, "corner merged", "corners merged");
        parts.join(", ")
    }
}

/// `mesh` mended, as its shells: corners closer than a millionth of its
/// size merged, flat and doubled faces dropped, faces turned to agree with
/// their neighbours and each shell outwards, and each shell's holes
/// filled. Shells that only touch along an edge (more than two faces
/// there), which no solid can hold, come apart, to be united.
pub(crate) fn repair(mesh: &TriMesh) -> (Vec<TriMesh>, Repairs) {
    let mut repairs = Repairs::default();
    // Corners on a grid a millionth of the size apart are one.
    let (lo, hi) = mesh.positions.iter().fold(([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]), |(lo, hi), p| {
        ([lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])], [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])])
    });
    let size = (0..3).map(|k| hi[k] - lo[k]).fold(0.0f64, f64::max);
    let grid = (size * 1e-6).max(1e-9);
    let mut index: HashMap<[i64; 3], u32> = HashMap::new();
    let mut positions = Vec::new();
    let remap: Vec<u32> = mesh
        .positions
        .iter()
        .map(|p| {
            let key = p.map(|c| (c / grid).round() as i64);
            *index.entry(key).or_insert_with(|| {
                positions.push(*p);
                (positions.len() - 1) as u32
            })
        })
        .collect();
    repairs.merged = mesh.positions.len() - positions.len();

    // Flat and doubled faces.
    let mut seen: HashMap<[u32; 3], ()> = HashMap::new();
    let mut triangles: Vec<[u32; 3]> = Vec::with_capacity(mesh.triangles.len());
    for t in &mesh.triangles {
        let t = t.map(|i| remap[i as usize]);
        let mut key = t;
        key.sort_unstable();
        if t[0] == t[1] || t[1] == t[2] || t[0] == t[2] || seen.insert(key, ()).is_some() {
            repairs.dropped += 1;
            continue;
        }
        triangles.push(t);
    }

    let (flipped, shells) = orient(&positions, &mut triangles);
    repairs.flipped = flipped;
    let mut meshes = Vec::with_capacity(shells.len());
    for shell in shells {
        // Its own corners, numbered in order of use.
        let mut number: HashMap<u32, u32> = HashMap::new();
        let mut own = Vec::new();
        let mut faces: Vec<[u32; 3]> = shell
            .iter()
            .map(|&t| {
                triangles[t].map(|i| {
                    *number.entry(i).or_insert_with(|| {
                        own.push(positions[i as usize]);
                        (own.len() - 1) as u32
                    })
                })
            })
            .collect();
        repairs.holes += fill_holes(&own, &mut faces);
        meshes.push(TriMesh { positions: own, triangles: faces });
    }
    (meshes, repairs)
}

/// Turns faces to agree with their neighbours across each edge two faces
/// share, then each shell so that it encloses a positive volume. Returns
/// how many faces were turned, and the shells: faces joined by such edges.
fn orient(positions: &[[f64; 3]], triangles: &mut [[u32; 3]]) -> (usize, Vec<Vec<usize>>) {
    let mut by_edge: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (t, &[a, b, c]) in triangles.iter().enumerate() {
        for (u, v) in [(a, b), (b, c), (c, a)] {
            by_edge.entry((u.min(v), u.max(v))).or_default().push(t);
        }
    }
    let runs = |t: &[u32; 3], u: u32, v: u32| (0..3).any(|k| t[k] == u && t[(k + 1) % 3] == v);
    let mut flip = vec![false; triangles.len()];
    let mut seen = vec![false; triangles.len()];
    let mut shells: Vec<Vec<usize>> = Vec::new();
    for seed in 0..triangles.len() {
        if seen[seed] {
            continue;
        }
        seen[seed] = true;
        let mut shell = vec![seed];
        let mut pending = vec![seed];
        while let Some(t) = pending.pop() {
            let mut own = triangles[t];
            if flip[t] {
                own.swap(1, 2);
            }
            for k in 0..3 {
                let (u, v) = (own[k], own[(k + 1) % 3]);
                // Only across edges two faces share: elsewhere the
                // neighbour is ambiguous.
                let Some(faces) = by_edge.get(&(u.min(v), u.max(v))).filter(|f| f.len() == 2) else { continue };
                let other = if faces[0] == t { faces[1] } else { faces[0] };
                if seen[other] {
                    continue;
                }
                seen[other] = true;
                // A neighbour that agrees runs the edge the other way.
                flip[other] = runs(&triangles[other], u, v);
                shell.push(other);
                pending.push(other);
            }
        }
        shells.push(shell);
    }
    let mut turned = 0;
    for shell in &shells {
        let volume: f64 = shell
            .iter()
            .map(|&t| {
                let mut c = triangles[t].map(|i| positions[i as usize]);
                if flip[t] {
                    c.swap(1, 2);
                }
                let [a, b, c] = c;
                a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])
            })
            .sum();
        let inside_out = volume < 0.0;
        for &t in shell {
            if flip[t] != inside_out {
                triangles[t].swap(1, 2);
                turned += 1;
            }
        }
    }
    (turned, shells)
}

/// Fills each loop of edges that only one face runs along with a fan of
/// triangles across it. Returns how many holes were filled.
fn fill_holes(positions: &[[f64; 3]], triangles: &mut Vec<[u32; 3]>) -> usize {
    let mut runs: HashMap<(u32, u32), usize> = HashMap::new();
    for &[a, b, c] in triangles.iter() {
        for edge in [(a, b), (b, c), (c, a)] {
            *runs.entry(edge).or_default() += 1;
        }
    }
    // The open edges, by where they start, in order: the hole runs the
    // other way round them.
    let mut from: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (&(u, v), &n) in &runs {
        if n == 1 && !runs.contains_key(&(v, u)) {
            from.entry(u).or_default().push(v);
        }
    }
    for ends in from.values_mut() {
        ends.sort_unstable();
    }
    let mut holes = 0;
    let mut fill = |contour: &[u32], triangles: &mut Vec<[u32; 3]>| {
        if contour.len() >= 3 {
            // The fill runs against the edges around it.
            let reversed: Vec<u32> = contour.iter().rev().copied().collect();
            triangles.extend(triangulate_face(positions, &reversed));
            holes += 1;
        }
    };
    while let Some(&start) = from.iter().find(|(_, ends)| !ends.is_empty()).map(|(v, _)| v) {
        let mut contour = vec![start];
        loop {
            let at = *contour.last().unwrap_or(&start);
            let Some(next) = from.get_mut(&at).and_then(Vec::pop) else { break };
            // Back at a corner it passed: what runs from there is a loop
            // of its own, and the walk goes on from that corner.
            match contour.iter().position(|&v| v == next) {
                Some(0) => {
                    fill(&contour, triangles);
                    break;
                }
                Some(again) => {
                    fill(&contour[again..], triangles);
                    contour.truncate(again + 1);
                }
                None => contour.push(next),
            }
        }
    }
    holes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube() -> TriMesh {
        let positions = (0..8).map(|i| [(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64]).collect();
        let quads = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let triangles = quads.iter().flat_map(|&[a, b, c, d]| [[a, b, c], [a, c, d]]).collect();
        TriMesh { positions, triangles }
    }

    fn is_closed(mesh: &TriMesh) -> bool {
        let mut runs: HashMap<(u32, u32), usize> = HashMap::new();
        for &[a, b, c] in &mesh.triangles {
            for edge in [(a, b), (b, c), (c, a)] {
                *runs.entry(edge).or_default() += 1;
            }
        }
        runs.iter().all(|(&(u, v), &n)| n == 1 && runs.get(&(v, u)) == Some(&1))
    }

    fn volume(mesh: &TriMesh) -> f64 {
        mesh.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
                a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])
            })
            .sum::<f64>()
            / 6.0
    }

    #[test]
    fn a_closed_mesh_is_left_as_it_is() {
        let cube = cube();
        assert!(is_closed(&cube) && (volume(&cube) - 1.0).abs() < 1e-12);
        let (mended, repairs) = repair(&cube);
        assert_eq!(repairs, Repairs::default());
        assert_eq!(mended.len(), 1);
        assert_eq!(volume(&mended[0]), volume(&cube));
    }

    #[test]
    fn a_hole_is_filled() {
        let mut open = cube();
        open.triangles.truncate(10);
        let (mended, repairs) = repair(&open);
        assert_eq!(repairs.holes, 1);
        assert!(is_closed(&mended[0]));
        assert!((volume(&mended[0]) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn turned_doubled_and_split_faces_are_mended() {
        let mut broken = cube();
        broken.triangles[3].swap(1, 2);
        broken.triangles[7].swap(0, 1);
        broken.triangles.push(broken.triangles[0]);
        // A corner listed again, a hair away.
        broken.positions.push([1.0 + 1e-9, 1.0, 1.0]);
        let last = broken.positions.len() as u32 - 1;
        for t in &mut broken.triangles {
            if let Some(i) = t.iter().position(|&i| i == 7) {
                t[i] = last;
                break;
            }
        }
        let (mended, repairs) = repair(&broken);
        assert_eq!((repairs.merged, repairs.dropped, repairs.flipped, repairs.holes), (1, 1, 2, 0));
        assert!(is_closed(&mended[0]));
        assert!((volume(&mended[0]) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_shell_inside_out_is_turned_outwards() {
        let mut inside_out = cube();
        for t in &mut inside_out.triangles {
            t.swap(1, 2);
        }
        let (mended, repairs) = repair(&inside_out);
        assert_eq!(repairs.flipped, 12);
        assert!((volume(&mended[0]) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn shells_touching_along_an_edge_come_apart() {
        // Two cubes sharing the edge x = 1, y = 1: four faces on it.
        let one = cube();
        let mut two = cube();
        for p in &mut two.positions {
            *p = [p[0] + 1.0, p[1] + 1.0, p[2]];
        }
        let offset = one.positions.len() as u32;
        let mut both = TriMesh { positions: one.positions.clone(), triangles: one.triangles.clone() };
        both.positions.extend(two.positions);
        both.triangles.extend(two.triangles.iter().map(|t| t.map(|i| i + offset)));
        let (mended, _) = repair(&both);
        assert_eq!(mended.len(), 2);
        assert!(mended.iter().all(|m| is_closed(m) && (volume(m) - 1.0).abs() < 1e-12));
    }
}
