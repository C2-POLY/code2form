//! The properties panel (C2F-38): the arguments of the selected call, to
//! change with the mouse, written back into the code.
//!
//! The panel lists the parameters of the call's module: a builtin's from a
//! table of what each takes ([`builtin_params`]), a module of the file or
//! of a `use`d library from its definition. A parameter the call gives a
//! literal gets a field that rewrites that literal in place
//! ([`code_edit::set_arg`]); one it leaves out shows its default, greyed,
//! and editing it adds the argument ([`code_edit::add_arg`]); one it
//! computes (`width / 2`) is never overwritten: the panel shows the
//! expression and its value, with a button that replaces it by that
//! number and one that goes to the variable's definition (or opens it in
//! the Customizer). Below come the transforms written around the object
//! (*Placement*) and, for a `cube` or a `cylinder`, its edges rounded or
//! chamfered ([`crate::fillet`]).
//!
//! Every change is one step of the editor's undo history. A number dragged
//! with the mouse (on its field or on its label) shows its value live but
//! writes the code once, when the drag ends: one drag is one undo step and
//! one render, where writing every frame would flood the history and
//! render dozens of times a second.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use c2f_eval::customizer::Literal;
use c2f_lang::ast::{BinaryOp, Element, Expr, ExprKind, File, Stmt, StmtKind, UnaryOp};
use eframe::egui::{self, Button, Color32, ComboBox, DragValue, Id, Label, RichText, Sense, TextEdit};

use super::StudioApp;
use super::workspace::{InspectorTab, tab_strip};
use crate::code_edit::{self, ArgKey, ArgValue, Call, Edit, Transform, Value};
use crate::fillet::{self, CubeEdges, Rims};
use crate::{customizer, theme};

// ---- the parameters of a module ----

/// A parameter the panel lists for a module.
#[derive(Clone, Debug, PartialEq)]
struct Param {
    name: String,
    default: Fallback,
    /// How many components a single number stands for (`cube(5)` is a
    /// cube of 5 by 5 by 5): the field then shows them one by one, and
    /// changing one turns the number into a vector. 0 for a plain number.
    splat: usize,
    /// A color (`color()`'s `c`): the field gets a color picker.
    color: bool,
    /// Left out of the panel while absent when one of these parameters is
    /// written: `cylinder(h, r1 = 2, r2 = 1)` does not show `r = 1`.
    unless: &'static [&'static str],
}

/// What a parameter is worth when the call leaves it out.
#[derive(Clone, Debug, PartialEq)]
enum Fallback {
    /// A literal, shown greyed.
    Value(Value),
    /// An expression of the module's definition (`d = r * 2`), or nothing
    /// (`undef`): a field to type a value in, with this text as its hint.
    Code(String),
    /// Listed only when written (`cylinder`'s `d1`, `text`'s `script`...).
    Hidden,
}

fn param(name: &str, default: Fallback) -> Param {
    Param { name: name.to_string(), default, splat: 0, color: false, unless: &[] }
}

fn number(name: &str, x: f64) -> Param {
    param(name, Fallback::Value(Value::Number(x)))
}

fn boolean(name: &str, b: bool) -> Param {
    param(name, Fallback::Value(Value::Bool(b)))
}

fn string(name: &str, s: &str) -> Param {
    param(name, Fallback::Value(Value::Str(s.to_string())))
}

fn vector(name: &str, v: &[f64]) -> Param {
    param(name, Fallback::Value(Value::Vector(v.to_vec())))
}

fn hidden(name: &str) -> Param {
    param(name, Fallback::Hidden)
}

fn splat(mut p: Param, n: usize) -> Param {
    p.splat = n;
    p
}

fn unless(mut p: Param, others: &'static [&'static str]) -> Param {
    p.unless = others;
    p
}

/// The parameters the panel lists for a builtin, in the order it shows
/// them; `None` for a name that is not one of these builtins.
fn builtin_params(name: &str) -> Option<Vec<Param>> {
    const RADII: &[&str] = &["r1", "r2", "d", "d1", "d2"];
    let fn_ = || number("$fn", 0.0);
    Some(match name {
        "cube" => vec![splat(number("size", 1.0), 3), boolean("center", false)],
        "square" => vec![splat(number("size", 1.0), 2), boolean("center", false)],
        "sphere" | "circle" => vec![unless(number("r", 1.0), &["d"]), hidden("d"), fn_()],
        "cylinder" => {
            let mut params = vec![number("h", 1.0), unless(number("r", 1.0), RADII)];
            params.extend(RADII.iter().map(|n| hidden(n)));
            params.extend([boolean("center", false), fn_()]);
            // In the order of the positional ones: h, r1, r2.
            params.swap(2, 1);
            params.swap(3, 2);
            params
        }
        "translate" => vec![vector("v", &[0.0, 0.0, 0.0])],
        "rotate" => vec![vector("a", &[0.0, 0.0, 0.0]), hidden("v")],
        "scale" => vec![splat(vector("v", &[1.0, 1.0, 1.0]), 3)],
        "mirror" => vec![vector("v", &[1.0, 0.0, 0.0])],
        "resize" => vec![vector("newsize", &[0.0, 0.0, 0.0]), hidden("auto")],
        "color" => vec![Param { color: true, ..param("c", Fallback::Hidden) }, number("alpha", 1.0)],
        "linear_extrude" => vec![
            number("height", 100.0),
            boolean("center", false),
            number("twist", 0.0),
            number("scale", 1.0),
            hidden("slices"),
            hidden("convexity"),
            hidden("$fn"),
        ],
        "rotate_extrude" => vec![number("angle", 360.0), hidden("convexity"), fn_()],
        "text" => vec![
            string("text", ""),
            number("size", 10.0),
            string("font", "Liberation Sans"),
            string("halign", "left"),
            string("valign", "baseline"),
            number("spacing", 1.0),
            hidden("direction"),
            hidden("language"),
            hidden("script"),
            hidden("$fn"),
        ],
        "offset" => vec![unless(number("r", 1.0), &["delta"]), hidden("delta"), hidden("chamfer")],
        _ => return None,
    })
}

/// The parameters of a module as its definition declares them: a literal
/// default shows greyed, another default (or none) as its code.
fn declared_params(src: &str, params: &[c2f_lang::ast::Param]) -> Vec<Param> {
    params
        .iter()
        .map(|p| {
            let default = match &p.default {
                Some(e) => match eval(&Env::default(), e, 0) {
                    Some(v) => Fallback::Value(v),
                    None => Fallback::Code(src.get(e.span.range()).unwrap_or("").to_string()),
                },
                None => Fallback::Code("undef".into()),
            };
            param(&p.name.name, default)
        })
        .collect()
}

/// The module `name` defined at the top of `file` (or in braces there).
fn module_def<'a>(stmts: &'a [Stmt], name: &str) -> Option<&'a c2f_lang::ast::ModuleDef> {
    stmts.iter().rev().find_map(|s| match &s.kind {
        StmtKind::Module(m) if m.name.name == name => Some(m),
        StmtKind::Block(inner) => module_def(inner, name),
        _ => None,
    })
}

/// The parameters of module `name`: a builtin's, a module of the file,
/// or a module of a file it `use`s or `include`s, whose source `library`
/// gives from the path as written. `None` when no definition is found.
fn module_params(src: &str, file: &File, name: &str, library: &dyn Fn(&str) -> Option<String>) -> Option<Vec<Param>> {
    if let Some(m) = module_def(&file.stmts, name) {
        return Some(declared_params(src, &m.params));
    }
    if let Some(params) = builtin_params(name) {
        return Some(params);
    }
    for stmt in &file.stmts {
        let (StmtKind::Use(path) | StmtKind::Include(path)) = &stmt.kind else { continue };
        let Some(text) = library(path) else { continue };
        let Ok(lib) = c2f_lang::parse(&text) else { continue };
        if let Some(m) = module_def(&lib.stmts, name) {
            return Some(declared_params(&text, &m.params));
        }
    }
    None
}

// ---- the fields of a call ----

/// Which argument a field edits; a positional argument is edited by its
/// position, which stays right whatever the editor knows of the module.
#[derive(Clone, Debug, PartialEq)]
enum Key {
    Name(String),
    Position(usize),
}

impl Key {
    fn arg(&self) -> ArgKey<'_> {
        match self {
            Key::Name(name) => ArgKey::Name(name),
            Key::Position(n) => ArgKey::Position(*n),
        }
    }
}

/// A row of the panel: a parameter of the call.
#[derive(Clone, Debug, PartialEq)]
struct Field {
    /// The parameter's name, or `argument 3` for a positional argument
    /// of a module whose parameters are unknown.
    label: String,
    key: Key,
    state: State,
    splat: usize,
    color: bool,
    /// The hint of a field to type a value in: the default's code.
    hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    /// Written as a literal, which the field rewrites.
    Written(Value),
    /// Computed or named: shown, never overwritten.
    Computed(Computed),
    /// Left out: the default (`Undef` when it is code, see
    /// [`Field::hint`]).
    Absent(Value),
}

/// A value the source computes.
#[derive(Clone, Debug, PartialEq)]
struct Computed {
    code: String,
    /// Where the expression is, to replace it by a number.
    range: Range<usize>,
    /// Its value, when the panel can work it out from top-level variables
    /// and the Customizer's values.
    value: Option<Value>,
    /// The variable it names (the first one with a top-level definition)
    /// and the byte of that definition.
    definition: Option<(String, usize)>,
    /// That variable is a Customizer parameter.
    parameter: bool,
}

