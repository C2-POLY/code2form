//! Recursive-descent parser.
//!
//! Precedence, loosest first: `function`/`let`/`assert`/`echo` prefixes,
//! `?:`, `||`, `&&`, `== !=`, `< <= > >=`, `+ -`, `* / %`, unary `- + !`,
//! then `^`, which is right-associative and binds tighter than unary minus
//! (`-2^2` is `-(2^2)`, `2^-1` is allowed), then calls, indexing and `.`.

use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::lexer::{Tok, Token, tokenize_file};
use crate::span::Span;

/// Supplies the files named by `include <...>` while lexing.
pub trait Includes {
    /// Opens `name`, included from file `from`, and returns an id for it
    /// (used in the spans of its tokens) and its text. `None` when it cannot
    /// be opened: the include then expands to nothing, as in the reference
    /// implementation. Implementors report why, and refuse files already
    /// open, which is how include cycles are cut.
    fn open(&mut self, name: &str, from: u32) -> Option<(u32, String)>;

    /// The included file `file` has been read to its end.
    fn close(&mut self, file: u32);
}

/// Parses a file whose includes are spliced in through `includes`.
/// Diagnostics point into whichever file their span names.
pub fn parse_with_includes(src: &str, includes: &mut dyn Includes) -> Result<File, Diagnostic> {
    // Lexing recurses only per include level; parsing needs the big stack.
    let tokens = tokenize_file(src, 0, Some(includes))?;
    run_on_big_stack(|| parse_tokens(tokens))
}

/// Parses a whole file.
///
/// Parsing is recursive, so it runs on its own thread with a stack large
/// enough for [`MAX_DEPTH`] levels of nesting even in debug builds,
/// whatever the caller's stack size.
pub fn parse(src: &str) -> Result<File, Diagnostic> {
    let tokens = tokenize_file(src, 0, None)?;
    run_on_big_stack(|| parse_tokens(tokens))
}

fn run_on_big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name("c2f-parse".into())
            .stack_size(PARSE_STACK)
            .spawn_scoped(scope, f)
            .expect("spawning the parser thread")
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
    })
}

/// Reserved, not committed: only the pages actually used cost memory.
const PARSE_STACK: usize = 256 << 20;

fn parse_tokens(tokens: Vec<Token>) -> Result<File, Diagnostic> {
    let mut parser = Parser { tokens, pos: 0, depth: 0 };
    let mut stmts = Vec::new();
    while !parser.at(&Tok::Eof) {
        stmts.push(parser.stmt(Context::File)?);
    }
    Ok(File { stmts })
}

type PResult<T> = Result<T, Diagnostic>;

/// Deepest nesting of expressions, blocks or child instances, about what
/// the reference's Bison parser (YYMAXDEPTH 10000) accepts. Recursion beyond
/// it would risk overflowing the parser's stack.
pub const MAX_DEPTH: usize = 10_000;

/// Where a statement appears. `use` is only valid at the top level of the
/// file; module children may hold only assignments and instances.
#[derive(Clone, Copy, PartialEq)]
enum Context {
    File,
    Scope,
    Children,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn peek_at(&self, ahead: usize) -> &Tok {
        let last = self.tokens.len() - 1;
        &self.tokens[(self.pos + ahead).min(last)].tok
    }

    fn span(&self) -> Span {
        self.tokens[self.pos].span
    }

    fn prev_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    fn at(&self, tok: &Tok) -> bool {
        self.peek() == tok
    }

    fn bump(&mut self) -> Token {
        let token = self.tokens[self.pos].clone();
        if token.tok != Tok::Eof {
            self.pos += 1;
        }
        token
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.at(tok) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, tok: &Tok, what: &str) -> PResult<Span> {
        if self.at(tok) {
            Ok(self.bump().span)
        } else {
            Err(self.unexpected(what))
        }
    }

    fn unexpected(&self, what: &str) -> Diagnostic {
        Diagnostic::new(format!("expected {what}, found {}", self.peek().describe()), self.span())
    }

    /// Runs `f` one nesting level deeper, failing past [`MAX_DEPTH`].
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<T> {
        if self.depth >= MAX_DEPTH {
            return Err(Diagnostic::new(format!("nesting is deeper than {MAX_DEPTH} levels"), self.span()));
        }
        self.depth += 1;
        let result = f(self);
        self.depth -= 1;
        result
    }

