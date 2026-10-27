//! The properties panel (C2F-38): what a call is given, and rewriting a
//! literal argument, one component of a vector, or adding an argument.

use std::ops::Range;

use c2f_lang::ast::{Arg, Element, ExprKind, Ident, Modifiers, Stmt, StmtKind};
use c2f_lang::lexer::Tok;

use super::{Doc, Edit, Splice, Value, call, child_lists, computed, indent_of, literal, number, scalar, single};

/// A module call as the panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    /// The module's name: `cube`, `translate`, a module of the file...
    pub name: String,
    pub name_range: Range<usize>,
    /// The whole statement, modifiers and children included.
    pub statement: Range<usize>,
    pub modifiers: Modifiers,
    pub args: Vec<CallArg>,
    /// From `(` through `)`.
    pub parens: Range<usize>,
    /// The statements of its children.
    pub children: Vec<Range<usize>>,
    /// From `{` through `}` when its children are braced.
    pub block: Option<Range<usize>>,
}

/// One argument of a call.
#[derive(Clone, Debug, PartialEq)]
pub struct CallArg {
    /// The name written before `=`, if any.
    pub name: Option<String>,
    /// The parameter the argument fills: its name, or for a positional
    /// argument the parameter at its position in the builtin's signature
    /// (see [`builtin_params`]) or in the module's definition when the
    /// file defines it; `None` when unknown.
    pub param: Option<String>,
    /// The whole argument, `name = ` included.
    pub range: Range<usize>,
    pub value_range: Range<usize>,
    pub value: ArgValue,
}

/// What an argument's value is.
#[derive(Clone, Debug, PartialEq)]
pub enum ArgValue {
    /// Written literally, so the panel can rewrite it.
    Literal(Value),
    /// Computed (`width / 2`) or named (`width`): its text. The panel
    /// shows it read-only.
    Expression(String),
}

/// Which argument an edit is about: the parameter it fills (named or
/// positional, see [`CallArg::param`]), or the `n`th positional argument.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ArgKey<'a> {
    Name(&'a str),
    Position(usize),
}

/// The positional parameters of a builtin, in order, as the evaluator
/// binds them; empty for names it does not know. `linear_extrude`'s first
/// positional argument is its height when it is a number.
pub fn builtin_params(name: &str) -> &'static [&'static str] {
    match name {
        "cube" | "square" => &["size", "center"],
        "sphere" | "circle" => &["r"],
        "cylinder" => &["h", "r1", "r2", "center"],
        "polyhedron" => &["points", "faces", "convexity"],
        "polygon" => &["points", "paths", "convexity"],
        "translate" | "scale" | "mirror" => &["v"],
        "rotate" => &["a", "v"],
        "multmatrix" => &["m"],
        "color" => &["c", "alpha"],
        "offset" => &["r"],
        "resize" => &["newsize", "auto", "convexity"],
        "linear_extrude" => &["height"],
        "text" => &["text", "size", "font"],
        "pause" => &["z", "message"],
        "surface" => &["file", "center", "convexity"],
        "import" => &["file", "layer", "convexity", "origin", "scale"],
        "render" | "minkowski" => &["convexity"],
        "projection" => &["cut"],
        _ => &[],
    }
}

/// The call written at `range`.
pub fn call_at(src: &str, range: Range<usize>) -> Result<Call, String> {
    let doc = Doc::parse(src)?;
    let r = Resolved::new(&doc, &range)?;
    let (open, close) = doc.parens(r.name).ok_or(super::STALE)?;
    let block = match doc.after_args(r.name) {
        Some(t) if t.tok == Tok::LBrace => Some(t.span.start as usize..r.stmt.span.end as usize),
        _ => None,
    };
    let modifiers = match &r.stmt.kind {
        StmtKind::Instance(i) => i.modifiers,
        _ => Modifiers::default(),
    };
    let args = r
        .args
        .iter()
        .enumerate()
        .map(|(i, arg)| CallArg {
            name: arg.name.as_ref().map(|n| n.name.clone()),
            param: r.param(i),
            range: arg.name.as_ref().map_or(arg.value.span.start, |n| n.span.start) as usize
                ..arg.value.span.end as usize,
            value_range: arg.value.span.range(),
            value: match literal(&arg.value) {
                Some(v) => ArgValue::Literal(v),
                None => ArgValue::Expression(src[arg.value.span.range()].to_string()),
            },
        })
        .collect();
    Ok(Call {
        name: r.name.name.clone(),
        name_range: r.name.span.range(),
        statement: range,
        modifiers,
        args,
        parens: open..close + 1,
        children: r.children.iter().map(|c| c.span.range()).collect(),
        block,
    })
}

