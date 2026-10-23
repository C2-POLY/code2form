//! STEP export (ISO 10303-21, AP214): the format CAD programs exchange
//! solids in (Fusion 360, SolidWorks, FreeCAD, Onshape).
//!
//! A design is built as triangles, not as exact surfaces, so the STEP is
//! faceted: each solid is a boundary representation whose faces are
//! planar polygons. Neighbouring triangles in one plane and one color
//! become one face, its holes inner loops, so a cube has six faces and the
//! CAD program can pick, measure and extrude them; a cylinder stays a
//! prism of facets. Each closed shell of the mesh is a solid, a shell
//! inside another (a cavity) its void. Faces keep their colors.

use std::collections::HashMap;
use std::fmt::Write;

use crate::build::{CUT_COLOR, DEFAULT_COLOR, Model};
use crate::export::colored_mesh;
use crate::mesh::Rendered;

/// How a model is written to STEP.
#[derive(Clone, Debug, PartialEq)]
pub struct StepOptions {
    /// The product's name, which CAD programs show.
    pub name: String,
    /// The color of what has no `color()`, in sRGB.
    pub color: [f32; 3],
}

impl Default for StepOptions {
    fn default() -> Self {
        let [r, g, b, _] = DEFAULT_COLOR;
        StepOptions { name: "model".into(), color: [r, g, b] }
    }
}

impl Rendered {
    /// The solid as a STEP file. `None` for an empty or 2D model.
    pub fn step(&self, options: &StepOptions) -> Option<String> {
        let Model::Solid(manifold) = &self.model else { return None };
        let mesh = colored_mesh(manifold);
        if mesh.triangles.is_empty() {
            return None;
        }
        let [r, g, b] = options.color;
        let colors: Vec<[f32; 4]> = mesh
            .colors
            .iter()
            .map(|&c| if c == DEFAULT_COLOR || c == CUT_COLOR { [r, g, b, 1.0] } else { c })
            .collect();
        let faces = faces(&mesh.vertices, &mesh.triangles, &colors);
        Some(write(&options.name, &mesh.vertices, &faces))
    }
}

/// A planar face: its loops of vertices, the outer one first, counter-
/// clockwise seen from where its normal points, its holes clockwise.
struct Face {
    loops: Vec<Vec<usize>>,
    normal: [f64; 3],
    color: [f32; 4],
}

/// The triangles grouped into faces: neighbours across an edge, in one
/// plane and one color.
fn faces(vertices: &[[f64; 3]], triangles: &[[usize; 3]], colors: &[[f32; 4]]) -> Vec<Face> {
    let size = vertices.iter().fold(0.0f64, |m, p| p.iter().fold(m, |m, c| m.max(c.abs())));
    let tolerance = 1e-9 * size.max(1.0);
    let normals: Vec<[f64; 3]> = triangles.iter().map(|&[a, b, c]| normal(vertices[a], vertices[b], vertices[c])).collect();
    // The triangle on the other side of each edge.
    let mut across: HashMap<(usize, usize), usize> = HashMap::new();
    for (t, &[a, b, c]) in triangles.iter().enumerate() {
        for (from, to) in [(a, b), (b, c), (c, a)] {
            across.insert((from, to), t);
        }
    }
    let mut face_of = vec![usize::MAX; triangles.len()];
    let mut faces = Vec::new();
    for seed in 0..triangles.len() {
        if face_of[seed] != usize::MAX {
            continue;
        }
        // The plane of the largest triangle met so far would be steadier,
        // but the first one's is exact enough for meshes built in floats.
        let index = faces.len();
        let n = unit(normals[seed]).unwrap_or([0.0, 0.0, 1.0]);
        let origin = vertices[triangles[seed][0]];
        let in_plane = |p: [f64; 3]| dot(sub(p, origin), n).abs() <= tolerance;
        let mut members = vec![seed];
        face_of[seed] = index;
        let mut pending = vec![seed];
        while let Some(t) = pending.pop() {
            let [a, b, c] = triangles[t];
            for (from, to) in [(a, b), (b, c), (c, a)] {
                let Some(&other) = across.get(&(to, from)) else { continue };
                if face_of[other] != usize::MAX || colors[other] != colors[seed] {
                    continue;
                }
                let facing = dot(normals[other], n) >= 0.0;
                if facing && triangles[other].iter().all(|&v| in_plane(vertices[v])) {
                    face_of[other] = index;
                    members.push(other);
                    pending.push(other);
                }
            }
        }
        faces.push(Face { loops: loops(triangles, &members, &face_of, index, &across, vertices, n), normal: n, color: colors[seed] });
    }
    faces
}

