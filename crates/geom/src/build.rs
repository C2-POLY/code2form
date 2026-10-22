//! From CSG nodes to geometry. Solids are Manifold meshes whose vertices
//! carry their color (four extra properties), so colors survive booleans;
//! 2D shapes are Manifold cross-sections.

use std::path::{Path, PathBuf};

use manifold_rust::cross_section::CrossSection;
use manifold_rust::linalg::{Mat3x4, Vec2, Vec3};
use manifold_rust::manifold::Manifold;
use manifold_rust::types::{MeshGL64, OpType, Polygons};
use c2f_eval::node::{FileSource, Matrix, Node, NodeKind, OffsetOp, Points, Segments};

use crate::import::{self, DxfParams, Imported};
use crate::mesh::{RenderMesh, Rendered, Stats};
use crate::offset;
use crate::primitives::{self, TriMesh, fragments};

/// The default preview colors of the reference implementation (its
/// "Cornfield" scheme).
const DEFAULT_COLOR: [f32; 4] = [0.976, 0.843, 0.173, 1.0];
/// Surfaces cut by a `difference`. (0.318 is 81/255, not 1/π.)
#[allow(clippy::approx_constant)]
const CUT_COLOR: [f32; 4] = [0.616, 0.796, 0.318, 1.0];
#[allow(clippy::approx_constant)]
const HIGHLIGHT_COLOR: [f32; 4] = [1.0, 0.318, 0.318, 0.5];
const BACKGROUND_COLOR: [f32; 4] = [0.706, 0.706, 0.706, 0.3];
/// Extra vertex properties: the color.
const COLOR_PROPS: usize = 4;

/// The geometry of a whole program, or of one node.
#[derive(Clone, Default)]
pub enum Model {
    #[default]
    Empty,
    Solid(Manifold),
    Flat(CrossSection),
}

impl Model {
    pub(crate) fn is_empty(&self) -> bool {
        match self {
            Model::Empty => true,
            Model::Solid(m) => m.is_empty(),
            Model::Flat(c) => c.is_empty(),
        }
    }
}

/// Renders top-level nodes: the model, its `#` and `%` parts, and stats.
/// Data files (`import()`, `surface()`, the extrusions' `file =`) are
/// read relative to the current directory; see [`render_in`].
pub fn render(nodes: &[Node]) -> Rendered {
    render_in(nodes, Path::new(""))
}

/// [`render`], reading data files relative to `base_dir`: the directory of
/// the main `.scad` file (`Program::files[0].path.parent()`), against which
/// `c2f_eval` names them in the nodes (a used library's files too).
/// Absolute names are read as they are.
pub fn render_in(nodes: &[Node], base_dir: &Path) -> Rendered {
    let mut builder = Builder { base_dir: base_dir.to_path_buf(), ..Builder::default() };
    let ctx = Ctx { color: None, cut: false };
    let parts: Vec<Model> = nodes.iter().map(|n| builder.node(n, ctx)).collect();
    let model = builder.union(parts);
    let is_2d = matches!(model, Model::Flat(_));
    let solid = display(&model, None);
    let stats = match &model {
        Model::Solid(m) => Stats { triangles: m.num_tri(), vertices: m.num_vert(), volume: m.volume() },
        Model::Flat(c) => Stats { triangles: 0, vertices: c.num_vert(), volume: c.area() },
        Model::Empty => Stats::default(),
    };
    let mut highlights = RenderMesh::default();
    for part in &builder.highlights {
        highlights.append(display(part, Some(HIGHLIGHT_COLOR)));
    }
    let mut backgrounds = RenderMesh::default();
    for part in &builder.backgrounds {
        backgrounds.append(display(part, Some(BACKGROUND_COLOR)));
    }
    Rendered { solid, highlights, backgrounds, is_2d, stats, warnings: builder.warnings, model, pauses: Vec::new() }
}

#[derive(Clone, Copy)]
struct Ctx {
    /// The innermost `color()` around this node; in the reference
    /// release (2021.01) the outermost wins, so it is only set when
    /// still unset.
    color: Option<[f32; 4]>,
    /// Inside the subtracted children of a `difference`.
    cut: bool,
}