/// Writes `value` in place of a literal argument. An argument the source
/// computes is refused, as the edit would lose the computation. A
/// parameter given no argument gets one, as [`add_arg`] adds it.
pub fn set_arg(src: &str, range: Range<usize>, key: ArgKey, value: &Value) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    let r = Resolved::new(&doc, &range)?;
    let Some(arg) = r.find(key) else {
        return match key {
            ArgKey::Name(name) => add_arg(src, range, name, value),
            ArgKey::Position(_) => Err(missing(key)),
        };
    };
    if literal(&arg.value).is_none() {
        return Err(computed(src, &arg.value));
    }
    single(src, Splice::new(arg.value.span.range(), value.code()), &range, false, true)
}

/// Writes `x` as component `index` (0 for x) of a vector argument, such
/// as `size.y` of `cube([10, 20, 5])`. Only that component is rewritten,
/// so the others may be computed; a single number (`cube(5)`) becomes a
/// vector of it (three components, two for `square`).
pub fn set_arg_component(src: &str, range: Range<usize>, key: ArgKey, index: usize, x: f64) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    let r = Resolved::new(&doc, &range)?;
    let arg = r.find(key).ok_or_else(|| missing(key))?;
    let splice = match &arg.value.kind {
        ExprKind::Vector(elements) => match elements.get(index) {
            Some(Element::Expr(e)) if scalar(e).is_some() => Splice::new(e.span.range(), number(x)),
            Some(Element::Expr(e)) => return Err(computed(src, e)),
            _ => return Err("This vector has no such component.".into()),
        },
        _ => match scalar(&arg.value) {
            Some(n) => {
                let size = if r.name.name == "square" { 2 } else { 3 };
                if index >= size {
                    return Err("This vector has no such component.".into());
                }
                let mut v = vec![n; size];
                v[index] = x;
                Splice::new(arg.value.span.range(), super::vector(&v))
            }
            None => return Err(computed(src, &arg.value)),
        },
    };
    single(src, splice, &range, false, true)
}

/// Adds the argument `name = value` to the call, after its other
/// arguments (`cube(5)` becomes `cube(5, center = true)`), on a line of
/// its own when they are each on their own line. Refused when the
/// parameter already has an argument.
pub fn add_arg(src: &str, range: Range<usize>, name: &str, value: &Value) -> Result<Edit, String> {
    let doc = Doc::parse(src)?;
    let r = Resolved::new(&doc, &range)?;
    if r.find(ArgKey::Name(name)).is_some() {
        return Err(format!("The parameter {name} already has a value."));
    }
    let (open, close) = doc.parens(r.name).ok_or(super::STALE)?;
    let new = format!("{name} = {}", value.code());
    let splice = match r.args.last() {
        None => Splice::insert(close, new),
        Some(last) => {
            let start = last.name.as_ref().map_or(last.value.span.start, |n| n.span.start) as usize;
            let end = last.value.span.end as usize;
            if src[open..start].contains('\n') {
                Splice::insert(end, format!(",\n{}{new}", indent_of(src, start)))
            } else {
                Splice::insert(end, format!(", {new}"))
            }
        }
    };
    single(src, splice, &range, false, true)
}

fn missing(key: ArgKey) -> String {
    match key {
        ArgKey::Name(name) => format!("The parameter {name} has no value."),
        ArgKey::Position(i) => format!("The call has no argument in position {}.", i + 1),
    }
}

/// A call located in the source, with the parameters its positional
/// arguments fill.
struct Resolved<'d> {
    stmt: &'d Stmt,
    name: &'d Ident,
    args: &'d [Arg],
    children: &'d [Stmt],
    params: Vec<String>,
}

impl<'d> Resolved<'d> {
    fn new(doc: &'d Doc, range: &Range<usize>) -> Result<Self, String> {
        let path = doc.locate_instance(range)?;
        let stmt = path.last().expect("located").stmt;
        let (name, args, children) = call(stmt).ok_or(super::NOT_A_CALL)?;
        let params = params_of(doc, &name.name);
        Ok(Resolved { stmt, name, args, children, params })
    }

