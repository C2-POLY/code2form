//! The customizer: the parameters a file exposes through comments,
//! the widget each one gets, and setting their values, from the panel or
//! from a parameter set (`-p file.json -P set`).
//!
//! Rules of the reference implementation (release 2021.01):
//!
//! - A parameter is a top-level assignment of the main file that carries an
//!   annotation (see [`c2f_lang::customizer`]) and whose value is not
//!   undef. Its group is "Parameters" when no `/* [Group] */` precedes it;
//!   the "Hidden" group is not shown.
//! - Setting a parameter replaces the value of every top-level assignment
//!   of that name, as the panel does, rather than appending one (`-D`).
//! - A parameter set stores every value as a string, read according to the
//!   type of the value in the file (a repeated name: its last value):
//!   strings are taken as written, booleans must read `true`, `false` or an
//!   integer equal to 1 or 0 (anything else aborts the rest of the set),
//!   and other values are read in the comments' grammar and kept only when
//!   they have the same type as the file's.

use c2f_lang::ast::{Element, Expr, ExprKind, Stmt, StmtKind, UnaryOp};
use c2f_lang::customizer::{annotated, parse_value};
use c2f_lang::dump::number;
use c2f_lang::scope::Scope;

pub use c2f_lang::customizer::Literal;

use crate::program::Program;
use crate::value::{Range, Value};

/// The group of parameters with no `/* [Group] */` above them.
pub const DEFAULT_GROUP: &str = "Parameters";
/// Parameters in this group are not shown.
pub const HIDDEN_GROUP: &str = "Hidden";

/// A parameter the customizer shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub name: String,
    pub group: String,
    /// The `//` comment on the line above.
    pub description: Option<String>,
    /// The value written in the file.
    pub default: Literal,
    pub widget: Widget,
}

/// How the customizer edits a parameter.
#[derive(Clone, Debug, PartialEq)]
pub enum Widget {
    Checkbox,
    /// A number from `min` to `max`, in steps of `step` from `min`:
    /// `// [min : max]`, `// [min : step : max]`, or `// [max]` (from 0).
    Slider { min: f64, max: f64, step: f64 },
    /// A free number, nudged by `step`: one unit of the default's last
    /// decimal.
    Spinbox { step: f64 },
    /// One of a list, `// [a, b, c]` or `// [value:label, ...]`: each
    /// option's value and label.
    Dropdown { options: Vec<(Literal, String)> },
    /// One to four numbers, each nudged by `step`.
    Vector { step: f64 },
    /// A string, limited to `max_length` characters by `// 8`, or another
    /// value typed in the comments' grammar (see [`parse_typed`]).
    Text { max_length: Option<usize> },
}

impl Program {
    /// The main file's customizer parameters; `src` is its text, where the
    /// annotations are read.
    pub fn parameters(&self, src: &str) -> Vec<Parameter> {
        annotated(&self.files[0].ast, src)
            .into_iter()
            .filter_map(|(assignment, annotation)| {
                let default = constant(&assignment.value);
                if default == Literal::Undef {
                    return None;
                }
                Some(Parameter {
                    name: assignment.name.name.clone(),
                    group: annotation.group.unwrap_or_else(|| DEFAULT_GROUP.into()),
                    description: annotation.description,
                    widget: widget(&default, &annotation.parameter),
                    default,
                })
            })
            .collect()
    }

    /// Sets the value of every top-level assignment of `name` (included
    /// files' too). Returns whether there was one.
    pub fn set_parameter(&mut self, name: &str, value: &Literal) -> bool {
        let mut found = false;
        for_each_assignment(&mut self.files[0].ast.stmts, &mut |assigned, expr| {
            if assigned == name {
                *expr = value.to_expr(expr.span);
                found = true;
            }
        });
        found
    }

