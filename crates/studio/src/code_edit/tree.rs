//! The tree of the program (C2F-41): its objects as nested nodes, their
//! names, their modifiers, and moving them by drag and drop.
//!
//! An object's name is the `// ...` comment that ends the line its
//! statement starts on: `cube(10); // Corps`, `difference() { // Coque`.
//! A line's comment names the outermost statement starting on it, and only
//! when no other statement of the same block starts after it on that line,
//! so that `translate([5, 0, 0]) cube(2); // Pied` names the `translate`.

use std::ops::Range;

use c2f_lang::ast::{InstanceKind, Modifiers, Stmt, StmtKind};
use c2f_lang::lexer::Tok;
use c2f_lang::span::LineIndex;

use super::create::{append_child, reindented};
use super::{
    Doc, Edit, Loc, Splice, UNBRACED, apply, chars, detached, in_list, indent_of, line_end, line_start, newline,
    reindent,
    removal, single, starts_line, trailing_comment, with_comment,
};

/// What an object of the tree is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    /// `union`, `difference`, `intersection`, `hull`, `minkowski`, `group`.
    Boolean,
    /// A builtin that transforms its children: `translate`, `color`,
    /// `linear_extrude`...
    Transform,
    /// A builtin shape: `cube`, `cylinder`, `text`, `import`...
    Primitive,
    /// A call of a module the program or a library defines, or of another
    /// builtin.
    Module,
    /// `if`, `for`, `let`, `each`, `intersection_for`.
    Control,
    /// `echo`, `assert`, `children`, `pause`.
    Other,
}

/// One instance statement of the main file and its children.
#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode {
    pub kind: NodeKind,
    /// What the tree shows: the statement's name, else its module.
    pub name: String,
    /// The module called, or `if`.
    pub call: String,
    /// The `// ...` comment that names the statement, without `//`.
    pub label: Option<String>,
    /// The whole statement, the range edits take.
    pub range: Range<usize>,
    pub modifiers: Modifiers,
    /// Its children, blocks flattened; an `if`'s `else` branch follows its
    /// `then` branch.
    pub children: Vec<TreeNode>,
}

/// The modifiers the tree toggles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    /// `*`: left out, as if commented out.
    Disable,
    /// `!`: only this object is shown.
    Root,
    /// `%`: drawn transparent, left out of the model.
    Background,
    /// `#`: highlighted.
    /// `#`: not offered by the tree, which shows the three others.
    #[allow(dead_code)]
    Highlight,
}

impl Modifier {
    pub fn symbol(self) -> char {
        match self {
            Modifier::Disable => '*',
            Modifier::Root => '!',
            Modifier::Background => '%',
            Modifier::Highlight => '#',
        }
    }

    fn token(self) -> Tok {
        match self {
            Modifier::Disable => Tok::Star,
            Modifier::Root => Tok::Bang,
            Modifier::Background => Tok::Percent,
            Modifier::Highlight => Tok::Hash,
        }
    }
}

/// Where [`move_into`] puts an object.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Just before the object at this range, in its block.
    Before(Range<usize>),
    /// Just after it.
    After(Range<usize>),
    /// As the last child of the call, block or module definition at this
    /// range (see [`super::Place::InBlock`]).
    IntoBlock(Range<usize>),
}

/// The program's objects: its instance statements, outermost first.
/// Module and function definitions, assignments and `include`s are left
/// out: what a module draws shows where it is called.
pub fn tree(src: &str) -> Result<Vec<TreeNode>, String> {
    let doc = Doc::parse(src)?;
    let lines = LineIndex::new(src);
    Ok(nodes(&doc, &lines, &doc.file.stmts, None))
}

fn nodes(doc: &Doc, lines: &LineIndex, list: &[Stmt], parent_line: Option<usize>) -> Vec<TreeNode> {
    let mut items = Vec::new();
    flatten(list, &mut items);
    let line = |s: &Stmt| lines.line(s.span.start as usize);
    (0..items.len())
        .map(|i| {
            let stmt = items[i];
            let StmtKind::Instance(instance) = &stmt.kind else { unreachable!("flatten keeps instances") };
            let l = line(stmt);
            let owns = parent_line != Some(l) && items[i + 1..].iter().all(|s| line(s) != l);
            let label = if owns { label(doc, lines, l) } else { None };
            let (call, children) = match &instance.kind {
                InstanceKind::Call { name, children, .. } => (name.name.clone(), nodes(doc, lines, children, Some(l))),
                InstanceKind::If { then, otherwise, .. } => {
                    let mut children = nodes(doc, lines, then, Some(l));
                    children.extend(nodes(doc, lines, otherwise.as_deref().unwrap_or(&[]), Some(l)));
                    ("if".to_string(), children)
                }
            };
            TreeNode {
                kind: kind_of(&call),
                name: label.clone().unwrap_or_else(|| call.clone()),
                call,
                label,
                range: stmt.span.range(),
                modifiers: instance.modifiers,
                children,
            }
        })
        .collect()
}

