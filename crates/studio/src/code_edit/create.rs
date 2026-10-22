//! Creating and combining (C2F-40): new shapes placed on a face,
//! booleans of selected objects, holes, and delete, duplicate, copy and
//! paste.
//!
//! Objects are moved, copied and removed with their placement (the
//! transforms written around them, see [`super::detached`]) and with the
//! `// ...` comment that follows them on their line.

use std::ops::Range;

use c2f_lang::ast::{Stmt, StmtKind};
use c2f_lang::lexer::Tok;

use super::{
    Doc, Edit, Splice, UNBRACED, apply, call, call_name, chars, detached, in_list, indent_of, indent_unit, line_end,
    line_start, newline, number, reindent, removal, snippet, starts_line, vector, with_comment,
};

/// Where new code goes.
#[derive(Clone, Debug, PartialEq)]
pub enum Place {
    /// On a line of its own at the end of the file.
    End,
    /// As the last child of the call, block or module definition written
    /// at this range (`difference() { ... }`); a call without braces gets
    /// them.
    InBlock(Range<usize>),
}

/// The shapes the toolbar creates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Cube,
    Cylinder,
    Sphere,
    /// A cylinder narrowing to a point.
    Cone,
    /// A cylinder with a cylindrical hole through it.
    Tube,
    /// Text extruded 2 mm.
    Text,
}

/// The boolean operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Boolean {
    Union,
    /// The first object minus the others.
    Difference,
    Intersection,
}

impl Boolean {
    fn name(self) -> &'static str {
        match self {
            Boolean::Union => "union",
            Boolean::Difference => "difference",
            Boolean::Intersection => "intersection",
        }
    }
}

/// The code of a new primitive of about 10 mm, standing on the point `at`
/// of a face whose outward normal is `normal` (+Z, the ground, when
/// `None`): its own Z axis is turned onto the normal, so it grows out of
/// the face. A statement at indentation 0, for [`insert_shape`].
pub fn primitive_code(kind: Primitive, at: [f64; 3], normal: Option<[f64; 3]>) -> String {
    let n = normal.unwrap_or([0.0, 0.0, 1.0]);
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    let n = if length > 1e-12 { [n[0] / length, n[1] / length, n[2] / length] } else { [0.0, 0.0, 1.0] };
    // rotate([0, tilt, turn]) takes Z to (sin tilt cos turn, sin tilt sin turn, cos tilt).
    let tilt = n[2].clamp(-1.0, 1.0).acos().to_degrees();
    let turn = if n[0].hypot(n[1]) > 1e-9 { n[1].atan2(n[0]).to_degrees() } else { 0.0 };
    let turned = number(tilt) != "0";
    // Shapes centered on their origin rise by half their height.
    let lifted = |h: f64| [at[0] + n[0] * h, at[1] + n[1] * h, at[2] + n[2] * h];
    let place = |origin: [f64; 3], turns: bool, body: &str| {
        let mut code = String::new();
        if origin.iter().any(|x| number(*x) != "0") {
            code += &format!("translate({}) ", vector(&origin));
        }
        if turns && turned {
            code += &format!("rotate([0, {}, {}]) ", number(tilt), number(turn));
        }
        code + body
    };
    match kind {
        Primitive::Cube => place(lifted(5.0), true, "cube(10, center = true);"),
        Primitive::Sphere => place(lifted(5.0), false, "sphere(r = 5);"),
        Primitive::Cylinder => place(at, true, "cylinder(h = 10, r = 5);"),
        Primitive::Cone => place(at, true, "cylinder(h = 10, r1 = 5, r2 = 0);"),
        Primitive::Tube => place(
            at,
            true,
            "difference() {\n    cylinder(h = 10, r = 5);\n    translate([0, 0, -1]) cylinder(h = 12, r = 3);\n}",
        ),
        Primitive::Text => place(
            at,
            true,
            "linear_extrude(height = 2) text(\"Texte\", size = 10, halign = \"center\", valign = \"center\");",
        ),
    }
}

