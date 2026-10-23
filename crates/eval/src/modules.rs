//! Built-in modules: primitives, transformations, booleans, extrusions.
//! Parameter defaults and fallbacks follow the reference implementation
//! (release 2021.01).

use c2f_lang::ast::Stmt;

use crate::{color, enclosure};
use crate::env::{self, Env, Frame};
use crate::interp::{ArgValue, EResult, Interp};
use crate::math::{cos_degrees, sin_degrees};
use crate::node::*;
use crate::value::Value;

/// Bound arguments of a builtin call.
pub(crate) struct Params<'p> {
    values: Vec<(&'static str, Value<'p>)>,
    /// Holds the call's `$` arguments; children and `$fn` lookups use it.
    frame: Env<'p>,
}

impl<'p> Params<'p> {
    fn bind(
        interp: &mut Interp<'p>,
        module: &str,
        positional: &[&'static str],
        named: &[&'static str],
        args: Vec<ArgValue<'p>>,
        env: &Env<'p>,
    ) -> Self {
        let frame = Frame::child(env);
        let mut values: Vec<(&'static str, Value<'p>)> = Vec::new();
        let mut next = 0;
        for (name, value) in args {
            match name {
                Some(name) if name.starts_with('$') => frame.set(name, value),
                Some(name) => match positional.iter().chain(named).find(|p| **p == name) {
                    Some(&p) => {
                        values.retain(|(n, _)| *n != p);
                        values.push((p, value));
                    }
                    None => interp.warn(format!("variable {name} not specified as parameter in {module}()")),
                },
                None => {
                    if let Some(&p) = positional.get(next) {
                        values.retain(|(n, _)| *n != p);
                        values.push((p, value));
                    } else {
                        interp.warn(format!("Too many unnamed arguments supplied to {module}()"));
                    }
                    next += 1;
                }
            }
        }
        Params { values, frame }
    }

    pub(crate) fn get(&self, name: &str) -> Value<'p> {
        self.values.iter().find(|(n, _)| *n == name).map_or(Value::Undef, |(_, v)| v.clone())
    }

    pub(crate) fn number(&self, name: &str) -> Option<f64> {
        self.get(name).as_number()
    }

    /// Only a real boolean counts: `center = 1` is false, as in the
    /// reference.
    fn flag(&self, name: &str) -> bool {
        self.get(name).as_bool() == Some(true)
    }

    fn string(&self, name: &str) -> Option<String> {
        self.get(name).as_str().map(str::to_string)
    }

    /// `$fn`, `$fa`, `$fs` as seen by this call. Only circles, spheres and
    /// cylinders clamp `$fa` and `$fs` to 0.01; other nodes keep them as is.
    pub(crate) fn segments(&self, clamp: bool) -> Segments {
        let special = |name: &str| env::lookup(&self.frame, name).and_then(|v| v.as_number()).unwrap_or(0.0);
        // Only values below the floor are raised: NaN stays NaN.
        let floor = |v: f64| if clamp && v < 0.01 { 0.01 } else { v };
        Segments { fn_: special("$fn"), fa: floor(special("$fa")), fs: floor(special("$fs")) }
    }

    /// A radius given either directly or as a diameter (which wins).
    fn radius(&self, interp: &mut Interp<'p>, diameter: &str, radius: &str) -> Option<f64> {
        match self.number(diameter) {
            Some(d) => {
                if !self.get(radius).is_undef() {
                    interp.warn(format!("Ignoring radius variable '{radius}' as diameter '{diameter}' is defined too."));
                }
                Some(d / 2.0)
            }
            None => self.number(radius),
        }
    }
}

/// C's `(int)` cast on x86: NaN, infinities and values out of range give
/// `INT_MIN`.
fn c_int(value: f64) -> i32 {
    if (-2_147_483_648.0..2_147_483_648.0).contains(&value) { value.trunc() as i32 } else { i32::MIN }
}

/// A convexity that must be at least 1 (else `default`).
fn convexity(value: Option<f64>, default: u32) -> u32 {
    match value.map(c_int) {
        Some(n) if n >= 1 => n as u32,
        _ => default,
    }
}

/// A convexity stored unsigned without a lower bound (minkowski, resize).
fn convexity_unsigned(value: Option<f64>) -> u32 {
    value.map_or(0, |v| c_int(v) as u32)
}

fn points(value: &Value) -> Points {
    let rows = value.as_vector().and_then(|rows| rows.iter().map(Value::as_numbers).collect());
    Points { repr: value.repr(), rows }
}

/// Instantiates a builtin; `Ok(None)` when no builtin has that name.
pub fn instantiate<'p>(
    interp: &mut Interp<'p>,
    name: &str,
    args: Vec<ArgValue<'p>>,
    children: &'p [Stmt],
    env: &Env<'p>,
) -> EResult<Option<Option<Node>>> {
    let leaf = |kind| Ok(Some(Some(Node::new(kind, Vec::new()))));
    macro_rules! bind {
        ($positional:expr) => {
            bind!($positional, [])
        };
        ($positional:expr, $named:expr) => {
            Params::bind(interp, name, &$positional, &$named, args, env)
        };
    }
    macro_rules! with_children {
        ($p:expr, $kind:expr) => {{
            let kind = $kind;
            let nodes = interp.eval_children(children, &$p.frame)?;
            Ok(Some(Some(Node::new(kind, nodes))))
        }};
    }

    match name {
        "cube" => {
            let p = bind!(["size", "center"]);
            let mut size = [1.0; 3];
            match p.get("size") {
                Value::Number(n) => size = [n; 3],
                Value::Undef => {}
                v => {
                    if !get_vec(&v, &mut size, None) {
                        interp.warn(format!(
                            "Unable to convert cube(size={}, ...) parameter to a number or a vec3 of numbers",
                            v.repr()
                        ));
                    }
                }
            }
            leaf(NodeKind::Cube { size, center: p.flag("center") })
        }
        "sphere" => {
            let p = bind!(["r"], ["d"]);
            let r = p.radius(interp, "d", "r").unwrap_or(1.0);
            leaf(NodeKind::Sphere { r, segments: p.segments(true) })
        }
        "cylinder" => {
            let p = bind!(["h", "r1", "r2", "center"], ["r", "d", "d1", "d2"]);
            let h = p.number("h").unwrap_or(1.0);
            let (mut r1, mut r2) = (1.0, 1.0);
            if let Some(r) = p.radius(interp, "d", "r") {
                (r1, r2) = (r, r);
            }
            if let Some(r) = p.radius(interp, "d1", "r1") {
                r1 = r;
            }
            if let Some(r) = p.radius(interp, "d2", "r2") {
                r2 = r;
            }
            leaf(NodeKind::Cylinder { h, r1, r2, center: p.flag("center"), segments: p.segments(true) })
        }
        "polyhedron" => {
            let p = bind!(["points", "faces", "convexity"], ["triangles"]);
            let faces = match p.get("faces") {
                Value::Undef => p.get("triangles"),
                faces => faces,
            };
            leaf(NodeKind::Polyhedron {
                points: points(&p.get("points")),
                faces: points(&faces),
                convexity: convexity(p.number("convexity"), 1),
            })
        }
        "square" => {
            let p = bind!(["size", "center"]);
            let mut size = [1.0; 2];
            match p.get("size") {
                Value::Number(n) => size = [n; 2],
                Value::Undef => {}
                // Unlike cube, square keeps nothing of a vector that fails.
                v => {
                    let mut read = size;
                    if get_vec(&v, &mut read, None) {
                        size = read;
                    } else {
                        interp.warn(format!(
                            "Unable to convert square(size={}, ...) parameter to a number or a vec2 of numbers",
                            v.repr()
                        ));
                    }
                }
            }
            leaf(NodeKind::Square { size, center: p.flag("center") })
        }
        "circle" => {
            let p = bind!(["r"], ["d"]);
            let r = p.radius(interp, "d", "r").unwrap_or(1.0);
            leaf(NodeKind::Circle { r, segments: p.segments(true) })
        }
        "polygon" => {
            let p = bind!(["points", "paths", "convexity"]);
            leaf(NodeKind::Polygon {
                points: points(&p.get("points")),
                paths: points(&p.get("paths")),
                convexity: convexity(p.number("convexity"), 1),
            })
        }
        // Not in the reference: a pause for the 3D print, with no geometry.
        "pause" => {
            let p = bind!(["z", "message"]);
            match finite(p.number("z")) {
                Some(z) => {
                    let message = match p.get("message") {
                        Value::Undef => String::new(),
                        other => other.to_display(),
                    };
                    let source = interp.pause_call.take();
                    interp.pauses.push(crate::Pause { z, message, source });
                }
                None => interp.warn(format!("Ignoring pause(): z = {} is not a height", p.get("z").repr())),
            }
            Ok(Some(None))
        }
        // Not in the reference: a part kept apart from the rest of the
        // model, for glTF exports.
        "part" => {
            let p = bind!(["name"], ["joint", "axis", "limits"]);
            let name = match p.get("name") {
                Value::Str(name) => name.to_string(),
                Value::Undef => "part".to_string(),
                other => other.to_display(),
            };
            let joint = joint(interp, &name, &p);
            with_children!(p, NodeKind::Part { name, joint })
        }
        // Not in the reference: an enclosure's standoffs, screw holes and
        // cutouts, and the parts themselves, from the circuit it names.
        "standoffs_for" => {
            let p = bind!(["part", "height"], ["d", "hole", "screw"]);
            Ok(Some(enclosure::standoffs(interp, &p)))
        }
        "holes_for" => {
            let p = bind!(["part", "depth"], ["hole", "screw"]);
            Ok(Some(enclosure::screw_holes(interp, &p)))
        }
        "cutout_for" => {
            let p = bind!(["part", "port"], ["clearance", "depth"]);
            Ok(Some(enclosure::cutout(interp, &p)))
        }
        "circuit_part" => {
            let p = bind!(["part"]);
            Ok(Some(enclosure::part(interp, &p)))
        }
        "text" => {
            let p = bind!(["text", "size", "font"], ["halign", "valign", "spacing", "direction", "language", "script"]);
            leaf(NodeKind::Text(text(&p)))
        }
        "surface" => {
            let p = bind!(["file", "center", "convexity"], ["invert"]);
            let (file, timestamp) = data_file(interp, &p.frame, &p.string("file").unwrap_or_default());
            leaf(NodeKind::Surface { file, center: p.flag("center"), invert: p.flag("invert"), timestamp })
        }
        "import" | "import_stl" | "import_off" | "import_dxf" => {
            let p = bind!(["file", "layer", "convexity", "origin", "scale"], ["filename", "layername", "object"]);
            // `filename`/`layername` are deprecated spellings; anything that
            // is not a string is written as text.
            let text = |value: Value, fallback: Value| match value {
                Value::Undef => match fallback {
                    Value::Undef => String::new(),
                    other => other.to_display(),
                },
                other => other.to_display(),
            };
            let file = text(p.get("file"), p.get("filename"));
            let (file, timestamp) = data_file(interp, &p.frame, &file);
            leaf(NodeKind::Import {
                timestamp,
                file,
                layer: text(p.get("layer"), p.get("layername")),
                origin: vec2(&p.get("origin")).unwrap_or([0.0, 0.0]),
                scale: p.number("scale").filter(|s| *s > 0.0).unwrap_or(1.0),
                convexity: convexity(p.number("convexity"), 1),
                segments: p.segments(false),
                object: text(p.get("object"), Value::Undef),
            })
        }

        "translate" => {
            let p = bind!(["v"]);
            let v = p.get("v");
            let mut t = [0.0; 3];
            let m = if get_vec(&v, &mut t, Some(0.0)) && t.iter().all(|c| c.is_finite()) {
                translation(t[0], t[1], t[2])
            } else {
                interp.warn(format!("Unable to convert translate({}) parameter to a vec3 or vec2 of numbers", v.repr()));
                IDENTITY
            };
            with_children!(p, NodeKind::Transform(m))
        }
        "rotate" => {
            let p = bind!(["a", "v"]);
            let m = rotation(interp, &p.get("a"), &p.get("v"));
            with_children!(p, NodeKind::Transform(m))
        }
        "scale" => {
            let p = bind!(["v"]);
            let v = p.get("v");
            // Components read before a bad one are kept.
            let mut s = [1.0; 3];
            if !get_vec(&v, &mut s, Some(1.0)) {
                match &v {
                    Value::Number(n) => s = [*n; 3],
                    _ => interp.warn(format!(
                        "Unable to convert scale({}) parameter to a number, a vec3 or vec2 of numbers or a number",
                        v.repr()
                    )),
                }
            }
            let m = linear(&diagonal3(s));
            with_children!(p, NodeKind::Transform(m))
        }
        "mirror" => {
            let p = bind!(["v"]);
            let v = p.get("v");
            let mut n = [1.0, 0.0, 0.0];
            if !get_vec(&v, &mut n, Some(0.0)) {
                interp.warn(format!("Unable to convert mirror({}) parameter to a vec3 or vec2 of numbers", v.repr()));
            }
            with_children!(p, NodeKind::Transform(reflection(n)))
        }
        "multmatrix" => {
            let p = bind!(["m"]);
            let mut m = IDENTITY;
            if let Some(rows) = p.get("m").as_vector() {
                for (i, row) in rows.iter().take(4).enumerate() {
                    for (j, cell) in row.as_vector().unwrap_or(&[]).iter().take(4).enumerate() {
                        if let Some(n) = cell.as_number() {
                            m[i][j] = n;
                        }
                    }
                }
            }
            // A homogeneous matrix is normalized by its corner.
            let w = m[3][3];
            if w != 1.0 {
                for row in &mut m {
                    for cell in row.iter_mut() {
                        *cell /= w;
                    }
                }
            }
            with_children!(p, NodeKind::Transform(m))
        }
        "color" => {
            let p = bind!(["c", "alpha"]);
            let mut rgba: [f64; 4] = [-1.0, -1.0, -1.0, 1.0];
            match p.get("c") {
                Value::Vector(items) => {
                    for (i, slot) in rgba.iter_mut().enumerate() {
                        *slot = items.get(i).map_or(1.0, Value::to_f64);
                    }
                }
                Value::Str(name) => match color::parse(&name) {
                    Some(parsed) => rgba = parsed,
                    None => interp.warn(format!("Unable to parse color \"{name}\"")),
                },
                _ => {}
            }
            if let Some(alpha) = p.number("alpha") {
                rgba[3] = alpha;
            }
            // The reference stores colors as single-precision floats.
            let rgba = rgba.map(|c| c as f32 as f64);
            with_children!(p, NodeKind::Color(rgba))
        }
        "offset" => {
            let p = bind!(["r"], ["delta", "chamfer"]);
            let op = match (p.number("r"), p.number("delta")) {
                (Some(r), _) => OffsetOp::Round { r },
                (None, Some(delta)) => OffsetOp::Delta { delta, chamfer: p.flag("chamfer") },
                (None, None) => OffsetOp::Round { r: 1.0 },
            };
            let segments = p.segments(false);
            with_children!(p, NodeKind::Offset { op, segments })
        }
        "resize" => {
            let p = bind!(["newsize", "auto", "convexity"]);
            let mut size = [0.0; 3];
            if let Some(items) = p.get("newsize").as_vector() {
                for (slot, item) in size.iter_mut().zip(items.iter()) {
                    *slot = item.to_f64();
                }
            }
            let auto = match p.get("auto") {
                Value::Bool(b) => [b; 3],
                Value::Vector(items) => {
                    let at = |i: usize| items.get(i).is_some_and(Value::truthy);
                    [at(0), at(1), at(2)]
                }
                _ => [false; 3],
            };
            let convexity = convexity_unsigned(p.number("convexity"));
            with_children!(p, NodeKind::Resize { size, auto, convexity })
        }
        "render" => {
            let p = bind!(["convexity"]);
            let convexity = p.number("convexity").map_or(1, |n| c_int(n) as i64);
            with_children!(p, NodeKind::Render { convexity })
        }
        "projection" => {
            let p = bind!(["cut"], ["convexity"]);
            let convexity = p.number("convexity").map_or(0, c_int);
            with_children!(p, NodeKind::Projection { cut: p.flag("cut"), convexity })
        }
        "minkowski" => {
            let p = bind!(["convexity"]);
            let convexity = convexity_unsigned(p.number("convexity"));
            with_children!(p, NodeKind::Minkowski { convexity })
        }
        "hull" => {
            let p = bind!([]);
            with_children!(p, NodeKind::Hull)
        }
        // The booleans and `group` drop their `$` arguments: the children
        // run in the caller's context.
        "union" | "difference" | "intersection" | "group" => {
            let _ = bind!([]);
            let kind = match name {
                "union" => NodeKind::Union,
                "difference" => NodeKind::Difference,
                "intersection" => NodeKind::Intersection,
                _ => NodeKind::Group,
            };
            let nodes = interp.eval_children(children, env)?;
            Ok(Some(Some(Node::new(kind, nodes))))
        }
        "dxf_linear_extrude" => {
            let p = bind!(LINEAR_EXTRUDE, ["convexity"]);
            {
                let source = file_source(interp, &p, false);
                leaf(linear_extrude(interp, &p, source))
            }
        }
        "dxf_rotate_extrude" => {
            let p = bind!(["file", "layer", "origin", "scale"], ["convexity", "angle"]);
            {
                let source = file_source(interp, &p, true);
                leaf(rotate_extrude(interp, &p, source))
            }
        }
        "linear_extrude" => {
            let mut p = bind!(LINEAR_EXTRUDE, ["convexity"]);
            // A leading number fills `file`; the reference then takes it as
            // the height when that is not given.
            if let Value::Number(_) = p.get("file") {
                if p.get("height").is_undef() {
                    let height = p.get("file");
                    p.values.retain(|(n, _)| *n != "height");
                    p.values.push(("height", height));
                }
            }
            let source = file_source(interp, &p, false);
            let file_based = source.is_some();
            let kind = linear_extrude(interp, &p, source);
            if file_based {
                return leaf(kind);
            }
            with_children!(p, kind)
        }
        "rotate_extrude" => {
            let p = bind!(["file", "layer", "origin", "scale"], ["angle", "convexity"]);
            let source = file_source(interp, &p, true);
            let file_based = source.is_some();
            let kind = rotate_extrude(interp, &p, source);
            if file_based {
                return leaf(kind);
            }
            with_children!(p, kind)
        }
        _ => Ok(None),
    }
}

