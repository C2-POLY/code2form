//! Circuits in the 3D view: each part of a `.c2fc` file rendered as a
//! design of its own, placed where the circuit puts it.
//!
//! Parts render on the worker, several at a time, and are kept by their
//! file, parameters and text: a part used twice renders once, and an edit
//! to the circuit re-renders only the parts it changed.
//!
//! The enclosure the circuit names shows around the parts, see-through,
//! and a part that runs into it or into another part shows in red; one
//! that comes closer than [`c2f_geom::CLEARANCE`] to them, in amber.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use c2f_circuit::{Circuit, Instance};
use c2f_geom::{Model, PartFit, RenderMesh, Stats};

use crate::console::{Level, Line};
use crate::worker::Scene;

/// The color c2f-geom gives what no `color()` colors.
const UNCOLORED: [f32; 4] = c2f_geom::DEFAULT_COLOR;

/// Colors for parts that leave some of their shapes uncolored, one per
/// part in turn, so that each stands apart from the next.
const PALETTE: [[f32; 4]; 8] = [
    [0.93, 0.76, 0.27, 1.0],
    [0.38, 0.66, 0.93, 1.0],
    [0.55, 0.80, 0.42, 1.0],
    [0.90, 0.50, 0.42, 1.0],
    [0.70, 0.56, 0.90, 1.0],
    [0.36, 0.80, 0.78, 1.0],
    [0.92, 0.60, 0.80, 1.0],
    [0.75, 0.75, 0.70, 1.0],
];

pub fn is_circuit(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == c2f_circuit::EXTENSION)
}

/// A color as a circuit file writes it: a common CSS/SVG name or
/// `#rrggbb`, case-insensitive.
pub fn parse_color(text: &str) -> Option<[f32; 4]> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix('#') {
        let bytes = hex.as_bytes();
        let value = |i: usize| u8::from_str_radix(std::str::from_utf8(&bytes[i..i + 2]).ok()?, 16).ok();
        return match bytes.len() {
            6 => Some([value(0)?, value(2)?, value(4)?, 255]),
            8 => Some([value(0)?, value(2)?, value(4)?, value(6)?]),
            _ => None,
        }
        .map(|rgba| rgba.map(|c| f32::from(c) / 255.0));
    }
    let name = text.to_ascii_lowercase();
    let [r, g, b] = NAMES.iter().find(|(n, _)| *n == name)?.1;
    Some([r, g, b, 255].map(|c| f32::from(c) / 255.0))
}

/// The CSS/SVG names wires and parts are most often given.
const NAMES: &[(&str, [u8; 3])] = &[
    ("black", [0, 0, 0]),
    ("white", [255, 255, 255]),
    ("red", [255, 0, 0]),
    ("darkred", [139, 0, 0]),
    ("crimson", [220, 20, 60]),
    ("maroon", [128, 0, 0]),
    ("orange", [255, 165, 0]),
    ("darkorange", [255, 140, 0]),
    ("gold", [255, 215, 0]),
    ("yellow", [255, 255, 0]),
    ("olive", [128, 128, 0]),
    ("lime", [0, 255, 0]),
    ("green", [0, 128, 0]),
    ("darkgreen", [0, 100, 0]),
    ("limegreen", [50, 205, 50]),
    ("teal", [0, 128, 128]),
    ("cyan", [0, 255, 255]),
    ("aqua", [0, 255, 255]),
    ("turquoise", [64, 224, 208]),
    ("skyblue", [135, 206, 235]),
    ("lightblue", [173, 216, 230]),
    ("blue", [0, 0, 255]),
    ("royalblue", [65, 105, 225]),
    ("darkblue", [0, 0, 139]),
    ("navy", [0, 0, 128]),
    ("purple", [128, 0, 128]),
    ("violet", [238, 130, 238]),
    ("magenta", [255, 0, 255]),
    ("fuchsia", [255, 0, 255]),
    ("pink", [255, 192, 203]),
    ("hotpink", [255, 105, 180]),
    ("brown", [165, 42, 42]),
    ("chocolate", [210, 105, 30]),
    ("tan", [210, 180, 140]),
    ("beige", [245, 245, 220]),
    ("silver", [192, 192, 192]),
    ("gray", [128, 128, 128]),
    ("grey", [128, 128, 128]),
    ("darkgray", [169, 169, 169]),
    ("darkgrey", [169, 169, 169]),
    ("lightgray", [211, 211, 211]),
    ("lightgrey", [211, 211, 211]),
    ("dimgray", [105, 105, 105]),
    ("dimgrey", [105, 105, 105]),
];