    /// Applies a parameter set: names and values as the JSON file stores
    /// them. On a value that cannot be read, the assignments that follow
    /// are left as they are and the error says why.
    pub fn apply_parameter_set(&mut self, set: &[(String, String)]) -> Result<(), String> {
        let mut updates = Vec::new();
        let mut error = None;
        // Each name once, at its first position, typed by its last value.
        'assignments: for (_, a) in Scope::collect(&self.files[0].ast.stmts).assignments {
            for (_, raw) in set.iter().filter(|(n, _)| *n == a.name.name) {
                match parse_typed(&constant(&a.value), raw) {
                    Ok(Some(value)) => updates.push((a.name.name.clone(), value)),
                    Ok(None) => {}
                    Err(why) => {
                        error = Some(format!("{}: {why}", a.name.name));
                        break 'assignments;
                    }
                }
            }
        }
        for (name, value) in updates {
            self.set_parameter(&name, &value);
        }
        error.map_or(Ok(()), Err)
    }
}

/// Reads `raw` as a new value for a parameter whose value in the file is
/// `default`, as parameter sets are read. `Ok(None)`: the text is ignored.
pub fn parse_typed(default: &Literal, raw: &str) -> Result<Option<Literal>, String> {
    match default {
        Literal::Str(_) => Ok(Some(Literal::Str(raw.to_string()))),
        Literal::Bool(_) => {
            let text = raw.trim();
            let flag = match text {
                "true" => Some(true),
                "false" => Some(false),
                // Read as an integer first, as C++ streams do: "01", "+1".
                _ => match text.parse::<i64>() {
                    Ok(0) => Some(false),
                    Ok(1) => Some(true),
                    _ => None,
                },
            };
            flag.map(|b| Some(Literal::Bool(b))).ok_or_else(|| format!("\"{raw}\" is not true or false"))
        }
        _ => Ok(parse_value(raw).filter(|value| same_type(value, default))),
    }
}

/// How a parameter set stores a value.
pub fn set_text(value: &Literal) -> String {
    match value {
        Literal::Str(s) => s.clone(),
        Literal::Number(n) => number(*n),
        other => other.to_source(),
    }
}

fn same_type(a: &Literal, b: &Literal) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

/// Calls `f` with each top-level assignment's name and value, braces
/// included (they do not open a scope).
fn for_each_assignment(stmts: &mut [Stmt], f: &mut impl FnMut(&str, &mut Expr)) {
    for stmt in stmts {
        match &mut stmt.kind {
            StmtKind::Assign(a) => f(&a.name.name, &mut a.value),
            StmtKind::Block(inner) => for_each_assignment(inner, f),
            _ => {}
        }
    }
}

/// The value of a literal expression, as the reference evaluates it without
/// any variable; undef for anything else.
pub fn constant(expr: &Expr) -> Literal {
    fn eval<'p>(expr: &Expr) -> Option<Value<'p>> {
        Some(match &expr.kind {
            ExprKind::Undef => Value::Undef,
            ExprKind::Bool(b) => Value::Bool(*b),
            ExprKind::Number(n) => Value::Number(*n),
            ExprKind::Str(s) => Value::str(s),
            ExprKind::Paren(inner) => eval(inner)?,
            ExprKind::Unary { op, operand } => {
                let value = eval(operand)?;
                match op {
                    UnaryOp::Not => Value::Bool(!value.truthy()),
                    UnaryOp::Plus => value,
                    UnaryOp::Neg => value.neg().unwrap_or(Value::Undef),
                }
            }
            ExprKind::Vector(elements) => Value::vector(
                elements
                    .iter()
                    .map(|e| match e {
                        Element::Expr(e) => eval(e),
                        Element::Comp(_) => None,
                    })
                    .collect::<Option<_>>()?,
            ),
            ExprKind::Range { start, step, end } => {
                let number = |e: &Expr| eval(e)?.as_number();
                let (Some(begin), Some(end)) = (number(start), number(end)) else { return Some(Value::Undef) };
                let step = match step {
                    Some(step) => Some(number(step)?),
                    None => None,
                };
                Value::Range(Range::new(begin, step, end).0)
            }
            _ => return None,
        })
    }
    eval(expr).map_or(Literal::Undef, |value| literal(&value))
}