/// What the panel knows of the source beyond the call: the top-level
/// variables, with the Customizer's values over those it exposes.
#[derive(Default)]
struct Env<'a> {
    exprs: HashMap<&'a str, (&'a Expr, usize)>,
    overrides: HashMap<String, Option<Value>>,
}

impl<'a> Env<'a> {
    fn new(file: &'a File, parameters: &[(String, Literal)]) -> Self {
        fn walk<'a>(stmts: &'a [Stmt], exprs: &mut HashMap<&'a str, (&'a Expr, usize)>) {
            for stmt in stmts {
                match &stmt.kind {
                    // The last assignment wins, as when the file runs.
                    StmtKind::Assign(a) if stmt.span.file == 0 => {
                        exprs.insert(&a.name.name, (&a.value, stmt.span.start as usize));
                    }
                    StmtKind::Block(inner) => walk(inner, exprs),
                    _ => {}
                }
            }
        }
        let mut exprs = HashMap::new();
        walk(&file.stmts, &mut exprs);
        let overrides = parameters.iter().map(|(n, v)| (n.clone(), from_literal(v))).collect();
        Env { exprs, overrides }
    }
}

fn from_literal(literal: &Literal) -> Option<Value> {
    Some(match literal {
        Literal::Undef => Value::Undef,
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Number(n) => Value::Number(*n),
        Literal::Str(s) => Value::Str(s.clone()),
        Literal::Vector(items) => Value::Vector(
            items.iter().map(|i| if let Literal::Number(n) = i { Some(*n) } else { None }).collect::<Option<_>>()?,
        ),
        Literal::Range { .. } => return None,
    })
}

/// The value of an expression of numbers, booleans, strings and vectors of
/// numbers, its variables taken from `env`: enough for the sizes a design
/// computes (`width / 2`, `[w, d, wall * 2]`, `sqrt(a)`); `None` beyond.
fn eval(env: &Env, expr: &Expr, depth: usize) -> Option<Value> {
    use Value::*;
    if depth > 32 {
        return None;
    }
    let num = |e: &Expr| match eval(env, e, depth + 1)? {
        Number(n) => Some(n),
        _ => None,
    };
    Some(match &expr.kind {
        ExprKind::Undef => Undef,
        ExprKind::Bool(b) => Bool(*b),
        ExprKind::Number(n) => Number(*n),
        ExprKind::Str(s) => Str(s.clone()),
        ExprKind::Paren(inner) => eval(env, inner, depth + 1)?,
        ExprKind::Ident(name) => match env.overrides.get(name) {
            Some(value) => value.clone()?,
            None => eval(env, env.exprs.get(name.as_str())?.0, depth + 1)?,
        },
        ExprKind::Vector(elements) => Vector(
            elements
                .iter()
                .map(|e| match e {
                    Element::Expr(e) => num(e),
                    Element::Comp(_) => None,
                })
                .collect::<Option<_>>()?,
        ),
        ExprKind::Unary { op, operand } => match (op, eval(env, operand, depth + 1)?) {
            (UnaryOp::Plus, v @ (Number(_) | Vector(_))) => v,
            (UnaryOp::Neg, Number(n)) => Number(-n),
            (UnaryOp::Neg, Vector(v)) => Vector(v.iter().map(|x| -x).collect()),
            (UnaryOp::Not, Bool(b)) => Bool(!b),
            _ => return None,
        },
        ExprKind::Binary { op, lhs, rhs } => {
            let (a, b) = (eval(env, lhs, depth + 1)?, eval(env, rhs, depth + 1)?);
            binary(*op, a, b)?
        }
        ExprKind::Ternary { cond, then, otherwise } => match eval(env, cond, depth + 1)? {
            Bool(true) => eval(env, then, depth + 1)?,
            Bool(false) => eval(env, otherwise, depth + 1)?,
            _ => return None,
        },
        ExprKind::Index { target, index } => match (eval(env, target, depth + 1)?, num(index)?) {
            (Vector(v), i) if i >= 0.0 => Number(*v.get(i as usize)?),
            _ => return None,
        },
        ExprKind::Member { target, name } => match eval(env, target, depth + 1)? {
            Vector(v) => Number(*v.get(["x", "y", "z"].iter().position(|n| *n == name.name)?)?),
            _ => return None,
        },
        ExprKind::Call { callee, args } => {
            let ExprKind::Ident(name) = &callee.kind else { return None };
            let xs: Vec<f64> = args.iter().map(|a| num(&a.value)).collect::<Option<_>>()?;
            let one = || (xs.len() == 1).then(|| xs[0]);
            Number(match name.as_str() {
                "sqrt" => one()?.sqrt(),
                "abs" => one()?.abs(),
                "floor" => one()?.floor(),
                "ceil" => one()?.ceil(),
                "round" => one()?.round(),
                "sin" => one()?.to_radians().sin(),
                "cos" => one()?.to_radians().cos(),
                "tan" => one()?.to_radians().tan(),
                "pow" if xs.len() == 2 => xs[0].powf(xs[1]),
                "min" if !xs.is_empty() => xs.iter().copied().fold(f64::INFINITY, f64::min),
                "max" if !xs.is_empty() => xs.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                _ => return None,
            })
        }
        _ => return None,
    })
}

fn binary(op: BinaryOp, a: Value, b: Value) -> Option<Value> {
    use Value::*;
    let zip = |v: &[f64], w: &[f64], f: fn(f64, f64) -> f64| -> Option<Value> {
        (v.len() == w.len()).then(|| Vector(v.iter().zip(w).map(|(x, y)| f(*x, *y)).collect()))
    };
    Some(match (op, &a, &b) {
        (BinaryOp::Add, Number(x), Number(y)) => Number(x + y),
        (BinaryOp::Sub, Number(x), Number(y)) => Number(x - y),
        (BinaryOp::Mul, Number(x), Number(y)) => Number(x * y),
        (BinaryOp::Div, Number(x), Number(y)) => Number(x / y),
        (BinaryOp::Mod, Number(x), Number(y)) => Number(x % y),
        (BinaryOp::Pow, Number(x), Number(y)) => Number(x.powf(*y)),
        (BinaryOp::Add, Vector(v), Vector(w)) => zip(v, w, |x, y| x + y)?,
        (BinaryOp::Sub, Vector(v), Vector(w)) => zip(v, w, |x, y| x - y)?,
        (BinaryOp::Mul, Vector(v), Number(k)) | (BinaryOp::Mul, Number(k), Vector(v)) => {
            Vector(v.iter().map(|x| x * k).collect())
        }
        (BinaryOp::Div, Vector(v), Number(k)) => Vector(v.iter().map(|x| x / k).collect()),
        (BinaryOp::Lt, Number(x), Number(y)) => Bool(x < y),
        (BinaryOp::Le, Number(x), Number(y)) => Bool(x <= y),
        (BinaryOp::Gt, Number(x), Number(y)) => Bool(x > y),
        (BinaryOp::Ge, Number(x), Number(y)) => Bool(x >= y),
        (BinaryOp::Eq, _, _) => Bool(a == b),
        (BinaryOp::Ne, _, _) => Bool(a != b),
        (BinaryOp::And, Bool(x), Bool(y)) => Bool(*x && *y),
        (BinaryOp::Or, Bool(x), Bool(y)) => Bool(*x || *y),
        _ => return None,
    })
}

/// The variables an expression reads, in order, function names aside.
fn names(expr: &Expr, out: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::Ident(name) => out.push(name.clone()),
        ExprKind::Paren(e) | ExprKind::Unary { operand: e, .. } => names(e, out),
        ExprKind::Binary { lhs, rhs, .. } => {
            names(lhs, out);
            names(rhs, out);
        }
        ExprKind::Ternary { cond, then, otherwise } => {
            for e in [cond, then, otherwise] {
                names(e, out);
            }
        }
        ExprKind::Vector(elements) => {
            for e in elements {
                if let Element::Expr(e) = e {
                    names(e, out);
                }
            }
        }
        ExprKind::Index { target, index } => {
            names(target, out);
            names(index, out);
        }
        ExprKind::Member { target, .. } => names(target, out),
        ExprKind::Call { args, .. } => args.iter().for_each(|a| names(&a.value, out)),
        _ => {}
    }
}

/// The expression of argument `value_range`, parsed again from the file.
fn expr_at<'f>(file: &'f File, range: &Range<usize>) -> Option<&'f Expr> {
    fn in_expr<'f>(e: &'f Expr, range: &Range<usize>) -> Option<&'f Expr> {
        (e.span.range() == *range).then_some(e)
    }
    fn in_stmts<'f>(stmts: &'f [Stmt], range: &Range<usize>) -> Option<&'f Expr> {
        for stmt in stmts {
            let r = stmt.span.range();
            if stmt.span.file != 0 || r.start > range.start || r.end < range.end {
                continue;
            }
            let found = match &stmt.kind {
                StmtKind::Block(inner) => in_stmts(inner, range),
                StmtKind::Module(m) => in_stmts(std::slice::from_ref(&*m.body), range),
                StmtKind::Instance(i) => match &i.kind {
                    c2f_lang::ast::InstanceKind::Call { args, children, .. } => {
                        args.iter().find_map(|a| in_expr(&a.value, range)).or_else(|| in_stmts(children, range))
                    }
                    c2f_lang::ast::InstanceKind::If { then, otherwise, .. } => {
                        in_stmts(then, range).or_else(|| otherwise.as_deref().and_then(|o| in_stmts(o, range)))
                    }
                },
                _ => None,
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }
    in_stmts(&file.stmts, range)
}