/// The color a part gives its uncolored shapes: its `color` in the
/// circuit, else the next one of the palette.
pub fn part_tint(instance: &Instance, index: usize) -> [f32; 4] {
    instance.color.as_deref().and_then(parse_color).unwrap_or(PALETTE[index % PALETTE.len()])
}

/// Appends `mesh` to `out`, placed by `matrix` (rows, as
/// [`Instance::matrix`]); `tint` replaces the color of uncolored shapes.
pub fn place(out: &mut RenderMesh, mesh: &RenderMesh, matrix: &[[f64; 4]; 4], tint: Option<[f32; 4]>) {
    let point = |p: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|row| {
            let m = matrix[row];
            (m[0] * f64::from(p[0]) + m[1] * f64::from(p[1]) + m[2] * f64::from(p[2]) + m[3]) as f32
        })
    };
    // A rotation: normals turn as directions do.
    let direction = |n: [f32; 3]| -> [f32; 3] {
        std::array::from_fn(|row| {
            let m = matrix[row];
            (m[0] * f64::from(n[0]) + m[1] * f64::from(n[1]) + m[2] * f64::from(n[2])) as f32
        })
    };
    out.positions.extend(mesh.positions.iter().map(|&p| point(p)));
    out.normals.extend(mesh.normals.iter().map(|&n| direction(n)));
    out.colors.extend(mesh.colors.iter().map(|&c| match tint {
        Some(tint) if c == UNCOLORED => tint,
        _ => c,
    }));
    // A part's origins index its own program's table, not the design's.
    out.origins.resize(out.triangle_count(), 0);
}

/// A part file rendered once, in its own coordinates.
pub struct PartRender {
    pub solid: RenderMesh,
    pub highlights: RenderMesh,
    pub backgrounds: RenderMesh,
    pub triangles: usize,
    /// Echo lines, warnings and errors, prefixed with the file's name.
    pub console: Vec<Line>,
    /// The solid, for collisions.
    pub model: Model,
}

impl PartRender {
    /// A part that did not render: nothing to draw.
    pub fn failed(console: Vec<Line>) -> Self {
        PartRender {
            solid: RenderMesh::default(),
            highlights: RenderMesh::default(),
            backgrounds: RenderMesh::default(),
            triangles: 0,
            console,
            model: Model::Empty,
        }
    }
}

/// What renders an enclosure: its file, its text, and the circuit's text
/// with its parts' (their pins, holes and ports shape it).
type EnclosureKey = (PathBuf, String, String);

/// What the collisions come from: each part's render and placement, and
/// the enclosure's render; held, so that a render compares by identity.
type CollisionKey = (Vec<(Arc<PartRender>, [[f64; 4]; 4])>, Option<Arc<PartRender>>);

/// What renders a part: its file, and its text with the parameters set.
type Key = (PathBuf, String);

fn key(instance: &Instance) -> Key {
    (instance.path.clone(), instance.source_with_params())
}

/// Rendered parts, kept from one render of the circuit to the next.
#[derive(Default)]
pub struct PartCache {
    parts: HashMap<Key, Arc<PartRender>>,
    enclosure: Option<(EnclosureKey, Arc<PartRender>)>,
    collisions: Option<(CollisionKey, Vec<Collision>)>,
    fits: Option<(CollisionKey, Vec<PartFit>)>,
    /// How many times collisions were computed, for tests.
    collision_runs: usize,
}

