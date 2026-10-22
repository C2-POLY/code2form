//! Selecting the objects of a model: what lies under the pointer in the 3D
//! view, found along the ray through it, and the statement of the code that
//! made it.
//!
//! Every drawn triangle knows the instantiation that made it (its origin,
//! see [`c2f_eval::Origin`]), and each instantiation the one it ran in. A
//! click picks the nearest triangle, then walks up that chain to the
//! statement it means: the outermost call of a module written in the file
//! (a module is an object of its own, as a component is in Fusion), else
//! the innermost shape. A double-click goes all the way in.
//!
//! An object is a statement, kept as its [`Statement`]: a statement run several
//! times (in a loop, or in a module called twice) selects every copy.

use std::ops::Range;

use c2f_eval::{CallKind, Origin};
use c2f_geom::RenderMesh;
use glam::Vec3;

use crate::pick::Ray;

/// A statement of the program: the file holding it and its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Statement {
    /// The file of the program: 0 for the main file and the files it
    /// includes, which `span.file` tells apart.
    pub unit: usize,
    pub span: c2f_lang::Span,
}

impl Statement {
    fn of(origin: &Origin) -> Statement {
        Statement { unit: origin.unit, span: origin.span }
    }

    /// Whether it is written in the main file itself, where the editor can
    /// change it; other statements are read-only.
    pub fn editable(&self) -> bool {
        self.unit == 0 && self.span.file == 0
    }

    /// Its bytes in the main file, when it is written there.
    pub fn range(&self) -> Option<Range<usize>> {
        self.editable().then(|| self.span.range())
    }
}

/// Which of the scene's meshes a triangle is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Solid,
    /// `#` parts.
    Highlights,
    /// `%` parts.
    Backgrounds,
    /// What differences subtract, which is not drawn.
    Cutters,
}

const LAYERS: [Layer; 4] = [Layer::Solid, Layer::Highlights, Layer::Backgrounds, Layer::Cutters];

/// A triangle the ray meets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub layer: Layer,
    pub triangle: usize,
    /// How far along the ray.
    pub distance: f32,
    /// Its origin, an index into [`Picking::origins`].
    pub origin: Option<u32>,
}

/// The triangles of one mesh, with a bounding volume hierarchy over them.
#[derive(Default)]
struct Triangles {
    positions: Vec<[f32; 3]>,
    origins: Vec<u32>,
    bvh: Bvh,
}

impl Triangles {
    fn new(mesh: &RenderMesh) -> Self {
        let mut origins = mesh.origins.clone();
        origins.resize(mesh.triangle_count(), 0);
        let positions = mesh.positions.clone();
        let bvh = Bvh::build(&positions);
        Triangles { positions, origins, bvh }
    }

    fn corners(&self, triangle: usize) -> [Vec3; 3] {
        let p = &self.positions[triangle * 3..triangle * 3 + 3];
        [Vec3::from(p[0]), Vec3::from(p[1]), Vec3::from(p[2])]
    }

    fn origin(&self, triangle: usize) -> Option<u32> {
        self.origins.get(triangle).and_then(|o| o.checked_sub(1))
    }

    fn len(&self) -> usize {
        self.positions.len() / 3
    }
}

/// What picking needs of a scene, built once on the render thread.
#[derive(Default)]
pub struct Picking {
    pub origins: Vec<Origin>,
    layers: [Triangles; 4],
}

impl Picking {
    pub fn new(origins: Vec<Origin>, solid: &RenderMesh, highlights: &RenderMesh, backgrounds: &RenderMesh, cutters: &RenderMesh) -> Self {
        Picking {
            origins,
            layers: [solid, highlights, backgrounds, cutters].map(Triangles::new),
        }
    }

    fn layer(&self, layer: Layer) -> &Triangles {
        &self.layers[layer as usize]
    }