/// Whether neither `node` nor any node under it is marked `#` or `%`.
fn unmarked(node: &Node) -> bool {
    !node.highlight && !node.background && node.children.iter().all(unmarked)
}

impl Ctx {
    fn paint(&self) -> [f32; 4] {
        self.color.unwrap_or(if self.cut { CUT_COLOR } else { DEFAULT_COLOR })
    }
}

#[derive(Default)]
struct Builder {
    warnings: Vec<String>,
    highlights: Vec<Model>,
    backgrounds: Vec<Model>,
    /// Where data files named by relative paths are.
    base_dir: PathBuf,
    /// The `projection()`s built so far: a design that outlines the same
    /// solid twice, as a fence around a part does, builds it once. Their
    /// outlines carry no color, so the context they are built in does not
    /// matter.
    projections: Vec<(Node, Model)>,
}

impl Builder {
    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }

    fn node(&mut self, node: &Node, ctx: Ctx) -> Model {
        let model = self.node_inner(node, ctx);
        if node.background {
            // Shown, but not part of the model.
            self.backgrounds.push(model);
            return Model::Empty;
        }
        if node.highlight {
            self.highlights.push(model.clone());
        }
        model
    }

    fn children(&mut self, node: &Node, ctx: Ctx) -> Vec<Model> {
        node.children.iter().map(|child| self.node(child, ctx)).collect()
    }

    fn node_inner(&mut self, node: &Node, ctx: Ctx) -> Model {
        match &node.kind {
            NodeKind::Group | NodeKind::Union | NodeKind::Render { .. } => {
                let parts = self.children(node, ctx);
                self.union(parts)
            }
            NodeKind::Difference => self.difference(node, ctx),
            NodeKind::Intersection => {
                let parts = self.children(node, ctx);
                self.intersection(parts)
            }
            NodeKind::Hull => {
                let parts = self.children(node, ctx);
                self.hull(parts, ctx)
            }
            NodeKind::Minkowski { .. } => {
                let parts = self.children(node, ctx);
                self.minkowski(parts, ctx)
            }
            NodeKind::Color(rgba) => {
                let color = if rgba[0] < 0.0 { None } else { Some(rgba.map(|c| c as f32)) };
                let ctx = Ctx { color: ctx.color.or(color), ..ctx };
                let parts = self.children(node, ctx);
                self.union(parts)
            }
            NodeKind::Transform(m) => {
                let parts = self.children(node, ctx);
                let model = self.union(parts);
                transform(model, m)
            }
            NodeKind::Cube { size, center } => solid(primitives::cube(*size, *center), ctx),
            NodeKind::Sphere { r, segments } => {
                solid(primitives::sphere(*r, frags(*r, segments)), ctx)
            }
            NodeKind::Cylinder { h, r1, r2, center, segments } => {
                solid(primitives::cylinder(*h, *r1, *r2, *center, frags(r1.max(*r2), segments)), ctx)
            }
            NodeKind::Polyhedron { points, faces, .. } => self.polyhedron(points, faces, ctx),
            NodeKind::Square { size, center } => {
                let [x, y] = *size;
                if !(x > 0.0 && y > 0.0) {
                    return Model::Empty;
                }
                Model::Flat(CrossSection::square_vec2(Vec2::new(x, y), *center))
            }
            NodeKind::Circle { r, segments } => {
                if !(r.is_finite() && *r > 0.0) {
                    return Model::Empty;
                }
                let points = primitives::circle(*r, frags(*r, segments));
                Model::Flat(CrossSection::new(vec![points.into_iter().map(|[x, y]| Vec2::new(x, y)).collect()]))
            }
            NodeKind::Polygon { points, paths, .. } => self.polygon(points, paths),
            NodeKind::Offset { op, segments } => {
                let parts = self.children(node, ctx);
                let Some(section) = self.flat(parts, "offset") else { return Model::Empty };
                Model::Flat(match *op {
                    OffsetOp::Round { r } => offset::round(&section, r, frags(r.abs(), segments)),
                    // chamfer cuts corners square; otherwise they are mitered.
                    OffsetOp::Delta { delta, chamfer } => {
                        section.offset_with_params(delta, if chamfer { 0 } else { 2 }, 2.0, 0)
                    }
                })
            }
            NodeKind::LinearExtrude { source, height, center, twist, slices, scale, segments, .. } => {
                let section = match source {
                    // The reference scales the drawing by the top scale's x.
                    Some(source) => self.file_source(source, scale[0], segments),
                    None => {
                        let parts = self.children(node, ctx);
                        self.flat(parts, "linear_extrude")
                    }
                };
                let Some(section) = section else { return Model::Empty };
                if height.is_nan() || *height <= 0.0 {
                    return Model::Empty;
                }
                let polygons = section.to_polygons();
                let divisions = match slices {
                    Some(n) => *n,
                    None if *twist == 0.0 => 1,
                    None => {
                        // The farthest point, at the bottom or on the scaled top.
                        let reach = polygons
                            .iter()
                            .flatten()
                            .map(|p| p.x.hypot(p.y).max((p.x * scale[0]).hypot(p.y * scale[1])))
                            .fold(0.0, f64::max);
                        ((frags(reach, segments) as f64 * twist.abs() / 360.0).ceil().max(1.0)) as u32
                    }
                };
                let contours: Vec<Vec<[f64; 2]>> = polygons.iter().map(|c| c.iter().map(|p| [p.x, p.y]).collect()).collect();
                let cap: Vec<[u32; 3]> = manifold_rust::polygon::triangulate(&polygons, -1.0, true)
                    .iter()
                    .map(|t| [t.x as u32, t.y as u32, t.z as u32])
                    .collect();
                let mesh = primitives::linear_extrude(&contours, &cap, *height, *center, *twist, divisions, *scale);
                match mesh {
                    Some(mesh) => {
                        let mesh = mesh_gl(&mesh, ctx.paint());
                        let strict = Manifold::from_mesh_gl64(&mesh);
                        // A top scaled to a point leaves degenerate faces.
                        Model::Solid(if strict.is_empty() { Manifold::from_mesh_gl64_robust(&mesh) } else { strict })
                    }
                    None => Model::Empty,
                }
            }
            NodeKind::RotateExtrude { source, angle, segments, .. } => {
                let section = match source {
                    Some(source) => self.file_source(source, source.scale, segments),
                    None => {
                        let parts = self.children(node, ctx);
                        self.flat(parts, "rotate_extrude")
                    }
                };
                let Some(section) = section else { return Model::Empty };
                let polygons = section.to_polygons();
                let xs = polygons.iter().flatten().map(|p| p.x);
                let (min_x, max_x) = xs.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), x| (lo.min(x), hi.max(x)));
                if min_x < 0.0 && max_x > 0.0 {
                    self.warn("all points for rotate_extrude() must have the same X coordinate sign");
                    return Model::Empty;
                }
                let polygons = if max_x <= 0.0 {
                    // Mirror shapes on the negative side over to the positive one.
                    polygons.iter().map(|p| p.iter().rev().map(|v| Vec2::new(-v.x, v.y)).collect()).collect()
                } else {
                    polygons
                };
                let reach = min_x.abs().max(max_x.abs());
                let n = ((frags(reach, segments) as f64 * angle.abs() / 360.0).ceil().max(1.0)) as i32;
                let mut m = Manifold::revolve(&polygons, n, angle.abs());
                if *angle < 0.0 {
                    m = m.mirror(Vec3::new(0.0, 1.0, 0.0));
                }
                colored(m, ctx)
            }
            NodeKind::Projection { cut, .. } => {
                // Marked children add to the highlights and backgrounds as
                // they are built, so only unmarked ones are reused.
                let reusable = unmarked(node);
                if reusable {
                    if let Some((_, model)) = self.projections.iter().find(|(n, _)| n == node) {
                        return model.clone();
                    }
                }
                let parts = self.children(node, ctx);
                let model = match self.union(parts) {
                    Model::Solid(m) => Model::Flat(if *cut { m.slice(0.0) } else { m.project() }),
                    other => other,
                };
                if reusable {
                    self.projections.push((node.clone(), model.clone()));
                }
                model
            }
            NodeKind::Resize { size, auto, .. } => {
                let parts = self.children(node, ctx);
                let model = self.union(parts);
                resize(model, *size, *auto)
            }
            NodeKind::Text(params) => {
                let mut warnings = Vec::new();
                let section = crate::text::render(params, &mut warnings);
                for warning in warnings {
                    self.warn(warning);
                }
                section.map_or(Model::Empty, Model::Flat)
            }
            NodeKind::Import { file, layer, origin, scale, segments, .. } => {
                let params = DxfParams { layer, origin: *origin, scale: *scale, segments: *segments };
                let mut warnings = Vec::new();
                let imported = import::import(&self.base_dir.join(file), file, &params, &mut warnings);
                warnings.into_iter().for_each(|w| self.warn(w));
                match imported {
                    Some(Imported::Flat(section)) => Model::Flat(section),
                    Some(Imported::Solid(meshes)) => self.meshes(meshes, ctx, &format!("import(\"{file}\")")),
                    None => Model::Empty,
                }
            }
            NodeKind::Surface { file, center, invert, .. } => {
                let mut warnings = Vec::new();
                let mesh = import::surface(&self.base_dir.join(file), file, *center, *invert, &mut warnings);
                warnings.into_iter().for_each(|w| self.warn(w));
                match mesh {
                    Some(mesh) => self.meshes(vec![mesh], ctx, &format!("surface(\"{file}\")")),
                    None => Model::Empty,
                }
            }
        }
    }

    /// The 2D union of `parts`, warning about 3D ones.
    fn flat(&mut self, parts: Vec<Model>, module: &str) -> Option<CrossSection> {
        let sections: Vec<CrossSection> = parts
            .into_iter()
            .filter_map(|part| match part {
                Model::Flat(c) => Some(c),
                Model::Solid(_) => {
                    self.warn(format!("{module}() ignores 3D children"));
                    None
                }
                Model::Empty => None,
            })
            .collect();
        if sections.is_empty() {
            return None;
        }
        Some(CrossSection::batch_boolean(&sections, OpType::Add))
    }

    /// Children of one dimension, the first's: the reference ignores the
    /// others.
    fn same_dimension(&mut self, parts: Vec<Model>) -> Vec<Model> {
        let parts: Vec<Model> = parts.into_iter().filter(|p| !p.is_empty()).collect();
        let Some(first) = parts.first() else { return parts };
        let solid = matches!(first, Model::Solid(_));
        let (kept, dropped): (Vec<Model>, Vec<Model>) =
            parts.into_iter().partition(|p| matches!(p, Model::Solid(_)) == solid);
        if !dropped.is_empty() {
            self.warn("Mixing 2D and 3D objects is not supported");
        }
        kept
    }

    fn union(&mut self, parts: Vec<Model>) -> Model {
        let parts = self.same_dimension(parts);
        combine(parts, OpType::Add)
    }

    fn intersection(&mut self, parts: Vec<Model>) -> Model {
        let parts: Vec<Model> = parts.into_iter().filter(|p| !p.is_empty()).collect();
        let parts = self.same_dimension(parts);
        combine(parts, OpType::Intersect)
    }

    fn difference(&mut self, node: &Node, ctx: Ctx) -> Model {
        let mut children = node.children.iter();
        let Some(first) = children.next() else { return Model::Empty };
        let base = self.node(first, ctx);
        let cutters: Vec<Model> = children.map(|c| self.node(c, Ctx { cut: true, ..ctx })).collect();
        let mut parts = vec![base];
        parts.extend(cutters);
        let mut parts = self.same_dimension(parts).into_iter();
        let Some(base) = parts.next() else { return Model::Empty };
        let rest: Vec<Model> = parts.collect();
        if rest.is_empty() {
            return base;
        }
        match (base, combine(rest, OpType::Add)) {
            (Model::Solid(a), Model::Solid(b)) => Model::Solid(a.difference(&b)),
            (Model::Flat(a), Model::Flat(b)) => Model::Flat(a.difference(&b)),
            (base, _) => base,
        }
    }

    fn hull(&mut self, parts: Vec<Model>, ctx: Ctx) -> Model {
        let parts = self.same_dimension(parts);
        match parts.first() {
            Some(Model::Solid(_)) => {
                let solids: Vec<Manifold> = parts.into_iter().filter_map(solid_of).collect();
                colored(Manifold::hull_manifolds(&solids), ctx)
            }
            Some(Model::Flat(_)) => {
                let sections: Vec<CrossSection> = parts.into_iter().filter_map(flat_of).collect();
                Model::Flat(CrossSection::hull_cross_sections(&sections))
            }
            _ => Model::Empty,
        }
    }

    fn minkowski(&mut self, parts: Vec<Model>, ctx: Ctx) -> Model {
        let parts = self.same_dimension(parts);
        let mut parts = parts.into_iter();
        let Some(first) = parts.next() else { return Model::Empty };
        let result = parts.fold(first, |acc, next| match (acc, next) {
            (Model::Solid(a), Model::Solid(b)) => Model::Solid(a.minkowski_sum(&b)),
            (Model::Flat(a), Model::Flat(b)) => Model::Flat(a.minkowski_sum(&b)),
            (acc, _) => acc,
        });
        match result {
            Model::Solid(m) => colored(m, ctx),
            other => other,
        }
    }

    fn polyhedron(&mut self, points: &Points, faces: &Points, ctx: Ctx) -> Model {
        let (Some(points), Some(faces)) = (&points.rows, &faces.rows) else {
            self.warn("polyhedron() needs points and faces");
            return Model::Empty;
        };
        let positions: Vec<[f64; 3]> = points
            .iter()
            .map(|p| [p.first().copied().unwrap_or(0.0), p.get(1).copied().unwrap_or(0.0), p.get(2).copied().unwrap_or(0.0)])
            .collect();
        let mut triangles = Vec::new();
        for face in faces {
            let indices: Vec<u32> = face.iter().map(|&i| i as u32).filter(|&i| (i as usize) < positions.len()).collect();
            // In the reference, faces run clockwise seen from outside.
            for j in 1..indices.len().saturating_sub(1) {
                triangles.push([indices[0], indices[j + 1], indices[j]]);
            }
        }
        if triangles.is_empty() {
            return Model::Empty;
        }
        // Faces listed the other way round turn the solid inside out; like
        // the reference, take it right side out.
        let signed_volume: f64 = triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| positions[i as usize]);
                a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])
            })
            .sum();
        if signed_volume < 0.0 {
            for t in &mut triangles {
                t.swap(1, 2);
            }
        }
        let mesh = mesh_gl(&TriMesh { positions, triangles }, ctx.paint());
        let manifold = Manifold::from_mesh_gl64_robust(&mesh);
        if manifold.is_empty() {
            self.warn("polyhedron() is not a closed solid");
            return Model::Empty;
        }
        Model::Solid(manifold)
    }

    fn polygon(&mut self, points: &Points, paths: &Points) -> Model {
        let Some(points) = &points.rows else {
            return Model::Empty;
        };
        let at = |i: usize| points.get(i).map(|p| Vec2::new(p.first().copied().unwrap_or(0.0), p.get(1).copied().unwrap_or(0.0)));
        let contours: Vec<Vec<Vec2>> = match &paths.rows {
            Some(paths) => paths.iter().map(|path| path.iter().filter_map(|&i| at(i as usize)).collect()).collect(),
            None => vec![(0..points.len()).filter_map(at).collect()],
        };
        // Paths combine even-odd: a path inside another is a hole.
        let mut result: Option<CrossSection> = None;
        for contour in contours.into_iter().filter(|c| c.len() >= 3) {
            let section = CrossSection::from_polygon_with_fill_rule(contour, 0);
            result = Some(match result {
                None => section,
                Some(acc) => acc.union(&section).difference(&acc.intersection(&section)),
            });
        }
        result.map_or(Model::Empty, Model::Flat)
    }
}