/// The instance statements of a list, those of its blocks in their place.
fn flatten<'a>(list: &'a [Stmt], out: &mut Vec<&'a Stmt>) {
    for stmt in list.iter().filter(|s| s.span.file == 0) {
        match &stmt.kind {
            StmtKind::Instance(_) => out.push(stmt),
            StmtKind::Block(stmts) => flatten(stmts, out),
            _ => {}
        }
    }
}

fn kind_of(call: &str) -> NodeKind {
    match call {
        "union" | "difference" | "intersection" | "hull" | "minkowski" | "group" => NodeKind::Boolean,
        "translate" | "rotate" | "scale" | "mirror" | "multmatrix" | "resize" | "color" | "offset" | "render"
        | "projection" | "linear_extrude" | "rotate_extrude" => NodeKind::Transform,
        "cube" | "sphere" | "cylinder" | "polyhedron" | "square" | "circle" | "polygon" | "text" | "import"
        | "import_stl" | "import_off" | "import_dxf" | "surface" => NodeKind::Primitive,
        "if" | "for" | "let" | "each" | "intersection_for" => NodeKind::Control,
        "echo" | "assert" | "children" | "pause" => NodeKind::Other,
        _ => NodeKind::Module,
    }
}

/// The comment ending line `line`, after its last token.
fn line_comment(doc: &Doc, lines: &LineIndex, line: usize) -> Option<(usize, Option<Range<usize>>)> {
    let start = lines.line_start(line);
    let end = line_end(doc.src, start);
    let last = doc.tokens.iter().take_while(|t| (t.span.start as usize) < end).filter(|t| t.tok != Tok::Eof).last()?;
    let last_end = last.span.end as usize;
    (last.span.start as usize >= start).then(|| (last_end, trailing_comment(doc.src, last_end)))
}

fn label(doc: &Doc, lines: &LineIndex, line: usize) -> Option<String> {
    let (_, comment) = line_comment(doc, lines, line)?;
    let text = doc.src[comment?].trim_start_matches('/').trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Names the object at `range`: writes `// name` at the end of the line
/// its statement starts on, or replaces the comment there; an empty name
/// removes it. The name goes to the outermost statement starting on that
/// line, as [`tree`] reads it.
pub fn rename(src: &str, range: Range<usize>, name: &str) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    let path = doc.locate_instance(&range)?;
    let lines = LineIndex::new(src);
    let line = lines.line(range.start);
    // Climb to the outermost instance starting on that line.
    let mut k = path.len() - 1;
    while let Some(j) = (0..k).rev().find(|&j| !matches!(path[j].stmt.kind, StmtKind::Block(_))) {
        if !matches!(path[j].stmt.kind, StmtKind::Instance(_)) || lines.line(path[j].stmt.span.start as usize) != line {
            break;
        }
        k = j;
    }
    let owner = path[k];
    if owner.list[owner.index + 1..].iter().any(|s| lines.line(s.span.start as usize) == line) {
        return Err(
            "Several objects start on this line: put this one on a line of its own to name it.".into(),
        );
    }
    let name: String = name.replace(['\n', '\r'], " ").trim().to_string();
    let (last_end, comment) = line_comment(&doc, &lines, line).ok_or(super::STALE)?;
    let splice = match (comment, name.is_empty()) {
        (Some(c), true) => {
            let before = &src[..c.start];
            Splice::new(before.trim_end_matches([' ', '\t']).len()..c.end, "")
        }
        (Some(c), false) => Splice::new(c, format!("// {name}")),
        (None, true) => return Ok(unchanged(src, &range)),
        (None, false) => Splice::insert(last_end, format!(" // {name}")),
    };
    let (text, _) = apply(src, std::slice::from_ref(&splice))?;
    // A comment written right after the statement stays out of it.
    let selection = splice.shift(range.start, false)..splice.shift(range.end, false);
    Ok(Edit { cursor: chars(&text, selection.start), text, selection })
}