/// The parts placed by `circuit`, and its enclosure, as the collisions and
/// fits know them.
fn collision_key(circuit: &Circuit, parts: &[Arc<PartRender>], enclosure: Option<&Arc<PartRender>>) -> CollisionKey {
    let placed = parts.iter().zip(&circuit.parts).map(|(part, instance)| (part.clone(), instance.matrix())).collect();
    (placed, enclosure.cloned())
}

/// Whether two keys hold the same renders, by identity, at the same places.
fn same_key(a: &CollisionKey, b: &CollisionKey) -> bool {
    a.0.len() == b.0.len()
        && a.0.iter().zip(&b.0).all(|(a, b)| Arc::ptr_eq(&a.0, &b.0) && a.1 == b.1)
        && match (&a.1, &b.1) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
}

impl PartCache {
    /// The number of part renders kept.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// Renders the parts of `circuit` it does not have yet, several at a
    /// time, and forgets those the circuit no longer uses. Returns the
    /// renders, in the order of the circuit's parts.
    pub fn update(
        &mut self,
        circuit: &Circuit,
        library_paths: &[PathBuf],
        render: impl Fn(&str, &Path, &[PathBuf]) -> PartRender + Sync,
    ) -> Vec<Arc<PartRender>> {
        let keys: Vec<Key> = circuit.parts.iter().map(key).collect();
        let mut missing: Vec<(&Key, &Instance)> = Vec::new();
        let mut seen = HashSet::new();
        for (key, instance) in keys.iter().zip(&circuit.parts) {
            if !self.parts.contains_key(key) && seen.insert(key) {
                missing.push((key, instance));
            }
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(missing.len());
        let next = AtomicUsize::new(0);
        let mut rendered: Vec<(usize, PartRender)> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..threads)
                .map(|_| {
                    let (missing, next, render) = (&missing, &next, &render);
                    std::thread::Builder::new()
                        .name("c2f-part".into())
                        .stack_size(crate::worker::STACK)
                        .spawn_scoped(scope, move || {
                            let mut done = Vec::new();
                            loop {
                                let i = next.fetch_add(1, Ordering::Relaxed);
                                let Some((_, instance)) = missing.get(i) else { break };
                                done.push((i, render(&instance.source_with_params(), &instance.path, library_paths)));
                            }
                            done
                        })
                        .expect("spawning a part render thread")
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });
        rendered.sort_by_key(|(i, _)| *i);
        for (i, part) in rendered {
            self.parts.insert(missing[i].0.clone(), Arc::new(part));
        }
        let used: HashSet<&Key> = keys.iter().collect();
        self.parts.retain(|key, _| used.contains(key));
        keys.iter().map(|key| self.parts[key].clone()).collect()
    }

    /// The enclosure of the circuit whose text is `source`, rendered by
    /// `render` from its file's text when that or the circuit changed.
    pub fn enclosure(
        &mut self,
        circuit: &Circuit,
        render: impl FnOnce(&str, &Path) -> PartRender,
    ) -> Option<Arc<PartRender>> {
        let Some(path) = &circuit.enclosure else {
            self.enclosure = None;
            return None;
        };
        let text = match c2f_eval::read_source(path) {
            Ok(text) => text,
            Err(err) => {
                let line = Line::new(Level::Error, tr!("Cannot read the enclosure {}: {}", "Impossible de lire le boîtier {} : {}", path.display(), err));
                return Some(Arc::new(PartRender::failed(vec![line])));
            }
        };
        // What the enclosure reads of the circuit: its parts, their files,
        // parameters and places. Not the wires: a wire's edit leaves it be.
        let mut inputs = String::new();
        for part in &circuit.parts {
            for field in [&part.name, &part.path.to_string_lossy().into_owned(), &part.source_with_params()] {
                inputs.push_str(field);
                inputs.push('\0');
            }
            inputs.push_str(&format!("{:?}\0", part.matrix()));
        }
        let key = (path.clone(), text, inputs);
        match &self.enclosure {
            Some((known, render)) if *known == key => Some(render.clone()),
            _ => {
                let rendered = Arc::new(render(&key.1, path));
                self.enclosure = Some((key, rendered.clone()));
                Some(rendered)
            }
        }
    }

