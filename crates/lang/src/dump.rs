//! Prints a [`File`] the way the reference implementation's `-o out.ast`
//! export does (as of 2021.01), so the parser can be diffed against it.
//!
//! The dump reflects the reference's scopes rather than the source text: in
//! each scope it lists functions, then modules, then assignments (a repeated
//! name keeps its first position and its last value), then module instances.
//! Braces are flattened, `*`-disabled instances vanish, other modifiers and
//! imports are not printed, and every binary operation is parenthesized.

use crate::ast::*;
use crate::customizer::Customizer;
use crate::scope::Scope;

pub fn ast_dump(file: &File, src: &str) -> String {
    let customizer = Customizer::new(src);
    let mut out = String::new();
    print_scope(&Scope::collect(&file.stmts), &mut out, 0, Some(&customizer));
    out
}

/// Prints one scope: functions, modules, assignments, then instances.
fn print_scope(scope: &Scope, out: &mut String, indent: usize, customizer: Option<&Customizer>) {
    let tabs = "\t".repeat(indent);
    for f in &scope.functions {
        out.push_str(&format!("{tabs}function {}({}) = {};\n", f.name.name, params(&f.params), expr(&f.body)));
    }
    for m in &scope.modules {
        out.push_str(&format!("{tabs}module {}({}) {{\n", m.name.name, params(&m.params)));
        print_scope(&Scope::collect(std::slice::from_ref(&m.body)), out, indent + 1, None);
        out.push_str(&format!("{tabs}}}\n"));
    }
    for (stmt, a) in &scope.assignments {
        if let Some(annotation) = customizer.and_then(|c| c.annotate(stmt.span, &a.value)) {
            if let Some(group) = &annotation.group {
                out.push_str(&format!("{tabs}//Group({})\n", string(group)));
            }
            if let Some(description) = &annotation.description {
                out.push_str(&format!("{tabs}//Description({})\n", string(description)));
            }
            out.push_str(&format!("{tabs}//Parameter({})\n", annotation.parameter.to_source()));
        }
        out.push_str(&format!("{tabs}{} = {};\n", a.name.name, expr(&a.value)));
    }
    for i in &scope.instances {
        out.push_str(&tabs);
        instance(out, i, indent);
    }
}

/// Prints an instance whose indentation has already been written.
fn instance(out: &mut String, inst: &Instance, indent: usize) {
    match &inst.kind {
        InstanceKind::Call { name, args: a, children: c } => {
            out.push_str(&format!("{}({})", name.name, args(a)));
            children(out, c, indent);
        }
        InstanceKind::If { cond, then, otherwise } => {
            out.push_str(&format!("if({})", expr(cond)));
            children(out, then, indent);
            if let Some(otherwise) = otherwise {
                out.push_str(&"\t".repeat(indent));
                out.push_str("else");
                // The reference prints an empty `else` without a line break.
                if Scope::collect(otherwise).is_empty() {
                    out.push(';');
                } else {
                    children(out, otherwise, indent);
                }
            }
        }
    }
}

/// No children print as `;`, a lone instance continues on the same line,
/// anything else becomes an indented block.
fn children(out: &mut String, stmts: &[Stmt], indent: usize) {
    let scope = Scope::collect(stmts);
    match (scope.len(), scope.instances.as_slice()) {
        (0, _) => out.push_str(";\n"),
        (1, [only]) => {
            out.push(' ');
            instance(out, only, indent);
        }
        _ => {
            out.push_str(" {\n");
            print_scope(&scope, out, indent + 1, None);
            out.push_str(&"\t".repeat(indent));
            out.push_str("}\n");
        }
    }
}

pub fn params(params: &[Param]) -> String {
    let parts: Vec<String> = params
        .iter()
        .map(|p| match &p.default {
            Some(default) => format!("{} = {}", p.name.name, expr(default)),
            None => p.name.name.clone(),
        })
        .collect();
    parts.join(", ")
}