/// The rows of the panel for `call`: its module's parameters in order,
/// each with its argument or its default, then the arguments the module
/// does not declare (`$fn`, or all of them when it is unknown).
fn fields(call: &Call, params: &[Param]) -> Vec<Field> {
    // The parameter each argument fills, and its key.
    let mut position = 0;
    let args: Vec<(Option<String>, Key)> = call
        .args
        .iter()
        .map(|arg| match &arg.name {
            Some(name) => (Some(name.clone()), Key::Name(name.clone())),
            None => {
                let p = arg.param.clone().or_else(|| params.get(position).map(|p| p.name.clone()));
                position += 1;
                (p, Key::Position(position - 1))
            }
        })
        .collect();
    let written = |name: &str| args.iter().any(|(p, _)| p.as_deref() == Some(name));
    let mut used = vec![false; args.len()];
    let mut out = Vec::new();
    for p in params {
        let found = args.iter().position(|(name, _)| name.as_deref() == Some(p.name.as_str()));
        let state = match found {
            Some(i) => {
                used[i] = true;
                written_state(&call.args[i].value, call.args[i].value_range.clone())
            }
            None if p.unless.iter().any(|o| written(o)) => continue,
            None => match &p.default {
                Fallback::Value(v) => State::Absent(v.clone()),
                Fallback::Code(_) => State::Absent(Value::Undef),
                Fallback::Hidden => continue,
            },
        };
        let key = found.map_or_else(|| Key::Name(p.name.clone()), |i| args[i].1.clone());
        let hint = match &p.default {
            Fallback::Code(code) => Some(code.clone()),
            _ => None,
        };
        out.push(Field { label: p.name.clone(), key, state, splat: p.splat, color: p.color, hint });
    }
    for (i, (name, key)) in args.iter().enumerate() {
        if used[i] {
            continue;
        }
        let label = match (name, key) {
            (Some(name), _) => name.clone(),
            (None, Key::Position(n)) => format!("argument {}", n + 1),
            (None, Key::Name(n)) => n.clone(),
        };
        let state = written_state(&call.args[i].value, call.args[i].value_range.clone());
        out.push(Field { label, key: key.clone(), state, splat: 0, color: false, hint: None });
    }
    out
}

fn written_state(value: &ArgValue, range: Range<usize>) -> State {
    match value {
        ArgValue::Literal(v) => State::Written(v.clone()),
        ArgValue::Expression(code) => State::Computed(Computed {
            code: code.clone(),
            range,
            value: None,
            definition: None,
            parameter: false,
        }),
    }
}

/// Works out what the panel can of each computed field: its value and the
/// variable it names.
fn resolve(fields: &mut [Field], file: &File, env: &Env) {
    for field in fields {
        let State::Computed(c) = &mut field.state else { continue };
        let Some(expr) = expr_at(file, &c.range) else { continue };
        c.value = eval(env, expr, 0);
        let mut read = Vec::new();
        names(expr, &mut read);
        c.definition = read.into_iter().find_map(|n| env.exprs.get(n.as_str()).map(|(_, at)| (n.clone(), *at)));
        c.parameter = c.definition.as_ref().is_some_and(|(n, _)| env.overrides.contains_key(n));
    }
}

// ---- edits ----

/// A change made in a field.
#[derive(Clone, Debug, PartialEq)]
enum Change {
    /// The whole value.
    Whole(Value),
    /// Component `i` of a vector (or of a number standing for one).
    Component(usize, f64),
}

/// The edit of the code a change of `field` makes, on the call at `range`.
fn edit_for(src: &str, range: Range<usize>, field: &Field, change: Change) -> Result<Edit, String> {
    match (&field.state, change) {
        (_, Change::Whole(v)) => code_edit::set_arg(src, range, field.key.arg(), &v),
        (State::Absent(default), Change::Component(i, x)) => {
            let mut v = match default {
                Value::Number(n) => vec![*n; field.splat.max(1)],
                Value::Vector(v) => v.clone(),
                _ => vec![0.0; field.splat.max(1)],
            };
            *v.get_mut(i).ok_or("This vector has no such component.")? = x;
            code_edit::set_arg(src, range, field.key.arg(), &Value::Vector(v))
        }
        (_, Change::Component(i, x)) => code_edit::set_arg_component(src, range, field.key.arg(), i, x),
    }
}

/// Replaces a computed argument by the number it is worth, keeping the
/// call at `range` selected.
fn replace_with_number(src: &str, range: Range<usize>, computed: &Computed) -> Result<Edit, String> {
    let value = computed.value.as_ref().filter(|v| matches!(v, Value::Number(_) | Value::Vector(_)));
    let value = value.ok_or("The value of this expression is not known here.")?;
    let at = computed.range.clone();
    if src.get(at.clone()) != Some(computed.code.as_str()) || at.start < range.start || at.end > range.end {
        return Err("The selection no longer matches the code: select the object again.".into());
    }
    let code = value.code();
    let text = format!("{}{code}{}", &src[..at.start], &src[at.end..]);
    let end = range.end + code.len() - at.len();
    let cursor = text[..at.start + code.len()].chars().count();
    Ok(Edit { text, cursor, selection: range.start..end })
}

/// The dimensions of a `cube` or a `cylinder` written as numbers, which
/// the edges section rounds or chamfers.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Primitive {
    Cube { size: [f64; 3], center: bool },
    Cylinder { h: f64, r1: f64, r2: f64, center: bool },
}

/// The primitive the call makes, when it is a `cube` or a `cylinder`;
/// refused (in English, for the panel) when a dimension is computed.
fn primitive(name: &str, fields: &[Field]) -> Option<Result<Primitive, String>> {
    if name != "cube" && name != "cylinder" {
        return None;
    }
    let get = |label: &str| -> Result<Option<Value>, String> {
        match fields.iter().find(|f| f.label == label).map(|f| &f.state) {
            None => Ok(None),
            Some(State::Written(v) | State::Absent(v)) => Ok(Some(v.clone())),
            Some(State::Computed(_)) => Err(format!("Rounding the edges needs {label} written as a number.")),
        }
    };
    let num = |label: &str| -> Result<Option<f64>, String> {
        match get(label)? {
            None => Ok(None),
            Some(Value::Number(n)) => Ok(Some(n)),
            Some(_) => Err(format!("Rounding the edges needs {label} written as a number.")),
        }
    };
    let center = || -> Result<bool, String> {
        match get("center")? {
            None | Some(Value::Bool(false)) => Ok(false),
            Some(Value::Bool(true)) => Ok(true),
            Some(_) => Err("Rounding the edges needs center written as true or false.".into()),
        }
    };
    let result = (|| {
        if name == "cube" {
            let size = match get("size")? {
                None => [1.0; 3],
                Some(Value::Number(n)) => [n; 3],
                Some(Value::Vector(v)) if v.len() == 3 => [v[0], v[1], v[2]],
                Some(_) => return Err("Rounding the edges needs a size of three numbers.".into()),
            };
            return Ok(Primitive::Cube { size, center: center()? });
        }
        let (r, d) = (num("r")?, num("d")?);
        let radius = |ri: &str, di: &str| -> Result<f64, String> {
            Ok(match (num(di)?, num(ri)?) {
                (Some(d), _) => d / 2.0,
                (None, Some(r)) => r,
                _ => d.map_or(r.unwrap_or(1.0), |d| d / 2.0),
            })
        };
        Ok(Primitive::Cylinder {
            h: num("h")?.unwrap_or(1.0),
            r1: radius("r1", "d1")?,
            r2: radius("r2", "d2")?,
            center: center()?,
        })
    })();
    Some(result)
}

/// How the edges section rounds or chamfers.
#[derive(Clone, Copy, Debug, PartialEq)]
struct EdgeSettings {
    fillet: bool,
    size: f64,
    cube: CubeEdges,
    rims: Rims,
}

impl Default for EdgeSettings {
    fn default() -> Self {
        EdgeSettings { fillet: true, size: 1.0, cube: CubeEdges::All, rims: Rims::Both }
    }
}

/// The library call that replaces the primitive, with the special
/// variables the call set (`$fn = 64`) passed on.
fn edge_call(primitive: Primitive, settings: EdgeSettings, specials: &[String]) -> Result<String, String> {
    let EdgeSettings { fillet: round, size, cube, rims } = settings;
    let mut call = match primitive {
        Primitive::Cube { size: s, center } if round => fillet::rounded_cube(s, center, size, cube)?,
        Primitive::Cube { size: s, center } => fillet::chamfered_cube(s, center, size, cube)?,
        Primitive::Cylinder { h, r1, r2, center } if round => fillet::rounded_cylinder(h, r1, r2, center, size, rims)?,
        Primitive::Cylinder { h, r1, r2, center } => fillet::chamfered_cylinder(h, r1, r2, center, size, rims)?,
    };
    if !specials.is_empty() {
        call.pop();
        call = format!("{call}, {})", specials.join(", "));
    }
    Ok(call)
}

/// The code with the call replaced by `replacement` (and the library
/// `use`d), the cursor (a char index) and the new statement's range.
fn edges_edit(src: &str, call: &Call, replacement: &str) -> (String, usize, Range<usize>) {
    let old = call.name_range.start..call.parens.end;
    let text = fillet::apply(src, old.clone(), replacement);
    let grown = replacement.len() as isize - old.len() as isize;
    // What `apply` put on top: the `use` line, or nothing.
    let shift = (text.len() as isize - src.len() as isize - grown) as usize;
    let start = call.statement.start + shift;
    let end = (call.statement.end as isize + shift as isize + grown) as usize;
    let cursor = text[..start].chars().count();
    (text, cursor, start..end)
}

