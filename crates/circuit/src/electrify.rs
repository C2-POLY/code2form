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
//!
//! A design made of pieces (`part("name")`) keeps them apart: a piece
//! inside a `difference()` would be merged into the geometry around it.
//! So the drawing stays where it is, and only the piece that holds the
//! electronics takes the cuts and mounts, wherever the design draws it:
//!
//! ```scad
//! part("Base") difference() { union() { base(); electronics_mounts(); } electronics_cuts(); }
//! ```
//!
//! They are written in the design's coordinates, which are the piece's
//! where the design draws it assembled, untransformed.

use c2f_lang::ast::{ExprKind, InstanceKind, Stmt, StmtKind};

/// The design's new text and its circuit's, or why the design cannot be
/// made ready.
#[derive(Clone, Debug, PartialEq)]
pub struct Electrified {
    pub design: String,
    pub circuit: String,
}

/// Prepares `src`, the design, for the circuit file `circuit_file`
/// (relative to the design), whose enclosure is the design itself,
/// `design_file`. A design made of pieces names the one that holds the
/// electronics, `piece`, unless it has only one.
pub fn electrify(src: &str, design_file: &str, circuit_file: &str, piece: Option<&str>) -> Result<Electrified, String> {
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
    let pieces = pieces(&file.stmts);
    if !pieces.is_empty() {
        let design = in_piece(src, &pieces, piece, circuit_file)?;
        return Ok(Electrified { design, circuit: circuit(design_file) });
    }
    if let Some(piece) = piece {
        return Err(format!("the design has no pieces: it draws no part(\"{piece}\")"));
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
    design.push_str(MODULES);
    Ok(Electrified { design, circuit: circuit(design_file) })
}

/// The two modules the electronics fill, with what they take.
const MODULES: &str = "// What the circuit's parts need from the design, subtracted: the openings\n\
    // of their ports, their screw holes, the wires' channels. For example:\n\
    //     cutout_for(\"xiao\", \"usb\", clearance = 0.5);\n\
    //     holes_for(\"driver\", screw = \"M2\");\n\
    module electronics_cuts() {\n}\n\n\
    // What they need added: the standoffs under their mount holes. For example:\n\
    //     standoffs_for(\"driver\", height = 5, screw = \"M2\");\n\
    module electronics_mounts() {\n}\n";

/// The circuit file's first text: its enclosure, and how to add parts and
/// wires.
fn circuit(design_file: &str) -> String {
    format!(
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
    )
}

/// A `part()` the design draws: its name when written as text, and the
/// statements it holds.
struct Piece<'a> {
    name: Option<&'a str>,
    children: &'a [Stmt],
}

/// The `part()`s drawn by the design's top-level statements, outer ones
/// first: those inside modules are drawn only where a module is called,
/// which electrify leaves alone.
fn pieces(stmts: &[Stmt]) -> Vec<Piece<'_>> {
    let mut found = Vec::new();
    let mut pending: Vec<&Stmt> = stmts.iter().rev().collect();
    while let Some(stmt) = pending.pop() {
        match &stmt.kind {
            StmtKind::Block(inner) => pending.extend(inner.iter().rev()),
            StmtKind::Instance(instance) => match &instance.kind {
                InstanceKind::Call { name, args, children } if name.name == "part" => {
                    let name = args.first().filter(|a| a.name.is_none()).and_then(|a| match &a.value.kind {
                        ExprKind::Str(text) => Some(text.as_str()),
                        _ => None,
                    });
                    found.push(Piece { name, children });
                }
                InstanceKind::Call { children, .. } => pending.extend(children.iter().rev()),
                InstanceKind::If { then, otherwise, .. } => {
                    pending.extend(otherwise.iter().flatten().rev());
                    pending.extend(then.iter().rev());
                }
            },
            _ => {}
        }
    }
    found
}