/// Inserts the statement `code` (as [`primitive_code`] or [`copy_text`]
/// write it) at `at`, indented as its new place wants; the selection is
/// the new statement.
pub fn insert_shape(src: &str, at: &Place, code: &str) -> Result<Edit, String> {
    let (text, range) = snippet(code, &indent_unit(src))?;
    let (splice, offset, indent) = match at {
        Place::End => {
            let gap = if src.is_empty() || src.ends_with('\n') { "" } else { "\n" };
            (Splice::insert(src.len(), format!("{gap}{text}\n")), gap.len(), String::new())
        }
        Place::InBlock(block) => {
            let doc = Doc::parse(src)?;
            append_child(&doc, block, &text)?
        }
    };
    let (new, starts) = apply(src, std::slice::from_ref(&splice))?;
    let inner = reindented(&text, &range, "", &indent);
    let selection = starts[0] + offset + inner.start..starts[0] + offset + inner.end;
    Ok(Edit { cursor: chars(&new, selection.start), text: new, selection })
}

/// Pastes text [`copy_text`] gave, as [`insert_shape`] inserts code.
pub fn paste(src: &str, at: &Place, text: &str) -> Result<Edit, String> {
    insert_shape(src, at, text)
}

/// The text of the object at `range` with its placement and its comment,
/// at indentation 0, for [`paste`].
pub fn copy_text(src: &str, range: Range<usize>) -> Result<String, String> {
    let doc = Doc::parse(src)?;
    let path = doc.locate_instance(&range)?;
    let top = detached(&doc, &path).last().expect("located").stmt.span.range();
    Ok(reindent(&src[with_comment(src, &top)], indent_of(src, top.start), ""))
}

/// Removes the object at `range` with its placement, and its line when it
/// stands alone on it. The single child of an `if` that has an `else`
/// becomes `{}`. The selection is empty, where the object was.
pub fn delete(src: &str, range: Range<usize>) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    let path = doc.locate_instance(&range)?;
    let top = detached(&doc, &path);
    let r = top.last().expect("located").stmt.span.range();
    let splice = if in_list(&doc, top) { Splice::new(removal(src, &r), "") } else { Splice::new(r, "{}") };
    let (text, starts) = apply(src, std::slice::from_ref(&splice))?;
    let at = starts[0];
    Ok(Edit { cursor: chars(&text, at), text, selection: at..at })
}

/// Writes a copy of the object at `range`, with its placement, right after
/// it: on the next line when it ends its line. The selection is the copy
/// of the statement at `range`.
pub fn duplicate(src: &str, range: Range<usize>) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    let path = doc.locate_instance(&range)?;
    let top = detached(&doc, &path);
    if !in_list(&doc, top) {
        return Err(UNBRACED.into());
    }
    let r = top.last().expect("located").stmt.span.range();
    let ext = with_comment(src, &r);
    let eol = line_end(src, ext.end);
    let (splice, offset) = if src[ext.end..eol].trim().is_empty() {
        let indent = indent_of(src, r.start);
        let nl = newline(src, eol);
        (Splice::insert(eol, format!("{nl}{indent}{}", &src[ext])), nl.len() + indent.len())
    } else {
        (Splice::insert(r.end, format!(" {}", &src[r.clone()])), 1)
    };
    let (text, starts) = apply(src, std::slice::from_ref(&splice))?;
    let start = starts[0] + offset + range.start - r.start;
    let selection = start..start + range.len();
    Ok(Edit { cursor: chars(&text, start), text, selection })
}