    /// The nearest triangle along the ray, below the layer view's cut if
    /// there is one. `hidden` looks only at what is not drawn solid: the
    /// shapes differences subtract and the `%` parts, to reach a hole's
    /// shape through the walls around it.
    pub fn cast(&self, ray: &Ray, cut: Option<f32>, hidden: bool) -> Option<Hit> {
        let layers: &[Layer] = if hidden { &[Layer::Cutters, Layer::Backgrounds] } else { &LAYERS[..3] };
        let mut best: Option<Hit> = None;
        for &layer in layers {
            let triangles = self.layer(layer);
            let limit = best.map_or(f32::INFINITY, |b| b.distance);
            if let Some((triangle, distance)) = triangles.bvh.cast(&triangles.positions, ray, cut, limit) {
                best = Some(Hit { layer, triangle, distance, origin: triangles.origin(triangle) });
            }
        }
        best
    }

    /// The statement a click on a triangle of `origin` means; `deep` goes
    /// past modules to the innermost shape.
    pub fn target(&self, origin: u32, deep: bool) -> Option<Statement> {
        let chain: Vec<&Origin> =
            std::iter::successors(Some(origin), |&o| self.origins.get(o as usize)?.parent)
                .filter_map(|o| self.origins.get(o as usize))
                .filter(|o| o.kind != CallKind::Flow)
                .collect();
        let written: Vec<&&Origin> = chain.iter().filter(|o| o.in_main_file()).collect();
        let chosen = match written.first() {
            // Made by a library or an included file only.
            None => chain.first()?,
            Some(innermost) if deep => innermost,
            Some(innermost) => written.iter().rev().find(|o| o.kind == CallKind::Module).unwrap_or(innermost),
        };
        Some(Statement::of(chosen))
    }

    /// The statement of the main file at byte `offset`: the innermost one
    /// that ran and holds it.
    pub fn at_offset(&self, offset: usize) -> Option<Statement> {
        self.origins
            .iter()
            .filter(|o| o.in_main_file() && o.kind != CallKind::Flow && o.span.range().contains(&offset))
            .min_by_key(|o| o.span.end - o.span.start)
            .map(Statement::of)
    }

    /// The point and the normal (facing the ray) of a hit.
    #[allow(dead_code)] // Until the features of manual modelling use it.
    pub fn surface(&self, ray: &Ray, hit: &Hit) -> (Vec3, Vec3) {
        let [a, b, c] = self.layer(hit.layer).corners(hit.triangle);
        let normal = (b - a).cross(c - a).normalize_or(Vec3::Z);
        let normal = if normal.dot(ray.direction) > 0.0 { -normal } else { normal };
        (ray.origin + ray.direction * hit.distance, normal)
    }

    /// The box around every triangle made by `keys` (drawn, `#` and `%`
    /// parts, and what differences subtract).
    #[allow(dead_code)] // Until the features of manual modelling use it.
    pub fn bounds(&self, keys: &[Statement]) -> Option<(Vec3, Vec3)> {
        let marked = self.marked(keys);
        let mut found: Option<(Vec3, Vec3)> = None;
        for &layer in &LAYERS {
            let triangles = self.layer(layer);
            for t in 0..triangles.len() {
                if triangles.origin(t).is_some_and(|o| marked.get(o as usize).copied().unwrap_or(false)) {
                    for p in triangles.corners(t) {
                        found = Some(found.map_or((p, p), |(lo, hi)| (lo.min(p), hi.max(p))));
                    }
                }
            }
        }
        found
    }

    /// Whether the scene still has a run of the statement `key`.
    pub fn contains(&self, key: &Statement) -> bool {
        self.origins.iter().any(|o| Statement::of(o) == *key)
    }

    /// For each origin, whether it or an instantiation it ran in is one of
    /// `keys`. An origin's parent always comes before it.
    fn marked(&self, keys: &[Statement]) -> Vec<bool> {
        let mut marked = vec![false; self.origins.len()];
        if keys.is_empty() {
            return marked;
        }
        for (i, origin) in self.origins.iter().enumerate() {
            marked[i] = keys.contains(&Statement::of(origin)) || origin.parent.is_some_and(|p| marked[p as usize]);
        }
        marked
    }