/// The design with the cuts and mounts inside each `part()` of the piece
/// that holds the electronics.
fn in_piece(src: &str, pieces: &[Piece], piece: Option<&str>, circuit_file: &str) -> Result<String, String> {
    let mut names: Vec<&str> = pieces.iter().filter_map(|p| p.name).collect();
    names.sort_unstable();
    names.dedup();
    let listed = names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(", ");
    let name = match piece {
        Some(name) if names.contains(&name) => name,
        Some(name) => return Err(format!("the design draws no part(\"{name}\"): its pieces are {listed}")),
        None if names.len() == 1 => names[0],
        None if names.is_empty() => return Err("the design's pieces are not named by plain text: name it in part(\"…\")".into()),
        None => return Err(format!("say which piece holds the electronics, with `piece`: {listed}")),
    };
    // Each place it is drawn, by the byte range of what the part holds.
    let mut ranges: Vec<std::ops::Range<usize>> = pieces
        .iter()
        .filter(|p| p.name == Some(name))
        .filter_map(|p| Some(p.children.first()?.span.start as usize..p.children.last()?.span.end as usize))
        .collect();
    if ranges.is_empty() {
        return Err(format!("part(\"{name}\") holds nothing"));
    }
    ranges.sort_by_key(|r| r.start);
    let mut out = String::with_capacity(src.len() + 200);
    let mut at = 0;
    for range in ranges {
        out.push_str(&src[at..range.start]);
        out.push_str("difference() { union() { ");
        out.push_str(&src[range.clone()]);
        out.push_str(" electronics_mounts(); } electronics_cuts(); }");
        at = range.end;
    }
    out.push_str(&src[at..]);
    let mut design = with_circuit_line(out.trim_end(), circuit_file);
    design.push_str(&format!(
        "\n\n// The electronics of part(\"{name}\"), in the design's coordinates (the\n\
         // piece's, assembled): see {circuit_file} for the parts and their wires.\n\n"
    ));
    design.push_str(MODULES);
    Ok(design)
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
        let out = electrify(DESIGN, "box.c2f", "box.c2fc", None).unwrap();
        let design = &out.design;
        assert!(design.starts_with("// A box with a lid.\n// Two parts.\n// @circuit box.c2fc\n\n/* [Size] */\nwidth = 40; // [20:80]\n"), "{design}");
        assert!(design.contains("module lid() cube([width, 30, 2]);"), "{design}");
        assert!(design.contains("module design() {\n    cube([width, 30, 20]);\n    translate([0, 0, 25])\n        lid();\n}"), "{design}");
        assert!(design.contains("module electronics_cuts() {\n}") && design.contains("module electronics_mounts() {\n}"));
        assert!(out.circuit.contains("enclosure = \"box.c2f\""));
    }

    #[test]
    fn it_draws_the_same_and_reads_its_circuit() {
        let out = electrify(DESIGN, "box.c2f", "box.c2fc", None).unwrap();
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
        assert!(electrify("// @circuit a.c2fc\ncube(1);", "a.c2f", "a.c2fc", None).is_err());
        assert!(electrify("x = 1;", "a.c2f", "a.c2fc", None).unwrap_err().contains("draws nothing"));
    }

    const PIECES: &str = "// A lamp.\nh = 30;\nif (true) {\n    part(\"Base\") color(\"gray\") cylinder(d = 40, h = h);\n    translate([0, 0, h]) part(\"Dome\") sphere(d = 20);\n} else\n    translate([50, 0, 0]) part(\"Base\") cylinder(d = 40, h = h);\n";

    #[test]
    fn a_design_in_pieces_takes_its_electronics_in_one_of_them() {
        let err = electrify(PIECES, "lamp.c2f", "lamp.c2fc", None).unwrap_err();
        assert!(err.contains("\"Base\", \"Dome\""), "{err}");
        assert!(electrify(PIECES, "lamp.c2f", "lamp.c2fc", Some("Lid")).is_err());
        let out = electrify(PIECES, "lamp.c2f", "lamp.c2fc", Some("Base")).unwrap();
        let design = &out.design;
        // Every place the piece is drawn, and only there.
        let wrapped = "difference() { union() { color(\"gray\") cylinder(d = 40, h = h); electronics_mounts(); } electronics_cuts(); }";
        assert!(design.contains(&format!("part(\"Base\") {wrapped}")), "{design}");
        assert!(design.contains("part(\"Base\") difference() { union() { cylinder(d = 40, h = h); electronics_mounts(); }"), "{design}");
        assert!(design.contains("part(\"Dome\") sphere(d = 20);") && !design.contains("module design()"), "{design}");
        assert!(design.starts_with("// A lamp.\n// @circuit lamp.c2fc\nh = 30;"), "{design}");
        c2f_lang::parser::parse(design).unwrap();
        assert!(design.contains("module electronics_cuts() {\n}") && design.contains("module electronics_mounts() {\n}"));
        // Without pieces, there is none to name.
        assert!(electrify(DESIGN, "box.c2f", "box.c2fc", Some("Base")).is_err());
    }
}
