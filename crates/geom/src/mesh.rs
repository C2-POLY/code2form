//! What a viewer draws: flat-shaded triangles with per-vertex colors.

use std::fmt::Write;

use crate::build::Model;

/// Triangles ready to draw, flat-shaded: three vertices per triangle, each
/// with the triangle's normal and the color of the part it came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub colors: Vec<[f32; 4]>,
}

impl RenderMesh {
    pub fn triangle_count(&self) -> usize {
        self.positions.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Smallest and largest corner, if there is anything.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = *self.positions.first()?;
        Some(self.positions.iter().fold((first, first), |(lo, hi), p| {
            (
                [lo[0].min(p[0]), lo[1].min(p[1]), lo[2].min(p[2])],
                [hi[0].max(p[0]), hi[1].max(p[1]), hi[2].max(p[2])],
            )
        }))
    }

    pub fn push_triangle(&mut self, corners: [[f32; 3]; 3], colors: [[f32; 4]; 3]) {
        let [a, b, c] = corners;
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let normal = if length > 0.0 { [n[0] / length, n[1] / length, n[2] / length] } else { [0.0, 0.0, 1.0] };
        self.positions.extend(corners);
        self.normals.extend([normal; 3]);
        self.colors.extend(colors);
    }

    pub fn append(&mut self, other: RenderMesh) {
        self.positions.extend(other.positions);
        self.normals.extend(other.normals);
        self.colors.extend(other.colors);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub triangles: usize,
    pub vertices: usize,
    pub volume: f64,
}

/// The rendering of a program.
pub struct Rendered {
    /// The model itself.
    pub solid: RenderMesh,
    /// Parts marked `#`, drawn translucent on top of the model.
    pub highlights: RenderMesh,
    /// Parts marked `%`, drawn translucent and left out of the model.
    pub backgrounds: RenderMesh,
    /// The model is 2D (drawn as a thin slab).
    pub is_2d: bool,
    pub stats: Stats,
    pub warnings: Vec<String>,
    /// The geometry, for exports.
    pub model: Model,
    /// The print's pauses, lowest first, within the solid's height; the 3MF
    /// export carries them. Set with [`Rendered::with_pauses`].
    pub pauses: Vec<c2f_eval::Pause>,
}

impl Rendered {
    /// Keeps the pauses the evaluation asked for that fall inside the
    /// solid's height, lowest first and one per height, and warns about the
    /// others. A 2D or empty model is not printed, so it keeps none.
    pub fn with_pauses(mut self, mut pauses: Vec<c2f_eval::Pause>) -> Self {
        let Model::Solid(manifold) = &self.model else { return self };
        let b = manifold.bounding_box();
        pauses.retain(|pause| {
            let inside = pause.z > b.min.z && pause.z < b.max.z;
            if !inside {
                self.warnings.push(format!(
                    "Ignoring pause at z = {}: the model spans z = {} to {}",
                    pause.z, b.min.z, b.max.z
                ));
            }
            inside
        });
        pauses.sort_by(|a, b| a.z.total_cmp(&b.z));
        pauses.dedup_by(|later, first| later.z == first.z);
        self.pauses = pauses;
        self
    }

    /// The model's smallest and largest corner, at z 0 for a 2D model;
    /// `None` when it is empty.
    pub fn bounds(&self) -> Option<([f64; 3], [f64; 3])> {
        match &self.model {
            _ if self.model.is_empty() => None,
            Model::Solid(m) => {
                let b = m.bounding_box();
                Some(([b.min.x, b.min.y, b.min.z], [b.max.x, b.max.y, b.max.z]))
            }
            Model::Flat(c) => {
                let b = c.bounds();
                Some(([b.min.x, b.min.y, 0.0], [b.max.x, b.max.y, 0.0]))
            }
            Model::Empty => None,
        }
    }

    /// The model as an ASCII STL file, laid out as the reference
    /// implementation writes them. `None` for an empty or 2D model.
    ///
    /// Numbers use Rust's debug notation, which switches to an exponent for
    /// tiny and huge values: plain notation would spell a 1e-31 normal
    /// component with 48 characters, and slicers built on admesh
    /// (PrusaSlicer, Bambu Studio) read each one into 31 and reject the file.
    pub fn stl(&self) -> Option<String> {
        let Model::Solid(manifold) = &self.model else { return None };
        let mesh = manifold.get_mesh_gl64(-1);
        if mesh.tri_verts.is_empty() {
            return None;
        }
        let stride = mesh.num_prop as usize;
        let vertex = |i: u64| {
            let i = i as usize * stride;
            [mesh.vert_properties[i], mesh.vert_properties[i + 1], mesh.vert_properties[i + 2]]
        };
        let mut out = String::from("solid Code2Form\n");
        for tri in mesh.tri_verts.chunks(3) {
            let [a, b, c] = [vertex(tri[0]), vertex(tri[1]), vertex(tri[2])];
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(f64::MIN_POSITIVE);
            let _ = writeln!(out, "  facet normal {:?} {:?} {:?}", n[0] / length, n[1] / length, n[2] / length);
            out.push_str("    outer loop\n");
            for p in [a, b, c] {
                let _ = writeln!(out, "      vertex {:?} {:?} {:?}", p[0], p[1], p[2]);
            }
            out.push_str("    endloop\n  endfacet\n");
        }
        out.push_str("endsolid Code2Form\n");
        Some(out)
    }
}