/// The boundary of a face's triangles as loops, the largest first.
fn loops(
    triangles: &[[usize; 3]],
    members: &[usize],
    face_of: &[usize],
    face: usize,
    across: &HashMap<(usize, usize), usize>,
    vertices: &[[f64; 3]],
    normal: [f64; 3],
) -> Vec<Vec<usize>> {
    // Edges whose other side is in another face, by where they start.
    let mut from: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut count = 0;
    for &t in members {
        let [a, b, c] = triangles[t];
        for (start, end) in [(a, b), (b, c), (c, a)] {
            let inside = across.get(&(end, start)).is_some_and(|&other| face_of[other] == face);
            if !inside {
                from.entry(start).or_default().push(end);
                count += 1;
            }
        }
    }
    let mut loops = Vec::new();
    let mut used = 0;
    while used < count {
        let Some(&start) = from.iter().find(|(_, ends)| !ends.is_empty()).map(|(v, _)| v) else { break };
        let mut contour = vec![start];
        let mut at = start;
        loop {
            let Some(next) = from.get_mut(&at).and_then(Vec::pop) else { break };
            used += 1;
            if next == start {
                break;
            }
            contour.push(next);
            at = next;
        }
        if contour.len() >= 3 {
            loops.push(contour);
        }
    }
    // The outer loop encloses the largest area.
    let area = |contour: &Vec<usize>| {
        let mut sum = [0.0; 3];
        for (i, &v) in contour.iter().enumerate() {
            let w = contour[(i + 1) % contour.len()];
            sum = add(sum, cross(vertices[v], vertices[w]));
        }
        dot(sum, normal)
    };
    loops.sort_by(|a, b| area(b).total_cmp(&area(a)));
    loops
}