/// Data files.
impl Builder {
    /// The DXF drawing an extrusion's `file =` names.
    fn file_source(&mut self, source: &FileSource, scale: f64, segments: &Segments) -> Option<CrossSection> {
        let params = DxfParams { layer: &source.layer, origin: source.origin, scale, segments: *segments };
        let mut warnings = Vec::new();
        let section = import::dxf(&self.base_dir.join(&source.file), &source.file, &params, &mut warnings);
        warnings.into_iter().for_each(|w| self.warn(w));
        // The reference cleans this drawing with Clipper, unlike import()'s.
        section.map(import::without_collinear)
    }

    /// Imported meshes as one solid painted `ctx`'s color; imperfect ones
    /// go through Manifold's robust import.
    fn meshes(&mut self, meshes: Vec<TriMesh>, ctx: Ctx, what: &str) -> Model {
        let mut solids = Vec::new();
        for mesh in &meshes {
            let mesh = mesh_gl(mesh, ctx.paint());
            let strict = Manifold::from_mesh_gl64(&mesh);
            let solid = if strict.is_empty() { Manifold::from_mesh_gl64_robust(&mesh) } else { strict };
            if solid.is_empty() {
                self.warn(format!("{what} is not a closed solid"));
            } else {
                solids.push(Model::Solid(solid));
            }
        }
        combine(solids, OpType::Add)
    }
}