/// Wraps the objects at `ranges`, with their placements, into one
/// `op() { ... }` written where the first one is, in the order given (the
/// first is what a difference cuts from). They must be in the same block.
/// The selection is the new boolean.
pub fn wrap_boolean(src: &str, ranges: &[Range<usize>], op: Boolean) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    if ranges.is_empty() {
        return Err("Sélectionnez au moins un objet.".into());
    }
    let paths = ranges.iter().map(|r| doc.locate_instance(r)).collect::<Result<Vec<_>, _>>()?;
    let tops: Vec<_> = paths.iter().map(|p| *detached(&doc, p).last().expect("located")).collect();
    let first = tops[0];
    if tops.iter().any(|t| !std::ptr::eq(t.list.as_ptr(), first.list.as_ptr())) {
        return Err("Ces objets sont dans des blocs différents : rassemblez-les dans le code d'abord.".into());
    }
    let mut indices: Vec<usize> = tops.iter().map(|t| t.index).collect();
    indices.sort_unstable();
    indices.dedup();
    if indices.len() != tops.len() {
        return Err("Le même objet est sélectionné deux fois.".into());
    }
    let start = first.stmt.span.start as usize;
    let indent = indent_of(src, start);
    let inner = format!("{indent}{}", indent_unit(src));
    let mut code = format!("{}() {{\n", op.name());
    for t in &tops {
        let r = t.stmt.span.range();
        code += &format!("{inner}{}\n", reindent(&src[with_comment(src, &r)], indent_of(src, r.start), &inner));
    }
    code += &format!("{indent}}}");
    let len = code.len();
    let mut splices = vec![Splice::new(with_comment(src, &first.stmt.span.range()), code)];
    splices.extend(tops[1..].iter().map(|t| Splice::new(removal(src, &t.stmt.span.range()), "")));
    let (text, starts) = apply(src, &splices)?;
    let selection = starts[0]..starts[0] + len;
    Ok(Edit { cursor: chars(&text, selection.start), text, selection })
}

/// Cuts `tool_code` out of the object at `target` (with its placement):
/// appends it to the `difference()` the object is the first child of, or
/// wraps both in a new one. The selection is the tool.
pub fn make_hole(src: &str, target: Range<usize>, tool_code: &str) -> Result<Edit, String> {
    let (tool, tool_range) = snippet(tool_code, &indent_unit(src))?;
    let doc = Doc::parse(src)?;
    let path = doc.locate_instance(&target)?;
    let top = detached(&doc, &path);
    let n = top.len();
    let loc = top[n - 1];
    if n >= 2 && loc.index == 0 && call_name(top[n - 2].stmt) == Some("difference") && in_list(&doc, top) {
        return insert_shape(src, &Place::InBlock(top[n - 2].stmt.span.range()), tool_code);
    }
    let r = loc.stmt.span.range();
    let indent = indent_of(src, r.start);
    let inner = format!("{indent}{}", indent_unit(src));
    let object = reindent(&src[with_comment(src, &r)], indent_of(src, r.start), &inner);
    let head = format!("difference() {{\n{inner}{object}\n{inner}");
    let code = format!("{head}{}\n{indent}}}", reindent(&tool, "", &inner));
    let (text, starts) = apply(src, &[Splice::new(with_comment(src, &r), code)])?;
    let inside = reindented(&tool, &tool_range, "", &inner);
    let selection = starts[0] + head.len() + inside.start..starts[0] + head.len() + inside.end;
    Ok(Edit { cursor: chars(&text, selection.start), text, selection })
}

/// Where `range` of `text` lands once `text` is reindented from `from` to
/// `to`: [`reindent`] works line by line, so the part before `range` moves
/// as it would in the whole text when given back the rest of its line.
pub(super) fn reindented(text: &str, range: &Range<usize>, from: &str, to: &str) -> Range<usize> {
    let start = reindent(&format!("{}x", &text[..range.start]), from, to).len() - 1;
    start..start + reindent(&text[range.clone()], from, to).len()
}

