//! Data files: what `import()`, `surface()` and the extrusions' legacy
//! `file =` argument read, parsed the way the reference implementation
//! (2021.01) reads them.
//!
//! 3D formats (STL, OFF, OBJ, AMF, 3MF, and STEP through FreeCAD) give a triangle mesh with coincident
//! corners merged; 2D formats (DXF, SVG) give a cross-section.

mod amf;
mod dxf;
mod obj;
mod off;
mod png;
mod step;
mod stl;
mod surface;
mod svg;
mod threemf;
mod xml;
pub(crate) mod zip;

use std::collections::HashMap;
use std::path::Path;

use clipper2_rust::{FillRule, PathD, PathsD, Point, union_subjects_d};
use manifold_rust::cross_section::CrossSection;
use manifold_rust::linalg::Vec2;
use c2f_eval::node::Segments;

use crate::primitives::TriMesh;

pub(crate) use surface::surface;

/// The geometry a file holds.
pub(crate) enum Imported {
    Flat(CrossSection),
    /// One mesh per object (AMF and 3MF files may hold several, which
    /// the reference unites).
    Solid(Vec<TriMesh>),
}

/// What a DXF import reads and how it places it.
pub(crate) struct DxfParams<'a> {
    /// Only entities on this layer; every layer when empty.
    pub layer: &'a str,
    /// Subtracted from every coordinate, before scaling.
    pub origin: [f64; 2],
    pub scale: f64,
    /// `$fn`, `$fa`, `$fs` for circles, arcs and ellipses.
    pub segments: Segments,
}

/// `import(file)`: the format follows the file's extension, as in the
/// reference. `name` is the file as the CSG names it, for messages.
pub(crate) fn import(path: &Path, name: &str, params: &DxfParams, warnings: &mut Vec<String>) -> Option<Imported> {
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    let read = |warnings: &mut Vec<String>| match std::fs::read(path) {
        Ok(data) => Some(data),
        Err(_) => {
            warnings.push(format!("Can't open import file '{name}'"));
            None
        }
    };
    match extension.as_str() {
        "stl" => stl::read(&read(warnings)?, name, warnings).map(|m| Imported::Solid(vec![m])),
        "off" => off::read(&read(warnings)?, name, warnings).map(|m| Imported::Solid(vec![m])),
        "obj" => obj::read(&read(warnings)?, name, warnings).map(|m| Imported::Solid(vec![m])),
        "amf" => amf::read(&read(warnings)?, name, warnings).map(Imported::Solid),
        "3mf" => threemf::read(&read(warnings)?, name, warnings).map(Imported::Solid),
        "step" | "stp" => step::read(path, name, &params.segments, warnings).map(|m| Imported::Solid(vec![m])),
        "dxf" => dxf(path, name, params, warnings).map(Imported::Flat),
        "svg" => svg::read(&read(warnings)?, name, warnings).map(Imported::Flat),
        "nef3" => {
            warnings.push(format!("import(\"{name}\"): NEF3 files are not supported"));
            None
        }
        _ => {
            warnings.push(format!("Unsupported file format while trying to import file '{name}'"));
            None
        }
    }
}

/// A DXF drawing as a cross-section: what `import()` of a `.dxf` gives,
/// and what `linear_extrude(file = ...)` and `rotate_extrude(file = ...)`
/// read whatever the extension.
pub(crate) fn dxf(path: &Path, name: &str, params: &DxfParams, warnings: &mut Vec<String>) -> Option<CrossSection> {
    let Ok(text) = std::fs::read(path) else {
        warnings.push(format!("Can't open DXF file '{name}'."));
        return None;
    };
    let outlines = dxf::read(&String::from_utf8_lossy(&text), name, params, warnings);
    let section = even_odd(&outlines);
    (!section.is_empty()).then_some(section)
}

