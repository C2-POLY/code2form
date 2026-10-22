//! glTF 2.0 export, as a binary `.glb`: the format Godot 4 imports as it
//! is and Unity through glTFast or UnityGLTF. glTF counts in meters with
//! Y up, where a design counts in millimeters with Z up: positions are
//! scaled ([`GltfOptions::scale`]) and turned a quarter turn about X, so
//! that +Z becomes +Y and +Y becomes -Z.
//!
//! Each `color()` becomes a PBR material and a primitive of its own.
//! Normals are smoothed across the facets of a curved surface and split
//! along sharp edges, so a cylinder reads round and a cube stays crisp.
//!
//! Each `part("name")` is a node of its own, named, holding its geometry
//! in its own frame, with the translation, rotation and scale written
//! around it, under the node of the part it is written in. What is outside
//! any part is the root node.
//!
//! A program that reads `$t` is animated ([`program_glb`]): its parts'
//! geometry is built once, at `$t = 0`, then the program is evaluated at
//! each frame, without building any geometry, for where its parts are.
//! Their movement becomes a glTF animation: a track of translations,
//! rotations or scales for each part that moves, interpolated linearly.

use std::collections::HashMap;
use std::path::Path;

use c2f_eval::Program;
use c2f_eval::node::{Matrix, Node};
use manifold_rust::manifold::Manifold;
use serde_json::{Value, json};

use crate::build::{CUT_COLOR, DEFAULT_COLOR, Model, render_in};
use crate::parts::{self, Parts};

/// How a model is written to glTF.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfOptions {
    /// Meters per model unit: 0.001 keeps a millimeter design at its real
    /// size, 1 makes each unit a meter.
    pub scale: f64,
    /// The name of the scene's root node, which holds what is outside any
    /// `part()`.
    pub name: String,
    /// The color of the parts without `color()`, in sRGB. The faces a
    /// `difference()` cuts take it too, rather than the preview's green.
    pub color: [f32; 3],
    /// How a program that reads `$t` is animated, by [`program_glb`];
    /// `None` writes it at `$t = 0`, still.
    pub animation: Option<GltfAnimation>,
}

impl Default for GltfOptions {
    fn default() -> Self {
        let [r, g, b, _] = DEFAULT_COLOR;
        GltfOptions {
            scale: 0.001,
            name: "Code2Form".into(),
            color: [r, g, b],
            animation: Some(GltfAnimation::default()),
        }
    }
}

/// The animation of the parts along `$t`, from 0 to 1 in `duration`.
#[derive(Clone, Debug, PartialEq)]
pub struct GltfAnimation {
    pub name: String,
    /// In seconds.
    pub duration: f64,
    /// Frames sampled a second.
    pub fps: f64,
    /// Plays over and over. Godot reads a name that ends in `-loop` so:
    /// the name gets that ending.
    pub looped: bool,
}

impl Default for GltfAnimation {
    fn default() -> Self {
        GltfAnimation { name: "animation".into(), duration: 2.0, fps: 30.0, looped: true }
    }
}

impl GltfAnimation {
    /// The name written: with `-loop` when it loops.
    pub fn written_name(&self) -> String {
        let looping = ["-loop", "_loop", "-cycle", "_cycle"].iter().any(|end| self.name.ends_with(end));
        if self.looped && !looping { format!("{}-loop", self.name) } else { self.name.clone() }
    }

    /// The steps between frames: at least one, so that `$t` = 0 and 1 are
    /// both sampled.
    fn intervals(&self) -> usize {
        ((self.duration * self.fps).round() as usize).max(1)
    }
}

/// A `.glb` file and what went into it.
#[derive(Clone, Debug, Default)]
pub struct Glb {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
    /// Triangles, across all primitives.
    pub triangles: usize,
    /// Vertices written: a corner shared by facets that meet at a sharp
    /// edge or in two colors is written once for each.
    pub vertices: usize,
    /// The frames of the animation, none when it is still.
    pub frames: usize,
}

/// Facets that meet at more than this angle keep their own normals.
const SHARP_DEGREES: f64 = 35.0;

/// Renders `nodes` and writes them as a `.glb`, a node for each `part()`.
/// Data files are read relative to `base_dir`, as [`render_in`] reads
/// them. An empty or 2D model has no glTF.
pub fn glb(nodes: &[Node], base_dir: &Path, options: &GltfOptions) -> Result<Glb, String> {
    build(parts::split(nodes), base_dir, options, None)
}

