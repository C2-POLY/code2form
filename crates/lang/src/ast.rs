//! Syntax tree. It keeps the program as written (blocks, parentheses,
//! modifiers, imports); scoping and desugaring belong to later stages.

use crate::span::Span;

#[derive(Clone, Debug, PartialEq)]
pub struct File {
    pub stmts: Vec<Stmt>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StmtKind {
    /// A lone `;`.
    Empty,
    /// `{ ... }`. Braces group statements without opening a new scope.
    Block(Vec<Stmt>),
    Include(String),
    Use(String),
    Assign(Assignment),
    Module(ModuleDef),
    Function(FunctionDef),
    Instance(Instance),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Assignment {
    pub name: Ident,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: Ident,
    pub default: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arg {
    pub name: Option<Ident>,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModuleDef {
    pub name: Ident,
    pub params: Vec<Param>,
    pub body: Box<Stmt>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionDef {
    pub name: Ident,
    pub params: Vec<Param>,
    pub body: Expr,
}

/// The `!`, `#`, `%` and `*` prefixes of a module instance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    /// `!`: render only this subtree.
    pub root: bool,
    /// `#`: highlight.
    pub highlight: bool,
    /// `%`: transparent background, excluded from the result.
    pub background: bool,
    /// `*`: disabled, as if commented out.
    pub disabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Instance {
    pub modifiers: Modifiers,
    pub kind: InstanceKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InstanceKind {
    /// `name(args) children`. `name` may also be `for`, `let`, `assert`,
    /// `echo` or `each`.
    Call { name: Ident, args: Vec<Arg>, children: Vec<Stmt> },
    If { cond: Expr, then: Vec<Stmt>, otherwise: Option<Vec<Stmt>> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    Undef,
    Bool(bool),
    Number(f64),
    Str(String),
    Ident(String),
    Paren(Box<Expr>),
    Vector(Vec<Element>),
    Range { start: Box<Expr>, step: Option<Box<Expr>>, end: Box<Expr> },
    Unary { op: UnaryOp, operand: Box<Expr> },
    Binary { op: BinaryOp, lhs: Box<Expr>, rhs: Box<Expr> },
    Ternary { cond: Box<Expr>, then: Box<Expr>, otherwise: Box<Expr> },
    Call { callee: Box<Expr>, args: Vec<Arg> },
    Index { target: Box<Expr>, index: Box<Expr> },
    Member { target: Box<Expr>, name: Ident },
    FunctionLit { params: Vec<Param>, body: Box<Expr> },
    Let { args: Vec<Arg>, body: Box<Expr> },
    Assert { args: Vec<Arg>, body: Option<Box<Expr>> },
    Echo { args: Vec<Arg>, body: Option<Box<Expr>> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    Plus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
}

impl BinaryOp {
    pub fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Or => "||",
            BinaryOp::And => "&&",
            BinaryOp::Eq => "==",
            BinaryOp::Ne => "!=",
            BinaryOp::Lt => "<",
            BinaryOp::Le => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Ge => ">=",
            BinaryOp::Add => "+",
            BinaryOp::Sub => "-",
            BinaryOp::Mul => "*",
            BinaryOp::Div => "/",
            BinaryOp::Mod => "%",
            BinaryOp::Pow => "^",
        }
    }
}

/// One element of a vector literal: a value or a list comprehension.
#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Expr(Expr),
    Comp(Box<Comp>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Comp {
    pub kind: CompKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CompKind {
    /// `for (a = ..., b = ...) body`, nested loops over each binding.
    For { bindings: Vec<Arg>, body: Element },
    /// `for (init; cond; update) body`.
    ForC { init: Vec<Arg>, cond: Expr, update: Vec<Arg>, body: Element },
    Each(Element),
    If { cond: Expr, then: Element, otherwise: Option<Element> },
    Let { bindings: Vec<Arg>, body: Element },
}

impl Expr {
    /// A constant made only of literals, which the customizer can expose.
    pub fn is_literal(&self) -> bool {
        match &self.kind {
            ExprKind::Undef | ExprKind::Bool(_) | ExprKind::Number(_) | ExprKind::Str(_) => true,
            ExprKind::Paren(inner) => inner.is_literal(),
            ExprKind::Unary { operand, .. } => operand.is_literal(),
            ExprKind::Vector(elements) => {
                elements.iter().all(|e| matches!(e, Element::Expr(e) if e.is_literal()))
            }
            ExprKind::Range { start, step, end } => {
                start.is_literal() && end.is_literal() && step.as_ref().is_none_or(|s| s.is_literal())
            }
            _ => false,
        }
    }
}

// Deep trees (a 100000-term `1 + 1 + ...`, 10000 nested blocks) must not
// overflow the stack when dropped, so expressions and statements free their
// subtrees iteratively.

enum Pending {
    Expr(Expr),
    Comp(Comp),
    Element(Element),
}

impl Drop for Expr {
    fn drop(&mut self) {
        let mut pending = Vec::new();
        take_expr(std::mem::replace(&mut self.kind, ExprKind::Undef), &mut pending);
        while let Some(item) = pending.pop() {
            match item {
                Pending::Expr(mut e) => take_expr(std::mem::replace(&mut e.kind, ExprKind::Undef), &mut pending),
                Pending::Comp(c) => take_comp(c.kind, &mut pending),
                Pending::Element(Element::Expr(e)) => pending.push(Pending::Expr(e)),
                Pending::Element(Element::Comp(c)) => pending.push(Pending::Comp(*c)),
            }
        }
    }
}

fn take_args(args: Vec<Arg>, out: &mut Vec<Pending>) {
    out.extend(args.into_iter().map(|a| Pending::Expr(a.value)));
}

fn take_params(params: Vec<Param>, out: &mut Vec<Pending>) {
    out.extend(params.into_iter().filter_map(|p| p.default).map(Pending::Expr));
}

fn take_expr(kind: ExprKind, out: &mut Vec<Pending>) {
    let mut push = |e: Box<Expr>| out.push(Pending::Expr(*e));
    match kind {
        ExprKind::Undef | ExprKind::Bool(_) | ExprKind::Number(_) | ExprKind::Str(_) | ExprKind::Ident(_) => {}
        ExprKind::Paren(e) | ExprKind::Unary { operand: e, .. } | ExprKind::Member { target: e, .. } => push(e),
        ExprKind::Binary { lhs, rhs, .. } => {
            push(lhs);
            push(rhs);
        }
        ExprKind::Index { target, index } => {
            push(target);
            push(index);
        }
        ExprKind::Ternary { cond, then, otherwise } => {
            push(cond);
            push(then);
            push(otherwise);
        }
        ExprKind::Range { start, step, end } => {
            push(start);
            if let Some(step) = step {
                push(step);
            }
            push(end);
        }
        ExprKind::Vector(elements) => out.extend(elements.into_iter().map(Pending::Element)),
        ExprKind::Call { callee, args } => {
            push(callee);
            take_args(args, out);
        }
        ExprKind::FunctionLit { params, body } => {
            push(body);
            take_params(params, out);
        }
        ExprKind::Let { args, body } => {
            push(body);
            take_args(args, out);
        }
        ExprKind::Assert { args, body } | ExprKind::Echo { args, body } => {
            if let Some(body) = body {
                push(body);
            }
            take_args(args, out);
        }
    }
}

fn take_comp(kind: CompKind, out: &mut Vec<Pending>) {
    match kind {
        CompKind::For { bindings, body } | CompKind::Let { bindings, body } => {
            take_args(bindings, out);
            out.push(Pending::Element(body));
        }
        CompKind::ForC { init, cond, update, body } => {
            take_args(init, out);
            take_args(update, out);
            out.push(Pending::Expr(cond));
            out.push(Pending::Element(body));
        }
        CompKind::Each(body) => out.push(Pending::Element(body)),
        CompKind::If { cond, then, otherwise } => {
            out.push(Pending::Expr(cond));
            out.push(Pending::Element(then));
            if let Some(otherwise) = otherwise {
                out.push(Pending::Element(otherwise));
            }
        }
    }
}

impl Drop for Stmt {
    fn drop(&mut self) {
        let mut pending: Vec<Stmt> = Vec::new();
        take_stmt(std::mem::replace(&mut self.kind, StmtKind::Empty), &mut pending);
        while let Some(mut stmt) = pending.pop() {
            take_stmt(std::mem::replace(&mut stmt.kind, StmtKind::Empty), &mut pending);
        }
    }
}

/// Moves a statement's sub-statements to `out`; its expressions drop with
/// their own iterative `Drop`.
fn take_stmt(kind: StmtKind, out: &mut Vec<Stmt>) {
    match kind {
        StmtKind::Block(stmts) => out.extend(stmts),
        StmtKind::Module(m) => out.push(*m.body),
        StmtKind::Instance(Instance { kind: InstanceKind::Call { children, .. }, .. }) => out.extend(children),
        StmtKind::Instance(Instance { kind: InstanceKind::If { then, otherwise, .. }, .. }) => {
            out.extend(then);
            out.extend(otherwise.into_iter().flatten());
        }
        _ => {}
    }
}
