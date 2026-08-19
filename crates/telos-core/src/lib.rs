pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod config;
pub mod emit;
pub mod error;
pub mod git;
pub mod graph;
pub mod ids;
pub mod lock;
pub mod model;
pub mod semantic;
pub mod span;
pub mod state;
pub mod suggest;
pub mod syntax;
pub mod workspace;