/// The STEP file of `faces`, each closed shell a solid.
fn write(name: &str, vertices: &[[f64; 3]], faces: &[Face]) -> String {
    let mut out = Entities::default();
    let product = escape(name);
    out.header = format!(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('Code2Form model'),'2;1');\n\
         FILE_NAME('{product}.step','1980-01-01T00:00:00',(''),(''),'Code2Form','Code2Form','');\n\
         FILE_SCHEMA(('AUTOMOTIVE_DESIGN {{ 1 0 10303 214 1 1 1 1 }}'));\nENDSEC;\nDATA;\n"
    );
    let application = out.add("APPLICATION_CONTEXT('core data for automotive mechanical design processes')");
    out.add(format!("APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,#{application})"));
    let product_context = out.add(format!("PRODUCT_CONTEXT('',#{application},'mechanical')"));
    let product_id = out.add(format!("PRODUCT('{product}','{product}','',(#{product_context}))"));
    out.add(format!("PRODUCT_RELATED_PRODUCT_CATEGORY('part',$,(#{product_id}))"));
    let definition_context = out.add(format!("PRODUCT_DEFINITION_CONTEXT('part definition',#{application},'design')"));
    let formation = out.add(format!("PRODUCT_DEFINITION_FORMATION('','',#{product_id})"));
    let definition = out.add(format!("PRODUCT_DEFINITION('design','',#{formation},#{definition_context})"));
    let shape = out.add(format!("PRODUCT_DEFINITION_SHAPE('','',#{definition})"));
    let length = out.add("(LENGTH_UNIT()NAMED_UNIT(*)SI_UNIT(.MILLI.,.METRE.))");
    let angle = out.add("(NAMED_UNIT(*)PLANE_ANGLE_UNIT()SI_UNIT($,.RADIAN.))");
    let solid_angle = out.add("(NAMED_UNIT(*)SI_UNIT($,.STERADIAN.)SOLID_ANGLE_UNIT())");
    let accuracy = out.add(format!(
        "UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-06),#{length},'distance_accuracy_value','confusion accuracy')"
    ));
    let context = out.add(format!(
        "(GEOMETRIC_REPRESENTATION_CONTEXT(3)GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT((#{accuracy}))\
         GLOBAL_UNIT_ASSIGNED_CONTEXT((#{length},#{angle},#{solid_angle}))REPRESENTATION_CONTEXT('',''))"
    ));
    let origin = out.point([0.0; 3]);
    let z = out.direction([0.0, 0.0, 1.0]);
    let x = out.direction([1.0, 0.0, 0.0]);
    let placement = out.add(format!("AXIS2_PLACEMENT_3D('',#{origin},#{z},#{x})"));

    // Shells: faces joined by their edges.
    let shells = shells(faces);
    let mut points: HashMap<usize, usize> = HashMap::new();
    let mut edges: HashMap<(usize, usize), usize> = HashMap::new();
    let mut written_faces: Vec<usize> = vec![0; faces.len()];
    let mut solids = Vec::new();
    let mut voids: Vec<(usize, Vec<usize>)> = Vec::new();
    let (outer, inner): (Vec<&Vec<usize>>, Vec<&Vec<usize>>) =
        shells.iter().partition(|shell| volume(vertices, faces, shell) >= 0.0);
    for (flip, group) in [(false, &outer), (true, &inner)] {
        for shell in group {
            let mut ids = Vec::new();
            for &f in shell.iter() {
                let id = out.face(vertices, &faces[f], flip, &mut points, &mut edges);
                written_faces[f] = id;
                ids.push(id);
            }
            let list = ids.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(",");
            let closed = out.add(format!("CLOSED_SHELL('',({list}))"));
            if flip {
                voids.push((closed, shell.to_vec()));
            } else {
                solids.push((closed, shell.to_vec()));
            }
        }
    }
    // Each cavity goes in the smallest solid around it.
    let bounds = |shell: &[usize]| {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for &f in shell {
            for &v in faces[f].loops.iter().flatten() {
                for k in 0..3 {
                    lo[k] = lo[k].min(vertices[v][k]);
                    hi[k] = hi[k].max(vertices[v][k]);
                }
            }
        }
        (lo, hi)
    };
    let mut cavities: Vec<Vec<usize>> = vec![Vec::new(); solids.len()];
    for (closed, shell) in &voids {
        let (lo, hi) = bounds(shell);
        let around = solids
            .iter()
            .enumerate()
            .filter(|(_, (_, solid))| {
                let (slo, shi) = bounds(solid);
                (0..3).all(|k| slo[k] <= lo[k] && hi[k] <= shi[k])
            })
            .min_by(|(_, (_, a)), (_, (_, b))| {
                let size = |s: &[usize]| {
                    let (lo, hi) = bounds(s);
                    (0..3).map(|k| hi[k] - lo[k]).product::<f64>()
                };
                size(a).total_cmp(&size(b))
            });
        if let Some((index, _)) = around {
            let oriented = out.add(format!("ORIENTED_CLOSED_SHELL('',*,#{closed},.F.)"));
            cavities[index].push(oriented);
        }
    }
    let mut items = Vec::new();
    for ((closed, shell), cavities) in solids.iter().zip(&cavities) {
        let brep = if cavities.is_empty() {
            out.add(format!("MANIFOLD_SOLID_BREP('',#{closed})"))
        } else {
            let list = cavities.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(",");
            out.add(format!("BREP_WITH_VOIDS('',#{closed},({list}))"))
        };
        items.push((brep, shell));
    }
    let list = std::iter::once(placement).chain(items.iter().map(|(brep, _)| *brep));
    let list = list.map(|id| format!("#{id}")).collect::<Vec<_>>().join(",");
    let representation = out.add(format!("ADVANCED_BREP_SHAPE_REPRESENTATION('{product}',({list}),#{context})"));
    out.add(format!("SHAPE_DEFINITION_REPRESENTATION(#{shape},#{representation})"));

    // Colors: a solid in one color is styled whole, another face by face.
    let mut styles: HashMap<[u32; 4], usize> = HashMap::new();
    let mut styled = Vec::new();
    for (brep, shell) in &items {
        let first = faces[shell[0]].color;
        let targets: Vec<(usize, [f32; 4])> = if shell.iter().all(|&f| faces[f].color == first) {
            vec![(*brep, first)]
        } else {
            shell.iter().map(|&f| (written_faces[f], faces[f].color)).collect()
        };
        for (target, color) in targets {
            let style = out.style(color, &mut styles);
            styled.push(out.add(format!("STYLED_ITEM('color',(#{style}),#{target})")));
        }
    }
    let list = styled.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(",");
    out.add(format!("MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION('',({list}),#{context})"));
    out.finish()
}

