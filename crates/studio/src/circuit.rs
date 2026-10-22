//! Circuits in the 3D view: each part of a `.c2fc` file rendered as a
//! design of its own, placed where the circuit puts it.
//!
//! Parts render on the worker, several at a time, and are kept by their
//! file, parameters and text: a part used twice renders once, and an edit
//! to the circuit re-renders only the parts it changed.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use c2f_circuit::{Circuit, Instance};
use c2f_geom::{RenderMesh, Stats};

use crate::console::{Level, Line};
use crate::worker::Scene;

/// The color c2f-geom gives what no `color()` colors.
const UNCOLORED: [f32; 4] = [0.976, 0.843, 0.173, 1.0];

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
}

/// A part file rendered once, in its own coordinates.
pub struct PartRender {
    pub solid: RenderMesh,
    pub highlights: RenderMesh,
    pub backgrounds: RenderMesh,
    pub triangles: usize,
    /// Echo lines, warnings and errors, prefixed with the file's name.
    pub console: Vec<Line>,
}

/// What renders a part: its file, and its text with the parameters set.
type Key = (PathBuf, String);

fn key(instance: &Instance) -> Key {
    (instance.path.clone(), instance.source_with_params())
}

/// Rendered parts, kept from one render of the circuit to the next.
#[derive(Default)]
pub struct PartCache {
    parts: HashMap<Key, Arc<PartRender>>,
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
}

/// The scene of a circuit: its parts placed, and where each one's
/// triangles are in the solid. The console gets each part file's lines
/// once.
pub fn scene(circuit: &Circuit, parts: &[Arc<PartRender>], source: &str, console: &mut Vec<Line>) -> Scene {
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
        ranges.push(start..solid.positions.len());
        place(&mut highlights, &part.highlights, &matrix, None);
        place(&mut backgrounds, &part.backgrounds, &matrix, None);
        triangles += part.triangles;
        if reported.insert(Arc::as_ptr(part)) {
            console.extend(part.console.iter().cloned());
        }
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
            _ => Line::new(line.level, format!("{name}: {}", line.text)),
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
        let scene = scene(&circuit, &parts, CIRCUIT, &mut console);
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
