//! `.tel` syntax layer: the lexer and the recursive-descent parser built on
//! top of it, including the change-file rule (`parse_change_file`), which
//! nests the block rules for the entities its ops carry.

pub(crate) mod lexer;
mod parser;

pub use parser::{
    parse_bindings_file, parse_capability_file, parse_change_file, parse_constraint_file,
    parse_context_file, parse_context_map_file, parse_expr, parse_intent_file, parse_notion_file,
    parse_owned_constraint_file, parse_owned_intent_file, parse_owned_notion_file,
};