fn frags(r: f64, s: &Segments) -> u32 {
    fragments(r, s.fn_, s.fs, s.fa)
}

fn solid_of(model: Model) -> Option<Manifold> {
    match model {
        Model::Solid(m) => Some(m),
        _ => None,
    }
}

fn flat_of(model: Model) -> Option<CrossSection> {
    match model {
        Model::Flat(c) => Some(c),
        _ => None,
    }
}

fn combine(parts: Vec<Model>, op: OpType) -> Model {
    match parts.first() {
        None => Model::Empty,
        Some(Model::Solid(_)) => {
            let solids: Vec<Manifold> = parts.into_iter().filter_map(solid_of).collect();
            if solids.len() == 1 {
                return Model::Solid(solids.into_iter().next().unwrap());
            }
            Model::Solid(Manifold::batch_boolean(&solids, op))
        }
        Some(Model::Flat(_)) => {
            let sections: Vec<CrossSection> = parts.into_iter().filter_map(flat_of).collect();
            if sections.len() == 1 {
                return Model::Flat(sections.into_iter().next().unwrap());
            }
            Model::Flat(CrossSection::batch_boolean(&sections, op))
        }
        Some(Model::Empty) => Model::Empty,
    }
}

/// A mesh with every vertex painted `color`.
fn mesh_gl(mesh: &TriMesh, color: [f32; 4]) -> MeshGL64 {
    let mut mesh_gl = MeshGL64 { num_prop: 3 + COLOR_PROPS as u64, ..Default::default() };
    for p in &mesh.positions {
        mesh_gl.vert_properties.extend(p);
        mesh_gl.vert_properties.extend(color.map(f64::from));
    }
    mesh_gl.tri_verts = mesh.triangles.iter().flatten().map(|&i| i as u64).collect();
    mesh_gl
}