fn args(args: &[Arg]) -> String {
    let parts: Vec<String> = args
        .iter()
        .map(|a| match &a.name {
            Some(name) => format!("{} = {}", name.name, expr(&a.value)),
            None => expr(&a.value),
        })
        .collect();
    parts.join(", ")
}

/// An expression in the reference's canonical form, as `echo` prints function
/// literals.
pub fn expr(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Undef => "undef".into(),
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::Number(n) => number(*n),
        ExprKind::Str(s) => string(s),
        ExprKind::Ident(name) => name.clone(),
        ExprKind::Paren(inner) => expr(inner),
        ExprKind::Vector(elements) => {
            let parts: Vec<String> = elements.iter().map(element).collect();
            format!("[{}]", parts.join(", "))
        }
        ExprKind::Range { start, step: Some(step), end } => {
            format!("[{} : {} : {}]", expr(start), expr(step), expr(end))
        }
        ExprKind::Range { start, step: None, end } => format!("[{} : {}]", expr(start), expr(end)),
        ExprKind::Unary { op: UnaryOp::Neg, operand } => format!("-{}", expr(operand)),
        ExprKind::Unary { op: UnaryOp::Not, operand } => format!("!{}", expr(operand)),
        ExprKind::Unary { op: UnaryOp::Plus, operand } => expr(operand),
        ExprKind::Binary { op, lhs, rhs } => format!("({} {} {})", expr(lhs), op.symbol(), expr(rhs)),
        ExprKind::Ternary { cond, then, otherwise } => {
            format!("({} ? {} : {})", expr(cond), expr(then), expr(otherwise))
        }
        ExprKind::Call { callee, args: a } => match &unparen(callee).kind {
            ExprKind::Ident(name) => format!("{name}({})", args(a)),
            _ => format!("({})({})", expr(callee), args(a)),
        },
        ExprKind::Index { target, index } => format!("{}[{}]", expr(target), expr(index)),
        ExprKind::Member { target, name } => format!("{}.{}", expr(target), name.name),
        ExprKind::FunctionLit { params: p, body } => format!("function({}) {}", params(p), expr(body)),
        ExprKind::Let { args: a, body } => format!("let({}) {}", args(a), expr(body)),
        ExprKind::Assert { args: a, body } => format!("assert({}){}", args(a), tail(body)),
        ExprKind::Echo { args: a, body } => format!("echo({}){}", args(a), tail(body)),
    }
}

fn unparen(e: &Expr) -> &Expr {
    match &e.kind {
        ExprKind::Paren(inner) => unparen(inner),
        _ => e,
    }
}

fn tail(body: &Option<Box<Expr>>) -> String {
    body.as_ref().map_or(String::new(), |b| format!(" {}", expr(b)))
}

fn element(e: &Element) -> String {
    match e {
        Element::Expr(e) => expr(e),
        Element::Comp(c) => comp(c),
    }
}

fn comp(c: &Comp) -> String {
    match &c.kind {
        // `for (a = ..., b = ...)` prints as nested single-variable loops.
        CompKind::For { bindings, body } => {
            let mut text = element(body);
            if bindings.is_empty() {
                return format!("for() ({text})");
            }
            for binding in bindings.iter().rev() {
                text = format!("for({}) ({text})", args(std::slice::from_ref(binding)));
            }
            text
        }
        CompKind::ForC { init, cond, update, body } => {
            format!("for({};{};{}) {}", args(init), expr(cond), args(update), element(body))
        }
        CompKind::Each(body) => format!("each ({})", element(body)),
        CompKind::If { cond, then, otherwise: None } => format!("if({}) ({})", expr(cond), element(then)),
        CompKind::If { cond, then, otherwise: Some(otherwise) } => {
            format!("if({}) ({}) else ({})", expr(cond), element(then), element(otherwise))
        }
        // The reference reads `let` with a plain-value body as a `let`
        // expression, and parenthesizes the body only when it is a real
        // comprehension.
        CompKind::Let { bindings, body } if !is_value(body) => {
            format!("let({}) ({})", args(bindings), element(body))
        }
        CompKind::Let { bindings, body } => format!("let({}) {}", args(bindings), element(body)),
    }
}

