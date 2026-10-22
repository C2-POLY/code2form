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