fn literal(value: &Value) -> Literal {
    match value {
        Value::Undef | Value::Function(_) => Literal::Undef,
        Value::Bool(b) => Literal::Bool(*b),
        Value::Number(n) => Literal::Number(*n),
        Value::Str(s) => Literal::Str(s.to_string()),
        Value::Vector(items) => Literal::Vector(items.iter().map(literal).collect()),
        Value::Range(r) => Literal::Range { begin: r.begin, step: Some(r.step), end: r.end },
    }
}

/// The widget the reference picks for a parameter with value `default`
/// and the allowed values `values` from its comment.
fn widget(default: &Literal, values: &Literal) -> Widget {
    if let Literal::Bool(_) = default {
        return Widget::Checkbox;
    }
    if let Literal::Vector(items) = default {
        if items.len() <= 4 {
            let numbers: Option<Vec<f64>> =
                items.iter().map(|item| if let Literal::Number(n) = item { Some(*n) } else { None }).collect();
            return match numbers {
                Some(numbers) if !numbers.is_empty() => Widget::Vector { step: step_for(&numbers) },
                _ => Widget::Text { max_length: None },
            };
        }
    }
    let is_number = matches!(default, Literal::Number(_));
    // `// [10]` on a number: a slider from 0 to 10, as on Thingiverse.
    let maker_bot_max = match values {
        Literal::Vector(items) if is_number => match items[..] {
            [Literal::Number(max)] => Some(max),
            _ => None,
        },
        _ => None,
    };
    match (values, maker_bot_max) {
        (Literal::Vector(items), None) => {
            let options: Vec<(Literal, String)> = items.iter().filter_map(|item| option(item, default)).collect();
            if !options.is_empty() {
                return Widget::Dropdown { options };
            }
        }
        (Literal::Range { begin, step, end }, _) if is_number => {
            let step = step.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(1.0);
            return Widget::Slider { min: begin.min(*end), max: begin.max(*end), step };
        }
        (_, Some(max)) => return Widget::Slider { min: 0.0, max, step: 1.0 },
        _ => {}
    }
    match (default, values) {
        // `// 0.25` on a number: the spinbox step.
        (Literal::Number(_), Literal::Number(step)) if step.is_finite() && *step > 0.0 => {
            Widget::Spinbox { step: *step }
        }
        (Literal::Number(n), _) => Widget::Spinbox { step: step_for(&[*n]) },
        (Literal::Str(_), Literal::Number(max)) if *max >= 0.0 => Widget::Text { max_length: Some(*max as usize) },
        _ => Widget::Text { max_length: None },
    }
}

/// A dropdown option, its value of the default's type: `value:label`
/// comes as `[value, label]`. A string parameter takes the option's text;
/// other parameters only options of their type.
fn option(item: &Literal, default: &Literal) -> Option<(Literal, String)> {
    let (value, label) = match item {
        Literal::Vector(pair) if pair.len() == 2 => (&pair[0], label(&pair[1])),
        other => (other, label(other)),
    };
    let value = match (default, value) {
        (Literal::Str(_), value) => Literal::Str(set_text(value)),
        (default, value) if same_type(default, value) => value.clone(),
        _ => return None,
    };
    Some((value, label))
}

fn label(value: &Literal) -> String {
    match value {
        Literal::Str(s) => s.clone(),
        other => set_text(other),
    }
}

/// One unit of the last decimal the numbers need (at most 6 decimals).
fn step_for(numbers: &[f64]) -> f64 {
    let decimals = numbers
        .iter()
        .map(|&n| (0..6).find(|&d| (n * 10f64.powi(d)).fract().abs() < 1e-9).unwrap_or(6))
        .max()
        .unwrap_or(0);
    10f64.powi(-decimals)
}

