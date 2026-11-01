//! Entry point: forwards the process arguments and standard streams to
//! [`modfence::run`], which holds all the logic.

use std::process::ExitCode;

fn main() -> ExitCode {
    modfence::run(
        std::env::args_os().skip(1),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    )
}