/// The splice that appends `text` (at indentation 0) as the last child of
/// the statement at `block`, the offset of `text` in the splice, and the
/// indentation it gets.
pub(super) fn append_child(doc: &Doc, block: &Range<usize>, text: &str) -> Result<(Splice, usize, String), String> {
    let src = doc.src;
    let path = doc.locate(block)?;
    let stmt = path.last().expect("located").stmt;
    let unit = indent_unit(src);
    let outer = indent_of(src, stmt.span.start as usize);
    let inner = format!("{outer}{unit}");
    let refused = || "On ne peut pas ajouter d'objet dans cet élément.".to_string();
    let braced = |open: usize, children: &[Stmt]| -> (Splice, usize, String) {
        let close = stmt.span.end as usize - 1;
        let indent = match children.first() {
            Some(c) if starts_line(src, c.span.start as usize) => indent_of(src, c.span.start as usize).to_string(),
            _ => format!("{}{unit}", indent_of(src, open)),
        };
        let body = reindent(text, "", &indent);
        if starts_line(src, close) {
            (Splice::insert(line_start(src, close), format!("{indent}{body}\n")), indent.len(), indent)
        } else {
            let gap = src[..close].len() - src[..close].trim_end_matches([' ', '\t']).len();
            let closing = indent_of(src, open);
            (Splice::new(close - gap..close, format!("\n{indent}{body}\n{closing}")), 1 + indent.len(), indent)
        }
    };
    match &stmt.kind {
        StmtKind::Block(stmts) => Ok(braced(stmt.span.start as usize, stmts)),
        StmtKind::Module(m) => match &m.body.kind {
            StmtKind::Block(stmts) => {
                let open = m.body.span.start as usize;
                let close = m.body.span.end as usize - 1;
                // The module's body closes where the module does.
                debug_assert_eq!(close, stmt.span.end as usize - 1);
                Ok(braced(open, stmts))
            }
            _ => Err(refused()),
        },
        StmtKind::Instance(_) => {
            let (name, _, children) = call(stmt).ok_or_else(refused)?;
            let (_, close_paren) = doc.parens(name).ok_or_else(refused)?;
            let after = doc.after_args(name).ok_or_else(refused)?;
            let body = reindent(text, "", &inner);
            match after.tok {
                Tok::LBrace => Ok(braced(after.span.start as usize, children)),
                Tok::Semi => {
                    let head = format!(" {{\n{inner}");
                    let splice =
                        Splice::new(close_paren + 1..after.span.end as usize, format!("{head}{body}\n{outer}}}"));
                    Ok((splice, head.len(), inner))
                }
                _ => {
                    let child = children[0].span.range();
                    let moved = reindent(&src[child.clone()], indent_of(src, child.start), &inner);
                    let head = format!(" {{\n{inner}{moved}\n{inner}");
                    let splice = Splice::new(close_paren + 1..child.end, format!("{head}{body}\n{outer}}}"));
                    Ok((splice, head.len(), inner))
                }
            }
        }
        _ => Err(refused()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{at, parses, same_solid, solid};
    use super::*;

    #[test]
    fn primitives_stand_on_the_face() {
        assert_eq!(primitive_code(Primitive::Cube, [0.0; 3], None), "translate([0, 0, 5]) cube(10, center = true);");
        assert_eq!(
            primitive_code(Primitive::Cylinder, [1.0, 2.0, 0.0], None),
            "translate([1, 2, 0]) cylinder(h = 10, r = 5);"
        );
        assert_eq!(
            primitive_code(Primitive::Cone, [10.0, 0.0, 5.0], Some([1.0, 0.0, 0.0])),
            "translate([10, 0, 5]) rotate([0, 90, 0]) cylinder(h = 10, r1 = 5, r2 = 0);"
        );
        // On a face looking along +X the shape grows along +X.
        let (_, min, max) = solid(&primitive_code(Primitive::Cube, [10.0, 0.0, 5.0], Some([2.0, 0.0, 0.0])));
        assert!((min[0] - 10.0).abs() < 1e-6 && (max[0] - 20.0).abs() < 1e-6, "{min:?} {max:?}");
        let (_, min, max) = solid(&primitive_code(Primitive::Cylinder, [0.0; 3], Some([0.0, -1.0, 0.0])));
        assert!((min[1] + 10.0).abs() < 1e-6 && max[1].abs() < 1e-6, "{min:?} {max:?}");
        let (_, min, _) = solid(&primitive_code(Primitive::Sphere, [0.0, 0.0, 3.0], Some([0.0, 0.0, -1.0])));
        assert!((min[2] + 7.0).abs() < 0.2, "{min:?}");
        for kind in [Primitive::Tube, Primitive::Text] {
            parses(&primitive_code(kind, [1.0, 2.0, 3.0], Some([0.0, 1.0, 1.0])));
        }
        let (volume, _, _) = solid(&primitive_code(Primitive::Tube, [0.0; 3], None));
        let (outer, _, _) = solid("cylinder(h = 10, r = 5);");
        assert!(volume < outer * 0.7);
    }

    #[test]
    fn shapes_go_at_the_end_or_in_a_block() {
        let edit = insert_shape("cube(1);", &Place::End, "sphere(2);").unwrap();
        assert_eq!(edit.text, "cube(1);\nsphere(2);\n");
        assert_eq!(&edit.text[edit.selection], "sphere(2);");
        assert_eq!(insert_shape("", &Place::End, "  sphere(2);  ").unwrap().text, "sphere(2);\n");

        let src = "module box() {\n  difference() {\n    cube(10);\n  }\n}\n";
        let block = at(src, "difference() {\n    cube(10);\n  }", 0);
        let tube = primitive_code(Primitive::Tube, [0.0; 3], None);
        let edit = insert_shape(src, &Place::InBlock(block), &tube).unwrap();
        assert_eq!(
            edit.text,
            "module box() {\n  difference() {\n    cube(10);\n    difference() {\n      cylinder(h = 10, r = 5);\n      translate([0, 0, -1]) cylinder(h = 12, r = 3);\n    }\n  }\n}\n"
        );
        assert_eq!(
            &edit.text[edit.selection],
            "difference() {\n      cylinder(h = 10, r = 5);\n      translate([0, 0, -1]) cylinder(h = 12, r = 3);\n    }"
        );
        // Into the module itself, then into calls without braces.
        let edit = insert_shape(src, &Place::InBlock(0..src.len() - 1), "sphere(1);").unwrap();
        assert!(edit.text.ends_with("  }\n  sphere(1);\n}\n"), "{}", edit.text);
        let edit = insert_shape("union();", &Place::InBlock(0..8), "cube(1);").unwrap();
        assert_eq!(edit.text, "union() {\n    cube(1);\n}");
        assert_eq!(&edit.text[edit.selection], "cube(1);");
        let edit = insert_shape("union() cube(1);", &Place::InBlock(0..16), "sphere(1);").unwrap();
        assert_eq!(edit.text, "union() {\n    cube(1);\n    sphere(1);\n}");
        let edit = insert_shape("union() {}", &Place::InBlock(0..10), "sphere(1);").unwrap();
        assert_eq!(edit.text, "union() {\n    sphere(1);\n}");
        assert!(insert_shape("cube(1);", &Place::End, "cube(").is_err());
    }

    #[test]
    fn booleans_wrap_siblings_in_order() {
        let src = "// parts\ncube(10); // body\nx = 2;\ntranslate([5, 5, 5])\n    sphere(4);\n";
        let sphere = at(src, "sphere(4);", 0);
        let edit = wrap_boolean(src, &[at(src, "cube(10);", 0), sphere.clone()], Boolean::Difference).unwrap();
        assert_eq!(
            edit.text,
            "// parts\ndifference() {\n    cube(10); // body\n    translate([5, 5, 5])\n        sphere(4);\n}\nx = 2;\n"
        );
        assert_eq!(&edit.text[edit.selection.clone()], &edit.text[9..edit.text.len() - 8]);
        same_solid(&edit.text, "difference() { cube(10); translate([5, 5, 5]) sphere(4); }");
        // The first given is the base, wherever it is.
        let edit = wrap_boolean(src, &[sphere, at(src, "cube(10);", 0)], Boolean::Union).unwrap();
        assert!(
            edit.text.starts_with(
                "// parts\nx = 2;\nunion() {\n    translate([5, 5, 5])\n        sphere(4);\n    cube(10); // body\n}\n"
            ),
            "{}",
            edit.text
        );

        let src = "union() {\n    cube(1);\n}\nsphere(1);\n";
        let err = wrap_boolean(src, &[at(src, "cube(1);", 0), at(src, "sphere(1);", 0)], Boolean::Union).unwrap_err();
        assert!(err.contains("blocs différents"));
        assert!(wrap_boolean(src, &[at(src, "sphere(1);", 0), at(src, "sphere(1);", 0)], Boolean::Union).is_err());
    }

    #[test]
    fn holes_join_or_start_a_difference() {
        let src = "translate([0, 0, 1]) cube(10);\n";
        let edit = make_hole(src, at(src, "cube(10);", 0), "cylinder(h = 30, r = 2);").unwrap();
        assert_eq!(edit.text, "difference() {\n    translate([0, 0, 1]) cube(10);\n    cylinder(h = 30, r = 2);\n}\n");
        assert_eq!(&edit.text[edit.selection.clone()], "cylinder(h = 30, r = 2);");
        same_solid(&edit.text, "difference() { translate([0, 0, 1]) cube(10); cylinder(h = 30, r = 2); }");
        let again = make_hole(&edit.text, at(&edit.text, "cube(10);", 0), "sphere(1);").unwrap();
        assert_eq!(
            again.text,
            "difference() {\n    translate([0, 0, 1]) cube(10);\n    cylinder(h = 30, r = 2);\n    sphere(1);\n}\n"
        );
        assert_eq!(&again.text[again.selection], "sphere(1);");
    }

    #[test]
    fn delete_duplicate_copy_paste() {
        let src = "union() {\n    translate([1, 0, 0]) cube(1); // a\n    sphere(1);\n}\n";
        let cube = at(src, "cube(1);", 0);
        let edit = delete(src, cube.clone()).unwrap();
        assert_eq!(edit.text, "union() {\n    sphere(1);\n}\n");
        assert_eq!(edit.selection, 10..10);

        let edit = duplicate(src, cube.clone()).unwrap();
        assert_eq!(
            edit.text,
            "union() {\n    translate([1, 0, 0]) cube(1); // a\n    translate([1, 0, 0]) cube(1); // a\n    sphere(1);\n}\n"
        );
        assert_eq!(edit.selection, at(&edit.text, "cube(1);", 1));
        let edit = duplicate("cube(1); sphere(1);", 0..8).unwrap();
        assert_eq!(edit.text, "cube(1); cube(1); sphere(1);");
        assert_eq!(edit.selection, 9..17);

        let copied = copy_text(src, cube).unwrap();
        assert_eq!(copied, "translate([1, 0, 0]) cube(1); // a");
        let edit = paste(src, &Place::End, &copied).unwrap();
        assert!(edit.text.ends_with("}\ntranslate([1, 0, 0]) cube(1); // a\n"));
        assert_eq!(&edit.text[edit.selection], "translate([1, 0, 0]) cube(1);");

        // The body of a loop goes with it; an `if` with an `else` keeps one.
        let src = "for (i = [0 : 2])\n    translate([i * 2, 0, 0]) cube(1);\nsphere(1);\n";
        assert_eq!(delete(src, at(src, "cube(1);", 0)).unwrap().text, "sphere(1);\n");
        let src = "if (true) cube(1); else sphere(1);";
        assert_eq!(delete(src, at(src, "cube(1);", 0)).unwrap().text, "if (true) {} else sphere(1);");
        assert!(duplicate(src, at(src, "cube(1);", 0)).is_err());
        parses(&delete(src, at(src, "sphere(1);", 0)).unwrap().text);
    }

    /// An object that ends a line others start leaves with the comment
    /// naming it, which its copies carry: no name is left behind twice.
    #[test]
    fn a_named_object_takes_its_name_along() {
        let src = "a(); b(); // B\nc();\n";
        assert_eq!(delete(src, at(src, "b();", 0)).unwrap().text, "a();\nc();\n");
        let src = "cube(1); sphere(1); // S\n";
        let edit = wrap_boolean(src, &[at(src, "cube(1);", 0), at(src, "sphere(1);", 0)], Boolean::Union).unwrap();
        assert_eq!(edit.text, "union() {\n    cube(1);\n    sphere(1); // S\n}\n");
    }

    #[test]
    fn copies_keep_the_sources_line_breaks() {
        let src = "union() {\r\n    cube(1); // a\r\n}\r\n";
        let edit = duplicate(src, at(src, "cube(1);", 0)).unwrap();
        assert_eq!(edit.text, "union() {\r\n    cube(1); // a\r\n    cube(1); // a\r\n}\r\n");
        assert_eq!(edit.selection, at(&edit.text, "cube(1);", 1));
        let edit = duplicate("cube(1);\r\nsphere(1);", 0..8).unwrap();
        assert_eq!(edit.text, "cube(1);\r\ncube(1);\r\nsphere(1);");
    }
}