/// The region inside an odd number of `outlines`: how the reference
/// sanitizes imported polygons (Clipper's even-odd union).
pub(crate) fn even_odd(outlines: &[Vec<[f64; 2]>]) -> CrossSection {
    let paths: PathsD = outlines
        .iter()
        .filter(|o| o.len() >= 3)
        .map(|o| o.iter().map(|&[x, y]| Point::new(x, y)).collect::<PathD>())
        .collect();
    if paths.is_empty() {
        return CrossSection::new(Vec::new());
    }
    let merged = union_subjects_d(&paths, FillRule::EvenOdd, 8);
    // Outer contours come out counter-clockwise and holes clockwise, which
    // the positive fill rule of `CrossSection::new` keeps as they are.
    CrossSection::new(merged.iter().map(|path| path.iter().map(|p| Vec2::new(p.x, p.y)).collect()).collect())
}

/// `section` without the vertices lying on a straight line between their
/// neighbours, which Clipper 1 drops whenever the reference cleans
/// polygons (Clipper 2, behind `CrossSection`, keeps them).
pub(crate) fn without_collinear(section: CrossSection) -> CrossSection {
    let mut polygons = section.to_polygons();
    let mut changed = false;
    for contour in &mut polygons {
        let mut i = 0;
        while contour.len() > 3 && i < contour.len() {
            let n = contour.len();
            let (a, b, c) = (contour[(i + n - 1) % n], contour[i], contour[(i + 1) % n]);
            let (u, v) = (b - a, c - b);
            // Within 1e-7 of the line through its neighbours (well under
            // the reference's 2^-16 grid, above CrossSection's 1e-8 one).
            let cross = u.x * v.y - u.y * v.x;
            let span = (c.x - a.x).hypot(c.y - a.y);
            if cross.abs() <= 1e-7 * span && u.x * v.x + u.y * v.y >= 0.0 {
                contour.remove(i);
                changed = true;
                // The previous vertex may now be collinear.
                i = i.saturating_sub(1);
            } else {
                i += 1;
            }
        }
    }
    if changed { CrossSection::new(polygons) } else { section }
}

/// Builds an indexed mesh from triangle corners, merging corners at the
/// same position so that the faces of a closed surface share their edges.
#[derive(Default)]
pub(crate) struct MeshBuilder {
    positions: Vec<[f64; 3]>,
    index: HashMap<[u64; 3], u32>,
    triangles: Vec<[u32; 3]>,
}

impl MeshBuilder {
    pub fn vertex(&mut self, p: [f64; 3]) -> u32 {
        // -0 and 0 are one position.
        let p = p.map(|c| if c == 0.0 { 0.0 } else { c });
        let next = self.positions.len() as u32;
        *self.index.entry(p.map(f64::to_bits)).or_insert_with(|| {
            self.positions.push(p);
            next
        })
    }

    /// A counter-clockwise triangle (seen from outside); triangles with
    /// two corners at the same position are dropped.
    pub fn triangle(&mut self, a: u32, b: u32, c: u32) {
        if a != b && b != c && c != a {
            self.triangles.push([a, b, c]);
        }
    }

    pub fn corners(&mut self, corners: [[f64; 3]; 3]) {
        let [a, b, c] = corners.map(|p| self.vertex(p));
        self.triangle(a, b, c);
    }

    /// A planar polygon, counter-clockwise seen from outside, split into
    /// triangles.
    pub fn polygon(&mut self, corners: &[u32]) {
        match corners.len() {
            0..=2 => {}
            3 => self.triangle(corners[0], corners[1], corners[2]),
            _ => {
                for [a, b, c] in triangulate_face(&self.positions, corners) {
                    self.triangle(a, b, c);
                }
            }
        }
    }

    pub fn finish(self) -> Option<TriMesh> {
        (!self.triangles.is_empty()).then_some(TriMesh { positions: self.positions, triangles: self.triangles })
    }
}