// ---- the model the panel shows ----

/// Everything the panel shows of one selected call.
#[derive(Clone, Debug)]
struct Model {
    call: Call,
    fields: Vec<Field>,
    /// The parameters of its module are unknown (a module not found).
    unknown: bool,
    placement: Vec<(Transform, Result<[f64; 3], String>)>,
    edges: Option<Result<Primitive, String>>,
    /// Its `$...` arguments, as written, kept by the edges section.
    specials: Vec<String>,
}

fn build_model(
    src: &str,
    range: Range<usize>,
    parameters: &[(String, Literal)],
    library: &dyn Fn(&str) -> Option<String>,
) -> Result<Model, String> {
    let call = code_edit::call_at(src, range.clone())?;
    let file = c2f_lang::parse(src).map_err(|e| e.message)?;
    let params = module_params(src, &file, &call.name, library);
    let mut fields = fields(&call, params.as_deref().unwrap_or(&[]));
    let env = Env::new(&file, parameters);
    resolve(&mut fields, &file, &env);
    let placement = [Transform::Translate, Transform::Rotate, Transform::Scale]
        .into_iter()
        .map(|kind| (kind, code_edit::transform_of(src, range.clone(), kind)))
        .collect();
    let specials = call
        .args
        .iter()
        .filter(|a| a.name.as_deref().is_some_and(|n| n.starts_with('$')))
        .map(|a| src[a.range.clone()].to_string())
        .collect();
    let edges = primitive(&call.name, &fields);
    Ok(Model { unknown: params.is_none(), call, fields, placement, edges, specials })
}

// ---- drawing ----

/// What the user did in the panel, for the app to apply.
#[derive(Clone, Debug, PartialEq)]
enum Action {
    Edit(usize, Change),
    Place(Transform, [f64; 3]),
    Replace(usize),
    GoTo(usize),
    OpenParameter(String),
    Edges(Result<String, String>),
    /// A value typed that does not read as one.
    Invalid,
}

/// A number being dragged: shown live, written when the drag ends.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Drag {
    field: Id,
    value: f64,
}

fn drag_id() -> Id {
    Id::new("properties-drag")
}

/// Settles one frame of a number field: `shown` is what it showed, `value`
/// what it holds after the user's input. While dragging the value is kept
/// in `drag`, not written; the drag's end writes it once (when it moved
/// off `current`, the source's value); a value typed or stepped with the
/// keys is written at once. Returns the value to write.
fn settle(
    drag: &mut Option<Drag>,
    field: Id,
    current: f64,
    shown: f64,
    value: f64,
    dragging: bool,
    released: bool,
) -> Option<f64> {
    let mine = drag.is_some_and(|d| d.field == field);
    if dragging {
        if value != shown || mine {
            *drag = Some(Drag { field, value });
        }
        return None;
    }
    if released && mine {
        let value = drag.take().map(|d| d.value)?;
        return (value != current).then_some(value);
    }
    (value != shown).then_some(value)
}

/// How fast a number moves under the mouse, and its decimals.
fn speed(label: &str) -> (f64, usize) {
    match label {
        l if l.starts_with('$') => (0.5, 0),
        "a" | "angle" | "twist" => (1.0, 3),
        _ => (0.1, 3),
    }
}

/// A number field: a drag value whose label (when given) also drags it.
/// Returns the value to write.
fn number_field(ui: &mut egui::Ui, id: Id, current: f64, label: &str, prefix: &str, width: f32) -> Option<f64> {
    let (step, decimals) = speed(label);
    let mut drag: Option<Drag> = ui.data(|d| d.get_temp(drag_id())).flatten();
    let shown = drag.filter(|d| d.field == id).map_or(current, |d| d.value);
    let mut value = shown;
    // Shown as the code writes it: `12`, not `12.0`.
    let field = DragValue::new(&mut value)
        .speed(step)
        .max_decimals(decimals)
        .custom_formatter(|n, _| code_edit::number(n))
        .update_while_editing(false)
        .prefix(prefix);
    let response = ui.add_sized([width, 20.0], field);
    #[cfg(test)]
    ui.data_mut(|d| d.insert_temp(id.with("rect"), response.rect));
    let commit = settle(&mut drag, id, current, shown, value, response.dragged(), response.drag_stopped());
    ui.data_mut(|d| d.insert_temp(drag_id(), drag));
    commit
}

/// The width of the labels before the fields.
const LABEL_WIDTH: f32 = 64.0;

/// A row's label, in a column of its own; `drag` makes it drag the number
/// next to it, as in Fusion.
fn row_label(ui: &mut egui::Ui, text: &str, color: Color32, drag: bool) -> egui::Response {
    let sense = if drag { Sense::click_and_drag() } else { Sense::hover() };
    let height = ui.spacing().interact_size.y.max(20.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(LABEL_WIDTH, height), sense);
    let galley = egui::WidgetText::from(RichText::new(text).color(color)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        LABEL_WIDTH,
        egui::TextStyle::Body,
    );
    let at = egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(at, galley, color);
    if drag { response.on_hover_cursor(egui::CursorIcon::ResizeHorizontal) } else { response }
}

/// Follows a drag of `label` over the number of field `id`; returns the
/// value to write.
fn label_drag(ui: &mut egui::Ui, label: &egui::Response, id: Id, current: f64, name: &str) -> Option<f64> {
    if !label.dragged() && !label.drag_stopped() {
        return None;
    }
    let (step, _) = speed(name);
    let mut drag: Option<Drag> = ui.data(|d| d.get_temp(drag_id())).flatten();
    let shown = drag.filter(|d| d.field == id).map_or(current, |d| d.value);
    let value = shown + label.drag_delta().x as f64 * step;
    let value = if name.starts_with('$') { value.round() } else { value };
    let commit = settle(&mut drag, id, current, shown, value, label.dragged(), label.drag_stopped());
    ui.data_mut(|d| d.insert_temp(drag_id(), drag));
    commit
}

/// A text field written when it loses focus (Enter or a click away), not
/// as each key is typed.
fn text_field(ui: &mut egui::Ui, id: Id, current: &str, hint: &str) -> Option<String> {
    let mut buffer: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| current.to_string());
    let response =
        ui.add(TextEdit::singleline(&mut buffer).id(id.with("edit")).hint_text(hint).desired_width(ui.available_width()));
    if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
        return (!escaped && buffer != current).then_some(buffer);
    }
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, buffer));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    None
}

/// A color button; while its picker is dragged the color stays a draft,
/// written once the mouse button is up.
fn color_button(ui: &mut egui::Ui, id: Id, current: [f32; 4]) -> Option<[f32; 4]> {
    let draft_id = id.with("color");
    let draft: Option<[f32; 4]> = ui.data(|d| d.get_temp(draft_id));
    let mut color = draft.unwrap_or(current);
    let response = ui.color_edit_button_rgba_unmultiplied(&mut color);
    let pending = response.changed() || draft.is_some();
    if !pending {
        return None;
    }
    if ui.input(|i| i.pointer.any_down()) {
        ui.data_mut(|d| d.insert_temp(draft_id, color));
        return None;
    }
    ui.data_mut(|d| d.remove::<[f32; 4]>(draft_id));
    (color != current).then_some(color)
}

/// The color a value writes (`[1, 0, 0]`, `"#ff0000"`), white when it is a
/// name the panel cannot read.
fn color_of(value: &Value) -> [f32; 4] {
    match value {
        Value::Vector(v) if v.len() >= 3 => {
            [v[0] as f32, v[1] as f32, v[2] as f32, v.get(3).copied().unwrap_or(1.0) as f32]
        }
        Value::Str(s) => {
            let hex = s.strip_prefix('#').unwrap_or("");
            let byte = |i: usize| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
            match (hex.len(), byte(0), byte(2), byte(4)) {
                (6 | 8, Some(r), Some(g), Some(b)) => {
                    let a = byte(6).unwrap_or(255);
                    [r, g, b, a].map(|c| c as f32 / 255.0)
                }
                _ => [1.0; 4],
            }
        }
        _ => [1.0; 4],
    }
}

/// A picked color written in the style of the value it replaces: a vector
/// stays a vector, anything else becomes `"#rrggbb"`.
fn color_value(old: &Value, color: [f32; 4]) -> Value {
    let round = |c: f32| (c as f64 * 1000.0).round() / 1000.0;
    match old {
        Value::Vector(v) => {
            let n = if v.len() == 4 || color[3] < 1.0 { 4 } else { 3 };
            Value::Vector(color[..n].iter().map(|c| round(*c)).collect())
        }
        _ => {
            let c = Color32::from_rgba_unmultiplied(
                (color[0] * 255.0).round() as u8,
                (color[1] * 255.0).round() as u8,
                (color[2] * 255.0).round() as u8,
                (color[3] * 255.0).round() as u8,
            );
            let mut hex = format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b());
            if c.a() < 255 {
                hex += &format!("{:02x}", c.a());
            }
            Value::Str(hex)
        }
    }
}

/// A value typed as code in an empty field: a number, `true`, a
/// `"string"`, a vector...
fn typed_value(text: &str) -> Option<Value> {
    let file = c2f_lang::parse(&format!("_ = {text};")).ok()?;
    let StmtKind::Assign(a) = &file.stmts.first()?.kind else { return None };
    eval(&Env::default(), &a.value, 0).filter(|v| *v != Value::Undef)
}