    /// [`collisions`], computed again only when a part's render or place,
    /// or the enclosure's render, changed: not for a wire's color.
    pub fn collisions(
        &mut self,
        circuit: &Circuit,
        parts: &[Arc<PartRender>],
        enclosure: Option<&Arc<PartRender>>,
    ) -> Vec<Collision> {
        let key = collision_key(circuit, parts, enclosure);
        if let Some((known, found)) = &self.collisions {
            if same_key(known, &key) {
                return found.clone();
            }
        }
        self.collision_runs += 1;
        let found = collisions(circuit, parts, enclosure.map(|e| &**e));
        self.collisions = Some((key, found.clone()));
        found
    }

    /// How each part fits ([`c2f_geom::fit`]), computed again when the
    /// collisions are.
    pub fn fits(
        &mut self,
        circuit: &Circuit,
        parts: &[Arc<PartRender>],
        enclosure: Option<&Arc<PartRender>>,
    ) -> Vec<PartFit> {
        let key = collision_key(circuit, parts, enclosure);
        if let Some((known, found)) = &self.fits {
            if same_key(known, &key) {
                return found.clone();
            }
        }
        let solids: Vec<(&Model, c2f_geom::Placement)> =
            circuit.parts.iter().zip(parts).map(|(instance, part)| (&part.model, instance.matrix())).collect();
        let found = c2f_geom::fit(circuit, &solids, enclosure.map(|e| &e.model));
        self.fits = Some((key, found.clone()));
        found
    }
}

/// Two solids of a circuit that share some volume.
#[derive(Clone, Debug, PartialEq)]
pub struct Collision {
    /// A part, by index.
    pub part: usize,
    /// The other part, or `None` for the enclosure.
    pub other: Option<usize>,
    /// In cubic millimeters.
    pub volume: f64,
}