fn unchanged(src: &str, range: &Range<usize>) -> Edit {
    Edit { text: src.to_string(), cursor: chars(src, range.start), selection: range.clone() }
}

/// Adds (`on`) or removes the modifier written before the statement at
/// `range`; `None` removes all its modifiers. Adding a modifier it has, or
/// removing one it has not, changes nothing.
pub fn set_modifier(src: &str, range: Range<usize>, modifier: Option<Modifier>, on: bool) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    let path = doc.locate_instance(&range)?;
    let stmt = path.last().expect("located").stmt;
    let first = doc.token_at(range.start);
    let name = doc.after_modifiers(stmt);
    let Some(modifier) = modifier else {
        return if name == range.start {
            Ok(unchanged(src, &range))
        } else {
            single(src, Splice::new(range.start..name, ""), &range, false, false)
        };
    };
    let present =
        doc.tokens[first..].iter().take_while(|t| (t.span.start as usize) < name).find(|t| t.tok == modifier.token());
    let splice = match (present, on) {
        (None, true) => Splice::insert(range.start, modifier.symbol().to_string()),
        (Some(t), false) => {
            let end = t.span.end as usize;
            let rest = &src[end..];
            Splice::new(t.span.start as usize..end + rest.len() - rest.trim_start_matches([' ', '\t']).len(), "")
        }
        _ => return Ok(unchanged(src, &range)),
    };
    single(src, splice, &range, false, false)
}

