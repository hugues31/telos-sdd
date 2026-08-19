//! `.tel` syntax layer: the lexer (Task 4) and the recursive-descent
//! parser (Tasks 5-6) built on top of it.

pub(crate) mod lexer;
mod parser;

pub use parser::{parse_expr, parse_notion_file};