    /// The triangles made by `selected`, then by `previewed` (hovered, or
    /// under the editor's cursor), in the colors the view draws them over
    /// the model with. A selected shape that is not drawn (what a
    /// difference subtracts) shows see-through, as `%` would.
    pub fn highlight(&self, selected: &[Statement], previewed: &[Statement]) -> RenderMesh {
        let mut out = RenderMesh::default();
        for (keys, shown, hidden) in [(selected, SELECTED, SELECTED_HIDDEN), (previewed, PREVIEWED, PREVIEWED_HIDDEN)] {
            let keys: Vec<Statement> = match shown == PREVIEWED {
                // What is selected already shows as such.
                true => keys.iter().filter(|k| !selected.contains(k)).copied().collect(),
                false => keys.to_vec(),
            };
            let marked = self.marked(&keys);
            if !marked.contains(&true) {
                continue;
            }
            for &layer in &LAYERS {
                let triangles = self.layer(layer);
                let color = if layer == Layer::Cutters { hidden } else { shown };
                for t in 0..triangles.len() {
                    if triangles.origin(t).is_some_and(|o| marked.get(o as usize).copied().unwrap_or(false)) {
                        out.push_triangle(triangles.corners(t).map(|c| c.to_array()), [color; 3]);
                    }
                }
            }
        }
        out
    }
}

const SELECTED: [f32; 4] = [0.22, 0.56, 1.0, 0.55];
const SELECTED_HIDDEN: [f32; 4] = [0.22, 0.56, 1.0, 0.3];
const PREVIEWED: [f32; 4] = [0.45, 0.72, 1.0, 0.3];
const PREVIEWED_HIDDEN: [f32; 4] = [0.45, 0.72, 1.0, 0.18];

/// A bounding volume hierarchy over triangles, split at the median of the
/// longest axis: a ray then meets a few dozen boxes instead of every
/// triangle.
#[derive(Default)]
struct Bvh {
    nodes: Vec<BvhNode>,
    /// Triangle indices, each leaf a run of them.
    order: Vec<u32>,
}

#[derive(Clone, Copy)]
struct BvhNode {
    lo: Vec3,
    hi: Vec3,
    /// A leaf's first index into `order`, or an inner node's first child
    /// (its second is the node after it).
    start: u32,
    /// How many triangles a leaf holds; 0 for an inner node.
    count: u32,
}

const LEAF_SIZE: usize = 4;

impl Bvh {
    fn build(positions: &[[f32; 3]]) -> Bvh {
        let count = positions.len() / 3;
        let mut bvh = Bvh { nodes: Vec::with_capacity(2 * count / LEAF_SIZE + 1), order: (0..count as u32).collect() };
        if count == 0 {
            return bvh;
        }
        let corners = |t: u32| {
            let t = t as usize * 3;
            [Vec3::from(positions[t]), Vec3::from(positions[t + 1]), Vec3::from(positions[t + 2])]
        };
        let centroids: Vec<Vec3> = (0..count as u32).map(|t| corners(t).into_iter().sum::<Vec3>() / 3.0).collect();
        // Depth-first with an explicit stack: (node, start, end).
        let mut stack = vec![(0usize, 0usize, count)];
        bvh.nodes.push(BvhNode { lo: Vec3::ZERO, hi: Vec3::ZERO, start: 0, count: 0 });
        while let Some((node, start, end)) = stack.pop() {
            let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
            let (mut clo, mut chi) = (lo, hi);
            for &t in &bvh.order[start..end] {
                for c in corners(t) {
                    lo = lo.min(c);
                    hi = hi.max(c);
                }
                clo = clo.min(centroids[t as usize]);
                chi = chi.max(centroids[t as usize]);
            }
            let extent = chi - clo;
            if end - start <= LEAF_SIZE || extent.max_element() <= 0.0 {
                bvh.nodes[node] = BvhNode { lo, hi, start: start as u32, count: (end - start) as u32 };
                continue;
            }
            let axis = if extent.x >= extent.y && extent.x >= extent.z { 0 } else if extent.y >= extent.z { 1 } else { 2 };
            let middle = (start + end) / 2;
            bvh.order[start..end].select_nth_unstable_by(middle - start, |&a, &b| {
                centroids[a as usize][axis].total_cmp(&centroids[b as usize][axis])
            });
            let left = bvh.nodes.len();
            bvh.nodes.push(BvhNode { lo: Vec3::ZERO, hi: Vec3::ZERO, start: 0, count: 0 });
            let right = bvh.nodes.len();
            bvh.nodes.push(BvhNode { lo: Vec3::ZERO, hi: Vec3::ZERO, start: 0, count: 0 });
            bvh.nodes[node] = BvhNode { lo, hi, start: left as u32, count: 0 };
            stack.push((right, middle, end));
            stack.push((left, start, middle));
        }
        bvh
    }

