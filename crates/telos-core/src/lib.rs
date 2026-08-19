pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod emit;
pub mod error;
pub mod ids;
pub mod model;
pub mod span;
pub mod suggest;
pub mod syntax;
