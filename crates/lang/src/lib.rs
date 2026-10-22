//! The .scad language: lexer, parser, AST and an AST dump compatible with
//! the reference implementation, used to check this parser against it.

pub mod annotations;
pub mod ast;
pub mod customizer;
pub mod diagnostic;
pub mod dump;
pub mod lexer;
pub mod parser;
pub mod scope;
pub mod span;

pub use diagnostic::Diagnostic;
pub use parser::{Includes, parse, parse_with_includes};
pub use span::Span;