    /// The nearest triangle the ray meets closer than `limit`, with the
    /// distance, ignoring the parts above `cut`.
    fn cast(&self, positions: &[[f32; 3]], ray: &Ray, cut: Option<f32>, limit: f32) -> Option<(usize, f32)> {
        let root = self.nodes.first()?;
        let inverse = ray.direction.recip();
        let mut best: Option<(usize, f32)> = None;
        let mut nearest = limit;
        let mut stack = Vec::with_capacity(64);
        if enters(root, ray.origin, inverse).is_some_and(|t| t < nearest) {
            stack.push(0u32);
        }
        while let Some(index) = stack.pop() {
            let node = self.nodes[index as usize];
            if enters(&node, ray.origin, inverse).is_none_or(|t| t >= nearest) {
                continue;
            }
            if node.count > 0 {
                for &t in &self.order[node.start as usize..(node.start + node.count) as usize] {
                    let i = t as usize * 3;
                    let corners = [Vec3::from(positions[i]), Vec3::from(positions[i + 1]), Vec3::from(positions[i + 2])];
                    if let Some(d) = intersect(ray.origin, ray.direction, corners) {
                        let above = cut.is_some_and(|z| (ray.origin + ray.direction * d).z > z + 1e-4);
                        if d < nearest && !above {
                            nearest = d;
                            best = Some((t as usize, d));
                        }
                    }
                }
            } else {
                // The nearer child last, to be looked at first.
                let (a, b) = (node.start, node.start + 1);
                let da = enters(&self.nodes[a as usize], ray.origin, inverse);
                let db = enters(&self.nodes[b as usize], ray.origin, inverse);
                match (da, db) {
                    (Some(da), Some(db)) if da <= db => stack.extend([b, a]),
                    (Some(_), Some(_)) => stack.extend([a, b]),
                    (Some(_), None) => stack.push(a),
                    (None, Some(_)) => stack.push(b),
                    (None, None) => {}
                }
            }
        }
        best
    }
}

/// The distance along the ray where it enters the node's box.
fn enters(node: &BvhNode, origin: Vec3, inverse: Vec3) -> Option<f32> {
    let (t1, t2) = ((node.lo - origin) * inverse, (node.hi - origin) * inverse);
    // NaN (a ray along a face of a flat box) compares false: min and max
    // then keep the other value.
    let near = t1.min(t2).max_element();
    let far = t1.max(t2).min_element();
    (far >= near.max(0.0)).then_some(near.max(0.0))
}