fn solid(mesh: Option<TriMesh>, ctx: Ctx) -> Model {
    match mesh {
        Some(mesh) => Model::Solid(Manifold::from_mesh_gl64(&mesh_gl(&mesh, ctx.paint()))),
        None => Model::Empty,
    }
}

/// Paints a whole solid one color (for shapes made by extrusion or hull,
/// which come without colors).
fn colored(m: Manifold, ctx: Ctx) -> Model {
    if m.is_empty() {
        return Model::Empty;
    }
    let color = ctx.paint().map(f64::from);
    Model::Solid(m.set_properties(COLOR_PROPS, move |props, _, _| props.copy_from_slice(&color)))
}

fn transform(model: Model, m: &Matrix) -> Model {
    match model {
        Model::Solid(solid) => {
            let col = |j: usize| Vec3::new(m[0][j], m[1][j], m[2][j]);
            Model::Solid(solid.transform(&Mat3x4::from_cols(col(0), col(1), col(2), col(3))))
        }
        Model::Flat(section) => {
            // 2D shapes take the x/y part of the matrix.
            let apply = |p: &Vec2| Vec2::new(m[0][0] * p.x + m[0][1] * p.y + m[0][3], m[1][0] * p.x + m[1][1] * p.y + m[1][3]);
            let flips = m[0][0] * m[1][1] - m[0][1] * m[1][0] < 0.0;
            let polygons: Polygons = section
                .to_polygons()
                .iter()
                .map(|contour| {
                    let mut moved: Vec<Vec2> = contour.iter().map(apply).collect();
                    if flips {
                        moved.reverse();
                    }
                    moved
                })
                .collect();
            Model::Flat(CrossSection::new(polygons))
        }
        Model::Empty => Model::Empty,
    }
}