/// A short text of a value: `10`, `[10, 20, 5]`, `"M3"`.
fn shown(value: &Value) -> String {
    value.code()
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).strong().size(13.0).color(theme::palette().text));
    ui.add_space(2.0);
}

/// Draws the panel's sections for `model`; returns what the user did.
fn show_model(ui: &mut egui::Ui, model: &Model) -> Vec<Action> {
    let mut actions = Vec::new();
    let call = &model.call;
    ui.horizontal(|ui| {
        ui.label(RichText::new(&call.name).monospace().strong().size(14.0).color(theme::palette().code_module));
    });
    if model.unknown && !call.args.is_empty() {
        ui.label(
            RichText::new("The definition of this module is not found: only its written arguments are listed.")
                .size(11.5)
                .color(theme::palette().text_faint),
        );
    }
    let base = Id::new(("properties", call.statement.start, &call.name));

    if !model.fields.is_empty() {
        section(ui, "Parameters");
        for (i, field) in model.fields.iter().enumerate() {
            ui.horizontal_top(|ui| field_row(ui, base.with(("field", &field.label)), i, field, &mut actions));
            ui.add_space(3.0);
        }
    } else if model.unknown {
        ui.label(RichText::new("No arguments.").color(theme::palette().text_dim));
    }

    section(ui, "Placement");
    for (kind, value) in &model.placement {
        let name = match kind {
            Transform::Translate => "Move",
            Transform::Rotate => "Rotate",
            Transform::Scale => "Scale",
        };
        ui.horizontal_top(|ui| {
            row_label(ui, name, theme::palette().text_dim, false);
            match value {
                Ok(v) => {
                    let width = (ui.available_width() / 3.0 - 4.0).clamp(40.0, 80.0);
                    for (c, prefix) in ["x ", "y ", "z "].iter().enumerate() {
                        let id = base.with(("place", name, c));
                        let label = if *kind == Transform::Rotate { "a" } else { "" };
                        if let Some(x) = number_field(ui, id, v[c], label, prefix, width) {
                            let mut new = *v;
                            new[c] = x;
                            actions.push(Action::Place(*kind, new));
                        }
                    }
                }
                Err(why) => {
                    ui.add(Label::new(RichText::new(why).size(11.5).color(theme::palette().text_faint)).wrap());
                }
            }
        });
        ui.add_space(3.0);
    }

    if let Some(edges) = &model.edges {
        section(ui, "Edges");
        edges_section(ui, base, edges, &model.specials, &mut actions);
    }

    // A drag whose widget went away: forget it.
    let stale = ui.data(|d| d.get_temp::<Option<Drag>>(drag_id())).flatten().is_some()
        && !ui.input(|i| i.pointer.any_down() || i.pointer.any_released());
    if stale {
        ui.data_mut(|d| d.insert_temp::<Option<Drag>>(drag_id(), None));
    }
    actions
}

fn field_row(ui: &mut egui::Ui, id: Id, i: usize, field: &Field, actions: &mut Vec<Action>) {
    let absent = matches!(field.state, State::Absent(_));
    let label_color = if absent { theme::palette().text_faint } else { theme::palette().text_dim };
    let label = |ui: &mut egui::Ui, drag| row_label(ui, &field.label, label_color, drag);
    let edit = |change| Action::Edit(i, change);
    let hover = |r: egui::Response| {
        if absent {
            let _ = r.on_hover_text("Not written: this is the default. Change it to add the argument to the call.");
        }
    };
    match &field.state {
        State::Computed(c) => {
            label(ui, false);
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&c.code).monospace().color(theme::palette().code_text));
                    if let Some(v) = &c.value {
                        ui.label(RichText::new(format!("= {}", shown(v))).color(theme::palette().text_dim));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    let numeric = matches!(c.value, Some(Value::Number(_) | Value::Vector(_)));
                    let replace = ui
                        .add_enabled(numeric, Button::new(RichText::new("Replace with a number").size(11.5)).small())
                        .on_hover_text("Write the value in place of the expression, to change it here")
                        .on_disabled_hover_text("Its value is not known here");
                    if replace.clicked() {
                        actions.push(Action::Replace(i));
                    }
                    if let Some((name, at)) = &c.definition {
                        let go = ui
                            .add(Button::new(RichText::new("Go to definition").size(11.5)).small())
                            .on_hover_text(format!("Put the editor on {name} ="));
                        if go.clicked() {
                            actions.push(Action::GoTo(*at));
                        }
                        if c.parameter {
                            let open = ui
                                .add(Button::new(RichText::new("Customizer").size(11.5)).small())
                                .on_hover_text(format!("{name} is a parameter: open it in the Customizer"));
                            if open.clicked() {
                                actions.push(Action::OpenParameter(name.clone()));
                            }
                        }
                    }
                });
            });
        }
        State::Written(Value::Number(n)) | State::Absent(Value::Number(n)) if field.splat > 0 => {
            hover(label(ui, false));
            components(ui, id, &vec![*n; field.splat], &field.label, absent, |c, x| actions.push(edit(Change::Component(c, x))));
        }
        State::Written(Value::Number(n)) | State::Absent(Value::Number(n)) => {
            let label = label(ui, true);
            let dragged = label_drag(ui, &label, id, *n, &field.label);
            hover(label);
            let width = ui.available_width().clamp(60.0, 200.0);
            let typed = dimmed(ui, absent, |ui| number_field(ui, id, *n, &field.label, "", width));
            if let Some(x) = dragged.or(typed) {
                actions.push(edit(Change::Whole(Value::Number(x))));
            }
        }
        State::Written(Value::Vector(v)) | State::Absent(Value::Vector(v)) => {
            hover(label(ui, false));
            ui.horizontal(|ui| {
                if field.color {
                    if let Some(c) = color_button(ui, id, color_of(&Value::Vector(v.clone()))) {
                        actions.push(edit(Change::Whole(color_value(&Value::Vector(v.clone()), c))));
                    }
                }
                components(ui, id, v, &field.label, absent, |c, x| actions.push(edit(Change::Component(c, x))));
            });
        }
        State::Written(Value::Bool(b)) | State::Absent(Value::Bool(b)) => {
            hover(label(ui, false));
            let mut on = *b;
            if dimmed(ui, absent, |ui| ui.checkbox(&mut on, "")).changed() {
                actions.push(edit(Change::Whole(Value::Bool(on))));
            }
        }
        State::Written(Value::Str(s)) | State::Absent(Value::Str(s)) => {
            hover(label(ui, false));
            ui.horizontal(|ui| {
                if field.color {
                    if let Some(c) = color_button(ui, id, color_of(&Value::Str(s.clone()))) {
                        actions.push(edit(Change::Whole(color_value(&Value::Str(s.clone()), c))));
                    }
                }
                let (current, hint) = if absent { ("", s.as_str()) } else { (s.as_str(), "") };
                if let Some(text) = text_field(ui, id, current, hint) {
                    actions.push(edit(Change::Whole(Value::Str(text))));
                }
            });
        }
        State::Written(Value::Undef) | State::Absent(Value::Undef) => {
            hover(label(ui, false));
            let hint = field.hint.as_deref().unwrap_or("undef");
            if let Some(text) = text_field(ui, id, "", hint) {
                match typed_value(&text) {
                    Some(v) => actions.push(edit(Change::Whole(v))),
                    None => actions.push(Action::Invalid),
                }
            }
        }
    }
}

/// Runs `add` with the text greyed when the value is a default.
fn dimmed<R>(ui: &mut egui::Ui, dim: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        if dim {
            ui.visuals_mut().override_text_color = Some(theme::palette().text_faint);
        }
        add(ui)
    })
    .inner
}

/// A field per component of a vector, `x`, `y`, `z` (`r`, `g`, `b` for
/// `color()`'s).
fn components(ui: &mut egui::Ui, id: Id, v: &[f64], label: &str, dim: bool, mut changed: impl FnMut(usize, f64)) {
    let names = if label == "c" { ["r ", "g ", "b ", "a "] } else { ["x ", "y ", "z ", "w "] };
    dimmed(ui, dim, |ui| {
        ui.horizontal(|ui| {
            let width = (ui.available_width() / v.len().max(1) as f32 - 4.0).clamp(40.0, 80.0);
            for (c, x) in v.iter().enumerate() {
                let prefix = names.get(c).copied().unwrap_or("");
                if let Some(new) = number_field(ui, id.with(c), *x, label, prefix, width) {
                    changed(c, new);
                }
            }
        });
    });
}

/// Where the edges section keeps its settings, and the flag that brings it
/// into view.
const EDGES: &str = "properties-edges";
const REVEAL_EDGES: &str = "properties-edges-reveal";

