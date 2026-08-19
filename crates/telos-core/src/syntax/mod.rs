//! `.tel` syntax layer: the lexer (Task 4) and the recursive-descent
//! parser (Tasks 5-6) built on top of it.

pub(crate) mod lexer;
mod parser;

pub use parser::{
    parse_bindings_file, parse_constraint_file, parse_expr, parse_intent_file, parse_notion_file,
};