const LINEAR_EXTRUDE: [&str; 8] = ["file", "layer", "height", "origin", "scale", "center", "twist", "slices"];

/// A finite number, else `None`.
fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite())
}

fn linear_extrude(_interp: &mut Interp, p: &Params, source: Option<FileSource>) -> NodeKind {
    // A finite number or a vector of finite numbers, else 1; negative
    // values become 0 (and -0 stays -0).
    let scale = match p.get("scale") {
        Value::Number(s) if s.is_finite() => [s, s],
        other => vec2(&other).filter(|v| v.iter().all(|c| c.is_finite())).unwrap_or([1.0, 1.0]),
    }
    .map(|s| if s < 0.0 { 0.0 } else { s });
    NodeKind::LinearExtrude {
        height: finite(p.number("height")).map_or(100.0, |h| h.max(0.0)),
        center: p.flag("center"),
        convexity: convexity(p.number("convexity"), 1),
        twist: finite(p.number("twist")).unwrap_or(0.0),
        slices: p.number("slices").map(c_int).filter(|s| *s >= 1).map(|s| s as u32),
        scale,
        segments: p.segments(false),
        source,
    }
}

fn rotate_extrude(_interp: &mut Interp, p: &Params, source: Option<FileSource>) -> NodeKind {
    NodeKind::RotateExtrude {
        angle: p.number("angle").filter(|a| *a > -360.0 && *a <= 360.0).unwrap_or(360.0),
        convexity: convexity(p.number("convexity"), 2),
        segments: p.segments(false),
        source,
    }
}

