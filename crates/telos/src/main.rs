//! The `telos` binary: a thin shell around `telos-core`. Parsing lives in
//! [`cli`], the answer shape in [`envelope`], output in [`render`], and the
//! work itself in [`commands`] -- which is really `telos-core`'s work,
//! arranged for a command line.

mod ci;
mod cli;
mod commands;
mod envelope;
mod projection;
mod render;
mod view;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}