/// Triangles of a polygon in 3D: projected along its dominant normal
/// axis, then ear-clipped (fanned when that fails).
fn triangulate_face(positions: &[[f64; 3]], corners: &[u32]) -> Vec<[u32; 3]> {
    let at = |i: usize| positions[corners[i] as usize];
    // Newell's normal.
    let mut n = [0.0f64; 3];
    for i in 0..corners.len() {
        let (p, q) = (at(i), at((i + 1) % corners.len()));
        n[0] += (p[1] - q[1]) * (p[2] + q[2]);
        n[1] += (p[2] - q[2]) * (p[0] + q[0]);
        n[2] += (p[0] - q[0]) * (p[1] + q[1]);
    }
    let axis = (0..3).max_by(|&a, &b| n[a].abs().total_cmp(&n[b].abs())).unwrap_or(2);
    let (u, v) = match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    };
    let flip = n[axis] < 0.0;
    let contour: Vec<Vec2> = (0..corners.len())
        .map(|i| {
            let p = at(i);
            Vec2::new(if flip { -p[u] } else { p[u] }, p[v])
        })
        .collect();
    let triangles = manifold_rust::polygon::triangulate(&vec![contour], -1.0, true);
    if triangles.is_empty() {
        return (1..corners.len() - 1).map(|j| [corners[0], corners[j], corners[j + 1]]).collect();
    }
    triangles.iter().map(|t| [corners[t.x as usize], corners[t.y as usize], corners[t.z as usize]]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_open_import_is_shown_but_not_built() {
        let dir = std::env::temp_dir().join(format!("c2f-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A tetrahedron missing a face.
        std::fs::write(dir.join("open.off"), "OFF\n4 3 0\n0 0 0\n1 0 0\n0 1 0\n0 0 1\n3 0 2 1\n3 0 1 3\n3 0 3 2\n").unwrap();
        let program = c2f_eval::Program::load("translate([10, 0, 0]) import(\"open.off\");", Path::new("t.scad"), &[]).unwrap();
        let rendered = crate::render_in(&c2f_eval::evaluate(&program).nodes, &dir);
        assert!(rendered.model.is_empty());
        assert_eq!(rendered.solid.triangle_count(), 3);
        assert_eq!(rendered.solid.bounds().map(|(lo, _)| lo[0]), Some(10.0));
        assert!(rendered.warnings[0].contains("it is shown"), "{:?}", rendered.warnings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn merged_corners() {
        let mut mesh = MeshBuilder::default();
        mesh.corners([[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
        mesh.corners([[-0.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
        mesh.corners([[0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]]);
        let mesh = mesh.finish().unwrap();
        assert_eq!(mesh.positions.len(), 4);
        assert_eq!(mesh.triangles, vec![[0, 1, 2], [0, 2, 3]]);
    }

    #[test]
    fn even_odd_nesting() {
        let square = |lo: f64, hi: f64| vec![[lo, lo], [hi, lo], [hi, hi], [lo, hi]];
        // A square in a square is a hole, whatever its direction.
        let mut inner = square(2.0, 8.0);
        assert!((even_odd(&[square(0.0, 10.0), inner.clone()]).area() - 64.0).abs() < 1e-9);
        inner.reverse();
        assert!((even_odd(&[square(0.0, 10.0), inner]).area() - 64.0).abs() < 1e-9);
    }

    #[test]
    fn quads_split_in_two() {
        let mut mesh = MeshBuilder::default();
        let corners: Vec<u32> = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]]
            .into_iter()
            .map(|p| mesh.vertex(p))
            .collect();
        mesh.polygon(&corners);
        let mesh = mesh.finish().unwrap();
        assert_eq!(mesh.triangles.len(), 2);
        // Both keep the polygon's counter-clockwise direction (+z).
        for [a, b, c] in mesh.triangles {
            let (p, q, r) = (mesh.positions[a as usize], mesh.positions[b as usize], mesh.positions[c as usize]);
            let z = (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0]);
            assert!(z > 0.0);
        }
    }
}