/// An element yielding exactly one value: an expression or a chain of
/// `let`s ending in one.
fn is_value(e: &Element) -> bool {
    match e {
        Element::Expr(_) => true,
        Element::Comp(c) => matches!(&c.kind, CompKind::Let { body, .. } if is_value(body)),
    }
}

/// Six significant digits, ties rounded away from zero; exponent notation
/// below 1e-5 or from 1e6 up.
pub fn number(n: f64) -> String {
    if n == 0.0 {
        return if n.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    if !n.is_finite() {
        return if n.is_nan() { "nan".into() } else if n > 0.0 { "inf".into() } else { "-inf".into() };
    }
    let (digits, exponent) = six_digits(n.abs());
    let digits = digits.trim_end_matches('0');
    let sign = if n < 0.0 { "-" } else { "" };

    if !(-5..6).contains(&exponent) {
        let (head, rest) = digits.split_at(1);
        let fraction = if rest.is_empty() { String::new() } else { format!(".{rest}") };
        let exp_sign = if exponent < 0 { '-' } else { '+' };
        return format!("{sign}{head}{fraction}e{exp_sign}{}", exponent.abs());
    }
    if exponent < 0 {
        let zeros = "0".repeat((-exponent - 1) as usize);
        return format!("{sign}0.{zeros}{digits}");
    }
    let int_len = exponent as usize + 1;
    if digits.len() <= int_len {
        format!("{sign}{digits}{}", "0".repeat(int_len - digits.len()))
    } else {
        format!("{sign}{}.{}", &digits[..int_len], &digits[int_len..])
    }
}

/// The first six significant digits of `x > 0` and its decimal exponent.
/// Rust rounds ties to even; the reference (double-conversion) rounds them
/// up, so the tie is decided on a long exact expansion instead.
fn six_digits(x: f64) -> (String, i32) {
    let sci = format!("{x:.40e}");
    let (mantissa, exponent) = sci.split_once('e').expect("{:e} always has an exponent");
    let mut exponent: i32 = exponent.parse().expect("exponent is an integer");
    let all: Vec<u8> = mantissa.bytes().filter(u8::is_ascii_digit).map(|b| b - b'0').collect();
    let (head, tail) = all.split_at(6);
    let mut digits = head.to_vec();
    if tail[0] >= 5 {
        let mut i = digits.len();
        loop {
            if i == 0 {
                digits.insert(0, 1);
                digits.pop();
                exponent += 1;
                break;
            }
            i -= 1;
            if digits[i] == 9 {
                digits[i] = 0;
            } else {
                digits[i] += 1;
                break;
            }
        }
    }
    (digits.iter().map(|d| (b'0' + d) as char).collect(), exponent)
}

/// A quoted string literal with the reference's escapes.
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::number;

    #[test]
    #[allow(clippy::approx_constant)]
    fn number_format() {
        let cases = [
            (1500.0, "1500"),
            (0.1, "0.1"),
            (123456789.0, "1.23457e+8"),
            (1e-7, "1e-7"),
            (100000.0, "100000"),
            (1e6, "1e+6"),
            (123456.7, "123457"),
            (999999.5, "1e+6"),
            (0.00001, "0.00001"),
            (1e-6, "1e-6"),
            (0.000012, "0.000012"),
            (0.30000000000000004, "0.3"),
            (3.14159265358979, "3.14159"),
            (1e100, "1e+100"),
            (-5.0, "-5"),
            (0.000123456789, "0.000123457"),
            (100.0625, "100.063"),
            (-100.0625, "-100.063"),
            (9.999995, "10"),
            // Stored just below the tie, so it rounds down.
            (0.1234565, "0.123456"),
        ];
        for (value, expected) in cases {
            assert_eq!(number(value), expected, "{value}");
        }
    }
}