    fn ident(&mut self) -> PResult<Ident> {
        match self.peek().clone() {
            Tok::Ident(name) => Ok(Ident { name, span: self.bump().span }),
            _ => Err(self.unexpected("an identifier")),
        }
    }

    // ---- statements ----

    fn stmt(&mut self, ctx: Context) -> PResult<Stmt> {
        self.nested(|p| p.stmt_inner(ctx))
    }

    fn stmt_inner(&mut self, ctx: Context) -> PResult<Stmt> {
        let start = self.span();
        let kind = match self.peek().clone() {
            Tok::Semi => {
                self.bump();
                StmtKind::Empty
            }
            Tok::LBrace => {
                let inner = if ctx == Context::Children { Context::Children } else { Context::Scope };
                StmtKind::Block(self.block(inner)?)
            }
            Tok::Include(path) => {
                self.bump();
                StmtKind::Include(path)
            }
            Tok::Use(_) if ctx != Context::File => {
                return Err(Diagnostic::new("`use` is only allowed at the top level of a file", self.span()));
            }
            Tok::Use(path) => {
                self.bump();
                StmtKind::Use(path)
            }
            Tok::Module | Tok::Function if ctx == Context::Children => {
                return Err(Diagnostic::new(
                    "definitions are not allowed among module children",
                    self.span(),
                ));
            }
            Tok::Module => StmtKind::Module(self.module_def()?),
            Tok::Function => StmtKind::Function(self.function_def()?),
            Tok::Ident(_) if self.peek_at(1) == &Tok::Assign => {
                let name = self.ident()?;
                self.bump();
                let value = self.expr()?;
                self.expect(&Tok::Semi, "`;` after assignment")?;
                StmtKind::Assign(Assignment { name, value })
            }
            _ => StmtKind::Instance(self.instance()?),
        };
        Ok(Stmt { kind, span: start.to(self.prev_span()) })
    }

    fn block(&mut self, ctx: Context) -> PResult<Vec<Stmt>> {
        self.expect(&Tok::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        while !self.eat(&Tok::RBrace) {
            if self.at(&Tok::Eof) {
                return Err(self.unexpected("`}`"));
            }
            stmts.push(self.stmt(ctx)?);
        }
        Ok(stmts)
    }

    fn module_def(&mut self) -> PResult<ModuleDef> {
        self.expect(&Tok::Module, "`module`")?;
        let name = self.ident()?;
        let params = self.params()?;
        let body = Box::new(self.stmt(Context::Scope)?);
        Ok(ModuleDef { name, params, body })
    }

    fn function_def(&mut self) -> PResult<FunctionDef> {
        self.expect(&Tok::Function, "`function`")?;
        let name = self.ident()?;
        let params = self.params()?;
        self.expect(&Tok::Assign, "`=` before the function body")?;
        let body = self.expr()?;
        self.expect(&Tok::Semi, "`;` after the function body")?;
        Ok(FunctionDef { name, params, body })
    }

    fn instance(&mut self) -> PResult<Instance> {
        let mut modifiers = Modifiers::default();
        loop {
            match self.peek() {
                Tok::Bang => modifiers.root = true,
                Tok::Hash => modifiers.highlight = true,
                Tok::Percent => modifiers.background = true,
                Tok::Star => modifiers.disabled = true,
                _ => break,
            }
            self.bump();
        }

        let kind = if self.eat(&Tok::If) {
            self.expect(&Tok::LParen, "`(` after `if`")?;
            let cond = self.expr()?;
            self.expect(&Tok::RParen, "`)` after the condition")?;
            let then = self.children()?;
            let otherwise = if self.eat(&Tok::Else) { Some(self.children()?) } else { None };
            InstanceKind::If { cond, then, otherwise }
        } else {
            let span = self.span();
            let name = match self.peek().clone() {
                Tok::Ident(name) => name,
                Tok::For => "for".into(),
                Tok::Let => "let".into(),
                Tok::Assert => "assert".into(),
                Tok::Echo => "echo".into(),
                Tok::Each => "each".into(),
                _ => return Err(self.unexpected("a statement")),
            };
            self.bump();
            let args = self.args()?;
            let children = self.children()?;
            InstanceKind::Call { name: Ident { name, span }, args, children }
        };
        Ok(Instance { modifiers, kind })
    }

    /// The statement after a module call or `if`: `;`, a block, or a
    /// single nested instance.
    fn children(&mut self) -> PResult<Vec<Stmt>> {
        match self.peek() {
            Tok::Semi => {
                self.bump();
                Ok(Vec::new())
            }
            Tok::LBrace => self.block(Context::Children),
            _ => {
                let start = self.span();
                let instance = self.nested(Self::instance)?;
                Ok(vec![Stmt { kind: StmtKind::Instance(instance), span: start.to(self.prev_span()) }])
            }
        }
    }

    /// `(a, b = 1)` in a definition. Stray commas are tolerated anywhere.
    fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect(&Tok::LParen, "`(` before the parameters")?;
        let mut params = Vec::new();
        loop {
            while self.eat(&Tok::Comma) {}
            if self.eat(&Tok::RParen) {
                return Ok(params);
            }
            if !params.is_empty() && !matches!(self.tokens[self.pos - 1].tok, Tok::Comma) {
                return Err(self.unexpected("`,` or `)`"));
            }
            let name = self.ident()?;
            let default = if self.eat(&Tok::Assign) { Some(self.expr()?) } else { None };
            params.push(Param { name, default });
        }
    }