/// The faces joined through their edges, by index.
fn shells(faces: &[Face]) -> Vec<Vec<usize>> {
    let mut by_edge: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (f, face) in faces.iter().enumerate() {
        for contour in &face.loops {
            for (i, &v) in contour.iter().enumerate() {
                let w = contour[(i + 1) % contour.len()];
                by_edge.entry((v.min(w), v.max(w))).or_default().push(f);
            }
        }
    }
    let mut shell_of = vec![usize::MAX; faces.len()];
    let mut shells = Vec::new();
    for seed in 0..faces.len() {
        if shell_of[seed] != usize::MAX {
            continue;
        }
        let index = shells.len();
        shell_of[seed] = index;
        let mut members = vec![seed];
        let mut pending = vec![seed];
        while let Some(f) = pending.pop() {
            for contour in &faces[f].loops {
                for (i, &v) in contour.iter().enumerate() {
                    let w = contour[(i + 1) % contour.len()];
                    for &other in &by_edge[&(v.min(w), v.max(w))] {
                        if shell_of[other] == usize::MAX {
                            shell_of[other] = index;
                            members.push(other);
                            pending.push(other);
                        }
                    }
                }
            }
        }
        members.sort_unstable();
        shells.push(members);
    }
    shells
}

/// The volume a shell encloses, negative for a cavity's.
fn volume(vertices: &[[f64; 3]], faces: &[Face], shell: &[usize]) -> f64 {
    let mut sum = 0.0;
    for &f in shell {
        for contour in &faces[f].loops {
            let first = vertices[contour[0]];
            for i in 1..contour.len().saturating_sub(1) {
                sum += dot(first, cross(vertices[contour[i]], vertices[contour[i + 1]]));
            }
        }
    }
    sum / 6.0
}

/// The data section, numbered from 1.
#[derive(Default)]
struct Entities {
    header: String,
    data: String,
    next: usize,
}

impl Entities {
    fn add(&mut self, entity: impl AsRef<str>) -> usize {
        self.next += 1;
        let _ = writeln!(self.data, "#{}={};", self.next, entity.as_ref());
        self.next
    }

    fn point(&mut self, [x, y, z]: [f64; 3]) -> usize {
        self.add(format!("CARTESIAN_POINT('',({},{},{}))", real(x), real(y), real(z)))
    }

    fn direction(&mut self, [x, y, z]: [f64; 3]) -> usize {
        self.add(format!("DIRECTION('',({},{},{}))", real(x), real(y), real(z)))
    }

    /// An `ADVANCED_FACE` on its plane; `flip` writes it facing the other
    /// way, as the shell of a cavity is written before it is turned out.
    fn face(
        &mut self,
        vertices: &[[f64; 3]],
        face: &Face,
        flip: bool,
        points: &mut HashMap<usize, usize>,
        edges: &mut HashMap<(usize, usize), usize>,
    ) -> usize {
        let mut bounds = Vec::new();
        for (k, contour) in face.loops.iter().enumerate() {
            let mut oriented = Vec::new();
            for (i, &v) in contour.iter().enumerate() {
                let w = contour[(i + 1) % contour.len()];
                let (lo, hi) = (v.min(w), v.max(w));
                let edge = match edges.get(&(lo, hi)) {
                    Some(&edge) => edge,
                    None => {
                        let start = self.vertex(vertices, lo, points);
                        let end = self.vertex(vertices, hi, points);
                        let from = vertices[lo];
                        let along = sub(vertices[hi], from);
                        let length = dot(along, along).sqrt();
                        let at = self.point(from);
                        let direction = self.direction(unit(along).unwrap_or([1.0, 0.0, 0.0]));
                        let vector = self.add(format!("VECTOR('',#{direction},{})", real(length)));
                        let line = self.add(format!("LINE('',#{at},#{vector})"));
                        let edge = self.add(format!("EDGE_CURVE('',#{start},#{end},#{line},.T.)"));
                        edges.insert((lo, hi), edge);
                        edge
                    }
                };
                let sense = if v == lo { ".T." } else { ".F." };
                oriented.push(self.add(format!("ORIENTED_EDGE('',*,*,#{edge},{sense})")));
            }
            let list = oriented.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(",");
            let edge_loop = self.add(format!("EDGE_LOOP('',({list}))"));
            let kind = if k == 0 { "FACE_OUTER_BOUND" } else { "FACE_BOUND" };
            let sense = if flip { ".F." } else { ".T." };
            bounds.push(self.add(format!("{kind}('',#{edge_loop},{sense})")));
        }
        let outer = &face.loops[0];
        let normal = if flip { face.normal.map(|c| -c) } else { face.normal };
        let at = self.point(vertices[outer[0]]);
        let axis = self.direction(normal);
        let reference = unit(sub(vertices[outer[1]], vertices[outer[0]]))
            .map(|d| sub(d, normal.map(|c| c * dot(d, normal))))
            .and_then(unit)
            .unwrap_or_else(|| perpendicular(normal));
        let reference = self.direction(reference);
        let placement = self.add(format!("AXIS2_PLACEMENT_3D('',#{at},#{axis},#{reference})"));
        let plane = self.add(format!("PLANE('',#{placement})"));
        let list = bounds.iter().map(|id| format!("#{id}")).collect::<Vec<_>>().join(",");
        self.add(format!("ADVANCED_FACE('',({list}),#{plane},.T.)"))
    }

