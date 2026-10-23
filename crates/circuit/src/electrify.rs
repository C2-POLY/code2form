//! A finished design made ready for its electronics, without rewriting it:
//! its circuit file is created beside it, the design names it
//! (`// @circuit`), and what the electronics need from the design goes in
//! two modules of its own, `electronics_cuts()` (subtracted: cutouts,
//! screw holes, wire channels) and `electronics_mounts()` (added:
//! standoffs). The design's geometry moves, unchanged, into a module
//! `design()`, which the file draws once, with the cuts and mounts:
//!
//! ```scad
//! difference() {
//!     union() {
//!         design();
//!         electronics_mounts();
//!     }
//!     electronics_cuts();
//! }
//! ```
//!
//! Only the statements that draw something move: assignments (the
//! customizer's parameters among them), modules, functions, `use` and
//! `include` stay where they are. File-level variables are seen from
//! inside a module, so the design draws the same.

use c2f_lang::ast::StmtKind;

/// The design's new text and its circuit's, or why the design cannot be
/// made ready.
#[derive(Clone, Debug, PartialEq)]
pub struct Electrified {
    pub design: String,
    pub circuit: String,
}

/// Prepares `src`, the design, for the circuit file `circuit_file`
/// (relative to the design), whose enclosure is the design itself,
/// `design_file`.
pub fn electrify(src: &str, design_file: &str, circuit_file: &str) -> Result<Electrified, String> {
    let (annotations, _) = c2f_lang::annotations::read(src);
    if let Some(link) = &annotations.circuit {
        return Err(format!("the design already names its circuit, {}", link.path));
    }
    let file = c2f_lang::parser::parse(src).map_err(|err| format!("the design does not read: {}", err.message))?;
    let names_taken = file.stmts.iter().any(|s| match &s.kind {
        StmtKind::Module(m) => ["design", "electronics_cuts", "electronics_mounts"].contains(&m.name.name.as_str()),
        _ => false,
    });
    if names_taken {
        return Err("the design already has a module named design, electronics_cuts or electronics_mounts".into());
    }
    // The statements that draw, by their byte ranges.
    let drawn: Vec<std::ops::Range<usize>> = file
        .stmts
        .iter()
        .filter(|s| matches!(s.kind, StmtKind::Instance(_)))
        .map(|s| s.span.start as usize..s.span.end as usize)
        .collect();
    if drawn.is_empty() {
        return Err("the design draws nothing at its top level".into());
    }

    let mut kept = String::with_capacity(src.len());
    let mut moved = Vec::new();
    let mut at = 0;
    for range in &drawn {
        // The whole lines the statement takes, when it has them to itself.
        let line_start = src[..range.start].rfind('\n').map_or(0, |i| i + 1);
        let alone_before = src[line_start..range.start].trim().is_empty();
        let line_end = src[range.end..].find('\n').map_or(src.len(), |i| range.end + i + 1);
        let alone_after = src[range.end..line_end].trim().is_empty();
        let (start, end) = if alone_before && alone_after { (line_start, line_end) } else { (range.start, range.end) };
        let start = start.max(at);
        kept.push_str(&src[at..start]);
        // Its lines after the first lose the first line's indentation.
        let column = if alone_before { range.start - line_start } else { 0 };
        moved.push(dedent(src[range.start..range.end].trim_end(), column));
        at = end;
    }
    kept.push_str(&src[at..]);

    let mut design = with_circuit_line(kept.trim_end(), circuit_file);
    design.push_str("\n\n// The design, drawn once with what its electronics need: see\n");
    design.push_str(&format!("// {circuit_file} for the parts and their wires.\n"));
    design.push_str("difference() {\n    union() {\n        design();\n        electronics_mounts();\n    }\n    electronics_cuts();\n}\n\n");
    design.push_str("module design() {\n");
    for statement in &moved {
        for line in statement.lines() {
            if line.is_empty() {
                design.push('\n');
            } else {
                design.push_str("    ");
                design.push_str(line);
                design.push('\n');
            }
        }
    }
    design.push_str("}\n\n");
    design.push_str(
        "// What the circuit's parts need from the design, subtracted: the openings\n\
         // of their ports, their screw holes, the wires' channels. For example:\n\
         //     cutout_for(\"xiao\", \"usb\", clearance = 0.5);\n\
         //     holes_for(\"driver\", screw = \"M2\");\n\
         module electronics_cuts() {\n}\n\n\
         // What they need added: the standoffs under their mount holes. For example:\n\
         //     standoffs_for(\"driver\", height = 5, screw = \"M2\");\n\
         module electronics_mounts() {\n}\n",
    );

    let circuit = format!(
        "# The electronics of {design_file}: its parts, where they sit in the\n\
         # design's coordinates, and the wires between their pins. The design\n\
         # is drawn see-through around them.\n\
         \n\
         version = 1\n\
         \n\
         enclosure = \"{design_file}\"\n\
         \n\
         # Each part as\n\
         #\n\
         # [parts.name]\n\
         # part = \"electronics/xiao_esp32s3.c2f\"\n\
         # at = [0, 0, 0]\n\
         # rotate = [0, 0, 0]\n\
         #\n\
         # and each wire as\n\
         #\n\
         # [[wires]]\n\
         # from = \"xiao.5V\"\n\
         # to = \"driver.VCC\"\n\
         # color = \"red\"\n"
    );
    Ok(Electrified { design, circuit })
}

