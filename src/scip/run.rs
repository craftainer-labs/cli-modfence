//! Running SCIP indexers for `--run-indexers` (ADR 0003, invariants 16 to 18).
//!
//! The only place modfence starts another program. An indexer is looked up on
//! `PATH` by this code (absolute entries only), started directly with an
//! argument vector (never through a shell) and with modfence's environment, and
//! the command line is shown before it runs. Nothing here downloads, installs
//! or resolves anything (NFR-0002).

use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::extract::Lang;

/// One indexer to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The language it indexes.
    pub lang: Lang,
    /// The executable to look up on `PATH`.
    pub program: &'static str,
    /// The arguments, without the program.
    pub args: Vec<OsString>,
    /// The index it is asked to write.
    pub output: PathBuf,
}

/// The indexers to run for `languages`, in language order. Each runs with the
/// analysis root `root` (absolute) as its working directory and writes below
/// `out_dir`. `project_name` is for `scip-python` and `has_tsconfig` says
/// whether `scip-typescript` can find a `tsconfig.json`; without one it is asked
/// to infer it.
///
/// The flags are those of ADR 0003's table, checked against the real indexers
/// (`tests/fixtures/scip/README.md` records the versions and the evidence).
#[must_use]
pub fn plan(
    languages: &BTreeSet<Lang>,
    root: &Path,
    out_dir: &Path,
    project_name: &str,
    has_tsconfig: bool,
) -> Vec<Invocation> {
    languages
        .iter()
        .map(|&lang| {
            let (program, file, mut args): (&'static str, &str, Vec<OsString>) = match lang {
                Lang::Py => (
                    "scip-python",
                    "python.scip",
                    vec![
                        "index".into(),
                        ".".into(),
                        format!("--project-name={project_name}").into(),
                        // Without a version scip-python takes the git revision,
                        // and crashes outside a git checkout.
                        "--project-version=0.0.0".into(),
                    ],
                ),
                Lang::Ts => {
                    let mut args = vec![OsString::from("index")];
                    if !has_tsconfig {
                        args.push("--infer-tsconfig".into());
                    }
                    ("scip-typescript", "typescript.scip", args)
                }
                Lang::Rs => (
                    "rust-analyzer",
                    "rust.scip",
                    vec!["scip".into(), root.as_os_str().to_owned()],
                ),
            };
            let output = out_dir.join(file);
            args.push("--output".into());
            args.push(output.as_os_str().to_owned());
            Invocation {
                lang,
                program,
                args,
                output,
            }
        })
        .collect()
}

/// The project name for `scip-python`: `project.name` of the root's
/// `pyproject.toml`, else the root directory's name. Only letters, digits, `.`,
/// `_` and `-` pass (the name is repository-controlled and ends up on a command
/// line and in the terminal); anything else is replaced or falls back.
#[must_use]
pub fn project_name(root: &Path) -> String {
    let declared = std::fs::read_to_string(root.join("pyproject.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|table| {
            table
                .get("project")?
                .get("name")?
                .as_str()
                .map(str::to_owned)
        });
    let allowed = |c: char| c.is_ascii_alphanumeric() || "._-".contains(c);
    let usable =
        |name: &str| !name.is_empty() && !name.starts_with(['-', '.']) && name.chars().all(allowed);
    if let Some(name) = declared.filter(|name| usable(name)) {
        return name;
    }
    let directory: String = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| if allowed(c) { c } else { '_' })
        .collect();
    if usable(&directory) {
        directory
    } else {
        "project".to_owned()
    }
}

/// The executable `program` that `path` (a `PATH` value) finds first: an
/// executable regular file in an absolute entry. Relative and empty entries are
/// skipped, so a program planted in the analysed checkout is never run by
/// accident.
#[must_use]
pub fn resolve(program: &str, path: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path)
        .filter(|directory| directory.is_absolute())
        .map(|directory| directory.join(program))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// `exe` and `args` as one line to show the user: arguments with anything but
/// plain characters are single-quoted (a quote is written `'\''`), and control
/// characters appear as escapes, so the line is unambiguous in a terminal.
#[must_use]
pub fn command_line(exe: &Path, args: &[OsString]) -> String {
    std::iter::once(exe.as_os_str())
        .chain(args.iter().map(OsString::as_os_str))
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(argument: &OsStr) -> String {
    let text = argument.to_string_lossy();
    let plain = |c: char| c.is_ascii_alphanumeric() || "_-./=:,@%+".contains(c);
    if !text.is_empty() && text.chars().all(plain) {
        return text.into_owned();
    }
    let mut quoted = String::from("'");
    for c in text.chars() {
        match c {
            '\'' => quoted.push_str("'\\''"),
            c if c.is_control() => quoted.extend(c.escape_default()),
            c => quoted.push(c),
        }
    }
    quoted.push('\'');
    quoted
}

/// Runs `exe` with `args` in `cwd` and waits. No shell, no timeout; the
/// environment is modfence's own, standard input is closed, and the indexer's
/// standard output goes to standard error so it cannot mix into a report.
///
/// # Errors
///
/// The operating system's error when the program cannot be started.
pub fn spawn(exe: &Path, args: &[OsString], cwd: &Path) -> std::io::Result<ExitStatus> {
    Command::new(exe)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::from(std::io::stderr()))
        .status()
}
