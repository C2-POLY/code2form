//! The engine the iPad app calls: renders, parameters, errors, exports
//! and cancelling, through the API Swift sees.

use std::sync::Arc;

use c2f_mobile::{Engine, EngineError, ExportFormat, ParameterValue, Request, Widget};

const NIGHTLIGHT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/nightlight/nightlight.c2f");

fn engine() -> Arc<Engine> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    Engine::new(vec![format!("{root}/fonts")], vec![format!("{root}/library")]).expect("an engine")
}

fn request(path: &str, source: &str, parameters: &[(&str, &str)]) -> Request {
    Request {
        source: source.into(),
        path: path.into(),
        parameters: parameters.iter().map(|(name, value)| ParameterValue { name: (*name).into(), value: (*value).into() }).collect(),
    }
}

fn nightlight(parameters: &[(&str, &str)]) -> Request {
    request(NIGHTLIGHT, &std::fs::read_to_string(NIGHTLIGHT).expect("the example"), parameters)
}

/// `c2f stl` and the MCP server's `render` tool give the night light
/// 13772 triangles, a volume of 34218.1639620145 and an 80 x 80 x 55.986
/// box from (-40, -40, 0).
#[test]
fn the_night_light_renders_as_c2f_renders_it() {
    let rendering = engine().render(nightlight(&[])).expect("renders");
    assert!(rendering.error.is_none(), "{:?}", rendering.error);
    assert!(rendering.warnings.is_empty(), "{:?}", rendering.warnings);
    assert_eq!(rendering.triangles, 13772);
    assert!((rendering.volume - 34218.1639620145).abs() < 1e-6, "{}", rendering.volume);
    let bounds = rendering.bounds.expect("a model");
    assert_eq!(bounds.min, [-40.0, -40.0, 0.0]);
    assert_eq!(&bounds.max[..2], [40.0, 40.0]);
    assert!((bounds.max[2] - 55.9860792743855).abs() < 1e-9, "{:?}", bounds.max);
    // Little-endian f32s: three per position and normal, four per color.
    let mesh = rendering.mesh;
    assert!(!mesh.positions.is_empty());
    assert_eq!(mesh.positions.len() % (9 * 4), 0);
    assert_eq!(mesh.normals.len(), mesh.positions.len());
    assert_eq!(mesh.colors.len() / 16, mesh.positions.len() / 12);
    let z: Vec<f32> = mesh.positions.chunks(4).skip(2).step_by(3).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
    let top = z.iter().copied().fold(f32::MIN, f32::max);
    assert!((f64::from(top) - 55.9860792743855).abs() < 1e-4, "{top}");
}

#[test]
fn parameters_come_with_their_group_value_and_widget() {
    let source = std::fs::read_to_string(NIGHTLIGHT).expect("the example");
    let parameters = engine().parameters(source, NIGHTLIGHT.into()).expect("loads");
    let base = parameters.iter().find(|p| p.name == "base_d").expect("base_d");
    assert_eq!(base.group, "Base");
    assert_eq!(base.value, "80");
    assert_eq!(base.description.as_deref(), Some("Diameter of the base."));
    assert!(matches!(base.widget, Widget::Slider { min: 60.0, max: 120.0, step: 1.0 }), "slider 60 to 120");
    assert!(parameters.iter().all(|p| p.group != "Hidden"));
}

#[test]
fn a_value_given_to_a_parameter_changes_the_model() {
    let rendering = engine().render(nightlight(&[("base_d", "100")])).expect("renders");
    let bounds = rendering.bounds.expect("a model");
    assert_eq!((bounds.min[0], bounds.max[0]), (-50.0, 50.0));
}

/// The program changed since the values were chosen: a value that no
/// longer applies is left out, with a warning, and the model still shows.
#[test]
fn a_value_that_no_longer_applies_is_left_out() {
    let rendering = engine().render(nightlight(&[("base_d", "\"wide\""), ("nothing", "1"), ("base_h", "40")])).expect("renders");
    assert!(rendering.error.is_none(), "{:?}", rendering.error);
    assert_eq!(rendering.bounds.expect("a model").min[0], -40.0, "base_d keeps its default");
    assert_eq!(rendering.warnings.len(), 2, "{:?}", rendering.warnings);
    assert!(rendering.warnings[0].contains("`base_d`"), "{:?}", rendering.warnings);
    assert!(rendering.warnings[1].contains("no such parameter"), "{:?}", rendering.warnings);
}