fn edges_section(
    ui: &mut egui::Ui,
    base: Id,
    edges: &Result<Primitive, String>,
    specials: &[String],
    actions: &mut Vec<Action>,
) {
    let primitive = match edges {
        Ok(p) => *p,
        Err(why) => {
            ui.add(Label::new(RichText::new(why).size(11.5).color(theme::palette().text_faint)).wrap());
            return;
        }
    };
    let id = Id::new(EDGES);
    let mut s: EdgeSettings = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    // Asked from the toolbar: the section comes into view.
    if ui.data_mut(|d| d.remove_temp::<bool>(Id::new(REVEAL_EDGES))).unwrap_or(false) {
        ui.scroll_to_cursor(Some(egui::Align::Min));
    }
    ui.horizontal(|ui| {
        row_label(ui, "Kind", theme::palette().text_dim, false);
        ui.selectable_value(&mut s.fillet, true, "Fillet");
        ui.selectable_value(&mut s.fillet, false, "Chamfer");
    });
    ui.horizontal(|ui| {
        row_label(ui, if s.fillet { "Radius" } else { "Distance" }, theme::palette().text_dim, false);
        let size = DragValue::new(&mut s.size).speed(0.05).range(0.0..=f64::MAX).max_decimals(3);
        ui.add_sized([80.0, 20.0], size.custom_formatter(|n, _| code_edit::number(n)));
    });
    ui.horizontal(|ui| {
        row_label(ui, "Edges", theme::palette().text_dim, false);
        match primitive {
            Primitive::Cube { .. } => {
                let names = [
                    (CubeEdges::All, "All"),
                    (CubeEdges::Vertical, "Vertical"),
                    (CubeEdges::Top, "Top"),
                    (CubeEdges::Bottom, "Bottom"),
                ];
                let current = names.iter().find(|(e, _)| *e == s.cube).map_or("All", |(_, n)| n);
                ComboBox::from_id_salt(base.with("cube-edges")).selected_text(current).show_ui(ui, |ui| {
                    for (e, n) in names {
                        ui.selectable_value(&mut s.cube, e, n);
                    }
                });
            }
            Primitive::Cylinder { .. } => {
                let names = [(Rims::Top, "Top"), (Rims::Bottom, "Bottom"), (Rims::Both, "Both")];
                let current = names.iter().find(|(e, _)| *e == s.rims).map_or("Both", |(_, n)| n);
                ComboBox::from_id_salt(base.with("rims")).selected_text(current).show_ui(ui, |ui| {
                    for (e, n) in names {
                        ui.selectable_value(&mut s.rims, e, n);
                    }
                });
            }
        }
    });
    ui.add_space(4.0);
    let what = if s.fillet { "Round the edges" } else { "Chamfer the edges" };
    let apply = ui.button("Apply").on_hover_text(format!(
        "{what}: the call becomes one of the library's shapes/rounded.c2f, keeping its size"
    ));
    if apply.clicked() {
        actions.push(Action::Edges(edge_call(primitive, s, specials)));
    }
    ui.data_mut(|d| d.insert_temp(id, s));
}

// ---- the panel in the app ----

/// What the Properties tab shows.
enum Content {
    /// Nothing selected.
    Nothing,
    /// The selection comes back with the render under way.
    Updating,
    Call(Arc<Model>, Range<usize>),
    /// A single statement it cannot edit, with its description and why.
    Note(String, String),
    Many(usize),
}

/// The Properties tab: the actions its fields ask for.
fn show_content(ui: &mut egui::Ui, content: &Content) -> Vec<Action> {
    let palette = theme::palette();
    match content {
        Content::Nothing => {
            ui.add_space(4.0);
            let hint = "Select an object in the view or in the browser to change it here.";
            ui.add(Label::new(RichText::new(hint).color(palette.text_dim)).wrap());
        }
        Content::Updating => {
            ui.label(RichText::new("Updating…").color(palette.text_dim));
        }
        Content::Call(model, _) => return show_model(ui, model),
        Content::Note(head, why) => {
            ui.label(RichText::new(head).monospace().color(palette.code_text));
            ui.add_space(4.0);
            ui.add(Label::new(RichText::new(why).color(palette.text_dim)).wrap());
        }
        Content::Many(n) => {
            ui.label(RichText::new(format!("{n} objects selected")).color(palette.text));
            ui.add_space(4.0);
            ui.label(RichText::new("Select a single object to edit its properties.").color(palette.text_dim));
        }
    }
    Vec::new()
}

/// The model cached for the source and the values it was built from.
type Cached = (u64, Arc<Result<Model, String>>);

