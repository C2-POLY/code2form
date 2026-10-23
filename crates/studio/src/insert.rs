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

/// What a design that shows the mesh at `path` holds.
pub fn mesh_design(path: &Path) -> String {
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    format!("// {name}\nimport(\"{}\");\n", escaped(&slashes(path)))
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
/// start of the line that holds char `cursor`; and the char after it.
pub fn insert_mesh(text: &str, cursor: usize, reference: &str) -> (String, usize) {
    let at = byte_of(text, cursor);
    let line_start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = format!("import(\"{}\");\n", escaped(reference));
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
    fn a_mesh_opens_as_a_design_that_imports_it() {
        assert_eq!(mesh_design(Path::new("/parts/motor \"17\".stl")), "// motor \"17\".stl\nimport(\"/parts/motor \\\"17\\\".stl\");\n");
        assert!(is_mesh(Path::new("a.STL")) && is_mesh(Path::new("a.step")) && !is_mesh(Path::new("a.c2f")));
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
        let (out, cursor) = insert_mesh(text, 15, "motor.stl");
        assert_eq!(out, "cube(1);\nimport(\"motor.stl\");\ntranslate([5, 0, 0]) sphere(2);\n");
        assert_eq!(&out[..cursor], "cube(1);\nimport(\"motor.stl\");\n");
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