    fn vertex(&mut self, vertices: &[[f64; 3]], v: usize, points: &mut HashMap<usize, usize>) -> usize {
        if let Some(&id) = points.get(&v) {
            return id;
        }
        let point = self.point(vertices[v]);
        let id = self.add(format!("VERTEX_POINT('',#{point})"));
        points.insert(v, id);
        id
    }

    /// The presentation style of a color, written once.
    fn style(&mut self, [r, g, b, _]: [f32; 4], styles: &mut HashMap<[u32; 4], usize>) -> usize {
        let key = [r, g, b, 0.0].map(f32::to_bits);
        if let Some(&id) = styles.get(&key) {
            return id;
        }
        let rgb = |c: f32| real(f64::from(c.clamp(0.0, 1.0)));
        let colour = self.add(format!("COLOUR_RGB('',{},{},{})", rgb(r), rgb(g), rgb(b)));
        let fill_colour = self.add(format!("FILL_AREA_STYLE_COLOUR('',#{colour})"));
        let fill = self.add(format!("FILL_AREA_STYLE('',(#{fill_colour}))"));
        let area = self.add(format!("SURFACE_STYLE_FILL_AREA(#{fill})"));
        let side = self.add(format!("SURFACE_SIDE_STYLE('',(#{area}))"));
        let usage = self.add(format!("SURFACE_STYLE_USAGE(.BOTH.,#{side})"));
        let id = self.add(format!("PRESENTATION_STYLE_ASSIGNMENT((#{usage}))"));
        styles.insert(key, id);
        id
    }

    fn finish(self) -> String {
        format!("{}{}ENDSEC;\nEND-ISO-10303-21;\n", self.header, self.data)
    }
}

/// A STEP real: always with a decimal point, the exponent as `E`.
fn real(v: f64) -> String {
    let v = if v == 0.0 { 0.0 } else { v };
    let text = format!("{v:?}");
    match text.split_once('e') {
        Some((mantissa, exponent)) => {
            let mantissa = if mantissa.contains('.') { mantissa.to_string() } else { format!("{mantissa}.") };
            format!("{mantissa}E{exponent}")
        }
        None => text,
    }
}

/// A STEP string: quotes doubled, outside ASCII left out.
fn escape(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii() && !c.is_ascii_control()).collect::<String>().replace('\'', "''")
}

fn normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    cross(sub(b, a), sub(c, a))
}