/// The legacy `file = ...` argument of the extrusions: `linear_extrude`
/// only takes a string, `rotate_extrude` writes anything else as text.
fn file_source<'p>(interp: &mut Interp<'p>, p: &Params<'p>, stringify: bool) -> Option<FileSource> {
    let file = match p.get("file") {
        Value::Str(s) => s.to_string(),
        Value::Undef => return None,
        _ if !stringify => return None,
        other => other.to_display(),
    };
    let (file, timestamp) = data_file(interp, &p.frame, &file);
    Some(FileSource {
        timestamp,
        layer: p.string("layer").unwrap_or_default(),
        origin: vec2(&p.get("origin")).unwrap_or([0.0, 0.0]),
        scale: p.number("scale").unwrap_or(1.0),
        file,
    })
}

/// A data file named by code running in `env`: the name the CSG shows and
/// its modification time in seconds (0 when it does not exist).
fn data_file<'p>(interp: &Interp<'p>, env: &Env<'p>, file: &str) -> (String, i64) {
    let (path, shown) = interp.data_path(env, file);
    let timestamp = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    (shown, timestamp)
}

fn vec2(value: &Value) -> Option<[f64; 2]> {
    value.as_vec_n(2).map(|v| [v[0], v[1]])
}

fn translation(x: f64, y: f64, z: f64) -> Matrix {
    let mut m = IDENTITY;
    m[0][3] = x;
    m[1][3] = y;
    m[2][3] = z;
    m
}