/// The design with `// @circuit` after its opening comment, so that the
/// comment that says what the design is stays first.
fn with_circuit_line(src: &str, circuit_file: &str) -> String {
    let line = format!("// @circuit {circuit_file}\n");
    let mut offset = 0;
    for raw in src.split_inclusive('\n') {
        if !raw.trim_start().starts_with("//") {
            break;
        }
        offset += raw.len();
    }
    if offset == 0 {
        return format!("{line}\n{src}");
    }
    format!("{}{line}{}", &src[..offset], &src[offset..])
}

/// `text` with up to `column` spaces taken off the start of its lines
/// after the first: a statement moved into a module keeps its own shape.
fn dedent(text: &str, column: usize) -> String {
    let mut lines = text.lines();
    let mut out = lines.next().unwrap_or_default().to_string();
    for line in lines {
        let indent = (line.len() - line.trim_start().len()).min(column);
        out.push('\n');
        out.push_str(&line[indent..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DESIGN: &str = "// A box with a lid.\n// Two parts.\n\n/* [Size] */\nwidth = 40; // [20:80]\n\ncube([width, 30, 20]);\nmodule lid() cube([width, 30, 2]);\ntranslate([0, 0, 25])\n    lid();\n";

    #[test]
    fn the_drawing_moves_into_design_and_the_rest_stays() {
        let out = electrify(DESIGN, "box.c2f", "box.c2fc").unwrap();
        let design = &out.design;
        assert!(design.starts_with("// A box with a lid.\n// Two parts.\n// @circuit box.c2fc\n\n/* [Size] */\nwidth = 40; // [20:80]\n"), "{design}");
        assert!(design.contains("module lid() cube([width, 30, 2]);"), "{design}");
        assert!(design.contains("module design() {\n    cube([width, 30, 20]);\n    translate([0, 0, 25])\n        lid();\n}"), "{design}");
        assert!(design.contains("module electronics_cuts() {\n}") && design.contains("module electronics_mounts() {\n}"));
        assert!(out.circuit.contains("enclosure = \"box.c2f\""));
    }

    #[test]
    fn it_draws_the_same_and_reads_its_circuit() {
        let out = electrify(DESIGN, "box.c2f", "box.c2fc").unwrap();
        let (annotations, errors) = c2f_lang::annotations::read(&out.design);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(annotations.circuit.map(|c| c.path), Some("box.c2fc".to_string()));
        let dir = std::env::temp_dir().join(format!("c2f-electrify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("box.c2fc"), &out.circuit).unwrap();
        std::fs::write(dir.join("box.c2f"), &out.design).unwrap();
        crate::parse(&out.circuit, &dir.join("box.c2fc"), &[]).unwrap();
        let volume = |src: &str| {
            let path = dir.join("box.c2f");
            let program = c2f_eval::Program::load(src, &path, &[]).unwrap();
            let output = c2f_eval::evaluate(&program);
            assert!(output.error.is_none(), "{:?}", output.error);
            c2f_geom::render_in(&output.nodes, &dir).stats.volume
        };
        assert!((volume(DESIGN) - volume(&out.design)).abs() < 1e-6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_design_with_its_circuit_already_is_left_alone() {
        assert!(electrify("// @circuit a.c2fc\ncube(1);", "a.c2f", "a.c2fc").is_err());
        assert!(electrify("x = 1;", "a.c2f", "a.c2fc").unwrap_err().contains("draws nothing"));
    }
}