/// Moves the object at `range`, with its placement and its comment, to
/// `target`, reindented for its new place: to reorder a block, or to put
/// an object into a `difference()`. The selection follows the statement
/// at `range`.
pub fn move_into(src: &str, range: Range<usize>, target: Target) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    let path = doc.locate_instance(&range)?;
    let top = detached(&doc, &path);
    if !in_list(&doc, top) {
        return Err(UNBRACED.into());
    }
    let m = top.last().expect("located").stmt.span.range();
    let ext = with_comment(src, &m);
    let from = indent_of(src, m.start);
    let inside = |t: &Range<usize>| m.start <= t.start && t.end <= m.end;
    let itself = || "An object cannot go inside itself.".to_string();
    let selected = range.start - m.start..range.end - m.start;

    let (insertion, offset, to) = match &target {
        Target::Before(t) | Target::After(t) => {
            let tpath = doc.locate_instance(t)?;
            let ttop = detached(&doc, &tpath);
            if !in_list(&doc, ttop) {
                return Err(UNBRACED.into());
            }
            let t: &Loc = ttop.last().expect("located");
            let t = t.stmt.span.range();
            if inside(&t) {
                return Err(itself());
            }
            let to = indent_of(src, t.start).to_string();
            let whole = reindent(&src[ext.clone()], from, &to);
            // Between statements on a line, an object named by a comment
            // takes it along and ends its line there, the rest going on
            // the next one.
            let named = ext.end > m.end;
            let nl = newline(src, line_end(src, t.start));
            let inline = if named { format!("{whole}{nl}{to}") } else { format!("{whole} ") };
            match target {
                Target::Before(_) if starts_line(src, t.start) => {
                    (Splice::insert(line_start(src, t.start), format!("{to}{whole}{nl}")), to.len(), to)
                }
                Target::Before(_) => (Splice::insert(t.start, inline), 0, to),
                _ => {
                    let text_end = with_comment(src, &t).end;
                    let eol = line_end(src, text_end);
                    if src[text_end..eol].trim().is_empty() {
                        let nl = newline(src, eol);
                        (Splice::insert(eol, format!("{nl}{to}{whole}")), nl.len() + to.len(), to)
                    } else if m.start >= t.end && src[t.end..m.start].trim().is_empty() {
                        // Already right after it.
                        return Ok(unchanged(src, &range));
                    } else {
                        let rest = &src[t.end..];
                        let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
                        (Splice::new(t.end..t.end + spaces, format!(" {inline}")), 1, to)
                    }
                }
            }
        }
        Target::IntoBlock(t) => {
            doc.locate(t)?;
            if inside(t) {
                return Err(itself());
            }
            let moved = reindent(&src[ext.clone()], from, "");
            append_child(&doc, t, &moved)?
        }
    };
    let splices = [Splice::new(removal(src, &m), ""), insertion];
    let (text, starts) = apply(src, &splices)?;
    let within = reindented(&src[ext.clone()], &selected, from, &to);
    let start = starts[1] + offset + within.start;
    let selection = start..starts[1] + offset + within.end;
    Ok(Edit { cursor: chars(&text, start), text, selection })
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{at, parses, same_solid};
    use super::*;

    const BOX: &str = "\
// Boîtier
difference() { // Coque
    cube(10, center = true);
    #translate([0, 0, 2]) cylinder(h = 20, r = 3); // Trou
}
for (i = [0 : 1]) translate([i * 20, 0, 0]) sphere(2);
module peg() cube(1);
{
    peg(); // Cheville
}
";

    #[test]
    fn the_tree_follows_the_statements() {
        let nodes = tree(BOX).unwrap();
        let names: Vec<_> = nodes.iter().map(|n| (n.kind, n.name.as_str())).collect();
        assert_eq!(names, [(NodeKind::Boolean, "Coque"), (NodeKind::Control, "for"), (NodeKind::Module, "Cheville")]);
        let shell = &nodes[0];
        assert_eq!(shell.call, "difference");
        assert_eq!(shell.children.len(), 2);
        assert_eq!(shell.children[0].kind, NodeKind::Primitive);
        assert_eq!(shell.children[0].label, None);
        let hole = &shell.children[1];
        assert_eq!((hole.kind, hole.name.as_str()), (NodeKind::Transform, "Trou"));
        assert!(hole.modifiers.highlight);
        assert_eq!(&BOX[hole.range.clone()], "#translate([0, 0, 2]) cylinder(h = 20, r = 3);");
        // The comment names the outer statement, not its child.
        assert_eq!(hole.children[0].label, None);
        assert_eq!(hole.children[0].name, "cylinder");
        assert_eq!(nodes[1].children[0].children[0].call, "sphere");
        assert!(tree("cube(").is_err());
    }

    #[test]
    fn names_are_written_as_comments() {
        let src = "cube(1);\ntranslate([1, 0, 0]) sphere(1); // Old\n";
        let edit = rename(src, at(src, "cube(1);", 0), "Base").unwrap();
        assert_eq!(edit.text, "cube(1); // Base\ntranslate([1, 0, 0]) sphere(1); // Old\n");
        assert_eq!(&edit.text[edit.selection], "cube(1);");
        // From the child, the line's owner is renamed.
        let edit = rename(src, at(src, "sphere(1);", 0), "Boule").unwrap();
        assert_eq!(edit.text, "cube(1);\ntranslate([1, 0, 0]) sphere(1); // Boule\n");
        assert_eq!(tree(&edit.text).unwrap()[1].name, "Boule");
        let edit = rename(src, at(src, "sphere(1);", 0), "").unwrap();
        assert_eq!(edit.text, "cube(1);\ntranslate([1, 0, 0]) sphere(1);\n");
        let src = "union() {\n    cube(1);\n}";
        let edit = rename(src, 0..src.len(), "Groupe").unwrap();
        assert_eq!(edit.text, "union() { // Groupe\n    cube(1);\n}");
        assert_eq!(edit.selection, 0..edit.text.len());
        assert!(rename("cube(1); sphere(1);", 0..8, "A").is_err());
    }

    #[test]
    fn modifiers_are_toggled() {
        let src = "union() {\n    cube(1);\n}\n";
        let range = at(src, "cube(1);", 0);
        let edit = set_modifier(src, range.clone(), Some(Modifier::Disable), true).unwrap();
        assert_eq!(edit.text, "union() {\n    *cube(1);\n}\n");
        assert_eq!(&edit.text[edit.selection.clone()], "*cube(1);");
        let again = set_modifier(&edit.text, edit.selection.clone(), Some(Modifier::Disable), true).unwrap();
        assert_eq!(again.text, edit.text);
        let off = set_modifier(&edit.text, edit.selection.clone(), Some(Modifier::Disable), false).unwrap();
        assert_eq!(off.text, src);
        assert_eq!(off.selection, range);
        let src = "#% cube(1);";
        let edit = set_modifier(src, 0..src.len(), Some(Modifier::Background), false).unwrap();
        assert_eq!(edit.text, "#cube(1);");
        assert_eq!(set_modifier(src, 0..src.len(), None, false).unwrap().text, "cube(1);");
    }

    #[test]
    fn objects_move_and_are_reindented() {
        let src = "a();\nb(); // B\nc();\n";
        let edit = move_into(src, at(src, "a();", 0), Target::After(at(src, "b();", 0))).unwrap();
        assert_eq!(edit.text, "b(); // B\na();\nc();\n");
        assert_eq!(&edit.text[edit.selection], "a();");
        let edit = move_into(src, at(src, "c();", 0), Target::Before(at(src, "a();", 0))).unwrap();
        assert_eq!(edit.text, "c();\na();\nb(); // B\n");
        let edit = move_into(src, at(src, "b();", 0), Target::Before(at(src, "c();", 0))).unwrap();
        assert_eq!(edit.text, src);

        let src = "difference() {\n    cube(10);\n}\ntranslate([2, 2, -1])\n    cylinder(h = 12, r = 1); // Trou\n";
        let tool = at(src, "cylinder(h = 12, r = 1);", 0);
        let block = at(src, "difference() {\n    cube(10);\n}", 0);
        let edit = move_into(src, tool, Target::IntoBlock(block)).unwrap();
        let expected =
            "difference() {\n    cube(10);\n    translate([2, 2, -1])\n        cylinder(h = 12, r = 1); // Trou\n}\n";
        assert_eq!(edit.text, expected);
        assert_eq!(&edit.text[edit.selection.clone()], "cylinder(h = 12, r = 1);");
        same_solid(&edit.text, "difference() { cube(10); translate([2, 2, -1]) cylinder(h = 12, r = 1); }");

        // Out again, before the difference.
        let edit = move_into(&edit.text, edit.selection.clone(), Target::Before(0..expected.len() - 1)).unwrap();
        assert_eq!(
            edit.text,
            "translate([2, 2, -1])\n    cylinder(h = 12, r = 1); // Trou\ndifference() {\n    cube(10);\n}\n"
        );
        parses(&edit.text);
        let whole = 0..expected.len() - 1;
        assert!(move_into(expected, whole.clone(), Target::IntoBlock(whole)).is_err());
    }

    /// The name goes where the object goes, whether it leaves a shared
    /// line or lands between statements on one.
    #[test]
    fn moved_objects_keep_their_name() {
        let src = "a(); b(); // B\nc();\n";
        let edit = move_into(src, at(src, "b();", 0), Target::After(at(src, "c();", 0))).unwrap();
        assert_eq!(edit.text, "a();\nc();\nb(); // B\n");
        assert_eq!(&edit.text[edit.selection], "b();");
        // Already right after its target: nothing changes.
        let edit = move_into(src, at(src, "b();", 0), Target::After(at(src, "a();", 0))).unwrap();
        assert_eq!(edit.text, src);

        let src = "a(); c();\nb(); // B\n";
        let edit = move_into(src, at(src, "b();", 0), Target::After(at(src, "a();", 0))).unwrap();
        assert_eq!(edit.text, "a(); b(); // B\nc();\n");
        assert_eq!(&edit.text[edit.selection], "b();");
        let edit = move_into(src, at(src, "b();", 0), Target::Before(at(src, "c();", 0))).unwrap();
        assert_eq!(edit.text, "a(); b(); // B\nc();\n");
        assert_eq!(&edit.text[edit.selection], "b();");
        assert_eq!(tree(&edit.text).unwrap()[1].name, "B");
        // Unnamed, it still joins the line.
        let src = "a(); c();\nb();\n";
        let edit = move_into(src, at(src, "b();", 0), Target::After(at(src, "a();", 0))).unwrap();
        assert_eq!(edit.text, "a(); b(); c();\n");
    }

    #[test]
    fn moves_keep_the_sources_line_breaks() {
        let src = "a();\r\nb(); // B\r\nc();\r\n";
        let edit = move_into(src, at(src, "a();", 0), Target::After(at(src, "b();", 0))).unwrap();
        assert_eq!(edit.text, "b(); // B\r\na();\r\nc();\r\n");
        assert_eq!(&edit.text[edit.selection], "a();");
        let edit = move_into(src, at(src, "c();", 0), Target::Before(at(src, "a();", 0))).unwrap();
        assert_eq!(edit.text, "c();\r\na();\r\nb(); // B\r\n");
    }
}
