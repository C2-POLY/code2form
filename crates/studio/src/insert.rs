//! Meshes and designs brought into a design: opened on their own, or
//! inserted in the one being edited, as the lines the user would type.

use std::path::{Component, Path};

/// The mesh files `import()` reads, which open as a design that imports
/// them: STEP when FreeCAD is installed.
pub const MESH_EXTENSIONS: &[&str] = &["stl", "3mf", "obj", "off", "amf", "step", "stp"];

/// The design files `use` and `include` read.
pub const DESIGN_EXTENSIONS: &[&str] = &["c2f", "scad"];

pub fn is_mesh(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| MESH_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// The design a mesh opens as, ready to change: its size, turn and place
/// in the customizer, each of `objects` (the names its file gives them) a
/// `part()`, and where to add or cut material. A mesh holds triangles,
/// not the shapes it was drawn with, so it is changed around, not inside.
pub fn mesh_design(path: &Path, objects: &[String]) -> String {
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut objects: Vec<&String> = objects.iter().filter(|o| !o.trim().is_empty()).collect();
    objects.dedup();
    let model = if objects.len() > 1 {
        let parts: Vec<String> = objects
            .iter()
            .map(|o| format!("    part(\"{0}\") import(file, object = \"{0}\");", escaped(o)))
            .collect();
        format!("// The mesh, each of its objects a part.\nmodule model() {{\n{}\n}}\n", parts.join("\n"))
    } else {
        "// The mesh.\nmodule model() import(file);\n".to_string()
    };
    format!(
        "// {name}, opened as a design. A mesh holds triangles, not the shapes it\n\
         // was drawn with: size, turn and place it with the parameters, and add or\n\
         // cut material below.\n\
         \n\
         /* [Placement] */\n\
         // Size, in percent.\n\
         size = 100; // [10:400]\n\
         // Turn about Z, in degrees.\n\
         turn = 0; // [-180:180]\n\
         // Move along X, in mm.\n\
         move_x = 0; // [-200:200]\n\
         // Move along Y, in mm.\n\
         move_y = 0; // [-200:200]\n\
         // Move along Z, in mm.\n\
         move_z = 0; // [-200:200]\n\
         \n\
         /* [Hidden] */\n\
         file = \"{}\";\n\
         \n\
         translate([move_x, move_y, move_z]) rotate([0, 0, turn]) scale(size / 100) {{\n\
         \x20   model();\n\
         \x20   // Add material here, as:\n\
         \x20   // translate([20, 0, 0]) cube(10);\n\
         }}\n\
         \n\
         // To cut material away, take the model's place in a difference():\n\
         // difference() {{\n\
         //     model();\n\
         //     translate([0, 0, 10]) cylinder(r = 3, h = 50);\n\
         // }}\n\
         \n\
         {model}",
        escaped(&slashes(path))
    )
}

/// `file` as the design at `design` names it: relative to its folder when
/// it has one and they are on one drive, with `/` between folders.
pub fn reference(file: &Path, design: Option<&Path>) -> String {
    let relative = design.and_then(Path::parent).and_then(|dir| relative(file, dir));
    match relative {
        Some(relative) => relative,
        None => slashes(file),
    }
}

/// The text with `import("reference");` on a line of its own, at the
/// start of the line that holds char `cursor`; and the char after it. As
/// a `background`, `%import(...)`: a reference to measure and build
/// around, drawn see-through and left out of the model, as a scan is.
pub fn insert_mesh(text: &str, cursor: usize, reference: &str, background: bool) -> (String, usize) {
    let at = byte_of(text, cursor);
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = format!("{}import(\"{}\");\n", if background { "%" } else { "" }, escaped(reference));
    let mut out = String::with_capacity(text.len() + line.len());
    out.push_str(&text[..line_start]);
    out.push_str(&line);
    out.push_str(&text[line_start..]);
    let cursor = text[..line_start].chars().count() + line.chars().count();
    (out, cursor)
}

/// The text with `use <reference>` (`include` when `whole`) at its top:
/// after the `use` and `include` lines it opens with, or after its opening
/// comment. `None` when the line is there already.
pub fn insert_design(text: &str, reference: &str, whole: bool) -> Option<(String, usize)> {
    let keyword = if whole { "include" } else { "use" };
    let line = format!("{keyword} <{reference}>");
    if text.lines().any(|l| l.trim() == line) {
        return None;
    }
    let mut offset = 0;
    let mut after = 0;
    let mut in_comment = true;
    for raw in text.split_inclusive('\n') {
        let trimmed = raw.trim();
        let library = trimmed.starts_with("use <") || trimmed.starts_with("include <");
        if library {
            after = offset + raw.len();
            in_comment = false;
        } else if in_comment && trimmed.starts_with("//") {
            after = offset + raw.len();
        } else if in_comment && trimmed.is_empty() && after > 0 {
            // The blank line that ends the opening comment stays with it.
            after = offset + raw.len();
            in_comment = false;
        } else if !trimmed.is_empty() || !in_comment {
            break;
        }
        offset += raw.len();
    }
    let mut out = String::with_capacity(text.len() + line.len() + 2);
    out.push_str(&text[..after]);
    if after > 0 && !text[..after].ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&line);
    out.push('\n');
    let cursor = out.chars().count();
    // A blank line between the libraries and the design.
    let next = &text[after..];
    if !next.is_empty() && !next.starts_with('\n') && !next.trim_start().starts_with("use <") && !next.trim_start().starts_with("include <") {
        out.push('\n');
    }
    out.push_str(next);
    Some((out, cursor))
}