/// Evaluates `program` and writes it as a `.glb`, as [`glb`] does, its
/// data files read next to its main file. When it reads `$t` and
/// `options.animation` is set, its parts move as `$t` goes from 0 to 1.
/// The error that stops the program is the error.
pub fn program_glb(program: &mut Program, options: &GltfOptions) -> Result<Glb, String> {
    let base_dir = program.files[0].path.parent().unwrap_or(Path::new("")).to_path_buf();
    let t = program.t;
    let result = sample(program, options).and_then(|(first, motion)| build(first, &base_dir, options, motion));
    program.t = t;
    result
}

/// Where each part is at each frame, and the animation's name.
struct Motion {
    name: String,
    /// In seconds.
    times: Vec<f64>,
    /// For each frame, each part's matrix, as [`parts::Part::matrix`].
    poses: Vec<Vec<Matrix>>,
}

/// The program at `$t = 0`, split into parts, and with an animation, where
/// they are at each frame. A frame that changes more than the parts'
/// places is a warning: the geometry is the first frame's.
fn sample(program: &mut Program, options: &GltfOptions) -> Result<(Parts, Option<Motion>), String> {
    let evaluate = |program: &mut Program, t: f64| {
        program.t = t;
        let output = c2f_eval::evaluate(program);
        match output.error {
            Some(error) if t == 0.0 => Err(error),
            Some(error) => Err(format!("at $t = {t}: {error}")),
            None => Ok(output),
        }
    };
    let output = evaluate(program, 0.0)?;
    let mut first = parts::split(&output.nodes);
    let Some(animation) = options.animation.as_ref().filter(|_| output.animated) else { return Ok((first, None)) };
    if first.parts.is_empty() {
        first.warnings.push(
            "The model reads $t but has no part(): put what moves in part(\"name\") for the glTF to animate it".into(),
        );
        return Ok((first, None));
    }
    let intervals = animation.intervals();
    let mut motion = Motion { name: animation.written_name(), times: Vec::new(), poses: Vec::new() };
    let mut warned: Vec<Option<usize>> = Vec::new();
    for frame in 0..=intervals {
        let t = frame as f64 / intervals as f64;
        motion.times.push(t * animation.duration);
        if frame == 0 {
            motion.poses.push(first.parts.iter().map(|p| p.matrix).collect());
            continue;
        }
        let now = parts::split(&evaluate(program, t)?.nodes);
        let same = now.parts.len() == first.parts.len()
            && now.parts.iter().zip(&first.parts).all(|(a, b)| a.name == b.name && a.parent == b.parent);
        if !same {
            if !warned.contains(&None) {
                warned.push(None);
                first.warnings.push(format!(
                    "The parts change with $t (at $t = {t}, they are not those at 0): the glTF keeps them where they were"
                ));
            }
            motion.poses.push(motion.poses[frame - 1].clone());
            continue;
        }
        let root_changed = now.root != first.root;
        for (i, (a, b)) in now.parts.iter().zip(&first.parts).enumerate() {
            if a.nodes != b.nodes && !warned.contains(&Some(i)) {
                warned.push(Some(i));
                first.warnings.push(format!(
                    "part(\"{}\") changes shape with $t, which a glTF cannot hold: it keeps its shape at $t = 0 and moves",
                    b.name
                ));
            }
        }
        if root_changed && !warned.contains(&Some(usize::MAX)) {
            warned.push(Some(usize::MAX));
            first.warnings.push(
                "What is outside any part() changes with $t, which a glTF cannot hold: it is kept as at $t = 0".into(),
            );
        }
        motion.poses.push(now.parts.iter().map(|p| p.matrix).collect());
    }
    Ok((first, Some(motion)))
}