    /// `(1, b = 2)` in a call. Leading and repeated commas are tolerated,
    /// a trailing one is not.
    fn args(&mut self) -> PResult<Vec<Arg>> {
        self.expect(&Tok::LParen, "`(` before the arguments")?;
        let mut args = Vec::new();
        if self.eat(&Tok::RParen) {
            return Ok(args);
        }
        while self.eat(&Tok::Comma) {}
        loop {
            args.push(self.arg()?);
            if self.eat(&Tok::RParen) {
                return Ok(args);
            }
            self.expect(&Tok::Comma, "`,` or `)`")?;
            while self.eat(&Tok::Comma) {}
        }
    }

    fn arg(&mut self) -> PResult<Arg> {
        let name = if matches!(self.peek(), Tok::Ident(_)) && self.peek_at(1) == &Tok::Assign {
            let name = self.ident()?;
            self.bump();
            Some(name)
        } else {
            None
        };
        Ok(Arg { name, value: self.expr()? })
    }

    // ---- expressions ----

    fn expr(&mut self) -> PResult<Expr> {
        self.nested(Self::expr_inner)
    }

    fn expr_inner(&mut self) -> PResult<Expr> {
        let start = self.span();
        let kind = match self.peek() {
            Tok::Function => {
                self.bump();
                let params = self.params()?;
                ExprKind::FunctionLit { params, body: Box::new(self.expr()?) }
            }
            Tok::Let => {
                self.bump();
                let args = self.args()?;
                ExprKind::Let { args, body: Box::new(self.expr()?) }
            }
            Tok::Assert | Tok::Echo => {
                let is_assert = self.bump().tok == Tok::Assert;
                let args = self.args()?;
                let body = if self.starts_expr() { Some(Box::new(self.expr()?)) } else { None };
                if is_assert { ExprKind::Assert { args, body } } else { ExprKind::Echo { args, body } }
            }
            _ => {
                let cond = self.binary(1)?;
                if !self.eat(&Tok::Question) {
                    return Ok(cond);
                }
                let then = self.expr()?;
                self.expect(&Tok::Colon, "`:` in the conditional expression")?;
                let otherwise = self.expr()?;
                ExprKind::Ternary { cond: Box::new(cond), then: Box::new(then), otherwise: Box::new(otherwise) }
            }
        };
        Ok(Expr { kind, span: start.to(self.prev_span()) })
    }

    fn starts_expr(&self) -> bool {
        matches!(
            self.peek(),
            Tok::Ident(_)
                | Tok::Number(_)
                | Tok::Str(_)
                | Tok::True
                | Tok::False
                | Tok::Undef
                | Tok::LParen
                | Tok::LBracket
                | Tok::Minus
                | Tok::Plus
                | Tok::Bang
                | Tok::Function
                | Tok::Let
                | Tok::Assert
                | Tok::Echo
        )
    }