    /// The parameter argument `i` fills.
    fn param(&self, i: usize) -> Option<String> {
        if let Some(name) = &self.args[i].name {
            return Some(name.name.clone());
        }
        let position = self.args[..i].iter().filter(|a| a.name.is_none()).count();
        self.params.get(position).cloned()
    }

    fn find(&self, key: ArgKey) -> Option<&'d Arg> {
        match key {
            ArgKey::Name(name) => (0..self.args.len()).find(|&i| self.param(i).as_deref() == Some(name)),
            ArgKey::Position(n) => {
                self.args.iter().enumerate().filter(|(_, a)| a.name.is_none()).nth(n).map(|(i, _)| i)
            }
        }
        .map(|i| &self.args[i])
    }
}

/// The positional parameters of module `name`: those of its definition in
/// the file (at the top level first, then anywhere), else the builtin's.
pub(super) fn params_of(doc: &Doc, name: &str) -> Vec<String> {
    fn find<'a>(stmts: &'a [Stmt], name: &str, deep: bool) -> Option<&'a Stmt> {
        for stmt in stmts {
            if matches!(&stmt.kind, StmtKind::Module(m) if m.name.name == name) {
                return Some(stmt);
            }
            if deep {
                if let Some(found) = child_lists(stmt).into_iter().find_map(|l| find(l, name, true)) {
                    return Some(found);
                }
            }
        }
        None
    }
    let stmts = &doc.file.stmts;
    match find(stmts, name, false).or_else(|| find(stmts, name, true)).map(|s| &s.kind) {
        Some(StmtKind::Module(m)) => m.params.iter().map(|p| p.name.name.clone()).collect(),
        _ => builtin_params(name).iter().map(|s| s.to_string()).collect(),
    }
}