/// Where the parts, placed, run into each other or into the enclosure.
pub fn collisions(circuit: &Circuit, parts: &[Arc<PartRender>], enclosure: Option<&PartRender>) -> Vec<Collision> {
    const IDENTITY: c2f_geom::Placement =
        [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    let mut solids: Vec<(&Model, c2f_geom::Placement)> =
        circuit.parts.iter().zip(parts).map(|(instance, part)| (&part.model, instance.matrix())).collect();
    if let Some(enclosure) = enclosure {
        solids.push((&enclosure.model, IDENTITY));
    }
    c2f_geom::collisions(&solids)
        .into_iter()
        .map(|(a, b, volume)| Collision { part: a, other: (b < circuit.parts.len()).then_some(b), volume })
        .collect()
}

/// A console warning for each collision.
pub fn collision_lines(circuit: &Circuit, collisions: &[Collision]) -> Vec<Line> {
    collisions
        .iter()
        .map(|c| {
            let name = |i: usize| format!("`{}`", circuit.parts[i].name);
            let what = match c.other {
                Some(other) => tr!("{} and {} overlap", "{} et {} se chevauchent", name(c.part), name(other)),
                None => tr!("{} runs into the enclosure", "{} heurte le boîtier", name(c.part)),
            };
            Line::new(Level::Warning, tr!("Collision: {} by {:.2} mm³", "Collision : {} de {:.2} mm³", what, c.volume))
        })
        .collect()
}

/// A part that collides, as it shows.
const COLLIDING: [f32; 4] = [0.95, 0.16, 0.14, 1.0];
/// A part closer to the enclosure or another part than the clearance.
const TIGHT: [f32; 4] = [0.98, 0.62, 0.10, 1.0];
/// An uncolored enclosure, and how much of it shows through: the view
/// scales this to the opacity its slider sets.
const ENCLOSURE: [f32; 4] = [0.74, 0.80, 0.88, 1.0];
pub const ENCLOSURE_ALPHA: f32 = 0.28;

/// The scene of a circuit: its parts placed, and where each one's
/// triangles are in the solid. The console gets each part file's lines
/// once, then the collisions and what `fits` finds besides.
pub fn scene(
    circuit: &Circuit,
    parts: &[Arc<PartRender>],
    enclosure: Option<&PartRender>,
    collisions: &[Collision],
    fits: &[PartFit],
    source: &str,
    console: &mut Vec<Line>,
) -> Scene {
    let mut solid = RenderMesh::default();
    let mut highlights = RenderMesh::default();
    let mut backgrounds = RenderMesh::default();
    let mut ranges = Vec::with_capacity(parts.len());
    let mut triangles = 0;
    let mut reported = HashSet::new();
    for (index, (instance, part)) in circuit.parts.iter().zip(parts).enumerate() {
        let matrix = instance.matrix();
        let start = solid.positions.len();
        place(&mut solid, &part.solid, &matrix, Some(part_tint(instance, index)));
        let tint = if collisions.iter().any(|c| c.part == index || c.other == Some(index)) {
            Some(COLLIDING)
        } else {
            fits.get(index).filter(|fit| fit.tight(c2f_geom::CLEARANCE)).map(|_| TIGHT)
        };
        if let Some(tint) = tint {
            for color in &mut solid.colors[start..] {
                *color = std::array::from_fn(|i| color[i] * 0.3 + tint[i] * 0.7);
            }
        }
        ranges.push(start..solid.positions.len());
        place(&mut highlights, &part.highlights, &matrix, None);
        place(&mut backgrounds, &part.backgrounds, &matrix, None);
        triangles += part.triangles;
        if reported.insert(Arc::as_ptr(part)) {
            console.extend(part.console.iter().cloned());
        }
    }
    let start = solid.positions.len();
    if let Some(enclosure) = enclosure {
        const IDENTITY: [[f64; 4]; 4] =
            [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        place(&mut solid, &enclosure.solid, &IDENTITY, Some(ENCLOSURE));
        for color in &mut solid.colors[start..] {
            color[3] *= ENCLOSURE_ALPHA;
        }
        triangles += enclosure.triangles;
        console.extend(enclosure.console.iter().cloned());
    }
    let enclosure_range = start..solid.positions.len();
    console.extend(collision_lines(circuit, collisions));
    for (index, fit) in fits.iter().enumerate() {
        let warnings = fit.warnings(circuit, index, c2f_geom::CLEARANCE, false);
        console.extend(warnings.into_iter().map(|warning| Line::new(Level::Warning, warning)));
    }
    let vertices = solid.positions.len();
    Scene {
        solid,
        highlights,
        backgrounds,
        is_2d: false,
        stats: Stats { triangles, vertices, volume: 0.0 },
        // Circuits are shown, never exported.
        formats: &[],
        pauses: Vec::new(),
        source: source.to_string(),
        parts: ranges,
        enclosure: enclosure_range,
        // A circuit's parts are placed by the circuit, not written in part().
        exploded: None,
        picking: None,
    }
}

/// The line (0-based) of the first problem written in the circuit file
/// itself, among those reading it found.
pub fn error_line(errors: &[c2f_circuit::Error], path: &Path, source: &str) -> Option<usize> {
    let err = errors.iter().find(|e| e.path == path && e.src == source)?;
    let offset = (err.diagnostic.span.start as usize).min(source.len());
    Some(source.as_bytes()[..offset].iter().filter(|&&b| b == b'\n').count())
}

/// Prefixes a part's lines with its file's name, but for errors that
/// name it already.
pub fn label_lines(lines: Vec<Line>, path: &Path) -> Vec<Line> {
    let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
    lines
        .into_iter()
        .map(|line| match line.level {
            Level::Error if line.text.contains(&name) => line,
            _ => Line::new(line.level, tr!("{}: {}", "{} : {}", name, line.text)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// A folder with a circuit's parts: a board colored in part, and a
    /// part without colors.
    fn parts_folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("code2form-circuit-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("board.c2f"),
            "// @pin A [0, 0, 1] up\n// @pin B [10, 0, 1] up\nw = 10;\ncolor(\"green\") cube([w, 5, 1]);\ntranslate([0, 0, 1]) cube(1);\n",
        )
        .unwrap();
        std::fs::write(dir.join("plain.c2f"), "// @pin X [0, 0, 0] -z\ncube(2);\n").unwrap();
        dir
    }

    const CIRCUIT: &str = r#"
[parts.a]
part = "board.c2f"

[parts.b]
part = "board.c2f"
at = [0, 20, 0]
rotate = [0, 0, 90]

[parts.c]
part = "board.c2f"
at = [30, 0, 0]
params = { w = 20 }

[parts.d]
part = "plain.c2f"
at = [0, 0, 5]
color = "red"

[[wires]]
from = "a.A"
to = "b.B"
"#;

    fn render_counted(count: &AtomicUsize) -> impl Fn(&str, &Path, &[PathBuf]) -> PartRender + Sync + '_ {
        move |source, path, libraries| {
            count.fetch_add(1, Ordering::Relaxed);
            crate::worker::render_part(source, path, libraries)
        }
    }

    #[test]
    fn parts_render_once_per_file_and_parameters_and_are_placed() {
        let dir = parts_folder("place");
        let path = dir.join("test.c2fc");
        let circuit = c2f_circuit::parse(CIRCUIT, &path, &[]).unwrap();
        let mut cache = PartCache::default();
        let count = AtomicUsize::new(0);
        let parts = cache.update(&circuit, &[], render_counted(&count));
        // a and b share a render; c has its own parameters.
        assert_eq!(count.load(Ordering::Relaxed), 3);
        assert!(Arc::ptr_eq(&parts[0], &parts[1]));
        assert_eq!(cache.len(), 3);

        let mut console = Vec::new();
        let scene = scene(&circuit, &parts, None, &[], &[], CIRCUIT, &mut console);
        assert_eq!(scene.parts.len(), 4);
        let bounds = |i: usize| {
            let r = scene.parts[i].clone();
            RenderMesh { positions: scene.solid.positions[r].to_vec(), ..RenderMesh::default() }.bounds().unwrap()
        };
        assert_eq!(bounds(0), ([0.0, 0.0, 0.0], [10.0, 5.0, 2.0]));
        // Turned a quarter around z, then moved 20 along y.
        let (lo, hi) = bounds(1);
        assert!((lo[0] + 5.0).abs() < 1e-5 && (hi[1] - 30.0).abs() < 1e-5, "{lo:?} {hi:?}");
        assert_eq!(bounds(2).1, [50.0, 5.0, 2.0]);
        assert_eq!(bounds(3), ([0.0, 0.0, 5.0], [2.0, 2.0, 7.0]));

        // `color()` stays; uncolored shapes take the part's tint, its
        // `color` when the circuit gives one.
        let colors = |i: usize| -> HashSet<[u32; 4]> {
            scene.solid.colors[scene.parts[i].clone()].iter().map(|c| c.map(f32::to_bits)).collect()
        };
        let bits = |name: &str| parse_color(name).unwrap().map(f32::to_bits);
        assert!(colors(0).contains(&bits("green")));
        assert!(!colors(0).contains(&UNCOLORED.map(f32::to_bits)));
        assert_ne!(colors(0), colors(1), "the same board twice stands apart");
        assert_eq!(colors(3), HashSet::from([bits("red")]));

        // Another render reuses all three; dropping a part forgets it.
        cache.update(&circuit, &[], render_counted(&count));
        assert_eq!(count.load(Ordering::Relaxed), 3);
        let fewer = CIRCUIT.replace("params = { w = 20 }", "");
        let circuit = c2f_circuit::parse(&fewer, &path, &[]).unwrap();
        cache.update(&circuit, &[], render_counted(&count));
        assert_eq!((count.load(Ordering::Relaxed), cache.len()), (3, 2));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Grogu, its circuit's text changed by `edit`: the circuit, its parts
    /// rendered and its plate.
    fn grogu(edit: impl Fn(&str) -> String) -> (Circuit, Vec<Arc<PartRender>>, Arc<PartRender>) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/grogu/grogu.c2fc");
        let path = std::fs::canonicalize(path).unwrap();
        let source = edit(&std::fs::read_to_string(&path).unwrap());
        let libraries = c2f_eval::default_library_paths();
        let circuit = c2f_circuit::parse(&source, &path, &libraries).unwrap_or_else(|e| panic!("{}", e[0].render()));
        let mut cache = PartCache::default();
        let parts = cache.update(&circuit, &libraries, crate::worker::render_part);
        for part in &parts {
            assert!(!part.console.iter().any(|l| l.level == Level::Error), "{:?}", part.console);
        }
        let enclosure = cache
            .enclosure(&circuit, |text, file| {
                crate::worker::render_enclosure(text, file, &path, &circuit, &libraries)
            })
            .expect("Grogu has an enclosure");
        assert!(enclosure.triangles > 0, "{:?}", enclosure.console);
        (circuit, parts, enclosure)
    }

    /// The collisions of Grogu, its circuit's text changed by `edit`, in
    /// its plate.
    fn grogu_collisions(edit: impl Fn(&str) -> String) -> (Circuit, Vec<Collision>) {
        let (circuit, parts, enclosure) = grogu(edit);
        let found = collisions(&circuit, &parts, Some(&enclosure));
        (circuit, found)
    }

    #[test]
    fn grogu_parts_closer_than_the_clearance_show_in_amber() {
        let (circuit, parts, enclosure) = grogu(str::to_string);
        let fits = PartCache::default().fits(&circuit, &parts, Some(&enclosure));
        let index = |name: &str| circuit.parts.iter().position(|p| p.name == name).unwrap();
        // The XIAO sits 0.3 mm inside its fence; the touch plate has 1 mm.
        let (xiao, touch) = (index("xiao"), index("touch"));
        assert!(fits[xiao].tight(c2f_geom::CLEARANCE) && !fits[touch].tight(c2f_geom::CLEARANCE), "{fits:?}");
        let mut console = Vec::new();
        let tinted = scene(&circuit, &parts, Some(&enclosure), &[], &fits, "", &mut console);
        let plain = scene(&circuit, &parts, Some(&enclosure), &[], &[], "", &mut Vec::new());
        let colors = |scene: &Scene, part: usize| scene.solid.colors[scene.parts[part].clone()].to_vec();
        assert_ne!(colors(&tinted, xiao), colors(&plain, xiao));
        assert_eq!(colors(&tinted, touch), colors(&plain, touch));
        assert!(console.iter().any(|l| l.level == Level::Warning && l.text.starts_with("`xiao` is 0.30 mm from the enclosure at [")));
    }

    #[test]
    fn grogu_sits_in_its_plate_without_collisions() {
        let (_, found) = grogu_collisions(str::to_string);
        assert_eq!(found, []);
    }

    #[test]
    fn a_part_moved_into_the_plate_collides() {
        // The touch plate sunk 1 mm into the plate.
        let (circuit, found) = grogu_collisions(|text| text.replace("at = [22, 0, 0]", "at = [22, 0, -1]"));
        let touch = circuit.parts.iter().position(|p| p.name == "touch").unwrap();
        assert!(found.iter().any(|c| c.part == touch && c.other.is_none() && c.volume > 1.0), "{found:?}");
        let lines = collision_lines(&circuit, &found);
        assert!(lines.iter().any(|l| l.level == Level::Warning && l.text.contains("`touch` runs into the enclosure")));
    }

    #[test]
    fn collisions_and_the_enclosure_follow_what_they_come_from() {
        let dir = parts_folder("enclosure");
        std::fs::write(dir.join("box.c2f"), "translate([-5, -5, -1]) cube([60, 40, 1.5]);\n").unwrap();
        let path = dir.join("test.c2fc");
        let source = format!("enclosure = \"box.c2f\"\n{CIRCUIT}");
        let circuit = c2f_circuit::parse(&source, &path, &[]).unwrap();
        let mut cache = PartCache::default();
        let parts = cache.update(&circuit, &[], crate::worker::render_part);
        let renders = AtomicUsize::new(0);
        let render = |text: &str, file: &Path| {
            renders.fetch_add(1, Ordering::Relaxed);
            crate::worker::render_enclosure(text, file, &path, &circuit, &[])
        };
        let enclosure = cache.enclosure(&circuit, render).unwrap();
        let found = cache.collisions(&circuit, &parts, Some(&enclosure));
        // Every board sinks 0.5 mm into the box; d sits in a, which
        // overlaps b turned onto it.
        assert!(found.iter().filter(|c| c.other.is_none()).count() >= 3, "{found:?}");
        // The same parts again: neither the enclosure nor the collisions
        // run again.
        let again = cache.enclosure(&circuit, |t, f| crate::worker::render_enclosure(t, f, &path, &circuit, &[]));
        assert!(Arc::ptr_eq(&again.unwrap(), &enclosure));
        assert_eq!(cache.collisions(&circuit, &parts, Some(&enclosure)), found);
        assert_eq!((renders.load(Ordering::Relaxed), cache.collision_runs), (1, 1));

        // A part file changed on disk renders the enclosure again.
        std::fs::write(dir.join("plain.c2f"), "// @pin X [0, 0, 0] -z\ncube(3);\n").unwrap();
        let circuit = c2f_circuit::parse(&source, &path, &[]).unwrap();
        let parts = cache.update(&circuit, &[], crate::worker::render_part);
        let changed = cache.enclosure(&circuit, |t, f| crate::worker::render_enclosure(t, f, &path, &circuit, &[]));
        assert!(!Arc::ptr_eq(&changed.clone().unwrap(), &enclosure));
        cache.collisions(&circuit, &parts, changed.as_ref());
        assert_eq!(cache.collision_runs, 2);

        // A wire's color changed: the enclosure stays as it is.
        let recolored = format!("{source}\n[[wires]]\nfrom = \"a.A\"\nto = \"b.B\"\ncolor = \"red\"\n");
        let circuit = c2f_circuit::parse(&recolored, &path, &[]).unwrap();
        let kept = cache.enclosure(&circuit, |_, _| panic!("the enclosure rendered again for a wire"));
        assert!(Arc::ptr_eq(&kept.unwrap(), &changed.unwrap()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn errors_in_the_circuit_point_at_its_line() {
        let dir = parts_folder("errors");
        let path = dir.join("test.c2fc");
        let source = CIRCUIT.replace("to = \"b.B\"", "to = \"b.Q\"");
        let errors = c2f_circuit::parse(&source, &path, &[]).unwrap_err();
        assert_eq!(error_line(&errors, &path, &source), Some(source.lines().position(|l| l.contains("b.Q")).unwrap()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reads_wire_colors() {
        assert_eq!(parse_color("Red"), Some([1.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse_color("#00ff00"), Some([0.0, 1.0, 0.0, 1.0]));
        assert_eq!(parse_color("#0000ff80").map(|c| c[3]), Some(128.0 / 255.0));
        assert_eq!(parse_color("grey"), parse_color("gray"));
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("no such color"), None);
        assert!(NAMES.windows(2).all(|w| w[0].0 != w[1].0));
    }
}