impl StudioApp {
    /// The inspector, on the right of the Model workspace: the selected
    /// call's arguments in one tab, the file's parameters in the other.
    pub(super) fn inspector_panel(&mut self, ctx: &egui::Context) {
        self.follow_selection();
        let content = match self.panels.inspector {
            InspectorTab::Properties => self.properties_content(ctx),
            InspectorTab::Parameters => Content::Nothing,
        };
        let width = ctx.screen_rect().width();
        let mut actions = Vec::new();
        egui::SidePanel::right("inspector")
            .frame(customizer::frame())
            .resizable(true)
            .default_width(290.0)
            .width_range(240.0..=(width * 0.4).max(260.0))
            .show(ctx, |ui| {
                let tabs = [(InspectorTab::Properties, "Properties"), (InspectorTab::Parameters, "Parameters")];
                tab_strip(ui, &mut self.panels.inspector, &tabs);
                match self.panels.inspector {
                    InspectorTab::Properties => {
                        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                            actions = show_content(ui, &content);
                        });
                    }
                    InspectorTab::Parameters => {
                        if self.doc.customizer.show(ui, &mut false) && self.settings.render_while_typing {
                            self.doc.render_deadline = Some(Instant::now() + super::PARAMETER_DEBOUNCE);
                        }
                    }
                }
            });
        if let Content::Call(model, range) = content {
            self.properties_actions(ctx, &model, range, actions);
        }
    }

    /// Whether the toolbar's Fillet and Chamfer apply: a single cube or
    /// cylinder whose edges the inspector can round; else why not.
    pub(super) fn edges_ready(&self, ctx: &egui::Context) -> Result<(), String> {
        let none = "Select a cube or a cylinder";
        let [key] = self.doc.viewer.selection() else { return Err(none.into()) };
        let (Some(range), Some(_)) = (key.range(), self.selected_ranges()) else { return Err(none.into()) };
        match &*self.properties_model(ctx, range) {
            Ok(Model { edges: Some(Ok(_)), .. }) => Ok(()),
            Ok(Model { edges: Some(Err(why)), .. }) => Err(why.clone()),
            _ => Err("Only a cube or a cylinder has edges to round".into()),
        }
    }

    /// The toolbar's Fillet (`fillet`) or Chamfer: the inspector's edges
    /// section, set to it and scrolled to.
    pub(super) fn ask_edges(&mut self, ctx: &egui::Context, fillet: bool) {
        let id = Id::new(EDGES);
        let mut settings: EdgeSettings = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
        settings.fillet = fillet;
        ctx.data_mut(|d| {
            d.insert_temp(id, settings);
            d.insert_temp(Id::new(REVEAL_EDGES), true);
        });
        self.panels.inspector = InspectorTab::Properties;
    }

    /// What the Properties tab shows of the selection.
    fn properties_content(&mut self, ctx: &egui::Context) -> Content {
        let shown_id = Id::new("properties-shown");
        let selection = self.doc.viewer.selection().to_vec();
        let content = match selection.as_slice() {
            // After an edit the selection comes back with the render: keep
            // the tab meanwhile, so that it does not flicker.
            [] if self.doc.busy() && ctx.data(|d| d.get_temp::<bool>(shown_id)).unwrap_or(false) => Content::Updating,
            [] => Content::Nothing,
            [key] => match (key.range(), self.selected_ranges()) {
                (None, _) => Content::Note(
                    self.doc.viewer.describe(*key).0,
                    "Read-only: written in a library or an included file.".into(),
                ),
                (Some(_), None) => Content::Note(
                    self.doc.viewer.describe(*key).0,
                    "The code changed since the last render: the panel comes back once it renders.".into(),
                ),
                (Some(range), Some(_)) => match &*self.properties_model(ctx, range.clone()) {
                    Ok(model) => Content::Call(Arc::new(model.clone()), range),
                    Err(why) => Content::Note(self.doc.viewer.describe(*key).0, why.clone()),
                },
            },
            keys => Content::Many(keys.len()),
        };
        let shown = !matches!(content, Content::Nothing);
        ctx.data_mut(|d| d.insert_temp(shown_id, shown));
        content
    }

    /// Does what the panel asked in one frame, until an action changes the
    /// code: the others were asked of the old code, at a range and with a
    /// model that no longer match it, and would be refused or misplaced.
    fn properties_actions(&mut self, ctx: &egui::Context, model: &Model, range: Range<usize>, actions: Vec<Action>) {
        let before = self.doc.text.clone();
        for action in actions {
            self.properties_action(ctx, model, range.clone(), action);
            if self.doc.text != before {
                break;
            }
        }
    }

    /// The model of the call at `range`, built once for each source,
    /// selection and set of Customizer values.
    fn properties_model(&self, ctx: &egui::Context, range: Range<usize>) -> Arc<Result<Model, String>> {
        let parameters = self.doc.customizer.parameter_values();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (&self.doc.text, &range, format!("{parameters:?}"), &self.doc.path).hash(&mut hasher);
        let key = hasher.finish();
        let id = Id::new("properties-model");
        if let Some((k, model)) = ctx.data(|d| d.get_temp::<Cached>(id)) {
            if k == key {
                return model;
            }
        }
        let mut folders: Vec<PathBuf> = self.doc.path.iter().filter_map(|p| p.parent().map(|d| d.to_path_buf())).collect();
        folders.extend(self.settings.library_folders.iter().cloned());
        folders.extend(c2f_eval::default_library_paths());
        let library = |path: &str| folders.iter().find_map(|f| std::fs::read_to_string(f.join(path)).ok());
        let model = Arc::new(build_model(&self.doc.text, range, &parameters, &library));
        ctx.data_mut(|d| d.insert_temp::<Cached>(id, (key, model.clone())));
        model
    }

    fn properties_action(&mut self, ctx: &egui::Context, model: &Model, range: Range<usize>, action: Action) {
        let src = self.doc.text.clone();
        match action {
            Action::Edit(i, change) => {
                let result = edit_for(&src, range, &model.fields[i], change);
                self.apply_code_edit(ctx, result, "");
            }
            Action::Place(kind, value) => {
                self.apply_code_edit(ctx, code_edit::set_transform(&src, range, kind, value), "");
            }
            Action::Replace(i) => {
                if let State::Computed(c) = &model.fields[i].state {
                    let result = replace_with_number(&src, range, c);
                    self.apply_code_edit(ctx, result, "");
                }
            }
            Action::GoTo(at) => self.go_to_code(ctx, at),
            Action::OpenParameter(name) => {
                self.panels.inspector = InspectorTab::Parameters;
                self.doc.customizer_open = Some(true);
                self.doc.customizer.reveal(&name);
            }
            Action::Edges(call) => {
                let result = call.map(|call| edges_edit(&src, &model.call, &call));
                let what = if result.as_ref().is_ok_and(|(t, _, _)| t.contains("chamfered_")) { "Chamfered" } else { "Rounded" };
                self.apply_code_edits(ctx, result.map(|(t, c, r)| (t, c, vec![r])), &format!("{what} the edges of {}", model.call.name));
            }
            Action::Invalid => {
                let why = "Enter a value: a number, true or false, a \"string\" or a vector such as [1, 2, 3].";
                self.doc.console.log(crate::console::Level::Warning, why);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(src: &str, text: &str) -> Range<usize> {
        let start = src.find(text).expect("in the source");
        start..start + text.len()
    }

    fn no_library(_: &str) -> Option<String> {
        None
    }

    fn model(src: &str, statement: &str) -> Model {
        build_model(src, at(src, statement), &[], &no_library).expect("a call")
    }

    fn labels(fields: &[Field]) -> Vec<&str> {
        fields.iter().map(|f| f.label.as_str()).collect()
    }

    fn parses(src: &str) {
        if let Err(err) = c2f_lang::parse(src) {
            panic!("{}\n{src}", err.message);
        }
    }

    #[test]
    fn a_cube_lists_its_size_and_center() {
        let m = model("cube([10, 20, 5]);", "cube([10, 20, 5]);");
        assert_eq!(labels(&m.fields), ["size", "center"]);
        assert_eq!(m.fields[0].state, State::Written(Value::Vector(vec![10.0, 20.0, 5.0])));
        assert_eq!(m.fields[0].key, Key::Position(0));
        assert_eq!(m.fields[1].state, State::Absent(Value::Bool(false)));
        assert_eq!(m.fields[1].key, Key::Name("center".into()));
        assert_eq!(m.edges, Some(Ok(Primitive::Cube { size: [10.0, 20.0, 5.0], center: false })));
        assert!(m.placement.iter().all(|(_, v)| v.is_ok()));
    }

    #[test]
    fn a_cylinder_lists_the_radii_it_writes() {
        let src = "w = 4;\ncylinder(10, 2, w / 2, center = true, $fn = 32);\n";
        let m = model(src, "cylinder(10, 2, w / 2, center = true, $fn = 32);");
        // `r` is not shown, as `r1` and `r2` are written.
        assert_eq!(labels(&m.fields), ["h", "r1", "r2", "center", "$fn"]);
        let State::Computed(c) = &m.fields[2].state else { panic!("computed") };
        assert_eq!(c.code, "w / 2");
        assert_eq!(c.value, Some(Value::Number(2.0)));
        assert_eq!(c.definition, Some(("w".into(), 0)));
        assert!(!c.parameter);
        assert_eq!(m.specials, ["$fn = 32"]);
        // A computed radius keeps the edges section away.
        assert!(matches!(m.edges, Some(Err(_))));

        let m = model("cylinder(h = 5, d = 4);", "cylinder(h = 5, d = 4);");
        assert_eq!(labels(&m.fields), ["h", "d", "center", "$fn"]);
        assert_eq!(m.edges, Some(Ok(Primitive::Cylinder { h: 5.0, r1: 2.0, r2: 2.0, center: false })));
        let m = model("cylinder(5);", "cylinder(5);");
        assert_eq!(labels(&m.fields), ["h", "r", "center", "$fn"]);
        assert_eq!(m.fields[1].state, State::Absent(Value::Number(1.0)));
    }

    #[test]
    fn builtins_list_their_parameters() {
        let names = |name: &str| -> Vec<String> {
            builtin_params(name).unwrap().iter().filter(|p| p.default != Fallback::Hidden).map(|p| p.name.clone()).collect()
        };
        assert_eq!(names("sphere"), ["r", "$fn"]);
        assert_eq!(names("translate"), ["v"]);
        assert_eq!(names("rotate"), ["a"]);
        assert_eq!(names("linear_extrude"), ["height", "center", "twist", "scale"]);
        assert_eq!(names("text"), ["text", "size", "font", "halign", "valign", "spacing"]);
        assert!(builtin_params("color").unwrap()[0].color);
        assert!(builtin_params("peg").is_none());
        // Positional order matches the evaluator's.
        let cylinder: Vec<String> = builtin_params("cylinder").unwrap().iter().take(3).map(|p| p.name.clone()).collect();
        assert_eq!(cylinder, ["h", "r1", "r2"]);
        for name in ["cube", "sphere", "translate", "rotate", "color", "linear_extrude", "text"] {
            let ours: Vec<String> = builtin_params(name).unwrap().iter().map(|p| p.name.clone()).collect();
            for (i, p) in code_edit::builtin_params(name).iter().enumerate() {
                assert_eq!(ours[i], *p, "{name}");
            }
        }
    }

    #[test]
    fn user_modules_list_their_declared_parameters() {
        let src = "module peg(h, d = 3, tip = d * 2) cylinder(h, d = d);\npeg(5);\n";
        let m = model(src, "peg(5);");
        assert_eq!(labels(&m.fields), ["h", "d", "tip"]);
        assert_eq!(m.fields[0].state, State::Written(Value::Number(5.0)));
        assert_eq!(m.fields[1].state, State::Absent(Value::Number(3.0)));
        assert_eq!(m.fields[2].state, State::Absent(Value::Undef));
        assert_eq!(m.fields[2].hint.as_deref(), Some("d * 2"));
        assert!(m.edges.is_none());
        // Editing the absent one adds it.
        let edit = edit_for(src, at(src, "peg(5);"), &m.fields[1], Change::Whole(Value::Number(4.0))).unwrap();
        assert!(edit.text.ends_with("peg(5, d = 4);\n"));
    }

    #[test]
    fn library_modules_are_read_from_the_library() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../library").join(fillet::LIBRARY);
        let rounded = std::fs::read_to_string(path).unwrap();
        let library = |p: &str| (p == fillet::LIBRARY).then(|| rounded.clone());
        let src = "use <shapes/rounded.c2f>\nrounded_cube([30, 20, 10], fillet = 2);\n";
        let range = at(src, "rounded_cube([30, 20, 10], fillet = 2);");
        let m = build_model(src, range.clone(), &[], &library).unwrap();
        assert!(!m.unknown);
        assert_eq!(labels(&m.fields), ["size", "fillet", "edges", "center"]);
        assert_eq!(m.fields[0].key, Key::Position(0));
        assert_eq!(m.fields[2].state, State::Absent(Value::Str("all".into())));
        let edit = edit_for(src, range.clone(), &m.fields[1], Change::Whole(Value::Number(3.0))).unwrap();
        assert!(edit.text.contains("rounded_cube([30, 20, 10], fillet = 3);"));
        let edit = edit_for(src, range.clone(), &m.fields[0], Change::Component(2, 12.0)).unwrap();
        assert!(edit.text.contains("rounded_cube([30, 20, 12], fillet = 2);"));

        // Without the library, only what is written.
        let m = build_model(src, range, &[], &no_library).unwrap();
        assert!(m.unknown);
        assert_eq!(labels(&m.fields), ["argument 1", "fillet"]);
    }

    #[test]
    fn fields_rewrite_literals_and_add_arguments() {
        let src = "// a box\ncube(5); // body\n";
        let range = at(src, "cube(5);");
        let m = model(src, "cube(5);");
        // A single number shows as three components.
        assert_eq!(m.fields[0].splat, 3);
        let edit = edit_for(src, range.clone(), &m.fields[0], Change::Component(1, 2.0)).unwrap();
        assert_eq!(edit.text, "// a box\ncube([5, 2, 5]); // body\n");
        let edit = edit_for(src, range.clone(), &m.fields[1], Change::Whole(Value::Bool(true))).unwrap();
        assert_eq!(edit.text, "// a box\ncube(5, center = true); // body\n");
        assert_eq!(&edit.text[edit.selection], "cube(5, center = true);");

        // An absent vector parameter is written whole.
        let src = "cube();";
        let m = model(src, src);
        let edit = edit_for(src, 0..7, &m.fields[0], Change::Component(0, 4.0)).unwrap();
        assert_eq!(edit.text, "cube(size = [4, 1, 1]);");
    }

    #[test]
    fn computed_values_are_not_overwritten() {
        let src = "width = 10; // [5:50]\ncube(width / 2);\n";
        let range = at(src, "cube(width / 2);");
        let parameters = [("width".to_string(), Literal::Number(30.0))];
        let m = build_model(src, range.clone(), &parameters, &no_library).unwrap();
        let State::Computed(c) = &m.fields[0].state else { panic!("computed") };
        // The Customizer's value wins over the file's.
        assert_eq!(c.value, Some(Value::Number(15.0)));
        assert!(c.parameter);
        assert!(edit_for(src, range.clone(), &m.fields[0], Change::Whole(Value::Number(3.0))).is_err());
        let edit = replace_with_number(src, range, c).unwrap();
        assert_eq!(edit.text, "width = 10; // [5:50]\ncube(15);\n");
        assert_eq!(&edit.text[edit.selection], "cube(15);");
    }

    #[test]
    fn expressions_evaluate_from_top_level_variables() {
        let src = "a = 3; b = [a, a * 2, 1]; c = b + [1, 1, 1]; d = max(a, 7) / 2; e = c.y; f = a > 2 ? 1 : 0;";
        let file = c2f_lang::parse(src).unwrap();
        let env = Env::new(&file, &[]);
        let value = |name: &str| eval(&env, env.exprs[name].0, 0);
        assert_eq!(value("b"), Some(Value::Vector(vec![3.0, 6.0, 1.0])));
        assert_eq!(value("c"), Some(Value::Vector(vec![4.0, 7.0, 2.0])));
        assert_eq!(value("d"), Some(Value::Number(3.5)));
        assert_eq!(value("e"), Some(Value::Number(7.0)));
        assert_eq!(value("f"), Some(Value::Number(1.0)));
        // A loop of definitions does not hang.
        let file = c2f_lang::parse("x = y; y = x;").unwrap();
        let env = Env::new(&file, &[]);
        assert_eq!(eval(&env, env.exprs["x"].0, 0), None);
        assert_eq!(typed_value("[1, 2]"), Some(Value::Vector(vec![1.0, 2.0])));
        assert_eq!(typed_value("\"M3\""), Some(Value::Str("M3".into())));
        assert_eq!(typed_value("nope"), None);
    }

    #[test]
    fn a_drag_writes_once_when_it_ends() {
        let id = Id::new("f");
        let mut drag = None;
        // Dragging from 5 to 7 writes nothing...
        assert_eq!(settle(&mut drag, id, 5.0, 5.0, 6.0, true, false), None);
        assert_eq!(settle(&mut drag, id, 5.0, 6.0, 7.0, true, false), None);
        assert_eq!(settle(&mut drag, id, 5.0, 7.0, 7.0, true, false), None);
        assert_eq!(drag, Some(Drag { field: id, value: 7.0 }));
        // ...until it ends: one write.
        assert_eq!(settle(&mut drag, id, 5.0, 7.0, 7.0, false, true), Some(7.0));
        assert_eq!(drag, None);
        // A drag back to where it started writes nothing.
        settle(&mut drag, id, 5.0, 5.0, 6.0, true, false);
        settle(&mut drag, id, 5.0, 6.0, 5.0, true, false);
        assert_eq!(settle(&mut drag, id, 5.0, 5.0, 5.0, false, true), None);
        // A value typed is written at once.
        assert_eq!(settle(&mut drag, id, 5.0, 5.0, 9.0, false, false), Some(9.0));
        // Another field's drag is left alone.
        let mut drag = Some(Drag { field: Id::new("g"), value: 1.0 });
        assert_eq!(settle(&mut drag, id, 5.0, 5.0, 5.0, false, true), None);
        assert!(drag.is_some());
    }

    /// Runs the panel's sections for `m` in a frame of `events`.
    fn frame(ctx: &egui::Context, m: &Model, events: Vec<egui::Event>) -> Vec<Action> {
        let input = egui::RawInput {
            events,
            time: Some(ctx.input(|i| i.time) + 0.25),
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 800.0))),
            ..Default::default()
        };
        let mut actions = Vec::new();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| actions = show_model(ui, m));
        });
        actions
    }

    #[test]
    fn dragging_a_field_in_the_panel_is_one_edit() {
        let src = "cube(5);";
        let m = model(src, src);
        let ctx = egui::Context::default();
        frame(&ctx, &m, Vec::new());
        // The x of the size.
        let base = Id::new(("properties", 0usize, &m.call.name));
        let rect_id = base.with(("field", "size")).with(0usize).with("rect");
        let rect: egui::Rect = ctx.data(|d| d.get_temp(rect_id)).expect("the field was drawn");
        let pos = rect.center();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut writes = Vec::new();
        writes.extend(frame(&ctx, &m, vec![egui::Event::PointerMoved(pos)]));
        writes.extend(frame(&ctx, &m, vec![button(pos, true)]));
        for step in 1..=6 {
            let to = pos + egui::vec2(step as f32 * 10.0, 0.0);
            writes.extend(frame(&ctx, &m, vec![egui::Event::PointerMoved(to)]));
        }
        assert!(writes.is_empty(), "nothing is written while dragging: {writes:?}");
        // The field shows the dragged value meanwhile.
        let drag: Option<Drag> = ctx.data(|d| d.get_temp(drag_id())).flatten();
        assert!(drag.is_some_and(|d| d.value > 5.0), "{drag:?}");
        let actions = frame(&ctx, &m, vec![button(pos + egui::vec2(60.0, 0.0), false)]);
        assert_eq!(actions.len(), 1, "{actions:?}");
        let Action::Edit(0, Change::Component(0, x)) = actions[0] else { panic!("{actions:?}") };
        assert!(x > 5.0);
        let edit = edit_for(src, 0..8, &m.fields[0], Change::Component(0, x)).unwrap();
        assert!(edit.text.starts_with("cube(["));
        parses(&edit.text);
        // Nothing more on the next frames.
        assert!(frame(&ctx, &m, Vec::new()).is_empty());
    }

    #[test]
    fn edges_become_a_library_call_in_one_edit() {
        let src = "// part\ntranslate([5, 0, 0])\n    cube([30, 20, 10]); // body\n";
        let m = model(src, "cube([30, 20, 10]);");
        let Some(Ok(p)) = m.edges else { panic!("edges") };
        let settings = EdgeSettings { fillet: true, size: 2.0, cube: CubeEdges::Vertical, rims: Rims::Both };
        let call = edge_call(p, settings, &m.specials).unwrap();
        let (text, cursor, range) = edges_edit(src, &m.call, &call);
        assert_eq!(
            text,
            "use <shapes/rounded.c2f>\n// part\ntranslate([5, 0, 0])\n    rounded_cube([30, 20, 10], fillet = 2, edges = \"vertical\"); // body\n"
        );
        assert_eq!(&text[range.clone()], "rounded_cube([30, 20, 10], fillet = 2, edges = \"vertical\");");
        assert_eq!(text.chars().count() - text[range.start..].chars().count(), cursor);
        parses(&text);

        // The library already used; a cylinder keeps its `$fn`.
        let src = "use <shapes/rounded.c2f>\n#cylinder(h = 10, d = 8, $fn = 64);\n";
        let m = model(src, "#cylinder(h = 10, d = 8, $fn = 64);");
        let Some(Ok(p)) = m.edges else { panic!("edges") };
        let settings = EdgeSettings { fillet: false, size: 1.0, cube: CubeEdges::All, rims: Rims::Top };
        let call = edge_call(p, settings, &m.specials).unwrap();
        let (text, _, range) = edges_edit(src, &m.call, &call);
        assert_eq!(&text[range], "#chamfered_cylinder(h = 10, r = 4, chamfer = 1, rims = \"top\", $fn = 64);");
        assert_eq!(text.matches("use <").count(), 1);
        // Too large is refused.
        let settings = EdgeSettings { size: 30.0, ..settings };
        assert!(edge_call(p, settings, &[]).is_err());
    }

    /// Two writes asked in the same frame: the first is made, the second,
    /// asked of the code before it, is dropped rather than refused.
    #[test]
    fn one_write_per_frame() {
        let ctx = egui::Context::default();
        let stores = super::super::Stores { session: None, settings: None };
        let settings = crate::settings::Settings::load(None);
        let recent = crate::recent::RecentFiles::default();
        let mut app = StudioApp::with(&ctx, None, stores, recent, settings, Vec::new());
        let src = "cube(5);";
        app.doc.text = src.to_string();
        let m = model(src, src);
        let actions = vec![
            Action::Place(Transform::Translate, [1.0, 0.0, 0.0]),
            Action::Place(Transform::Translate, [2.0, 0.0, 0.0]),
        ];
        app.properties_actions(&ctx, &m, 0..src.len(), actions);
        assert_eq!(app.doc.text, "translate([1, 0, 0]) cube(5);");
        assert_eq!(app.doc.console.count(crate::console::Level::Warning), 0);
    }

    #[test]
    fn colors_keep_their_style() {
        assert_eq!(color_of(&Value::Str("#ff8000".into())), [1.0, 128.0 / 255.0, 0.0, 1.0]);
        assert_eq!(color_value(&Value::Str("red".into()), [0.0, 1.0, 0.0, 1.0]), Value::Str("#00ff00".into()));
        assert_eq!(color_value(&Value::Vector(vec![1.0, 0.0, 0.0]), [0.5, 0.25, 0.0, 1.0]), Value::Vector(vec![0.5, 0.25, 0.0]));
        let m = model("color(\"red\") cube(1);", "color(\"red\") cube(1);");
        assert_eq!(labels(&m.fields), ["c", "alpha"]);
        assert!(m.fields[0].color);
    }
}