/// `file` relative to the folder `dir`, through `..` where it must; `None`
/// when they share no root (two drives).
fn relative(file: &Path, dir: &Path) -> Option<String> {
    let file: Vec<Component> = file.components().collect();
    let dir: Vec<Component> = dir.components().collect();
    if file.first() != dir.first() {
        return None;
    }
    let common = file.iter().zip(&dir).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".into(); dir.len() - common];
    parts.extend(file[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    Some(parts.join("/"))
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// A string as the language reads it between quotes.
fn escaped(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

fn byte_of(text: &str, chars: usize) -> usize {
    text.char_indices().nth(chars).map_or(text.len(), |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mesh_opens_as_a_design_to_change() {
        let one = mesh_design(Path::new("/parts/motor \"17\".stl"), &[]);
        assert!(one.starts_with("// motor \"17\".stl, opened as a design."), "{one}");
        assert!(one.contains("file = \"/parts/motor \\\"17\\\".stl\";"), "{one}");
        assert!(one.contains("module model() import(file);") && !one.contains("part("), "{one}");
        let names = ["Body".to_string(), "Lid".to_string()];
        let two = mesh_design(Path::new("/parts/box.3mf"), &names);
        assert!(two.contains("    part(\"Body\") import(file, object = \"Body\");\n    part(\"Lid\") import(file, object = \"Lid\");"), "{two}");
        assert!(is_mesh(Path::new("a.STL")) && is_mesh(Path::new("a.step")) && !is_mesh(Path::new("a.c2f")));
    }

    #[test]
    fn the_design_renders_its_parts_and_parameters() {
        let dir = std::env::temp_dir().join(format!("c2f-mesh-design-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("box.obj");
        let tetra = |o: &str, x: f64| {
            format!("o {o}\nv {x} 0 0\nv {} 0 0\nv {x} 1 0\nv {x} 0 1\nf -4 -2 -3\nf -4 -3 -1\nf -4 -1 -2\nf -3 -2 -1\n", x + 1.0)
        };
        std::fs::write(&file, format!("{}{}", tetra("Body", 0.0), tetra("Lid", 5.0))).unwrap();
        let names = c2f_geom::mesh_objects(&file);
        let src = mesh_design(&file, &names);
        let program = c2f_eval::Program::load(&src, &dir.join("box.c2f"), &[]).unwrap();
        assert_eq!(program.parameters(&src).iter().filter(|p| p.group == "Placement").count(), 5);
        let output = c2f_eval::evaluate(&program);
        assert!(output.error.is_none() && output.warnings.is_empty(), "{:?} {:?}", output.error, output.warnings);
        let rendered = c2f_geom::render_in(&output.nodes, &dir);
        assert!((rendered.stats.volume - 2.0 / 6.0).abs() < 1e-9, "{:?} {:?}", rendered.stats, rendered.warnings);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn files_are_named_from_the_designs_folder() {
        let design = Path::new("/home/me/robot/arm.c2f");
        assert_eq!(reference(Path::new("/home/me/robot/parts/motor.stl"), Some(design)), "parts/motor.stl");
        assert_eq!(reference(Path::new("/home/me/lib/bolt.c2f"), Some(design)), "../lib/bolt.c2f");
        assert_eq!(reference(Path::new("/home/me/lib/bolt.c2f"), None), "/home/me/lib/bolt.c2f");
    }

    #[test]
    fn a_mesh_goes_on_the_cursors_line() {
        let text = "cube(1);\ntranslate([5, 0, 0]) sphere(2);\n";
        // The cursor in the middle of the second line.
        let (out, cursor) = insert_mesh(text, 15, "motor.stl", false);
        assert_eq!(out, "cube(1);\nimport(\"motor.stl\");\ntranslate([5, 0, 0]) sphere(2);\n");
        assert_eq!(&out[..cursor], "cube(1);\nimport(\"motor.stl\");\n");
        let (out, _) = insert_mesh(text, 0, "scans/watch.stl", true);
        assert_eq!(out, "%import(\"scans/watch.stl\");\ncube(1);\ntranslate([5, 0, 0]) sphere(2);\n");
    }

    #[test]
    fn a_design_goes_after_the_libraries_or_the_opening_comment() {
        let (out, _) = insert_design("// A box.\n// With a lid.\n\ncube(1);\n", "lid.c2f", false).unwrap();
        assert_eq!(out, "// A box.\n// With a lid.\n\nuse <lid.c2f>\n\ncube(1);\n");
        let (out, _) = insert_design("use <a.c2f>\ninclude <b.scad>\ncube(1);\n", "c.c2f", true).unwrap();
        assert_eq!(out, "use <a.c2f>\ninclude <b.scad>\ninclude <c.c2f>\n\ncube(1);\n");
        let (out, cursor) = insert_design("cube(1);", "c.c2f", false).unwrap();
        assert_eq!((out.as_str(), cursor), ("use <c.c2f>\n\ncube(1);", 12));
        assert!(insert_design("use <c.c2f>\ncube(1);", "c.c2f", false).is_none());
    }
}