type Matrix3 = [[f64; 3]; 3];

fn diagonal3(d: [f64; 3]) -> Matrix3 {
    [[d[0], 0.0, 0.0], [0.0, d[1], 0.0], [0.0, 0.0, d[2]]]
}

/// A transform whose linear part is `identity × m`, multiplied out the
/// naive way Eigen does it, so a NaN anywhere in `m` spreads as in the
/// reference (0 × NaN is NaN).
fn linear(m: &Matrix3) -> Matrix {
    let mut out = IDENTITY;
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = (0..3).map(|k| IDENTITY[i][k] * m[k][j]).sum();
        }
    }
    out
}

/// The reference's `getVec3`/`getVec2`: a vector of exactly `N` numbers (or
/// one fewer, completed with `last`), read in order. The first non-number
/// fails the conversion; the components read before it stay in `out`.
/// The joint of `part(name, joint = …, axis = …, limits = …)`, with a
/// warning for what does not read; `None` without `joint`.
fn joint<'p>(interp: &mut Interp<'p>, name: &str, p: &Params<'p>) -> Option<Joint> {
    let kind = match p.get("joint") {
        Value::Undef => return None,
        Value::Str(kind) => match JointKind::named(&kind) {
            Some(kind) => kind,
            None => {
                interp.warn(format!(
                    "part(\"{name}\"): unknown joint \"{kind}\"; give \"revolute\", \"continuous\", \"prismatic\" or \"fixed\""
                ));
                return None;
            }
        },
        other => {
            interp.warn(format!("part(\"{name}\"): joint = {} is not a name", other.repr()));
            return None;
        }
    };
    let mut axis = [0.0, 0.0, 1.0];
    match p.get("axis") {
        Value::Undef => {}
        v => {
            let mut read = [0.0; 3];
            if get_vec(&v, &mut read, None) && read.iter().all(|c| c.is_finite()) && read.iter().any(|&c| c != 0.0) {
                let length = read.iter().map(|c| c * c).sum::<f64>().sqrt();
                axis = read.map(|c| c / length);
            } else {
                interp.warn(format!("part(\"{name}\"): axis = {} is not a direction; it turns about Z", v.repr()));
            }
        }
    }
    let limits = match p.get("limits") {
        Value::Undef => None,
        v => {
            let mut read = [0.0; 2];
            if get_vec(&v, &mut read, None) && read.iter().all(|c| c.is_finite()) && read[0] <= read[1] {
                Some(read)
            } else {
                interp.warn(format!("part(\"{name}\"): limits = {} is not [lowest, highest]", v.repr()));
                None
            }
        }
    };
    Some(Joint { kind, axis, limits })
}