/// The parameter sets of a customizer JSON file (`<model>.json` next to
/// the model): named lists of parameter names and values, in file order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParameterSets {
    pub sets: Vec<(String, Vec<(String, String)>)>,
    /// The whole file, so that writing it back keeps what else it holds.
    root: serde_json::Map<String, serde_json::Value>,
}

impl ParameterSets {
    pub fn parse(json: &str) -> Result<Self, String> {
        let serde_json::Value::Object(root) = serde_json::from_str(json).map_err(|e| e.to_string())? else {
            return Err("not a JSON object".into());
        };
        let Some(sets) = root.get("parameterSets").and_then(|s| s.as_object()) else {
            return Err("no \"parameterSets\" object".into());
        };
        let sets = sets
            .iter()
            .map(|(name, values)| {
                let values = values.as_object().into_iter().flatten().map(|(param, value)| {
                    let text = match value {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (param.clone(), text)
                });
                (name.clone(), values.collect())
            })
            .collect();
        Ok(ParameterSets { sets, root })
    }

    /// The file, as the reference writes it.
    pub fn to_json(&self) -> String {
        let sets: serde_json::Map<String, serde_json::Value> = self
            .sets
            .iter()
            .map(|(name, values)| {
                let values = values.iter().map(|(p, v)| (p.clone(), serde_json::Value::String(v.clone()))).collect();
                (name.clone(), serde_json::Value::Object(values))
            })
            .collect();
        let mut root = self.root.clone();
        root.insert("parameterSets".into(), serde_json::Value::Object(sets));
        root.entry("fileFormatVersion").or_insert_with(|| "1".into());
        let mut out = serde_json::to_string_pretty(&root).unwrap_or_default();
        out.push('\n');
        out
    }

    pub fn get(&self, name: &str) -> Option<&[(String, String)]> {
        self.sets.iter().find(|(n, _)| n == name).map(|(_, values)| values.as_slice())
    }

    /// Adds the set, or replaces the one with that name in place.
    pub fn insert(&mut self, name: &str, values: Vec<(String, String)>) {
        match self.sets.iter_mut().find(|(n, _)| n == name) {
            Some(set) => set.1 = values,
            None => self.sets.push((name.to_string(), values)),
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.sets.retain(|(n, _)| n != name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn program(src: &str) -> Program {
        Program::load(src, Path::new("test.scad"), &[]).expect("parses")
    }

    fn widgets(src: &str) -> Vec<(String, Widget)> {
        program(src).parameters(src).into_iter().map(|p| (p.name, p.widget)).collect()
    }

    #[test]
    fn picks_widgets() {
        let src = "b = true;\n\
            n = 5; // [0:10]\n\
            s = 1.25; // [0:0.25:2]\n\
            m = 3; // [8]\n\
            f = 1.5;\n\
            d = 10; // [10, 20, 30]\n\
            l = 50; // [70:large, 50:medium]\n\
            v = [1, 2.5, 3];\n\
            big = [1, 2, 3, 4, 5];\n\
            mixed = [1, \"a\"];\n\
            t = \"abc\";\n\
            short = \"x\"; // 8\n\
            u = undef;\n\
            digits = \"1\"; // [1, 2]\n\
            ranged = \"abc\"; // [0:10]\n\
            one = \"a\"; // [8]\n\
            stepped = 1; // 0.25\n\
            listed = [1, 2, 3, 4, 5]; // [8, 9]\n\
            cube(n);\n\
            late = 2;\n";
        let mut found = widgets(src).into_iter();
        let mut next = || found.next().unwrap();
        assert_eq!(next(), ("b".into(), Widget::Checkbox));
        assert_eq!(next(), ("n".into(), Widget::Slider { min: 0.0, max: 10.0, step: 1.0 }));
        assert_eq!(next(), ("s".into(), Widget::Slider { min: 0.0, max: 2.0, step: 0.25 }));
        assert_eq!(next(), ("m".into(), Widget::Slider { min: 0.0, max: 8.0, step: 1.0 }));
        assert_eq!(next(), ("f".into(), Widget::Spinbox { step: 0.1 }));
        let numbers = |ns: &[f64]| ns.iter().map(|&n| (Literal::Number(n), number(n))).collect();
        assert_eq!(next(), ("d".into(), Widget::Dropdown { options: numbers(&[10.0, 20.0, 30.0]) }));
        let labelled = vec![(Literal::Number(70.0), "large".into()), (Literal::Number(50.0), "medium".into())];
        assert_eq!(next(), ("l".into(), Widget::Dropdown { options: labelled }));
        assert_eq!(next(), ("v".into(), Widget::Vector { step: 0.1 }));
        assert_eq!(next(), ("big".into(), Widget::Text { max_length: None }));
        assert_eq!(next(), ("mixed".into(), Widget::Text { max_length: None }));
        assert_eq!(next(), ("t".into(), Widget::Text { max_length: None }));
        assert_eq!(next(), ("short".into(), Widget::Text { max_length: Some(8) }));
        // Options take the default's type; ranges and `[max]` need a number.
        let texts = |ts: &[&str]| ts.iter().map(|t| (Literal::Str(t.to_string()), t.to_string())).collect();
        assert_eq!(next(), ("digits".into(), Widget::Dropdown { options: texts(&["1", "2"]) }));
        assert_eq!(next(), ("ranged".into(), Widget::Text { max_length: None }));
        assert_eq!(next(), ("one".into(), Widget::Dropdown { options: texts(&["8"]) }));
        assert_eq!(next(), ("stepped".into(), Widget::Spinbox { step: 0.25 }));
        // No option of the vector's type: typed in instead.
        assert_eq!(next(), ("listed".into(), Widget::Text { max_length: None }));
        // undef is left out; assignments after a module call still count.
        assert_eq!(next(), ("late".into(), Widget::Spinbox { step: 1.0 }));
    }

    #[test]
    fn groups_and_descriptions() {
        let src = "a = 1;\n/* [Size] */\n// The width\nw = 2;\n/* [Hidden] */\nh = 3;\nmodule m() { x = 4; }\n";
        let params = program(src).parameters(src);
        let summary: Vec<(&str, &str, Option<&str>)> =
            params.iter().map(|p| (p.name.as_str(), p.group.as_str(), p.description.as_deref())).collect();
        assert_eq!(summary, [("a", "Parameters", None), ("w", "Size", Some("The width")), ("h", "Hidden", None)]);
    }

    #[test]
    fn sets_every_assignment_of_a_name() {
        let src = "x = 1; // [0:10]\n{ x = 2; }\ny = x;\n";
        let mut program = program(src);
        assert!(program.set_parameter("x", &Literal::Number(7.0)));
        assert!(!program.set_parameter("nope", &Literal::Number(7.0)));
        let output = crate::evaluate(&program);
        assert!(output.warnings.is_empty(), "{:?}", output.warnings);
        let mut values = Vec::new();
        for_each_assignment(&mut program.files[0].ast.stmts, &mut |name, expr| {
            values.push((name.to_string(), constant(expr)));
        });
        assert_eq!(values[0], ("x".into(), Literal::Number(7.0)));
        assert_eq!(values[1], ("x".into(), Literal::Number(7.0)));
    }

    #[test]
    fn reads_set_values_by_type() {
        let number = Literal::Number(1.0);
        assert_eq!(parse_typed(&number, " 3 "), Ok(Some(Literal::Number(3.0))));
        assert_eq!(parse_typed(&number, "1e2"), Ok(Some(Literal::Number(100.0))));
        assert_eq!(parse_typed(&number, "true"), Ok(None));
        assert_eq!(parse_typed(&number, "[1:2:9]"), Ok(None));
        let text = Literal::Str("a".into());
        assert_eq!(parse_typed(&text, "\"q\""), Ok(Some(Literal::Str("\"q\"".into()))));
        let flag = Literal::Bool(true);
        assert_eq!(parse_typed(&flag, " false "), Ok(Some(Literal::Bool(false))));
        assert_eq!(parse_typed(&flag, "0"), Ok(Some(Literal::Bool(false))));
        assert!(parse_typed(&flag, "yes").is_err());
        assert_eq!(parse_typed(&flag, "01"), Ok(Some(Literal::Bool(true))));
        assert_eq!(parse_typed(&flag, "-0"), Ok(Some(Literal::Bool(false))));
        assert!(parse_typed(&flag, "1.0").is_err());
        assert!(parse_typed(&flag, "2").is_err());
        let vector = Literal::Vector(vec![Literal::Number(1.0)]);
        let words = Literal::Vector(vec![Literal::Str("a".into()), Literal::Str("b".into())]);
        assert_eq!(parse_typed(&vector, "[a, b]"), Ok(Some(words)));
        assert_eq!(parse_typed(&vector, "[1:3]"), Ok(None));
        assert_eq!(parse_typed(&Literal::Undef, "9"), Ok(None));
        assert_eq!(set_text(&Literal::Number(2.5)), "2.5");
        assert_eq!(set_text(&Literal::Str("hi".into())), "hi");
        assert_eq!(set_text(&Literal::Vector(vec![Literal::Number(1.0), Literal::Number(2.0)])), "[1, 2]");
    }

    #[test]
    fn constants() {
        let value = |src: &str| {
            let file = c2f_lang::parse(&format!("x = {src};")).unwrap();
            let StmtKind::Assign(a) = &file.stmts[0].kind else { panic!() };
            constant(&a.value)
        };
        assert_eq!(value("-2"), Literal::Number(-2.0));
        assert_eq!(value("!0"), Literal::Bool(true));
        assert_eq!(value("-[1, (2)]"), Literal::Vector(vec![Literal::Number(-1.0), Literal::Number(-2.0)]));
        assert_eq!(value("[0:2]"), Literal::Range { begin: 0.0, step: Some(1.0), end: 2.0 });
        assert_eq!(value("1 + 1"), Literal::Undef);
        assert_eq!(value("-\"a\""), Literal::Undef);
    }

    #[test]
    fn parameter_set_files() {
        let json = r#"{ "parameterSets": { "Big": { "w": "10", "on": true, "n": 3 }, "Small": {} }, "fileFormatVersion": "1" }"#;
        let mut sets = ParameterSets::parse(json).unwrap();
        let big: Vec<(String, String)> = [("w", "10"), ("on", "true"), ("n", "3")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        assert_eq!(sets.get("Big"), Some(&big[..]));
        sets.insert("Small", vec![("w".into(), "2".into())]);
        sets.insert("Tiny", vec![]);
        sets.remove("Big");
        let again = ParameterSets::parse(&sets.to_json()).unwrap();
        let names: Vec<&str> = again.sets.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["Small", "Tiny"]);
        assert!(sets.to_json().contains("\"fileFormatVersion\": \"1\""));
        assert!(ParameterSets::parse("{}").is_err());
        // Whatever else the file holds is written back.
        let other = r#"{ "parameterSets": {}, "fileFormatVersion": "1", "note": [1] }"#;
        assert!(ParameterSets::parse(other).unwrap().to_json().contains("\"note\""));
    }

    #[test]
    fn sets_use_the_value_that_wins() {
        // The reference reads `x` as a number, its last value.
        let src = "x = \"a\";\ny = 2;\nx = 1;\necho(x, y);\n";
        let mut program = program(src);
        let set = [("x".to_string(), "abc".to_string()), ("y".to_string(), "5".to_string())];
        assert_eq!(program.apply_parameter_set(&set), Ok(()));
        assert_eq!(crate::evaluate(&program).echo, ["ECHO: 1, 5"]);
    }
}