/// Writes the parts, and their motion, as a `.glb`.
fn build(split: Parts, base_dir: &Path, options: &GltfOptions, motion: Option<Motion>) -> Result<Glb, String> {
    let Parts { root, parts, mut warnings } = split;
    let mut writer = Writer::new(options);
    // The root, then each part: node `i + 1` is part `i`.
    let mut flat = false;
    let names = unique(std::iter::once(options.name.as_str()).chain(parts.iter().map(|p| p.name.as_str())));
    let geometry = std::iter::once(&root).chain(parts.iter().map(|p| &p.nodes));
    for (name, nodes) in names.iter().zip(geometry) {
        let rendered = render_in(nodes, base_dir);
        for warning in rendered.warnings {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
        let mut node = json!({ "name": name });
        match rendered.model {
            Model::Solid(manifold) => {
                if let Some(mesh) = writer.mesh(name, &manifold) {
                    node["mesh"] = json!(mesh);
                }
            }
            Model::Flat(_) => flat = true,
            Model::Empty => {}
        }
        writer.nodes.push(node);
    }
    if writer.meshes.is_empty() {
        return Err(if flat { "the model is 2D; glTF holds solids" } else { "the model is empty; no glTF written" }.into());
    }
    if flat {
        warnings.push("The 2D shapes of the model are left out: glTF holds solids".into());
    }
    for (i, part) in parts.iter().enumerate() {
        let parent = part.parent.map_or(0, |p| p + 1);
        let node = &mut writer.nodes[i + 1];
        let (transform, sheared) = Transform::of(&part.matrix, options.scale);
        if sheared {
            warnings.push(format!(
                "part(\"{}\") is sheared or skewed, which a glTF node cannot hold: it is written with the nearest rotation",
                part.name
            ));
        }
        transform.write(node);
        let children = &mut writer.nodes[parent]["children"];
        match children.as_array_mut() {
            Some(children) => children.push(json!(i + 1)),
            None => *children = json!([i + 1]),
        }
    }
    let frames = match motion {
        Some(motion) => writer.animation(&motion, parts.len(), &mut warnings),
        None => 0,
    };
    let (bytes, triangles, vertices) = writer.finish(vec![0]);
    Ok(Glb { bytes, warnings, triangles, vertices, frames })
}

/// `names`, a number added to those met before (`lid`, `lid 2`), so that
/// engines that key nodes by name keep them as written.
fn unique<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        let mut candidate = name.to_string();
        let mut n = 1;
        while out.contains(&candidate) {
            n += 1;
            candidate = format!("{name} {n}");
        }
        out.push(candidate);
    }
    out
}

/// A node's place in glTF: translation in meters with Y up, rotation as a
/// unit quaternion `[x, y, z, w]`, and scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Transform {
    pub translation: [f64; 3],
    pub rotation: [f64; 4],
    pub scale: [f64; 3],
}

impl Transform {
    /// `matrix`, which places a part in its parent's frame, in glTF's
    /// frame; and whether it was sheared, which a glTF node cannot hold,
    /// so that its rotation is the nearest one.
    pub(crate) fn of(matrix: &Matrix, scale: f64) -> (Transform, bool) {
        // Y up: the turn C applied on both sides, C · M · Cᵀ.
        let turn = |[x, y, z]: [f64; 3]| [x, z, -y];
        let translation = turn([matrix[0][3], matrix[1][3], matrix[2][3]]).map(|c| c * scale);
        let column = |j: usize| turn([matrix[0][j], matrix[1][j], matrix[2][j]]);
        let mut columns = [column(0), column(2), column(1).map(|c| -c)];
        let mut scales = columns.map(|c| dot(c, c).sqrt());
        // A mirror is a negative scale along X.
        if dot(cross(columns[0], columns[1]), columns[2]) < 0.0 {
            scales[0] = -scales[0];
        }
        for (column, s) in columns.iter_mut().zip(scales) {
            if s != 0.0 {
                *column = column.map(|c| c / s);
            }
        }
        // The nearest rotation, by Gram-Schmidt.
        let x = unit(columns[0]).unwrap_or([1.0, 0.0, 0.0]);
        let y = unit(sub(columns[1], x.map(|c| c * dot(x, columns[1])))).unwrap_or_else(|| any_normal(x));
        let z = cross(x, y);
        let sheared = scales.iter().all(|&s| s != 0.0)
            && [dot(columns[0], columns[1]), dot(columns[1], columns[2]), dot(columns[0], columns[2])]
                .iter()
                .any(|d| d.abs() > 1e-6);
        (Transform { translation, rotation: quaternion([x, y, z]), scale: scales }, sheared)
    }

    /// Sets the node's `translation`, `rotation` and `scale`, leaving out
    /// those that change nothing.
    fn write(&self, node: &mut Value) {
        if self.translation != [0.0; 3] {
            node["translation"] = json!(self.translation);
        }
        if self.rotation != [0.0, 0.0, 0.0, 1.0] {
            node["rotation"] = json!(self.rotation);
        }
        if self.scale != [1.0; 3] {
            node["scale"] = json!(self.scale);
        }
    }
}

