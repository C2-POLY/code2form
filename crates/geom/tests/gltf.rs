//! The glTF export: a `.glb` whose accessors, materials and bounds read
//! back as the model, in meters with Y up.

use std::path::Path;

use c2f_geom::GltfOptions;
use serde_json::Value;

/// A `.glb` read back: its JSON and its binary chunk.
struct Gltf {
    json: Value,
    bin: Vec<u8>,
}

impl Gltf {
    fn read(glb: &[u8]) -> Gltf {
        assert_eq!(&glb[..4], b"glTF");
        let word = |at: usize| u32::from_le_bytes(glb[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!((word(4), word(8)), (2, glb.len()), "version and length");
        let json_length = word(12);
        assert_eq!(&glb[16..20], b"JSON");
        let json = serde_json::from_slice(&glb[20..20 + json_length]).unwrap();
        let at = 20 + json_length;
        let bin = if at < glb.len() {
            assert_eq!(&glb[at + 4..at + 8], b"BIN\0");
            glb[at + 8..at + 8 + word(at)].to_vec()
        } else {
            Vec::new()
        };
        Gltf { json, bin }
    }

    /// The bytes of an accessor's items, checked against its view.
    fn bytes(&self, accessor: usize) -> (&Value, &[u8]) {
        let accessor = &self.json["accessors"][accessor];
        let view = &self.json["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
        let offset = view["byteOffset"].as_u64().unwrap() as usize;
        let length = view["byteLength"].as_u64().unwrap() as usize;
        assert_eq!(offset % 4, 0, "aligned");
        (accessor, &self.bin[offset..offset + length])
    }

    fn floats(&self, accessor: usize) -> Vec<f32> {
        let (accessor, bytes) = self.bytes(accessor);
        assert_eq!(accessor["componentType"], 5126);
        bytes.chunks(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect()
    }

    fn indices(&self, accessor: usize) -> Vec<u32> {
        let (accessor, bytes) = self.bytes(accessor);
        assert_eq!(accessor["componentType"], 5125);
        bytes.chunks(4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).collect()
    }

    fn primitives(&self) -> Vec<&Value> {
        self.json["meshes"].as_array().unwrap().iter().flat_map(|m| m["primitives"].as_array().unwrap()).collect()
    }

    /// The corners of all primitives, as written.
    fn positions(&self) -> Vec<[f32; 3]> {
        self.primitives()
            .iter()
            .flat_map(|p| self.floats(p["attributes"]["POSITION"].as_u64().unwrap() as usize))
            .collect::<Vec<_>>()
            .chunks(3)
            .map(|c| [c[0], c[1], c[2]])
            .collect()
    }
}

fn export(src: &str, options: &GltfOptions) -> Result<c2f_geom::Glb, String> {
    let program = c2f_eval::Program::load(src, Path::new("t.scad"), &[]).expect("parses");
    c2f_geom::glb(&c2f_eval::evaluate(&program).nodes, Path::new(""), options)
}

fn gltf(src: &str) -> Gltf {
    Gltf::read(&export(src, &GltfOptions::default()).unwrap().bytes)
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn a_cube_in_meters_with_y_up() {
    // 40 wide (x), 20 deep (y), 10 high (z), in millimeters.
    let model = gltf("translate([0, 5, 0]) cube([40, 20, 10]);");
    assert_eq!(model.json["asset"]["version"], "2.0");
    let primitive = model.primitives()[0];
    let accessor = &model.json["accessors"][primitive["attributes"]["POSITION"].as_u64().unwrap() as usize];
    // Height on Y; the model's +Y is glTF's -Z.
    let (min, max) = (&accessor["min"], &accessor["max"]);
    let bound = |v: &Value, k: usize| v[k].as_f64().unwrap() as f32;
    assert!(close(bound(min, 0), 0.0) && close(bound(max, 0), 0.04), "{min} {max}");
    assert!(close(bound(min, 1), 0.0) && close(bound(max, 1), 0.01), "{min} {max}");
    assert!(close(bound(min, 2), -0.025) && close(bound(max, 2), -0.005), "{min} {max}");
    // The bounds are those of the positions written.
    let positions = model.positions();
    for k in 0..3 {
        let lo = positions.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min);
        assert_eq!(lo, bound(min, k));
    }
}

#[test]
fn a_cube_keeps_its_edges_sharp() {
    let model = gltf("cube(10);");
    let primitive = model.primitives()[0];
    let normals = model.floats(primitive["attributes"]["NORMAL"].as_u64().unwrap() as usize);
    let indices = model.indices(primitive["indices"].as_u64().unwrap() as usize);
    assert_eq!(indices.len(), 12 * 3);
    // Each of the 8 corners once for each of its 3 faces.
    assert_eq!(normals.len() / 3, 24);
    for n in normals.chunks(3) {
        assert_eq!(n.iter().filter(|c| c.abs() == 1.0).count(), 1, "{n:?} is a face's normal");
    }
}

#[test]
fn a_cylinder_is_smooth_around_and_sharp_at_its_rims() {
    let model = gltf("cylinder(r = 10, h = 20, $fn = 32);");
    let primitive = model.primitives()[0];
    let normals: Vec<Vec<f32>> =
        model.floats(primitive["attributes"]["NORMAL"].as_u64().unwrap() as usize).chunks(3).map(<[f32]>::to_vec).collect();
    let positions = model.positions();
    // A side vertex's normal points out from the axis, flat (y = 0).
    let side = normals.iter().zip(&positions).filter(|(n, _)| n[1].abs() < 1e-6).collect::<Vec<_>>();
    assert!(side.len() >= 64, "{} side vertices", side.len());
    for (n, p) in side {
        let radial = [p[0] / 0.01, -p[2] / 0.01];
        assert!((n[0] - radial[0]).abs() < 1e-3 && (-n[2] - radial[1]).abs() < 1e-3, "{n:?} at {p:?}");
    }
    // The caps' vertices point straight up or down.
    let caps = normals.iter().filter(|n| n[1].abs() == 1.0).count();
    assert!(caps >= 64, "{caps} cap vertices");
}

#[test]
fn each_color_is_a_material_and_a_primitive() {
    let glb = export(
        "color(\"red\") cube(10);
        color([0, 0, 1, 0.5]) translate([20, 0, 0]) cube(10);
        translate([40, 0, 0]) cube(10);",
        &GltfOptions::default(),
    )
    .unwrap();
    let model = Gltf::read(&glb.bytes);
    let materials = model.json["materials"].as_array().unwrap();
    assert_eq!(materials.len(), 3);
    assert_eq!(model.primitives().len(), 3);
    let factor = |m: &Value| m["pbrMetallicRoughness"]["baseColorFactor"].clone();
    let red = materials.iter().find(|m| m["name"] == "#ff0000").unwrap();
    assert_eq!(factor(red), serde_json::json!([1.0, 0.0, 0.0, 1.0]));
    assert_eq!(red.get("alphaMode"), None);
    let blue = materials.iter().find(|m| m["name"] == "#0000ff").unwrap();
    assert_eq!((factor(blue)[3].as_f64(), &blue["alphaMode"]), (Some(0.5), &Value::from("BLEND")));
    // The parts without color() take the default one, sRGB made linear.
    let default = materials.iter().find(|m| m["name"] == "#f9d72c").unwrap();
    assert!((factor(default)[1].as_f64().unwrap() - 0.679).abs() < 1e-3, "{default}");
    assert_eq!(glb.triangles, 36);
}

#[test]
fn plain_and_cut_faces_take_the_color_asked_for() {
    let options = GltfOptions { color: [0.0, 1.0, 0.0], ..GltfOptions::default() };
    let glb = export("difference() { cube(10); translate([5, 5, 5]) cube(10); }", &options).unwrap();
    let model = Gltf::read(&glb.bytes);
    // The cube's faces and the faces the difference cuts are one surface.
    let materials = model.json["materials"].as_array().unwrap();
    assert_eq!(materials.len(), 1, "{materials:?}");
    assert_eq!(materials[0]["pbrMetallicRoughness"]["baseColorFactor"], serde_json::json!([0.0, 1.0, 0.0, 1.0]));
}

#[test]
fn the_scale_is_a_setting() {
    let options = GltfOptions { scale: 1.0, ..GltfOptions::default() };
    let model = Gltf::read(&export("cube(10);", &options).unwrap().bytes);
    let top = model.positions().iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
    assert_eq!(top, 10.0);
}

#[test]
fn the_root_node_is_named() {
    let options = GltfOptions { name: "chest".into(), ..GltfOptions::default() };
    let model = Gltf::read(&export("cube(1);", &options).unwrap().bytes);
    assert_eq!(model.json["scenes"][0]["nodes"], serde_json::json!([0]));
    assert_eq!(model.json["nodes"][0]["name"], "chest");
    assert_eq!(model.json["nodes"][0]["mesh"], 0);
}

#[test]
fn flat_and_empty_models_have_no_gltf() {
    assert!(export("square(10);", &GltfOptions::default()).unwrap_err().contains("2D"));
    assert!(export("", &GltfOptions::default()).unwrap_err().contains("empty"));
}

#[test]
fn same_model_same_file() {
    let a = export("color(\"red\") sphere(5);", &GltfOptions::default()).unwrap().bytes;
    let b = export("color(\"red\") sphere(5);", &GltfOptions::default()).unwrap().bytes;
    assert_eq!(a, b);
}

fn close3(a: &Value, b: [f64; 3]) -> bool {
    (0..3).all(|k| (a[k].as_f64().unwrap() - b[k]).abs() < 1e-7)
}

#[test]
fn parts_are_named_nodes_in_their_hierarchy() {
    let glb = export(
        "cube([40, 40, 5]);
        translate([20, 20, 5]) rotate([0, 0, 90]) part(\"turret\") {
            cylinder(h = 10, r = 12, $fn = 36);
            translate([0, 10, 5]) part(\"barrel\") rotate([-90, 0, 0]) cylinder(h = 20, r = 2);
        }
        translate([100, 0, 0]) part(\"pivot\");",
        &GltfOptions { name: "tower".into(), ..GltfOptions::default() },
    )
    .unwrap();
    assert!(glb.warnings.is_empty(), "{:?}", glb.warnings);
    let model = Gltf::read(&glb.bytes);
    let nodes = model.json["nodes"].as_array().unwrap();
    let names: Vec<&str> = nodes.iter().map(|n| n["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["tower", "turret", "barrel", "pivot"]);
    assert_eq!(nodes[0]["children"], serde_json::json!([1, 3]));
    assert_eq!(nodes[1]["children"], serde_json::json!([2]));
    // The turret at (20, 20, 5) mm: Y up, the model's Y is -Z.
    assert!(close3(&nodes[1]["translation"], [0.02, 0.005, -0.02]), "{}", nodes[1]);
    // A quarter turn about the model's Z is one about glTF's Y.
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let rotation = &nodes[1]["rotation"];
    assert!((0..4).all(|k| (rotation[k].as_f64().unwrap() - [0.0, half, 0.0, half][k]).abs() < 1e-9), "{rotation}");
    assert_eq!(nodes[1].get("scale"), None);
    // The barrel, relative to the turret.
    assert!(close3(&nodes[2]["translation"], [0.0, 0.005, -0.01]), "{}", nodes[2]);
    // Each part's mesh in its own frame: the turret's cylinder is centered.
    let turret = &model.json["meshes"][nodes[1]["mesh"].as_u64().unwrap() as usize];
    let position = turret["primitives"][0]["attributes"]["POSITION"].as_u64().unwrap() as usize;
    let min = &model.json["accessors"][position]["min"];
    assert!(close3(min, [-0.012, 0.0, -0.012]), "{min}");
    // A part without geometry is a pivot, without a mesh.
    assert_eq!(nodes[3].get("mesh"), None);
    assert_eq!(model.json["meshes"].as_array().unwrap().len(), 3);
}

#[test]
fn node_names_are_unique() {
    let glb = export("part(\"lid\") cube(1); part(\"lid\") cube(2); part(\"model\") cube(3);", &GltfOptions {
        name: "model".into(),
        ..GltfOptions::default()
    });
    let model = Gltf::read(&glb.unwrap().bytes);
    let names: Vec<&str> = model.json["nodes"].as_array().unwrap().iter().map(|n| n["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["model", "lid", "lid 2", "model 2"]);
}

#[test]
fn mirrors_and_scales_are_node_scales() {
    let model = gltf("part(\"a\") cube(1); mirror([1, 0, 0]) scale([1, 2, 3]) part(\"b\") cube(1);");
    let b = &model.json["nodes"][2];
    // x mirrored, the model's y and z scales as glTF's z and y.
    assert!(close3(&b["scale"], [-1.0, 3.0, 2.0]), "{b}");
    assert_eq!(b.get("rotation"), None, "{b}");
}

#[test]
fn a_sheared_part_warns() {
    let glb = export("multmatrix([[1, 1, 0, 0], [0, 1, 0, 0], [0, 0, 1, 0]]) part(\"skew\") cube(1);", &GltfOptions::default()).unwrap();
    assert!(glb.warnings.iter().any(|w| w.contains("part(\"skew\") is sheared")), "{:?}", glb.warnings);
}

#[test]
fn a_part_in_a_boolean_is_merged() {
    let glb = export("difference() { cube(10); part(\"hole\") cylinder(r = 2, h = 30); }", &GltfOptions::default()).unwrap();
    assert!(glb.warnings[0].contains("merged"), "{:?}", glb.warnings);
    assert_eq!(Gltf::read(&glb.bytes).json["nodes"].as_array().unwrap().len(), 1);
}

fn animated(src: &str, options: &GltfOptions) -> Result<c2f_geom::Glb, String> {
    let mut program = c2f_eval::Program::load(src, Path::new("t.scad"), &[]).expect("parses");
    c2f_geom::program_glb(&mut program, options)
}

/// The keyframes of the animation's channel for `node`'s `path`: the times
/// and the values, `width` numbers each.
fn track(model: &Gltf, node: u64, path: &str) -> Option<(Vec<f32>, Vec<Vec<f32>>)> {
    let animation = &model.json["animations"][0];
    let channel = animation["channels"].as_array()?.iter().find(|c| c["target"]["node"] == node && c["target"]["path"] == path)?;
    let sampler = &animation["samplers"][channel["sampler"].as_u64().unwrap() as usize];
    assert_eq!(sampler["interpolation"], "LINEAR");
    let times = model.floats(sampler["input"].as_u64().unwrap() as usize);
    let width = if path == "rotation" { 4 } else { 3 };
    let values = model.floats(sampler["output"].as_u64().unwrap() as usize).chunks(width).map(<[f32]>::to_vec).collect();
    Some((times, values))
}

const TURRET: &str = "cube([40, 40, 5]);
    translate([20, 20, 5]) rotate([0, 0, 360 * $t]) part(\"turret\") {
        cylinder(h = 10, r = 12);
        translate([0, 8, 5 + 2 * sin(360 * $t)]) part(\"barrel\") cube([2, 20, 2]);
    }";

#[test]
fn a_turret_turns_once_and_loops() {
    let glb = animated(TURRET, &GltfOptions::default()).unwrap();
    assert!(glb.warnings.is_empty(), "{:?}", glb.warnings);
    // 2 seconds at 30 frames a second, both ends included.
    assert_eq!(glb.frames, 61);
    let model = Gltf::read(&glb.bytes);
    assert_eq!(model.json["animations"].as_array().unwrap().len(), 1);
    assert_eq!(model.json["animations"][0]["name"], "animation-loop");
    let (times, rotations) = track(&model, 1, "rotation").expect("the turret turns");
    assert_eq!((times[0], times[60]), (0.0, 2.0));
    assert!((times[15] - 0.5).abs() < 1e-6);
    // A turn about glTF's Y, a sixtieth of it a frame, by the shorter way.
    for (k, q) in rotations.iter().enumerate() {
        let half = (k as f64 / 60.0 * std::f64::consts::PI) as f32;
        let expected = [0.0, half.sin(), 0.0, half.cos()];
        assert!((0..4).all(|c| (q[c] - expected[c]).abs() < 1e-5), "frame {k}: {q:?} vs {expected:?}");
    }
    // It turns in place: no translation track.
    assert!(track(&model, 1, "translation").is_none());
    assert!(track(&model, 1, "scale").is_none());
    // The barrel goes up and down within the turret, which carries it.
    let (_, translations) = track(&model, 2, "translation").expect("the barrel moves");
    assert!(track(&model, 2, "rotation").is_none(), "it turns with its parent");
    let height = |k: usize| translations[k][1];
    assert!((height(0) - 0.005).abs() < 1e-6 && (height(15) - 0.007).abs() < 1e-6 && (height(45) - 0.003).abs() < 1e-6);
    // The last frame is the first: the loop does not jump.
    assert!((0..3).all(|c| (translations[60][c] - translations[0][c]).abs() < 1e-6));
}

#[test]
fn duration_fps_and_name_are_settings() {
    let animation = c2f_geom::GltfAnimation { name: "spin".into(), duration: 4.0, fps: 5.0, looped: false };
    let options = GltfOptions { animation: Some(animation), ..GltfOptions::default() };
    let glb = animated(TURRET, &options).unwrap();
    assert_eq!(glb.frames, 21);
    let model = Gltf::read(&glb.bytes);
    assert_eq!(model.json["animations"][0]["name"], "spin");
    let (times, _) = track(&model, 1, "rotation").unwrap();
    assert_eq!(times.last(), Some(&4.0));
    // The times' accessor has its bounds, as glTF asks.
    let input = model.json["animations"][0]["samplers"][0]["input"].as_u64().unwrap() as usize;
    assert_eq!(model.json["accessors"][input]["max"], serde_json::json!([4.0]));
}

#[test]
fn a_still_model_or_export_has_no_animation() {
    let glb = animated("part(\"lid\") cube(1);", &GltfOptions::default()).unwrap();
    assert_eq!(glb.frames, 0);
    assert!(Gltf::read(&glb.bytes).json.get("animations").is_none());
    let still = GltfOptions { animation: None, ..GltfOptions::default() };
    let glb = animated(TURRET, &still).unwrap();
    assert!(Gltf::read(&glb.bytes).json.get("animations").is_none());
    // Evaluated at $t = 0.
    assert!(Gltf::read(&glb.bytes).json["nodes"][1].get("rotation").is_none());
}

#[test]
fn what_a_glb_cannot_hold_is_a_warning() {
    let glb = animated("part(\"grow\") cube(1 + $t);", &GltfOptions::default()).unwrap();
    assert!(glb.warnings.iter().any(|w| w.contains("part(\"grow\") changes shape")), "{:?}", glb.warnings);
    assert!(glb.warnings.iter().any(|w| w.contains("no part() moves")), "{:?}", glb.warnings);
    let glb = animated("rotate(360 * $t) cube(1);", &GltfOptions::default()).unwrap();
    assert!(glb.warnings.iter().any(|w| w.contains("has no part()")), "{:?}", glb.warnings);
    let glb = animated("rotate(360 * $t) part(\"a\") cube(1); cube(1 + $t);", &GltfOptions::default()).unwrap();
    assert!(glb.warnings.iter().any(|w| w.contains("outside any part() changes")), "{:?}", glb.warnings);
    assert_eq!(glb.frames, 61);
}

#[test]
fn an_error_at_any_frame_is_the_error() {
    let error = animated("assert($t < 0.5); rotate(360 * $t) part(\"a\") cube(1);", &GltfOptions::default()).unwrap_err();
    assert!(error.starts_with("at $t = 0.5"), "{error}");
}

#[test]
fn the_chest_example_opens_and_loops() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/game/chest.c2f");
    let src = c2f_eval::read_source(&path).unwrap();
    let mut program = c2f_eval::Program::load(&src, &path, &[]).unwrap();
    let glb = c2f_geom::program_glb(&mut program, &GltfOptions { name: "chest".into(), ..GltfOptions::default() }).unwrap();
    assert!(glb.warnings.is_empty(), "{:?}", glb.warnings);
    let model = Gltf::read(&glb.bytes);
    let names: Vec<&str> = model.json["nodes"].as_array().unwrap().iter().map(|n| n["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["chest", "lid", "gem"]);
    // Closed at both ends of the loop, open halfway.
    let (_, rotations) = track(&model, 1, "rotation").expect("the lid turns");
    let angle = |q: &[f32]| 2.0 * q[0].atan2(q[3]).to_degrees();
    assert!(angle(&rotations[0]).abs() < 1e-3 && angle(&rotations[60]).abs() < 1e-3);
    assert!((angle(&rotations[30]) + 100.0).abs() < 1e-3, "{}", angle(&rotations[30]));
    assert!(track(&model, 2, "rotation").is_some(), "the gem turns");
}

#[test]
fn a_part_scaled_to_nothing_does_not_turn() {
    let glb = animated("scale([1, $t, 1]) part(\"pop\") cube(1);", &GltfOptions::default()).unwrap();
    let model = Gltf::read(&glb.bytes);
    assert!(track(&model, 1, "rotation").is_none(), "{}", model.json["animations"]);
    assert!(track(&model, 1, "scale").is_some());
    assert_eq!(model.json["nodes"][1].get("rotation"), None);
}

#[test]
fn a_part_mirrored_through_zero_warns() {
    let glb = animated("scale([1, 1, cos(360 * $t)]) part(\"flip\") cube(1);", &GltfOptions::default()).unwrap();
    assert!(glb.warnings.iter().any(|w| w.contains("part(\"flip\") is mirrored")), "{:?}", glb.warnings);
}

#[test]
fn a_part_with_no_finite_place_is_an_error() {
    let error = animated("scale(1 / $t) part(\"far\") cube(1);", &GltfOptions::default()).unwrap_err();
    assert!(error.contains("part(\"far\") has no place"), "{error}");
}

#[test]
fn a_still_export_is_at_the_programs_time() {
    let mut program = c2f_eval::Program::load(TURRET, Path::new("t.scad"), &[]).unwrap();
    program.t = 0.25;
    let still = GltfOptions { animation: None, ..GltfOptions::default() };
    let model = Gltf::read(&c2f_geom::program_glb(&mut program, &still).unwrap().bytes);
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let rotation = &model.json["nodes"][1]["rotation"];
    assert!((rotation[1].as_f64().unwrap() - half).abs() < 1e-9, "{rotation}");
    assert_eq!(program.t, 0.25);
}

#[test]
fn frames_are_capped() {
    let animation = c2f_geom::GltfAnimation { fps: 1e9, ..Default::default() };
    let options = GltfOptions { animation: Some(animation), ..GltfOptions::default() };
    // A model without parts: the cap is said, and no frame is evaluated.
    let glb = animated("x = $t; cube(1);", &options).unwrap();
    assert!(glb.warnings.iter().any(|w| w.contains("cut to 10000 frames")), "{:?}", glb.warnings);
}

fn example(name: &str) -> (std::path::PathBuf, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/game").join(name);
    let src = c2f_eval::read_source(&path).unwrap();
    (path, src)
}

#[test]
fn the_gears_and_the_walker_export_animated() {
    for (name, parts) in [("gears.c2f", 3), ("walker.c2f", 11)] {
        let (path, src) = example(name);
        let mut program = c2f_eval::Program::load(&src, &path, &[]).unwrap();
        let glb = c2f_geom::program_glb(&mut program, &GltfOptions::default()).unwrap();
        assert!(glb.warnings.is_empty(), "{name}: {:?}", glb.warnings);
        assert_eq!(glb.frames, 61, "{name}");
        let model = Gltf::read(&glb.bytes);
        assert_eq!(model.json["nodes"].as_array().unwrap().len(), parts + 1, "{name}");
    }
}

#[test]
fn the_gears_teeth_never_overlap() {
    let (path, src) = example("gears.c2f");
    // The gears alone, placed as the example places them, two at a time.
    let gears = &src[..src.find("// The plate and the axles").unwrap()];
    for (a, b) in [(0, 1), (1, 2), (0, 2)] {
        for t in [0.0, 0.013, 0.05, 0.21, 0.5, 0.77, 0.993] {
            let check = format!(
                "{gears}
                module placed(i) {{ c = [c1, c2, c3][i]; translate([c[0], c[1], 0]) rotate([0, 0, [a1, a2, a3][i]]) gear(teeth[i]); }}
                intersection() {{ placed({a}); placed({b}); }}"
            );
            let mut program = c2f_eval::Program::load(&check, &path, &[]).unwrap();
            program.t = t;
            let rendered = c2f_geom::render(&c2f_eval::evaluate(&program).nodes);
            assert_eq!(rendered.stats.volume, 0.0, "gears {a} and {b} at $t = {t}");
        }
    }
}