fn get_vec<const N: usize>(value: &Value, out: &mut [f64; N], last: Option<f64>) -> bool {
    let Some(items) = value.as_vector() else { return false };
    let n = match (items.len(), last) {
        (len, _) if len == N => N,
        (len, Some(_)) if len + 1 == N => N - 1,
        _ => return false,
    };
    for (slot, item) in out.iter_mut().zip(items.iter()).take(n) {
        match item.as_number() {
            Some(v) => *slot = v,
            None => return false,
        }
    }
    if n < N {
        out[N - 1] = last.unwrap_or(0.0);
    }
    true
}

/// `rotate(a)`, `rotate([x, y, z])` (applied x, then y, then z) or
/// `rotate(a, v)` around an axis.
fn rotation(interp: &mut Interp, a: &Value, v: &Value) -> Matrix {
    match a {
        Value::Vector(items) => {
            // The reference reads z, then y, then x, into one variable: a
            // component that is not a number reuses the previous one.
            let mut ok = items.len() <= 3;
            let mut angle = 0.0;
            let mut sc = [(0.0, 1.0); 3];
            for i in (0..items.len().min(3)).rev() {
                match items[i].as_number() {
                    Some(value) => angle = value,
                    None => ok = false,
                }
                ok &= angle.is_finite();
                sc[i] = (sin_degrees(angle), cos_degrees(angle));
            }
            if !ok {
                interp.warn(format!("Problem converting rotate(a={}) parameter", a.repr()));
            }
            let [(sx, cx), (sy, cy), (sz, cz)] = sc;
            linear(&[
                [cy * cz, cz * sx * sy - cx * sz, cx * cz * sy + sx * sz],
                [cy * sz, cx * cz + sx * sy * sz, -cz * sx + cx * sy * sz],
                [-sy, cy * sx, cx * cy],
            ])
        }
        Value::Number(angle) if !angle.is_finite() => IDENTITY,
        Value::Number(angle) => {
            let mut axis = [0.0, 0.0, 1.0];
            if !v.is_undef() {
                get_vec(v, &mut axis, Some(0.0));
            }
            let squared = axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2];
            if squared == 0.0 {
                return IDENTITY;
            }
            // The reference's angle_axis_degrees: the symmetric part
            // divides by the squared length (no square root, so it stays
            // exact), only the sine part uses the unit axis.
            let (c, s) = (cos_degrees(*angle), sin_degrees(*angle));
            let cx = axis.map(|a| a * ((1.0 - c) / squared));
            let length = squared.sqrt();
            let su = axis.map(|a| s * (a / length));
            linear(&[
                [cx[0] * axis[0] + c, cx[1] * axis[0] - su[2], cx[2] * axis[0] + su[1]],
                [cx[0] * axis[1] + su[2], cx[1] * axis[1] + c, cx[2] * axis[1] - su[0]],
                [cx[0] * axis[2] - su[1], cx[1] * axis[2] + su[0], cx[2] * axis[2] + c],
            ])
        }
        other => {
            interp.warn(format!("Problem converting rotate(a={}) parameter", other.repr()));
            IDENTITY
        }
    }
}