/// A unit vector at right angles to `v`.
fn any_normal(v: [f64; 3]) -> [f64; 3] {
    let other = if v[0].abs() < 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 1.0, 0.0] };
    unit(cross(v, other)).unwrap_or([0.0, 0.0, 1.0])
}

/// The unit quaternion `[x, y, z, w]` of the rotation whose columns are
/// `columns`, with `w` at least 0.
fn quaternion(columns: [[f64; 3]; 3]) -> [f64; 4] {
    let m = |i: usize, j: usize| columns[j][i];
    let trace = m(0, 0) + m(1, 1) + m(2, 2);
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(m(2, 1) - m(1, 2)) / s, (m(0, 2) - m(2, 0)) / s, (m(1, 0) - m(0, 1)) / s, s / 4.0]
    } else if m(0, 0) > m(1, 1) && m(0, 0) > m(2, 2) {
        let s = (1.0 + m(0, 0) - m(1, 1) - m(2, 2)).sqrt() * 2.0;
        [s / 4.0, (m(0, 1) + m(1, 0)) / s, (m(0, 2) + m(2, 0)) / s, (m(2, 1) - m(1, 2)) / s]
    } else if m(1, 1) > m(2, 2) {
        let s = (1.0 + m(1, 1) - m(0, 0) - m(2, 2)).sqrt() * 2.0;
        [(m(0, 1) + m(1, 0)) / s, s / 4.0, (m(1, 2) + m(2, 1)) / s, (m(0, 2) - m(2, 0)) / s]
    } else {
        let s = (1.0 + m(2, 2) - m(0, 0) - m(1, 1)).sqrt() * 2.0;
        [(m(0, 2) + m(2, 0)) / s, (m(1, 2) + m(2, 1)) / s, s / 4.0, (m(1, 0) - m(0, 1)) / s]
    };
    let length = q.iter().map(|c| c * c).sum::<f64>().sqrt();
    let sign = if q[3] < 0.0 { -1.0 } else { 1.0 };
    q.map(|c| sign * c / length)
}

/// The glTF document being written, and its binary buffer.
struct Writer {
    scale: f64,
    /// For the parts without `color()`.
    plain: [f32; 3],
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
    meshes: Vec<Value>,
    nodes: Vec<Value>,
    materials: Vec<Value>,
    animations: Vec<Value>,
    /// The material of each color, by its bits.
    material_of: HashMap<[u32; 4], usize>,
    triangles: usize,
    vertices: usize,
}

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;

impl Writer {
    fn new(options: &GltfOptions) -> Self {
        Writer {
            scale: options.scale,
            plain: options.color,
            bin: Vec::new(),
            views: Vec::new(),
            accessors: Vec::new(),
            meshes: Vec::new(),
            nodes: Vec::new(),
            materials: Vec::new(),
            animations: Vec::new(),
            material_of: HashMap::new(),
            triangles: 0,
            vertices: 0,
        }
    }

    /// Appends `bytes` as a buffer view, 4-byte aligned, and returns its
    /// index.
    fn view(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while self.bin.len() % 4 != 0 {
            self.bin.push(0);
        }
        let mut view = json!({ "buffer": 0, "byteOffset": self.bin.len(), "byteLength": bytes.len() });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(view);
        self.views.len() - 1
    }