fn resize(model: Model, size: [f64; 3], auto: [bool; 3]) -> Model {
    let (lo, hi) = match &model {
        Model::Solid(m) => {
            let b = m.bounding_box();
            ([b.min.x, b.min.y, b.min.z], [b.max.x, b.max.y, b.max.z])
        }
        Model::Flat(c) => {
            let r = c.bounds();
            ([r.min.x, r.min.y, 0.0], [r.max.x, r.max.y, 0.0])
        }
        Model::Empty => return model,
    };
    let mut scale = [1.0; 3];
    for i in 0..3 {
        let extent = hi[i] - lo[i];
        if size[i] > 0.0 && extent > 0.0 {
            scale[i] = size[i] / extent;
        }
    }
    let auto_scale = (0..3).filter(|&i| size[i] > 0.0).map(|i| scale[i]).fold(f64::NAN, f64::max);
    for i in 0..3 {
        if size[i] <= 0.0 && auto[i] && auto_scale.is_finite() {
            scale[i] = auto_scale;
        }
    }
    let mut m = c2f_eval::node::IDENTITY;
    for (i, s) in scale.iter().enumerate() {
        m[i][i] = *s;
    }
    transform(model, &m)
}

/// Flat-shaded triangles for drawing; `paint` overrides the colors.
fn display(model: &Model, paint: Option<[f32; 4]>) -> RenderMesh {
    let manifold = match model {
        Model::Solid(m) => m.clone(),
        // 2D shapes show as a 1-unit slab, as in the reference's preview.
        Model::Flat(c) => {
            let slab = Manifold::extrude(&c.to_polygons(), 1.0, 0, 0.0, Vec2::new(1.0, 1.0));
            match colored(slab, Ctx { color: None, cut: false }) {
                Model::Solid(m) => m,
                _ => return RenderMesh::default(),
            }
        }
        Model::Empty => return RenderMesh::default(),
    };
    let mesh = manifold.get_mesh_gl64(-1);
    let stride = mesh.num_prop as usize;
    let mut out = RenderMesh::default();
    for tri in mesh.tri_verts.chunks(3) {
        let corner = |i: u64| {
            let p = &mesh.vert_properties[i as usize * stride..(i as usize + 1) * stride];
            let color = match paint {
                Some(color) => color,
                None if stride >= 3 + COLOR_PROPS => [p[3] as f32, p[4] as f32, p[5] as f32, p[6] as f32],
                None => DEFAULT_COLOR,
            };
            ([p[0] as f32, p[1] as f32, p[2] as f32], color)
        };
        let [(a, ca), (b, cb), (c, cc)] = [corner(tri[0]), corner(tri[1]), corner(tri[2])];
        out.push_triangle([a, b, c], [ca, cb, cc]);
    }
    out
}