/// The argument of a call that fills parameter `param`.
pub(super) fn bound_arg<'a>(doc: &Doc, name: &Ident, args: &'a [Arg], param: &str) -> Option<&'a Arg> {
    let params = params_of(doc, &name.name);
    let mut position = 0;
    for arg in args {
        match &arg.name {
            Some(n) if n.name == param => return Some(arg),
            Some(_) => {}
            None => {
                if params.get(position).is_some_and(|p| p == param) {
                    return Some(arg);
                }
                position += 1;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::test_util::{at, parses, same_solid};
    use super::*;

    #[test]
    fn a_call_lists_its_arguments() {
        let src = "w = 4;\n#cylinder(10, 2, w / 2, center = true, $fn = 32);\n";
        let call = call_at(src, at(src, "#cylinder(10, 2, w / 2, center = true, $fn = 32);", 0)).unwrap();
        assert_eq!(call.name, "cylinder");
        assert!(call.modifiers.highlight);
        let params: Vec<_> = call.args.iter().map(|a| a.param.as_deref()).collect();
        assert_eq!(params, [Some("h"), Some("r1"), Some("r2"), Some("center"), Some("$fn")]);
        assert_eq!(call.args[0].value, ArgValue::Literal(Value::Number(10.0)));
        assert_eq!(call.args[2].value, ArgValue::Expression("w / 2".into()));
        assert_eq!(&src[call.args[3].range.clone()], "center = true");
        assert_eq!(&src[call.parens.clone()], "(10, 2, w / 2, center = true, $fn = 32)");
        assert!(call.children.is_empty() && call.block.is_none());

        let src = "translate([1, -2, 3]) { cube(1); sphere(2); }";
        let call = call_at(src, 0..src.len()).unwrap();
        assert_eq!(call.args[0].value, ArgValue::Literal(Value::Vector(vec![1.0, -2.0, 3.0])));
        assert_eq!(call.args[0].param.as_deref(), Some("v"));
        assert_eq!(call.children.len(), 2);
        assert_eq!(&src[call.block.unwrap()], "{ cube(1); sphere(2); }");
        assert!(call_at("x = 1;", 0..6).is_err());
    }

    #[test]
    fn user_modules_bind_by_their_declared_order() {
        let src = "module peg(h, d = 3) cylinder(h, d = d);\npeg(5, 2);\n";
        let range = at(src, "peg(5, 2);", 0);
        let call = call_at(src, range.clone()).unwrap();
        assert_eq!(call.args[1].param.as_deref(), Some("d"));
        let edit = set_arg(src, range, ArgKey::Name("d"), &Value::Number(4.0)).unwrap();
        assert!(edit.text.ends_with("peg(5, 4);\n"));
        assert_eq!(&edit.text[edit.selection], "peg(5, 4);");
    }

    #[test]
    fn a_literal_is_rewritten_in_place() {
        let src = "// box\ncube([10, 20, 5], center = false); // body\n";
        let range = at(src, "cube([10, 20, 5], center = false);", 0);
        let edit = set_arg(src, range.clone(), ArgKey::Name("center"), &Value::Bool(true)).unwrap();
        assert_eq!(edit.text, "// box\ncube([10, 20, 5], center = true); // body\n");
        assert_eq!(edit.text.chars().nth(edit.cursor - 1), Some('e'));
        let edit = set_arg(src, range.clone(), ArgKey::Position(0), &Value::Number(8.0)).unwrap();
        assert_eq!(edit.text, "// box\ncube(8, center = false); // body\n");
        assert_eq!(&edit.text[edit.selection], "cube(8, center = false);");
        let edit = set_arg_component(src, range, ArgKey::Name("size"), 1, 7.5).unwrap();
        assert_eq!(edit.text, "// box\ncube([10, 7.5, 5], center = false); // body\n");
        same_solid(&edit.text, "cube([10, 7.5, 5]);");
    }

    #[test]
    fn computed_values_are_refused() {
        let src = "width = 10;\ncube(width / 2);\ntranslate([width, 0, 0]) cube(2);\n";
        let err = set_arg(src, at(src, "cube(width / 2);", 0), ArgKey::Name("size"), &Value::Number(3.0)).unwrap_err();
        assert_eq!(err, "The value is computed (width / 2): replace it with a number to change it.");
        let range = at(src, "translate([width, 0, 0]) cube(2);", 0);
        assert!(set_arg_component(src, range.clone(), ArgKey::Name("v"), 0, 1.0).is_err());
        // Its literal components can still change.
        let edit = set_arg_component(src, range, ArgKey::Name("v"), 2, 4.0).unwrap();
        assert!(edit.text.contains("translate([width, 0, 4]) cube(2);"));
        parses(&edit.text);
    }

    #[test]
    fn a_number_becomes_a_vector_for_one_component() {
        let src = "cube(5);";
        let edit = set_arg_component(src, 0..8, ArgKey::Position(0), 1, 2.0).unwrap();
        assert_eq!(edit.text, "cube([5, 2, 5]);");
        let edit = set_arg_component("square(4);", 0..10, ArgKey::Name("size"), 0, 1.0).unwrap();
        assert_eq!(edit.text, "square([1, 4]);");
        assert!(set_arg_component("square(4);", 0..10, ArgKey::Name("size"), 2, 1.0).is_err());
    }

    #[test]
    fn missing_arguments_are_added() {
        let edit = add_arg("cube(5);", 0..8, "center", &Value::Bool(true)).unwrap();
        assert_eq!(edit.text, "cube(5, center = true);");
        assert_eq!(edit.selection, 0..23);
        let edit = add_arg("cube();", 0..7, "center", &Value::Bool(true)).unwrap();
        assert_eq!(edit.text, "cube(center = true);");
        let edit = add_arg("cube(5 /* side */);", 0..19, "center", &Value::Bool(true)).unwrap();
        assert_eq!(edit.text, "cube(5, center = true /* side */);");
        let src = "text(\n    \"Hi\",\n    size = 4\n);";
        let edit = add_arg(src, 0..src.len(), "font", &Value::Str("Arial".into())).unwrap();
        assert_eq!(edit.text, "text(\n    \"Hi\",\n    size = 4,\n    font = \"Arial\"\n);");
        // Through set_arg, by name.
        let edit = set_arg("sphere(2);", 0..10, ArgKey::Name("$fn"), &Value::Number(64.0)).unwrap();
        assert_eq!(edit.text, "sphere(2, $fn = 64);");
        // A parameter given positionally is not added twice.
        assert!(add_arg("cube(5);", 0..8, "size", &Value::Number(1.0)).is_err());
        assert!(set_arg("cube(5);", 0..8, ArgKey::Position(1), &Value::Bool(true)).is_err());
    }

    #[test]
    fn the_selection_must_match_the_source() {
        assert!(set_arg("cube(5);", 0..5, ArgKey::Position(0), &Value::Number(1.0)).is_err());
        assert!(set_arg("cube(5", 0..6, ArgKey::Position(0), &Value::Number(1.0)).is_err());
    }
}