/// Möller–Trumbore: the distance along the ray to the triangle, from either
/// side.
fn intersect(origin: Vec3, direction: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = direction.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(-1e-6..=1.0 + 1e-6).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = direction.dot(q) * inv;
    if v < -1e-6 || u + v > 1.0 + 1e-6 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 1e-6).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picking(src: &'static str) -> Picking {
        let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
        let output = c2f_eval::evaluate(&program);
        let r = c2f_geom::render(&output.nodes);
        Picking::new(output.origins, &r.solid, &r.highlights, &r.backgrounds, &r.cutters)
    }

    /// A ray straight down onto (`x`, `y`).
    fn down(x: f32, y: f32) -> Ray {
        Ray { origin: Vec3::new(x, y, 100.0), direction: -Vec3::Z, slack: 0.0 }
    }

    fn text(src: &'static str, key: Statement) -> &'static str {
        &src[key.range().expect("in the main file")]
    }

    fn click(p: &Picking, src: &'static str, ray: Ray, hidden: bool, deep: bool) -> Option<&'static str> {
        let hit = p.cast(&ray, None, hidden)?;
        Some(text(src, p.target(hit.origin?, deep)?))
    }

    const BOX: &str = "module post() { cylinder(h = 8, r = 1, $fn = 16); }\n\
        difference() {\n    cube([20, 20, 5]);\n    translate([10, 10, -1]) cylinder(h = 7, r = 3, $fn = 24);\n}\n\
        translate([30, 0, 0]) post();\n\
        for (i = [0:1]) translate([40 + i * 5, 0, 0]) sphere(1, $fn = 12);";

    #[test]
    fn a_click_finds_the_statement_under_the_pointer() {
        let p = picking(BOX);
        assert_eq!(click(&p, BOX, down(2.0, 2.0), false, false), Some("cube([20, 20, 5]);"));
        // A module written in the file is one object; a double-click goes in.
        assert_eq!(click(&p, BOX, down(30.0, 0.0), false, false), Some("post();"));
        assert_eq!(click(&p, BOX, down(30.0, 0.0), false, true), Some("cylinder(h = 8, r = 1, $fn = 16);"));
        // Loops are not objects: the shape inside is.
        assert_eq!(click(&p, BOX, down(45.0, 0.0), false, false), Some("sphere(1, $fn = 12);"));
        assert_eq!(click(&p, BOX, down(60.0, 60.0), false, false), None);
    }

    #[test]
    fn the_shape_of_a_hole_is_picked_through_it_or_with_alt() {
        let p = picking(BOX);
        let hole = "cylinder(h = 7, r = 3, $fn = 24);";
        // Straight down the hole, the ray meets nothing of the solid; at an
        // angle, it meets the hole's wall, which the cylinder made.
        let slanted = Ray { origin: Vec3::new(10.0, 10.0, 6.0), direction: Vec3::new(1.0, 0.0, -1.0).normalize(), slack: 0.0 };
        assert_eq!(click(&p, BOX, slanted, false, false), Some(hole));
        // Alt reaches it from anywhere above, through the cube's top.
        let above = Ray { origin: Vec3::new(10.0, 11.0, 50.0), direction: -Vec3::Z, slack: 0.0 };
        assert_eq!(click(&p, BOX, above, true, false), Some(hole));
    }

    #[test]
    fn the_cut_of_the_layer_view_hides_what_is_above() {
        let src = "cube(10);";
        let p = picking(src);
        let hit = p.cast(&down(5.0, 5.0), Some(4.0), false);
        // The top is above the cut: the ray reaches the bottom face.
        assert!(hit.is_some_and(|h| (h.distance - 100.0).abs() < 1e-3), "{hit:?}");
    }

    #[test]
    fn the_cursor_finds_the_innermost_statement() {
        let p = picking(BOX);
        let at = |needle: &str| p.at_offset(BOX.find(needle).unwrap() + 1).map(|k| text(BOX, k));
        assert_eq!(at("cube(["), Some("cube([20, 20, 5]);"));
        assert_eq!(at("translate([10"), Some("translate([10, 10, -1]) cylinder(h = 7, r = 3, $fn = 24);"));
        assert_eq!(at("post();"), Some("post();"));
        assert_eq!(at("module"), None);
    }

    #[test]
    fn a_highlight_covers_every_run_of_a_statement() {
        let p = picking(BOX);
        let sphere = p.at_offset(BOX.find("sphere").unwrap()).unwrap();
        let mesh = p.highlight(&[sphere], &[]);
        let spheres = p.layer(Layer::Solid);
        let expected = (0..spheres.len()).filter(|&t| p.target(spheres.origin(t).unwrap(), true) == Some(sphere)).count();
        assert_eq!(mesh.triangle_count(), expected);
        assert!(expected > 0);
        // Both copies of the loop.
        let (lo, hi) = mesh.bounds().unwrap();
        assert!(lo[0] < 40.0 && hi[0] > 45.0, "{lo:?} {hi:?}");
        // The hole's shape shows although it is not drawn.
        let hole = p.at_offset(BOX.find("cylinder(h = 7").unwrap()).unwrap();
        assert!(p.highlight(&[hole], &[]).colors.contains(&SELECTED_HIDDEN));
    }

    #[test]
    fn an_included_statement_is_read_only() {
        let dir = std::env::temp_dir().join(format!("c2f-select-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("part.scad"), "cube(3);\n").unwrap();
        let program = c2f_eval::Program::load("include <part.scad>", &dir.join("main.scad"), &[]).expect("parses");
        let output = c2f_eval::evaluate(&program);
        std::fs::remove_dir_all(&dir).ok();
        let r = c2f_geom::render(&output.nodes);
        let p = Picking::new(output.origins, &r.solid, &r.highlights, &r.backgrounds, &r.cutters);
        let hit = p.cast(&down(1.0, 1.0), None, false).unwrap();
        let key = p.target(hit.origin.unwrap(), false).unwrap();
        assert!(!key.editable());
        assert_eq!(key.range(), None);
    }

    #[test]
    fn picking_is_fast_on_half_a_million_triangles() {
        // A grid of 250 000 quads.
        let n = 500;
        let mut mesh = RenderMesh::default();
        for i in 0..n {
            for j in 0..n {
                let (x, y) = (i as f32, j as f32);
                let z = ((x * 0.1).sin() + (y * 0.1).cos()) * 3.0;
                let q = [[x, y, z], [x + 1.0, y, z], [x + 1.0, y + 1.0, z], [x, y + 1.0, z]];
                mesh.push_triangle_from([q[0], q[1], q[2]], [[1.0; 4]; 3], Some(0));
                mesh.push_triangle_from([q[0], q[2], q[3]], [[1.0; 4]; 3], Some(0));
            }
        }
        assert_eq!(mesh.triangle_count(), 500_000);
        let empty = RenderMesh::default();
        let p = Picking::new(Vec::new(), &mesh, &empty, &empty, &empty);
        let rays: Vec<Ray> = (0..200)
            .map(|k| {
                let target = Vec3::new((k * 37 % n) as f32 + 0.3, (k * 91 % n) as f32 + 0.6, 0.0);
                let origin = Vec3::new(-50.0, -80.0, 200.0);
                Ray { origin, direction: (target - origin).normalize(), slack: 0.0 }
            })
            .collect();
        let start = std::time::Instant::now();
        let hits: Vec<Option<Hit>> = rays.iter().map(|r| p.cast(r, None, false)).collect();
        let each = start.elapsed() / rays.len() as u32;
        // The same triangles as trying them all.
        let solid = p.layer(Layer::Solid);
        for (ray, hit) in rays.iter().zip(&hits).step_by(20) {
            let brute = (0..solid.len())
                .filter_map(|t| intersect(ray.origin, ray.direction, solid.corners(t)).map(|d| (t, d)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            assert_eq!(hit.map(|h| h.triangle), brute.map(|b| b.0));
        }
        assert!(hits.iter().flatten().count() > 100);
        // Well under a frame (16 ms), even in a debug build.
        assert!(each < std::time::Duration::from_millis(16), "{each:?} per pick");
    }
}