/// A unit vector across `n`.
fn perpendicular(n: [f64; 3]) -> [f64; 3] {
    let other = if n[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    unit(cross(n, other)).unwrap_or([1.0, 0.0, 0.0])
}

fn cross(u: [f64; 3], v: [f64; 3]) -> [f64; 3] {
    [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
}

fn dot(u: [f64; 3], v: [f64; 3]) -> f64 {
    u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(v, v).sqrt();
    (length > 0.0 && length.is_finite()).then(|| v.map(|c| c / length))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(src: &str) -> String {
        let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).expect("parses");
        crate::render(&c2f_eval::evaluate(&program).nodes).step(&StepOptions::default()).expect("a solid")
    }

    fn count(text: &str, entity: &str) -> usize {
        text.matches(&format!("={entity}(")).count()
    }

    #[test]
    fn a_cube_has_six_faces() {
        let text = step("cube(10);");
        assert_eq!(count(&text, "ADVANCED_FACE"), 6);
        assert_eq!(count(&text, "EDGE_CURVE"), 12);
        assert_eq!(count(&text, "VERTEX_POINT"), 8);
        assert_eq!(count(&text, "MANIFOLD_SOLID_BREP"), 1);
        assert!(text.starts_with("ISO-10303-21;\n") && text.ends_with("END-ISO-10303-21;\n"));
        // One color, styled once on the solid.
        assert_eq!(count(&text, "STYLED_ITEM"), 1);
    }

    #[test]
    fn a_hole_is_an_inner_loop() {
        let text = step("difference() { cube(10); translate([5, 5, -1]) cylinder(r = 2, h = 12, $fn = 8); }");
        // Six sides, eight facets in the hole.
        assert_eq!(count(&text, "ADVANCED_FACE"), 14);
        assert_eq!(count(&text, "FACE_BOUND"), 2, "the top and bottom have the hole");
    }

    #[test]
    fn two_solids_and_a_cavity() {
        let text = step("difference() { cube(10); translate([3, 3, 3]) cube(4); } translate([20, 0, 0]) cube(1);");
        assert_eq!(count(&text, "MANIFOLD_SOLID_BREP"), 1);
        assert_eq!(count(&text, "BREP_WITH_VOIDS"), 1);
        assert_eq!(count(&text, "ORIENTED_CLOSED_SHELL"), 1);
    }

    #[test]
    fn colors_are_styled_face_by_face() {
        let text = step("color(\"red\") cube(10); color(\"blue\") translate([10, 0, 0]) cube(10);");
        assert_eq!(count(&text, "COLOUR_RGB"), 2);
        assert!(count(&text, "STYLED_ITEM") > 2);
    }

    #[test]
    fn reals_have_a_decimal_point() {
        assert_eq!(real(1.0), "1.0");
        assert_eq!(real(-0.0), "0.0");
        assert_eq!(real(1e-7), "1.E-7");
        assert_eq!(real(2.5e20), "2.5E20");
    }

    /// FreeCAD, when installed, reads each export back as one valid solid
    /// of the same volume. Set `C2F_FREECAD` to its command-line program
    /// (`freecadcmd`) to run it.
    #[test]
    fn freecad_reads_it_back() {
        let Some(freecad) = std::env::var_os("C2F_FREECAD") else { return };
        let dir = std::env::temp_dir().join(format!("c2f-step-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cases = [
            ("cube", "cube(10);", 1000.0),
            ("hole", "difference() { cube(10); translate([5, 5, -1]) cylinder(r = 2, h = 12, $fn = 8); }", 0.0),
            ("cavity", "difference() { cube(10); translate([3, 3, 3]) cube(4); }", 936.0),
            ("colors", "color(\"red\") cube(10); color(\"blue\") translate([10, 0, 0]) sphere(5, $fn = 24);", 0.0),
        ];
        for (name, src, expected) in cases {
            let program = c2f_eval::Program::load(src, std::path::Path::new("t.scad"), &[]).unwrap();
            let rendered = crate::render(&c2f_eval::evaluate(&program).nodes);
            let file = dir.join(format!("{name}.step"));
            std::fs::write(&file, rendered.step(&StepOptions::default()).unwrap()).unwrap();
            let script = dir.join(format!("{name}.py"));
            std::fs::write(
                &script,
                format!(
                    "import Part\ns = Part.read({:?})\nprint('RESULT', len(s.Solids), s.isValid(), s.Volume)\n",
                    file.display().to_string()
                ),
            )
            .unwrap();
            let output = std::process::Command::new(&freecad).arg(&script).output().unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let line = stdout.lines().find(|l| l.starts_with("RESULT")).unwrap_or_else(|| panic!("{name}: {stdout}"));
            let words: Vec<&str> = line.split_whitespace().collect();
            assert_eq!(&words[1..3], ["1", "True"], "{name}: {line}");
            let volume: f64 = words[3].parse().unwrap();
            let expected = if expected > 0.0 { expected } else { rendered.stats.volume };
            assert!((volume - expected).abs() < 1e-6 * expected.max(1.0), "{name}: {volume} vs {expected}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