    /// Left-associative binary operators of at least `min` precedence,
    /// by precedence climbing (one stack frame per operand, not per level).
    fn binary(&mut self, min: u8) -> PResult<Expr> {
        let mut lhs = self.unary()?;
        while let Some((op, prec)) = binary_op(self.peek()).filter(|&(_, prec)| prec >= min) {
            self.bump();
            let rhs = self.binary(prec + 1)?;
            let span = lhs.span.to(rhs.span);
            lhs = Expr { kind: ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PResult<Expr> {
        let op = match self.peek() {
            Tok::Minus => UnaryOp::Neg,
            Tok::Plus => UnaryOp::Plus,
            Tok::Bang => UnaryOp::Not,
            _ => return self.power(),
        };
        let start = self.bump().span;
        let operand = self.nested(Self::unary)?;
        let span = start.to(operand.span);
        Ok(Expr { kind: ExprKind::Unary { op, operand: Box::new(operand) }, span })
    }

    fn power(&mut self) -> PResult<Expr> {
        let base = self.postfix()?;
        if !self.eat(&Tok::Caret) {
            return Ok(base);
        }
        // `2^2^2^...` nests through here, not through `expr`.
        let exponent = self.nested(Self::unary)?;
        let span = base.span.to(exponent.span);
        Ok(Expr {
            kind: ExprKind::Binary { op: BinaryOp::Pow, lhs: Box::new(base), rhs: Box::new(exponent) },
            span,
        })
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut expr = self.primary()?;
        loop {
            let start = expr.span;
            let kind = match self.peek() {
                Tok::LParen => {
                    let args = self.args()?;
                    ExprKind::Call { callee: Box::new(expr), args }
                }
                Tok::LBracket => {
                    self.bump();
                    let index = self.expr()?;
                    self.expect(&Tok::RBracket, "`]` after the index")?;
                    ExprKind::Index { target: Box::new(expr), index: Box::new(index) }
                }
                Tok::Dot => {
                    self.bump();
                    let name = self.ident()?;
                    ExprKind::Member { target: Box::new(expr), name }
                }
                _ => return Ok(expr),
            };
            expr = Expr { kind, span: start.to(self.prev_span()) };
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let start = self.span();
        let kind = match self.peek().clone() {
            Tok::True => ExprKind::Bool(true),
            Tok::False => ExprKind::Bool(false),
            Tok::Undef => ExprKind::Undef,
            Tok::Number(n) => ExprKind::Number(n),
            Tok::Str(s) => ExprKind::Str(s),
            Tok::Ident(name) => ExprKind::Ident(name),
            Tok::LParen => {
                self.bump();
                let inner = self.expr()?;
                self.expect(&Tok::RParen, "`)`")?;
                return Ok(Expr { kind: ExprKind::Paren(Box::new(inner)), span: start.to(self.prev_span()) });
            }
            Tok::LBracket => return self.vector(),
            _ => return Err(self.unexpected("an expression")),
        };
        self.bump();
        Ok(Expr { kind, span: start })
    }

    /// `[]`, `[a : b]`, `[a : step : b]` or `[elements...]`.
    fn vector(&mut self) -> PResult<Expr> {
        let start = self.expect(&Tok::LBracket, "`[`")?;
        let finish = |p: &Self, kind| Ok(Expr { kind, span: start.to(p.prev_span()) });
        if self.eat(&Tok::RBracket) {
            return finish(self, ExprKind::Vector(Vec::new()));
        }

        // A leading `(` always opens an expression here, so `[(for ...)]`
        // is rejected, as in the reference.
        let first = if self.starts_comp() {
            Element::Comp(Box::new(self.nested(Self::comp)?))
        } else {
            Element::Expr(self.expr()?)
        };
        // `[let (a = 1) a : 5]`: a `let` with a plain value, read as a
        // comprehension, is the start of a range after all.
        let first = match first {
            Element::Comp(comp) if self.at(&Tok::Colon) => match let_as_expr(*comp) {
                Ok(expr) => Element::Expr(expr),
                Err(comp) => Element::Comp(Box::new(comp)),
            },
            other => other,
        };

        let first = match first {
            Element::Expr(first) if self.eat(&Tok::Colon) => {
                let second = self.expr()?;
                let (step, end) =
                    if self.eat(&Tok::Colon) { (Some(Box::new(second)), self.expr()?) } else { (None, second) };
                self.expect(&Tok::RBracket, "`]` after the range")?;
                return finish(self, ExprKind::Range { start: Box::new(first), step, end: Box::new(end) });
            }
            first => first,
        };

        let mut elements = vec![first];
        loop {
            if self.eat(&Tok::RBracket) {
                return finish(self, ExprKind::Vector(elements));
            }
            self.expect(&Tok::Comma, "`,` or `]`")?;
            while self.eat(&Tok::Comma) {}
            if self.eat(&Tok::RBracket) {
                return finish(self, ExprKind::Vector(elements));
            }
            elements.push(self.element()?);
        }
    }

    fn starts_comp(&self) -> bool {
        matches!(self.peek(), Tok::For | Tok::Each | Tok::If | Tok::Let)
    }

    /// A vector element or comprehension body: a comprehension, possibly
    /// parenthesized, or an expression.
    fn element(&mut self) -> PResult<Element> {
        self.nested(Self::element_inner)
    }

    fn element_inner(&mut self) -> PResult<Element> {
        if self.starts_comp() {
            return Ok(Element::Comp(Box::new(self.comp()?)));
        }
        if self.at(&Tok::LParen) && matches!(self.peek_at(1), Tok::For | Tok::Each | Tok::If | Tok::Let) {
            self.bump();
            let comp = self.comp()?;
            self.expect(&Tok::RParen, "`)`")?;
            return Ok(Element::Comp(Box::new(comp)));
        }
        Ok(Element::Expr(self.expr()?))
    }

    fn comp(&mut self) -> PResult<Comp> {
        let start = self.span();
        let kind = match self.bump().tok {
            Tok::For => {
                self.expect(&Tok::LParen, "`(` after `for`")?;
                let init = self.comp_args(&[Tok::RParen, Tok::Semi])?;
                if self.eat(&Tok::Semi) {
                    let cond = self.expr()?;
                    self.expect(&Tok::Semi, "`;` after the loop condition")?;
                    let update = self.comp_args(&[Tok::RParen])?;
                    self.expect(&Tok::RParen, "`)`")?;
                    CompKind::ForC { init, cond, update, body: self.element()? }
                } else {
                    self.expect(&Tok::RParen, "`)`")?;
                    CompKind::For { bindings: init, body: self.element()? }
                }
            }
            Tok::Each => CompKind::Each(self.element()?),
            Tok::If => {
                self.expect(&Tok::LParen, "`(` after `if`")?;
                let cond = self.expr()?;
                self.expect(&Tok::RParen, "`)` after the condition")?;
                let then = self.element()?;
                let otherwise = if self.eat(&Tok::Else) { Some(self.element()?) } else { None };
                CompKind::If { cond, then, otherwise }
            }
            Tok::Let => {
                let bindings = self.args()?;
                CompKind::Let { bindings, body: self.element()? }
            }
            _ => unreachable!("comp() is only called when starts_comp()"),
        };
        Ok(Comp { kind, span: start.to(self.prev_span()) })
    }

    /// Argument lists inside a `for (...)` header, which end at `)` or `;`
    /// instead of consuming the closing parenthesis.
    fn comp_args(&mut self, terminators: &[Tok]) -> PResult<Vec<Arg>> {
        let mut args = Vec::new();
        if terminators.iter().any(|t| self.at(t)) {
            return Ok(args);
        }
        while self.eat(&Tok::Comma) {}
        loop {
            args.push(self.arg()?);
            if terminators.iter().any(|t| self.at(t)) {
                return Ok(args);
            }
            self.expect(&Tok::Comma, "`,`")?;
            while self.eat(&Tok::Comma) {}
        }
    }
}

/// A chain of comprehension `let`s ending in a plain value, as the
/// equivalent `let` expression; anything else comes back unchanged.
fn let_as_expr(comp: Comp) -> Result<Expr, Comp> {
    match comp.kind {
        CompKind::Let { bindings, body: Element::Expr(body) } => {
            Ok(Expr { kind: ExprKind::Let { args: bindings, body: Box::new(body) }, span: comp.span })
        }
        CompKind::Let { bindings, body: Element::Comp(inner) } => match let_as_expr(*inner) {
            Ok(body) => Ok(Expr { kind: ExprKind::Let { args: bindings, body: Box::new(body) }, span: comp.span }),
            Err(inner) => Err(Comp {
                kind: CompKind::Let { bindings, body: Element::Comp(Box::new(inner)) },
                span: comp.span,
            }),
        },
        kind => Err(Comp { kind, span: comp.span }),
    }
}

fn binary_op(tok: &Tok) -> Option<(BinaryOp, u8)> {
    Some(match tok {
        Tok::OrOr => (BinaryOp::Or, 1),
        Tok::AndAnd => (BinaryOp::And, 2),
        Tok::EqEq => (BinaryOp::Eq, 3),
        Tok::Ne => (BinaryOp::Ne, 3),
        Tok::Lt => (BinaryOp::Lt, 4),
        Tok::Le => (BinaryOp::Le, 4),
        Tok::Gt => (BinaryOp::Gt, 4),
        Tok::Ge => (BinaryOp::Ge, 4),
        Tok::Plus => (BinaryOp::Add, 5),
        Tok::Minus => (BinaryOp::Sub, 5),
        Tok::Star => (BinaryOp::Mul, 6),
        Tok::Slash => (BinaryOp::Div, 6),
        Tok::Percent => (BinaryOp::Mod, 6),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_position_of_syntax_errors() {
        let err = parse("cube(1)\nsphere(;").unwrap_err();
        assert_eq!(err.span, Span::new(15, 16));
        assert!(err.message.starts_with("expected an expression"), "{}", err.message);
    }

    #[test]
    fn rejects_definitions_among_children() {
        assert!(parse("translate() { module m() {} }").is_err());
        assert!(parse("module m() { module n() {} }").is_ok());
    }

    #[test]
    fn let_starts_a_range() {
        let file = parse("v = [let (a = 1) a : 5];").unwrap();
        let StmtKind::Assign(a) = &file.stmts[0].kind else { panic!() };
        assert!(matches!(a.value.kind, ExprKind::Range { .. }), "{:?}", a.value.kind);
        assert!(parse("v = [let (a = 1) for (i = [a]) i];").is_ok());
    }

    struct Files(Vec<(&'static str, &'static str)>, Vec<u32>);

    impl Includes for Files {
        fn open(&mut self, name: &str, _from: u32) -> Option<(u32, String)> {
            let id = self.0.iter().position(|(n, _)| *n == name)? as u32 + 1;
            if self.1.contains(&id) {
                return None;
            }
            self.1.push(id);
            Some((id, self.0[id as usize - 1].1.to_string()))
        }

        fn close(&mut self, file: u32) {
            self.1.retain(|f| *f != file);
        }
    }

    #[test]
    fn includes_splice_tokens() {
        let mut files = Files(vec![("three.scad", "3 +"), ("self.scad", "cube(); include <self.scad>")], vec![]);
        let file = parse_with_includes("x = 1 + include <three.scad> 2; include <self.scad>", &mut files).unwrap();
        assert_eq!(crate::dump::ast_dump(&file, ""), "x = ((1 + 3) + 2);\ncube();\n");
        // The error is at the `;` after the included text: the includer's.
        let err = parse_with_includes("x = include <three.scad>;", &mut files).unwrap_err();
        assert_eq!((err.span.file, err.span.start), (0, 24));
        // An error inside the included text points into that file.
        let err = parse_with_includes("include <three.scad>", &mut files).unwrap_err();
        assert_eq!(err.span.file, 1);
    }

    #[test]
    fn use_only_at_file_level() {
        assert!(parse("use <a.scad>\ncube();").is_ok());
        assert!(parse("{ use <a.scad> }").is_err());
        assert!(parse("module m() { use <a.scad> cube(); }").is_err());
        assert!(parse("module m() { include <a.scad> cube(); }").is_ok());
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        let nest = |open: &str, close: &str, n: usize| format!("x = {}1{};", open.repeat(n), close.repeat(n));
        // Printing a tree this deep recurses: do it on a big stack.
        let deep = nest("(", ")", MAX_DEPTH - 2);
        let dumped = std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || crate::dump::ast_dump(&parse(&deep).unwrap(), &deep))
            .unwrap()
            .join()
            .unwrap();
        assert!(dumped.ends_with("x = 1;\n"));
        // A long flat chain parses in a loop and must also drop without
        // recursing, here on the test thread's small stack.
        drop(parse(&format!("x = {};", vec!["1"; 200_000].join(" + "))).unwrap());
        for (open, close) in [("(", ")"), ("[", "]"), ("-", ""), ("!", ""), ("[for(i=[1]) ", "]")] {
            let err = parse(&nest(open, close, 100_000)).unwrap_err();
            assert!(err.message.contains("nesting"), "{open}: {}", err.message);
        }
        let blocks = format!("{}cube();{}", "{".repeat(100_000), "}".repeat(100_000));
        assert!(parse(&blocks).unwrap_err().message.contains("nesting"));
        let chain = format!("{}cube();", "translate() ".repeat(100_000));
        assert!(parse(&chain).unwrap_err().message.contains("nesting"));
    }

    #[test]
    fn comma_rules() {
        assert!(parse("m(,1);").is_ok());
        assert!(parse("m(1,);").is_err());
        assert!(parse("v = f(,);").is_err());
        assert!(parse("module m(,a,,b,){}").is_ok());
        assert!(parse("v = [1,,2,,];").is_ok());
        assert!(parse("v = [,1];").is_err());
    }
}