/// Swift calls from threads of 512 KiB, where 40 nested transformations
/// once overflowed the stack: the engine renders on a thread of its own.
#[test]
fn a_deep_model_renders_when_called_from_a_small_stack() {
    let engine = engine();
    let deep = request(
        "/tmp/deep.c2f",
        "module nest(n) { if (n > 0) translate([0.001, 0, 0]) nest(n - 1); else cube(1); }\nnest(1000);",
        &[],
    );
    let small = std::thread::Builder::new().stack_size(512 * 1024);
    let rendering = small.spawn(move || engine.render(deep)).expect("a thread").join().expect("no overflow").expect("renders");
    assert!(rendering.error.is_none(), "{:?}", rendering.error);
    assert!((rendering.volume - 1.0).abs() < 1e-9);
}

#[test]
fn a_syntax_error_is_given_at_its_line_and_column() {
    let rendering = engine().render(request("/tmp/part.c2f", "cube(10);\nsphere(5;\n", &[])).expect("a rendering");
    let problem = rendering.error.expect("an error");
    assert_eq!((problem.line, problem.column), (Some(2), Some(9)), "{problem:?}");
    assert_eq!(problem.file.as_deref(), Some("/tmp/part.c2f"));
    assert!(rendering.mesh.positions.is_empty());

    let err = engine().parameters("x = ;".into(), "/tmp/part.c2f".into()).err().expect("refused");
    assert!(matches!(err, EngineError::Program { problem: c2f_mobile::Problem { line: Some(1), .. } }), "{err}");
}

#[test]
fn an_error_while_running_keeps_what_was_echoed() {
    let rendering = engine().render(request("/tmp/part.c2f", "echo(\"before\");\nassert(false, \"too thin\");", &[])).expect("a rendering");
    let problem = rendering.error.expect("an error");
    assert!(problem.message.contains("too thin"), "{problem:?}");
    assert_eq!(problem.line, None);
    assert_eq!(rendering.echo, ["ECHO: \"before\""]);
}

#[test]
fn exports_write_stl_and_3mf() {
    let dir = std::env::temp_dir().join(format!("c2f-mobile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a folder");
    let stl = dir.join("nightlight.stl");
    let written = engine().export(nightlight(&[]), ExportFormat::Stl, stl.display().to_string()).expect("exports");
    // A binary STL: an 80-byte header, a count, 50 bytes a triangle.
    assert_eq!(written, 84 + 50 * 13772);
    assert_eq!(std::fs::metadata(&stl).expect("written").len(), written);

    let three_mf = dir.join("nightlight.3mf");
    let written = engine().export(nightlight(&[]), ExportFormat::ThreeMf, three_mf.display().to_string()).expect("exports");
    let bytes = std::fs::read(&three_mf).expect("written");
    assert_eq!(bytes.len() as u64, written);
    assert_eq!(&bytes[..2], b"PK", "a ZIP package");

    let err = engine().export(request("/tmp/flat.c2f", "square(10);", &[]), ExportFormat::ThreeMf, three_mf.display().to_string());
    assert!(matches!(err, Err(EngineError::Export { .. })));
    // Only the two files: nothing half-written left beside them.
    assert_eq!(std::fs::read_dir(&dir).expect("the folder").count(), 2);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_render_gives_up_once_cancelled() {
    let engine = engine();
    // Five million elements, a second or so to evaluate: well over the
    // time it takes to cancel it. (One range stops at a million.)
    let slow = request("/tmp/slow.c2f", "n = len([for (i = [0 : 999], j = [0 : 4999]) i]);\ncube(n);", &[]);
    let running = {
        let engine = Arc::clone(&engine);
        std::thread::spawn(move || engine.render(slow))
    };
    std::thread::sleep(std::time::Duration::from_millis(50));
    engine.cancel();
    assert!(matches!(running.join().expect("no panic"), Err(EngineError::Cancelled)));
    // The engine renders again afterwards.
    let rendering = engine.render(request("/tmp/cube.c2f", "cube(10);", &[])).expect("renders");
    assert!((rendering.volume - 1000.0).abs() < 1e-9);
}