/// Reflection through the plane with normal `n` (identity for a zero `n`).
/// Dividing by the squared length keeps `mirror([1, 1, 0])` exact.
fn reflection(n: [f64; 3]) -> Matrix {
    let squared = n[0] * n[0] + n[1] * n[1] + n[2] * n[2];
    if squared == 0.0 {
        return IDENTITY;
    }
    let mut m = IDENTITY;
    for i in 0..3 {
        for j in 0..3 {
            m[i][j] = if i == j { 1.0 } else { 0.0 } - 2.0 * n[i] * n[j] / squared;
        }
    }
    m
}

fn text(p: &Params) -> TextParams {
    let text = p.string("text").unwrap_or_default();
    let script = match p.string("script").filter(|s| !s.is_empty()) {
        Some(given) if given.chars().count() >= 4 => {
            let head: String = given.chars().take(4).collect::<String>().to_lowercase();
            let mut chars = head.chars();
            Some(chars.next().map_or(String::new(), |c| c.to_uppercase().chain(chars).collect()))
        }
        Some(given) => Some(given),
        None => detect_script(&text).map(str::to_string),
    };
    let rtl = script.as_deref().is_some_and(|s| matches!(s, "Arab" | "Hebr" | "Syrc" | "Thaa" | "Nkoo"));
    TextParams {
        size: p.number("size").unwrap_or(10.0),
        spacing: p.number("spacing").unwrap_or(1.0),
        font: p.string("font").unwrap_or_default(),
        direction: p.string("direction").unwrap_or_else(|| if rtl { "rtl" } else { "ltr" }.into()),
        language: p.string("language").unwrap_or_else(|| "en".into()),
        halign: p.string("halign").unwrap_or_else(|| "left".into()),
        valign: p.string("valign").unwrap_or_else(|| "baseline".into()),
        segments: p.segments(false),
        script,
        text,
    }
}