    /// An accessor of `f32` items of `width` components (`kind` is
    /// `SCALAR`, `VEC3`...), with their bounds when `bounded`.
    fn floats(&mut self, values: &[f32], width: usize, kind: &str, target: Option<u32>, bounded: bool) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, target);
        let count = values.len() / width;
        let mut accessor = json!({ "bufferView": view, "componentType": FLOAT, "count": count, "type": kind });
        if bounded && count > 0 {
            let mut lo = vec![f32::INFINITY; width];
            let mut hi = vec![f32::NEG_INFINITY; width];
            for item in values.chunks(width) {
                for (k, &v) in item.iter().enumerate() {
                    lo[k] = lo[k].min(v);
                    hi[k] = hi[k].max(v);
                }
            }
            accessor["min"] = json!(lo);
            accessor["max"] = json!(hi);
        }
        self.accessors.push(accessor);
        self.accessors.len() - 1
    }

    fn indices(&mut self, values: &[u32]) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes, Some(ELEMENT_ARRAY_BUFFER));
        self.accessors.push(json!({ "bufferView": view, "componentType": UNSIGNED_INT, "count": values.len(), "type": "SCALAR" }));
        self.accessors.len() - 1
    }

    /// The material for a color from `color()`, made on first use.
    fn material(&mut self, rgba: [f32; 4]) -> usize {
        let key = rgba.map(f32::to_bits);
        if let Some(&material) = self.material_of.get(&key) {
            return material;
        }
        let [r, g, b] = [rgba[0], rgba[1], rgba[2]].map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        // `color()` takes sRGB values; glTF's factor is linear.
        let factor = [linear(rgba[0]), linear(rgba[1]), linear(rgba[2]), rgba[3].clamp(0.0, 1.0)];
        let mut material = json!({
            "name": format!("#{r:02x}{g:02x}{b:02x}"),
            "pbrMetallicRoughness": { "baseColorFactor": factor, "metallicFactor": 0.0, "roughnessFactor": 0.6 },
        });
        if rgba[3] < 1.0 {
            material["alphaMode"] = json!("BLEND");
        }
        self.materials.push(material);
        self.material_of.insert(key, self.materials.len() - 1);
        self.materials.len() - 1
    }

    /// Writes `manifold` as a mesh named `name`, a primitive per color,
    /// and returns its index; `None` when it has no triangles.
    fn mesh(&mut self, name: &str, manifold: &Manifold) -> Option<usize> {
        let surfaces = surfaces(manifold, self.scale, self.plain);
        if surfaces.is_empty() {
            return None;
        }
        let mut primitives = Vec::new();
        for surface in surfaces {
            self.triangles += surface.indices.len() / 3;
            self.vertices += surface.positions.len() / 3;
            let position = self.floats(&surface.positions, 3, "VEC3", Some(ARRAY_BUFFER), true);
            let normal = self.floats(&surface.normals, 3, "VEC3", Some(ARRAY_BUFFER), false);
            let indices = self.indices(&surface.indices);
            let material = self.material(surface.color);
            primitives.push(json!({
                "attributes": { "POSITION": position, "NORMAL": normal },
                "indices": indices,
                "material": material,
            }));
        }
        self.meshes.push(json!({ "name": name, "primitives": primitives }));
        Some(self.meshes.len() - 1)
    }

    /// Writes the parts' `motion` as an animation, a track for each of
    /// their translations, rotations and scales that changes; node `i + 1`
    /// is part `i`. Returns how many frames it has: none when nothing moves.
    fn animation(&mut self, motion: &Motion, parts: usize, warnings: &mut Vec<String>) -> usize {
        let times: Vec<f32> = motion.times.iter().map(|&t| t as f32).collect();
        let mut samplers = Vec::new();
        let mut channels = Vec::new();
        let mut input = None;
        for i in 0..parts {
            let transforms: Vec<Transform> =
                motion.poses.iter().map(|pose| Transform::of(&pose[i], self.scale).0).collect();
            let mut rotations: Vec<[f64; 4]> = transforms.iter().map(|t| t.rotation).collect();
            // q and -q are the same turn; the shorter way between frames is
            // the one meant.
            for k in 1..rotations.len() {
                let previous = rotations[k - 1];
                if (0..4).map(|c| previous[c] * rotations[k][c]).sum::<f64>() < 0.0 {
                    rotations[k] = rotations[k].map(|c| -c);
                }
            }
            let translations: Vec<Vec<f64>> = transforms.iter().map(|t| t.translation.to_vec()).collect();
            let scales: Vec<Vec<f64>> = transforms.iter().map(|t| t.scale.to_vec()).collect();
            let rotations: Vec<Vec<f64>> = rotations.iter().map(|q| q.to_vec()).collect();
            for (path, values, kind) in
                [("translation", translations, "VEC3"), ("rotation", rotations, "VEC4"), ("scale", scales, "VEC3")]
            {
                let still = values.iter().all(|v| v.iter().zip(&values[0]).all(|(a, b)| (a - b).abs() <= 1e-9));
                if still {
                    continue;
                }
                let input = *input.get_or_insert_with(|| self.floats(&times, 1, "SCALAR", None, true));
                let flat: Vec<f32> = values.iter().flatten().map(|&v| v as f32).collect();
                let output = self.floats(&flat, values[0].len(), kind, None, false);
                samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
                channels.push(json!({ "sampler": samplers.len() - 1, "target": { "node": i + 1, "path": path } }));
            }
        }
        if channels.is_empty() {
            warnings.push("The model reads $t, but no part() moves with it: no animation written".into());
            return 0;
        }
        self.animations.push(json!({ "name": motion.name, "samplers": samplers, "channels": channels }));
        times.len()
    }

    /// The `.glb` of a scene whose root nodes are `roots`, with its triangle
    /// and vertex counts.
    fn finish(self, roots: Vec<usize>) -> (Vec<u8>, usize, usize) {
        let mut document = json!({
            "asset": { "version": "2.0", "generator": concat!("Code2Form ", env!("CARGO_PKG_VERSION")) },
            "scene": 0,
            "scenes": [{ "nodes": roots }],
            "nodes": self.nodes,
        });
        if !self.animations.is_empty() {
            document["animations"] = json!(self.animations);
        }
        if !self.meshes.is_empty() {
            document["meshes"] = json!(self.meshes);
            document["materials"] = json!(self.materials);
            document["accessors"] = json!(self.accessors);
            document["bufferViews"] = json!(self.views);
            document["buffers"] = json!([{ "byteLength": self.bin.len() }]);
        }
        let bytes = container(&serde_json::to_vec(&document).expect("glTF JSON"), &self.bin);
        (bytes, self.triangles, self.vertices)
    }
}

