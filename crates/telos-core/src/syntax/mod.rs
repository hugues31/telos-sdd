//! `.tel` syntax layer: the lexer (M1 Task 4) and the recursive-descent
//! parser (M1 Tasks 5-6) built on top of it, extended in M2 with the
//! change-file rule of Annex C (`parse_change_file`), which nests the M1
//! block rules for the entities its ops carry.

pub(crate) mod lexer;
mod parser;

pub use parser::{
    parse_bindings_file, parse_change_file, parse_constraint_file, parse_expr, parse_intent_file,
    parse_notion_file,
};
