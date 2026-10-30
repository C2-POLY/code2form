//! The outline a section leaves on what the view draws, with its size: the
//! model, and the parts marked `%`, such as a scan imported as a reference
//! to measure. A `%` part is only triangles, with no solid behind it to
//! cut, so the triangles themselves are cut here.

use std::collections::HashMap;

/// Where a section crosses the triangles: polylines in the plane, in
/// world space, and the box around them in the plane's two axes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outline {
    pub lines: Vec<Vec<[f32; 3]>>,
    /// Smallest and largest point, in the plane's two axes: (y, z) across
    /// x, (x, z) across y, (x, y) across z.
    pub bounds: Option<([f32; 2], [f32; 2])>,
}

impl Outline {
    /// The outline's width and height in its plane.
    pub fn size(&self) -> Option<[f32; 2]> {
        self.bounds.map(|(lo, hi)| [hi[0] - lo[0], hi[1] - lo[1]])
    }
}

/// The plane's two axes across `axis`.
pub fn plane_axes(axis: usize) -> [usize; 2] {
    match axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    }
}

/// The triangles of `soup` (three corners each) cut across `axis` at `at`.
pub fn outline(soup: &[[f32; 3]], axis: usize, at: f32) -> Outline {
    let axis = axis.min(2);
    // A corner exactly on the plane counts as just above it: each edge
    // then crosses it once or not at all.
    let side = |p: &[f32; 3]| if p[axis] - at >= 0.0 { 1.0 } else { -1.0 };
    let mut segments = Vec::new();
    for t in soup.chunks_exact(3) {
        let mut ends = Vec::with_capacity(2);
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if side(&a) != side(&b) {
                let s = (at - a[axis]) / (b[axis] - a[axis]);
                let mut p = [0.0; 3];
                for i in 0..3 {
                    p[i] = a[i] + s * (b[i] - a[i]);
                }
                p[axis] = at;
                ends.push(p);
            }
        }
        if let [a, b] = ends[..] {
            if a != b {
                segments.push([a, b]);
            }
        }
    }
    let [u, v] = plane_axes(axis);
    let bounds = segments.iter().flatten().fold(None, |b: Option<([f32; 2], [f32; 2])>, p| {
        let q = [p[u], p[v]];
        let (lo, hi) = b.unwrap_or((q, q));
        Some(([lo[0].min(q[0]), lo[1].min(q[1])], [hi[0].max(q[0]), hi[1].max(q[1])]))
    });
    Outline { lines: chain(&segments), bounds }
}

/// The segments joined end to end where they meet, into as few polylines
/// as the mesh allows; a closed one ends on its first point.
fn chain(segments: &[[[f32; 3]; 2]]) -> Vec<Vec<[f32; 3]>> {
    // Points a hundredth of a micrometre apart are one.
    let key = |p: [f32; 3]| p.map(|c| (f64::from(c) * 1e5).round() as i64);
    let mut at: HashMap<[i64; 3], Vec<usize>> = HashMap::new();
    for (i, s) in segments.iter().enumerate() {
        for &p in s {
            at.entry(key(p)).or_default().push(i);
        }
    }
    let mut used = vec![false; segments.len()];
    let mut lines = Vec::new();
    // The next unused segment that touches `p`, and its other end.
    let next = |p: [f32; 3], used: &[bool]| -> Option<(usize, [f32; 3])> {
        let i = *at.get(&key(p))?.iter().find(|&&i| !used[i])?;
        let [a, b] = segments[i];
        Some((i, if key(a) == key(p) { b } else { a }))
    };
    for start in 0..segments.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let [a, b] = segments[start];
        let mut line = vec![a, b];
        while let Some((i, p)) = next(*line.last().unwrap(), &used) {
            used[i] = true;
            line.push(p);
        }
        // An open mesh: walk the other way from the start too.
        let mut back = Vec::new();
        while let Some((i, p)) = next(*back.last().unwrap_or(&a), &used) {
            used[i] = true;
            back.push(p);
        }
        back.reverse();
        back.extend(line);
        lines.push(back);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box from `lo` to `hi`, as twelve triangles.
    fn cuboid(lo: [f32; 3], hi: [f32; 3]) -> Vec<[f32; 3]> {
        let c = |i: usize| [if i & 1 == 0 { lo[0] } else { hi[0] }, if i & 2 == 0 { lo[1] } else { hi[1] }, if i & 4 == 0 { lo[2] } else { hi[2] }];
        let faces = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        faces.iter().flat_map(|f| [c(f[0]), c(f[1]), c(f[2]), c(f[0]), c(f[2]), c(f[3])]).collect()
    }

    #[test]
    fn cuts_a_box_into_one_closed_rectangle_of_its_size() {
        let soup = cuboid([-20.0, -10.0, 0.0], [20.0, 10.0, 30.0]);
        let cut = outline(&soup, 2, 15.0);
        assert_eq!(cut.lines.len(), 1);
        let line = &cut.lines[0];
        assert_eq!(line.first(), line.last(), "closed");
        assert!(line.iter().all(|p| p[2] == 15.0));
        assert_eq!(cut.bounds, Some(([-20.0, -10.0], [20.0, 10.0])));
        assert_eq!(cut.size(), Some([40.0, 20.0]));
    }

    #[test]
    fn reads_the_size_in_the_planes_own_axes() {
        let soup = cuboid([-20.0, -10.0, 0.0], [20.0, 10.0, 30.0]);
        // Across x: (y, z).
        assert_eq!(outline(&soup, 0, 0.0).size(), Some([20.0, 30.0]));
        // Across y: (x, z).
        assert_eq!(outline(&soup, 1, 0.0).size(), Some([40.0, 30.0]));
    }

    #[test]
    fn misses_what_lies_on_one_side() {
        let soup = cuboid([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        let cut = outline(&soup, 2, 20.0);
        assert!(cut.lines.is_empty());
        assert_eq!(cut.size(), None);
    }

    #[test]
    fn keeps_two_separate_solids_apart() {
        let mut soup = cuboid([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        soup.extend(cuboid([20.0, 0.0, 0.0], [30.0, 5.0, 10.0]));
        let cut = outline(&soup, 2, 5.0);
        assert_eq!(cut.lines.len(), 2);
        assert_eq!(cut.size(), Some([30.0, 10.0]));
    }

    #[test]
    fn follows_an_open_mesh_from_end_to_end() {
        // A box without its top and one side: the cut is an open U.
        let soup: Vec<[f32; 3]> = cuboid([0.0, 0.0, 0.0], [10.0, 10.0, 10.0])
            .chunks_exact(6)
            .enumerate()
            .filter(|&(face, _)| face != 1 && face != 3)
            .flat_map(|(_, f)| f.to_vec())
            .collect();
        let cut = outline(&soup, 2, 5.0);
        assert_eq!(cut.lines.len(), 1, "{:?}", cut.lines);
        let line = &cut.lines[0];
        assert_ne!(line.first(), line.last(), "open");
        // Three faces of two triangles each: six segments end to end.
        assert_eq!(line.len(), 7);
        assert_eq!(cut.size(), Some([10.0, 10.0]));
    }
}
