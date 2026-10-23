//! Wavefront OBJ export: the mesh format nearly every 3D program reads
//! (Blender, Maya, game engines, slicers). The `.obj` holds the geometry
//! in millimeters, Z up as designed, an object (`o`) for the root and for
//! each `part()`, placed where the model puts it; the `.mtl` beside it
//! holds a material for each color, which the faces name with `usemtl`.

use std::fmt::Write;
use std::path::Path;

use c2f_eval::node::Node;

use crate::build::{CUT_COLOR, DEFAULT_COLOR, Model, transform};
use crate::export::colored_mesh;
use crate::pieces::pieces;

/// How a model is written to OBJ.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjOptions {
    /// The root object's name, and the `.mtl` file's stem: `name.mtl`.
    pub name: String,
    /// The color of what has no `color()`, in sRGB.
    pub color: [f32; 3],
}

impl Default for ObjOptions {
    fn default() -> Self {
        let [r, g, b, _] = DEFAULT_COLOR;
        ObjOptions { name: "model".into(), color: [r, g, b] }
    }
}

/// An `.obj` file and the `.mtl` it names.
#[derive(Clone, Debug, Default)]
pub struct Obj {
    pub obj: String,
    pub mtl: String,
    /// The `.mtl` file's name, which the `.obj` gives: the name, its
    /// spaces made `_`, which `mtllib` would read as two files.
    pub mtl_file: String,
    pub warnings: Vec<String>,
    pub triangles: usize,
}

/// Renders `nodes`, data files read relative to `base_dir`, and writes
/// them as OBJ. An empty or 2D model has none.
pub fn obj(nodes: &[Node], base_dir: &Path, options: &ObjOptions) -> Result<Obj, String> {
    let split = pieces(nodes, base_dir, &options.name);
    let mut warnings = split.warnings;
    let mut out = Obj { mtl_file: format!("{}.mtl", options.name.replace(char::is_whitespace, "_")), ..Obj::default() };
    let _ = writeln!(out.obj, "# Code2Form\nmtllib {}", out.mtl_file);
    out.mtl.push_str("# Code2Form\n");
    let mut materials: Vec<[f32; 4]> = Vec::new();
    let mut written = 0;
    let mut flat = false;
    for piece in &split.pieces {
        let solid = match transform(piece.model.clone(), &piece.world) {
            Model::Solid(solid) => solid,
            Model::Flat(_) => {
                flat = true;
                continue;
            }
            Model::Empty => continue,
        };
        let mesh = colored_mesh(&solid);
        if mesh.triangles.is_empty() {
            continue;
        }
        let _ = writeln!(out.obj, "o {}", piece.name);
        for [x, y, z] in &mesh.vertices {
            let _ = writeln!(out.obj, "v {x:?} {y:?} {z:?}");
        }
        // Grouped by color, in order of first use.
        let [r, g, b] = options.color;
        let colors: Vec<[f32; 4]> = mesh
            .colors
            .iter()
            .map(|&c| if c == DEFAULT_COLOR || c == CUT_COLOR { [r, g, b, 1.0] } else { c })
            .collect();
        let mut used: Vec<[f32; 4]> = Vec::new();
        for color in &colors {
            if !used.contains(color) {
                used.push(*color);
            }
        }
        for color in used {
            let index = match materials.iter().position(|&m| m == color) {
                Some(index) => index,
                None => {
                    materials.push(color);
                    let [r, g, b, a] = color;
                    let _ = writeln!(out.mtl, "\nnewmtl {}\nKd {r:?} {g:?} {b:?}\nd {a:?}\nillum 1", material(materials.len() - 1));
                    materials.len() - 1
                }
            };
            let _ = writeln!(out.obj, "usemtl {}", material(index));
            for (triangle, _) in mesh.triangles.iter().zip(&colors).filter(|(_, c)| **c == color) {
                let [a, b, c] = triangle.map(|v| v + written + 1);
                let _ = writeln!(out.obj, "f {a} {b} {c}");
            }
        }
        written += mesh.vertices.len();
        out.triangles += mesh.triangles.len();
    }
    if out.triangles == 0 {
        return Err(if flat { "the model is 2D; OBJ holds solids" } else { "the model is empty; no OBJ written" }.into());
    }
    if flat {
        warnings.push("The 2D shapes of the model are left out: OBJ holds solids".into());
    }
    out.warnings = warnings;
    Ok(out)
}

fn material(index: usize) -> String {
    format!("color{}", index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(src: &str) -> Obj {
        let program = c2f_eval::Program::load(src, Path::new("t.scad"), &[]).expect("parses");
        obj(&c2f_eval::evaluate(&program).nodes, Path::new(""), &ObjOptions::default()).unwrap()
    }

    #[test]
    fn an_object_for_each_part_and_a_material_for_each_color() {
        let out = write(
            "color(\"red\") cube(10);
            translate([20, 0, 0]) part(\"lid\") { cube(5); color(\"blue\") translate([0, 0, 6]) cube(1); }",
        );
        assert!(out.obj.starts_with("# Code2Form\nmtllib model.mtl\no model\n"), "{}", out.obj);
        assert!(out.obj.contains("\no lid\n"));
        // Red, the model's color for the lid's cube, then blue.
        assert_eq!(out.mtl.matches("newmtl").count(), 3, "{}", out.mtl);
        assert!(out.mtl.contains("newmtl color1\nKd 1.0 0.0 0.0\nd 1.0\n"), "{}", out.mtl);
        assert_eq!(out.triangles, 36);
        // The lid sits where the model puts it.
        let lid = out.obj.split("o lid\n").nth(1).unwrap();
        assert!(lid.lines().filter(|l| l.starts_with("v ")).all(|l| l[2..].split(' ').next().unwrap().parse::<f64>().unwrap() >= 20.0));
    }

    #[test]
    fn it_reads_back_as_the_same_solid() {
        let out = write("difference() { cube(10); translate([5, 5, -1]) cylinder(r = 2, h = 12, $fn = 24); }");
        let dir = std::env::temp_dir().join(format!("c2f-obj-out-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.obj"), &out.obj).unwrap();
        let program = c2f_eval::Program::load("import(\"model.obj\");", Path::new("t.scad"), &[]).unwrap();
        let back = crate::render_in(&c2f_eval::evaluate(&program).nodes, &dir);
        let original = crate::render(&c2f_eval::evaluate(
            &c2f_eval::Program::load(
                "difference() { cube(10); translate([5, 5, -1]) cylinder(r = 2, h = 12, $fn = 24); }",
                Path::new("t.scad"),
                &[],
            )
            .unwrap(),
        )
        .nodes);
        assert!((back.stats.volume - original.stats.volume).abs() < 1e-6, "{:?} {:?}", back.stats, back.warnings);
        assert_eq!(back.bounds(), original.bounds());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_flat_model_has_no_obj() {
        let program = c2f_eval::Program::load("square(3);", Path::new("t.scad"), &[]).unwrap();
        let err = obj(&c2f_eval::evaluate(&program).nodes, Path::new(""), &ObjOptions::default()).unwrap_err();
        assert!(err.contains("2D"), "{err}");
    }
}