/// sRGB to linear, as the glTF factors take colors.
fn linear(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// The GLB container: a 12-byte header, the JSON chunk padded with
/// spaces, then the binary chunk padded with zeros.
fn container(json: &[u8], bin: &[u8]) -> Vec<u8> {
    let pad = |n: usize| (4 - n % 4) % 4;
    let json_length = json.len() + pad(json.len());
    let bin_length = bin.len() + pad(bin.len());
    let total = 12 + 8 + json_length + if bin.is_empty() { 0 } else { 8 + bin_length };
    let mut out = Vec::with_capacity(total);
    out.extend(b"glTF");
    out.extend(2u32.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend((json_length as u32).to_le_bytes());
    out.extend(b"JSON");
    out.extend(json);
    out.extend(std::iter::repeat_n(b' ', pad(json.len())));
    if !bin.is_empty() {
        out.extend((bin_length as u32).to_le_bytes());
        out.extend(b"BIN\0");
        out.extend(bin);
        out.extend(std::iter::repeat_n(0, pad(bin.len())));
    }
    out
}

/// The triangles of one color, ready for a glTF primitive: positions in
/// meters with Y up, unit normals, and indices.
struct Surface {
    color: [f32; 4],
    positions: Vec<f32>,
    normals: Vec<f32>,
    indices: Vec<u32>,
}

/// From Z up to Y up: a quarter turn about X.
fn y_up([x, y, z]: [f64; 3]) -> [f64; 3] {
    [x, z, -y]
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

fn unit(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = dot(v, v).sqrt();
    (length > 0.0 && length.is_finite()).then(|| v.map(|c| c / length))
}

/// The solid's triangles, grouped by color in order of first use. Each
/// corner's normal averages, weighted by their angles there, the facets
/// around its vertex that turn by at most [`SHARP_DEGREES`] from its own.
fn surfaces(manifold: &Manifold, scale: f64, [r, g, b]: [f32; 3]) -> Vec<Surface> {
    let mesh = manifold.get_mesh_gl64(-1);
    let stride = mesh.num_prop as usize;
    let count = mesh.vert_properties.len() / stride.max(1);
    // Colors split vertices; the merge lists say which copies are one.
    let mut merged: Vec<usize> = (0..count).collect();
    for (&from, &to) in mesh.merge_from_vert.iter().zip(&mesh.merge_to_vert) {
        merged[from as usize] = to as usize;
    }
    let point = |i: usize| {
        let p = &mesh.vert_properties[i * stride..i * stride + 3];
        [p[0], p[1], p[2]]
    };
    let triangles: Vec<[usize; 3]> =
        mesh.tri_verts.chunks(3).map(|t| [t[0] as usize, t[1] as usize, t[2] as usize]).collect();
    let normal: Vec<Option<[f64; 3]>> = triangles
        .iter()
        .map(|&[a, b, c]| unit(cross(sub(point(b), point(a)), sub(point(c), point(a)))))
        .collect();
    // The angle of a facet at its corner `k`: weighted by it, the facets
    // around a vertex give the same normal however they are split.
    let angle = |t: usize, k: usize| {
        let corners = triangles[t];
        let at = point(corners[k]);
        let (u, v) = (unit(sub(point(corners[(k + 1) % 3]), at)), unit(sub(point(corners[(k + 2) % 3]), at)));
        u.zip(v).map_or(0.0, |(u, v)| dot(u, v).clamp(-1.0, 1.0).acos())
    };
    let mut around: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (t, corners) in triangles.iter().enumerate() {
        for &i in corners {
            around[merged[i]].push(t);
        }
    }
    let sharp = SHARP_DEGREES.to_radians().cos();

    let mut surfaces: Vec<Surface> = Vec::new();
    let mut surface_of: HashMap<[u32; 4], usize> = HashMap::new();
    // Each written vertex, by surface, vertex and normal.
    let mut written: HashMap<(usize, usize, [i64; 3]), u32> = HashMap::new();
    for (t, corners) in triangles.iter().enumerate() {
        let Some(own) = normal[t] else { continue };
        let color = match &mesh.vert_properties[corners[0] * stride..] {
            p if stride >= 7 => [p[3] as f32, p[4] as f32, p[5] as f32, p[6] as f32],
            _ => DEFAULT_COLOR,
        };
        let color = if color == DEFAULT_COLOR || color == CUT_COLOR { [r, g, b, 1.0] } else { color };
        let s = *surface_of.entry(color.map(f32::to_bits)).or_insert_with(|| {
            surfaces.push(Surface { color, positions: Vec::new(), normals: Vec::new(), indices: Vec::new() });
            surfaces.len() - 1
        });
        for &i in corners {
            let v = merged[i];
            let mut sum = [0.0; 3];
            for &other in &around[v] {
                let Some(n) = normal[other].filter(|&n| dot(n, own) >= sharp) else { continue };
                let k = (0..3).find(|&k| merged[triangles[other][k]] == v).expect("a facet around its vertex");
                let weight = angle(other, k);
                sum = [sum[0] + weight * n[0], sum[1] + weight * n[1], sum[2] + weight * n[2]];
            }
            let n = y_up(unit(sum).unwrap_or(own));
            // Normals that round alike are one vertex.
            let key = (s, v, n.map(|c| (c * 1e5).round() as i64));
            let surface = &mut surfaces[s];
            let index = *written.entry(key).or_insert_with(|| {
                let p = y_up(point(v));
                surface.positions.extend(p.map(|c| (c * scale) as f32));
                surface.normals.extend(n.map(|c| c as f32));
                (surface.positions.len() / 3 - 1) as u32
            });
            surface.indices.push(index);
        }
    }
    surfaces.retain(|s| !s.indices.is_empty());
    surfaces
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The JSON and binary chunks of a `.glb`, checked as the container
    /// lays them out.
    pub(crate) fn chunks(glb: &[u8]) -> (Value, Vec<u8>) {
        assert_eq!(&glb[..4], b"glTF");
        let word = |at: usize| u32::from_le_bytes(glb[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(word(4), 2, "version");
        assert_eq!(word(8), glb.len(), "length");
        let json_length = word(12);
        assert_eq!(&glb[16..20], b"JSON");
        assert_eq!(json_length % 4, 0);
        let json = serde_json::from_slice(&glb[20..20 + json_length]).unwrap();
        let at = 20 + json_length;
        if at == glb.len() {
            return (json, Vec::new());
        }
        assert_eq!(&glb[at + 4..at + 8], b"BIN\0");
        (json, glb[at + 8..at + 8 + word(at)].to_vec())
    }

    #[test]
    fn container_pads_its_chunks() {
        let glb = container(b"{}", &[1, 2, 3, 4, 5]);
        assert_eq!(glb.len(), 12 + 8 + 4 + 8 + 8);
        assert_eq!(&glb[20..24], b"{}  ");
        assert_eq!(&glb[32..40], &[1, 2, 3, 4, 5, 0, 0, 0]);
        assert_eq!(chunks(&glb).1.len(), 8);
    }

    #[test]
    fn colors_are_linear() {
        assert_eq!(linear(0.0), 0.0);
        assert_eq!(linear(1.0), 1.0);
        assert!((linear(0.5) - 0.214).abs() < 1e-3);
    }
}