/// The single script of a text's letters, ignoring digits, spaces and
/// punctuation; `None` when there are none or several.
fn detect_script(text: &str) -> Option<&'static str> {
    let mut found = None;
    for c in text.chars() {
        let Some(script) = script_of(c) else { continue };
        match found {
            None => found = Some(script),
            Some(previous) if previous != script => return None,
            _ => {}
        }
    }
    found
}

fn script_of(c: char) -> Option<&'static str> {
    Some(match c as u32 {
        0x41..=0x5A | 0x61..=0x7A | 0xAA | 0xBA | 0xC0..=0xD6 | 0xD8..=0xF6 | 0xF8..=0x24F | 0x1E00..=0x1EFF => "Latn",
        0x370..=0x373 | 0x375..=0x37D | 0x37F..=0x3FF | 0x1F00..=0x1FFF => "Grek",
        0x400..=0x52F => "Cyrl",
        0x531..=0x58F => "Armn",
        0x591..=0x5F4 => "Hebr",
        0x600..=0x60B | 0x60D..=0x61A | 0x61C..=0x61E | 0x620..=0x63F | 0x641..=0x6FF | 0x750..=0x77F => "Arab",
        0x900..=0x97F => "Deva",
        0xE01..=0xE7F => "Thai",
        0x10A0..=0x10FF => "Geor",
        0x3041..=0x309F => "Hira",
        0x30A1..=0x30FA | 0x30FD..=0x30FF => "Kana",
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF => "Hani",
        0x1100..=0x11FF | 0x3131..=0x318E | 0xAC00..=0xD7A3 => "Hang",
        _ => return None,
    })
}
