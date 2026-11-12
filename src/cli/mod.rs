//! The command line: argument parsing, the three subcommands (`check`,
//! `graph`, `trace`), the file walker and the exit codes.
//!
//! [`run`] takes the arguments and the output streams, so the whole program is
//! testable without spawning a process.

mod args;
mod commands;
mod precise;
mod text;
mod trace;
mod walker;

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use clap::Parser;

use self::args::Cli;

/// Nothing at or above `--fail-on` was found.
const OK: u8 = 0;
/// A usage or configuration error (ADR 0004).
const USAGE: u8 = 2;
/// A stream or file could not be read or written (`EX_IOERR`).
const IO_ERROR: u8 = 74;

/// Why a command stopped: the exit status and the message for standard error.
#[derive(Debug)]
struct Failure {
    code: u8,
    message: String,
}

impl Failure {
    /// A usage or configuration error.
    fn usage(message: impl Into<String>) -> Self {
        Self {
            code: USAGE,
            message: message.into(),
        }
    }

    /// A file or stream that could not be read or written, naming `what`.
    fn io(what: impl std::fmt::Display, error: &std::io::Error) -> Self {
        Self {
            code: IO_ERROR,
            message: format!("{what}: {error}"),
        }
    }
}

/// A write to standard output (or error) that failed.
impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Self::io("cannot write output", &error)
    }
}

/// Runs the program with `args` (without the program name), writing to `out`
/// and `err`, and returns the process exit code: 0 when nothing is at or above
/// `--fail-on`, 1 when something is, 2 for a usage or configuration error and
/// 74 when a file or stream cannot be read or written.
pub fn run<I>(args: I, out: &mut impl Write, err: &mut impl Write) -> ExitCode
where
    I: IntoIterator<Item = OsString>,
{
    let program = std::iter::once(OsString::from("modfence"));
    let cli = match Cli::try_parse_from(program.chain(args)) {
        Ok(cli) => cli,
        Err(error) => {
            let shown = error.render().to_string();
            return if error.use_stderr() {
                finish(&write!(err, "{shown}"), USAGE)
            } else {
                finish(&write!(out, "{shown}"), OK)
            };
        }
    };
    match commands::execute(&cli, out, err) {
        Ok(code) => ExitCode::from(code),
        Err(failure) => finish(&writeln!(err, "error: {}", failure.message), failure.code),
    }
}

/// Maps a write result to `code`, or to 74 (`EX_IOERR`) when the stream broke.
fn finish(written: &std::io::Result<()>, code: u8) -> ExitCode {
    match written {
        Ok(()) => ExitCode::from(code),
        Err(_) => ExitCode::from(IO_ERROR),
    }
}
