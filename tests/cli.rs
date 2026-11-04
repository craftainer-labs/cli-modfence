//! The command line: in-process through `modfence::run`, and through the built
//! binary for what depends on the working directory.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A stream that refuses every write.
struct Broken;

impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The result of one in-process run.
struct Run {
    code: u8,
    out: String,
    err: String,
}

fn exit_code(code: std::process::ExitCode) -> u8 {
    (0..=255)
        .find(|candidate| code == std::process::ExitCode::from(*candidate))
        .expect("an exit code")
}

fn exec(args: &[&str]) -> Run {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = modfence::run(args.iter().map(OsString::from), &mut out, &mut err);
    Run {
        code: exit_code(code),
        out: String::from_utf8(out).unwrap(),
        err: String::from_utf8(err).unwrap(),
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cli")
}

/// `args` followed by `--config` for the fixture `name`.
fn on(name: &str, args: &[&str]) -> Run {
    let config = fixtures().join(name).join("modfence.toml");
    let mut all: Vec<&str> = args.to_vec();
    all.push("--config");
    all.push(config.to_str().unwrap());
    exec(&all)
}

fn binary(args: &[&str], directory: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_modfence"))
        .args(args)
        .current_dir(directory)
        .output()
        .expect("the binary runs")
}

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("modfence-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn prints_help_and_version_on_stdout() {
    let help = exec(&["--help"]);
    assert_eq!(help.code, 0);
    assert!(help.out.contains("Usage: modfence"));
    let version = exec(&["--version"]);
    assert_eq!(version.code, 0);
    assert!(version.out.starts_with("modfence "));
    assert!(exec(&["check", "--help"]).out.contains("--fail-on"));
}

#[test]
fn bad_usage_is_exit_2_on_stderr() {
    for args in [
        &[][..],
        &["--nope"],
        &["check", "--format", "xml"],
        &["trace"],
    ] {
        let run = exec(args);
        assert_eq!(run.code, 2, "{args:?}");
        assert_eq!(run.out, "");
        assert!(run.err.starts_with("error:"), "{args:?}: {}", run.err);
    }
}

#[test]
fn check_reports_violations_and_exits_1() {
    let run = on("app", &["check"]);
    assert_eq!(run.code, 1);
    assert!(run.out.contains("ui/view.py:2:"), "{}", run.out);
    assert!(run.out.contains("[no-db-from-ui]"));
    assert!(
        run.out
            .contains("modfence: 1 error(s), 0 warning(s), 0 note(s)")
    );
    assert!(run.err.contains("note: call edges were not checked"));
    assert!(run.err.contains("warning: web/broken.ts"), "{}", run.err);
}

#[test]
fn the_walker_honours_exclude_and_skips_modfence() {
    let run = on("app", &["check"]);
    // ui/vendor and ui/.modfence would each add a violation if they were read.
    assert!(run.out.contains("1 error(s)"), "{}", run.out);
    // Python, TypeScript and nothing else: the Rust file is not read.
    let graph = on("app", &["graph"]);
    assert!(graph.out.contains("languages: py, ts\n"), "{}", graph.out);
    assert!(!graph.out.contains("core/lib.rs"));
}

#[test]
fn fail_on_picks_the_lowest_failing_severity() {
    assert_eq!(on("warn", &["check"]).code, 0);
    assert_eq!(on("warn", &["check", "--fail-on", "error"]).code, 0);
    assert_eq!(on("warn", &["check", "--fail-on", "warning"]).code, 1);
    assert_eq!(on("warn", &["check", "--fail-on", "note"]).code, 1);
    let warned = on("warn", &["check"]);
    assert!(warned.out.contains("warning [no-b]"), "{}", warned.out);
}

#[test]
fn a_clean_repository_exits_0() {
    let run = on("clean", &["check"]);
    assert_eq!(run.code, 0);
    assert!(run.out.contains("0 error(s), 0 warning(s), 0 note(s)"));
}

#[test]
fn machine_formats_need_an_explicit_format() {
    let sarif = on("app", &["check", "--format", "sarif"]);
    assert_eq!(sarif.code, 1);
    assert!(sarif.out.contains("\"version\": \"2.1.0\""));
    assert!(sarif.out.contains("no-db-from-ui"));
    let github = on("app", &["check", "--format", "github"]);
    assert_eq!(github.code, 1);
    assert!(github.out.contains("::error file=ui/view.py,line=2"));
}

#[test]
fn output_goes_to_a_file_when_asked() {
    let directory = scratch("output");
    let file = directory.join("report.sarif");
    let target = file.to_str().unwrap();
    let run = on("app", &["check", "--format", "sarif", "--output", target]);
    assert_eq!(run.code, 1);
    assert_eq!(run.out, "");
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("no-db-from-ui")
    );
    let unwritable = on("app", &["check", "--output", directory.to_str().unwrap()]);
    assert_eq!(unwritable.code, 2);
    assert!(
        unwritable.err.contains("cannot write"),
        "{}",
        unwritable.err
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn graph_prints_a_text_summary() {
    let run = on("app", &["graph"]);
    assert_eq!(run.code, 0);
    assert!(run.out.starts_with("modfence graph\nlanguages: py, ts\n"));
    assert!(run.out.contains("nodes: "));
    assert!(run.out.contains("edges: "));
    assert!(run.out.contains("diagnostics: "), "{}", run.out);
    assert_eq!(on("missing", &["graph"]).code, 2);
}

#[test]
fn what_the_output_layer_lacks_is_a_usage_error() {
    for format in ["json", "dot", "mermaid", "graphml", "html"] {
        for command in [&["graph"][..], &["trace", "layer=ui"]] {
            let mut args = command.to_vec();
            args.extend(["--format", format]);
            let run = on("app", &args);
            assert_eq!(run.code, 2, "{format}");
            assert_eq!(
                run.err,
                format!("error: --format {format} is not implemented yet (NC5)\n")
            );
        }
    }
    let group = on("app", &["graph", "--group-by", "layer"]);
    assert_eq!(group.code, 2);
    assert!(
        group
            .err
            .contains("--group-by is not implemented yet (NC5)")
    );
    for level in ["package", "module", "symbol"] {
        let run = on("app", &["trace", "layer=ui", "--level", level]);
        assert_eq!(run.code, 2);
        assert!(run.err.contains("--level is not implemented yet (NC5)"));
    }
}

#[test]
fn trace_prints_matching_nodes_and_their_edges() {
    let run = on("app", &["trace", "layer=ui"]);
    assert_eq!(run.code, 0);
    assert!(run.out.starts_with("nodes: 1\n  mod:py:"), "{}", run.out);
    assert!(run.out.contains("view.py  layer=ui\n"), "{}", run.out);
    assert!(
        run.out.contains("store.py  imports resolved"),
        "{}",
        run.out
    );
    let either = on("app", &["trace", "layer=ui,data", "lang=py"]);
    assert!(
        either.out.contains("store.py  layer=data\n"),
        "{}",
        either.out
    );
    let none = on("app", &["trace", "owner=\"x,y\""]);
    assert_eq!(none.code, 0);
    assert!(none.out.starts_with("nodes: 0\nedges: 0\n"), "{}", none.out);
    let external = on("app", &["trace", "kind=external"]);
    assert!(!external.out.starts_with("nodes: 0"));
}

#[test]
fn trace_rejects_malformed_selectors() {
    for (selector, message) in [
        (&["nokey"][..], "is not a key=value selector"),
        (&["layer=\"a"], "unterminated double quote"),
        (&["layer=a", "layer=b"], "`layer` is given twice"),
        (&["Layer=a"], "selector key"),
        (&["lang=java"], "unknown language"),
        (&["layer="], "must not be empty"),
    ] {
        let run = on("app", &[&["trace"][..], selector].concat());
        assert_eq!(run.code, 2, "{selector:?}");
        assert!(run.err.contains(message), "{selector:?}: {}", run.err);
    }
}

#[test]
fn configuration_errors_are_exit_2() {
    let missing = exec(&["check", "--config", "/nonexistent/modfence.toml"]);
    assert_eq!(missing.code, 2);
    assert!(
        missing
            .err
            .contains("cannot read /nonexistent/modfence.toml")
    );
    let directory = scratch("bad-config");
    let file = directory.join("modfence.toml");
    std::fs::write(&file, "version = 1\nunknown = true\n").unwrap();
    let bad = exec(&["check", "--config", file.to_str().unwrap()]);
    assert_eq!(bad.code, 2);
    assert!(bad.err.contains("modfence.toml:2:1"), "{}", bad.err);
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn broken_streams_exit_74() {
    let config = fixtures().join("app/modfence.toml");
    let config = config.to_str().unwrap();
    let run = |args: &[&str], out: &mut Broken, err: &mut Broken| {
        modfence::run(args.iter().map(OsString::from), out, err)
    };
    let broken = std::process::ExitCode::from(74);
    // Help on stdout, a usage error and a failure on stderr.
    assert_eq!(run(&["--help"], &mut Broken, &mut Broken), broken);
    assert_eq!(run(&[], &mut Broken, &mut Broken), broken);
    assert_eq!(
        run(
            &["check", "--config", "/nonexistent"],
            &mut Broken,
            &mut Broken
        ),
        broken
    );
    // The report, and the notices and diagnostics before it.
    assert_eq!(
        run(&["graph", "--config", config], &mut Broken, &mut Broken),
        broken
    );
    let mut sink = Vec::new();
    let mut err = Broken;
    let code = modfence::run(
        ["check", "--config", config, "--format", "sarif"].map(OsString::from),
        &mut sink,
        &mut err,
    );
    assert_eq!(code, broken);
    let code = modfence::run(
        ["check", "--config", config].map(OsString::from),
        &mut Broken,
        &mut Vec::new(),
    );
    assert_eq!(code, broken);
}

#[test]
fn the_binary_finds_modfence_toml_from_the_working_directory() {
    let nested = fixtures().join("app/ui");
    let found = binary(&["check"], &nested);
    assert_eq!(found.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&found.stdout).contains("[no-db-from-ui]"));
    // A relative --config: the analysis root is its directory.
    let relative = binary(
        &["check", "--config", "modfence.toml"],
        &fixtures().join("warn"),
    );
    assert_eq!(relative.status.code(), Some(0));
}

#[test]
fn without_a_modfence_toml_the_working_directory_is_analysed() {
    let directory = scratch("no-config");
    std::fs::write(directory.join("a.py"), "import os\n").unwrap();
    let run = binary(&["graph"], &directory);
    assert_eq!(run.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&run.stdout).contains("languages: py\n"));
    std::fs::remove_dir_all(&directory).unwrap();
}
