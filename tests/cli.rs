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
        assert!(!run.err.is_empty(), "{args:?}");
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
    assert_eq!(unwritable.code, 74);
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
    assert_eq!(on("missing", &["graph"]).code, 74);
}

// DOT, Mermaid and GraphML of the `json` fixture. The diagrams draw the module
// view; GraphML is a data format and carries the whole graph.
#[test]
fn graph_matches_the_golden_file_of_each_format() {
    let goldens = [
        ("dot", include_str!("golden/graph.dot")),
        ("mermaid", include_str!("golden/graph.mmd")),
        ("graphml", include_str!("golden/graph.graphml")),
    ];
    for (format, golden) in goldens {
        let run = on("json", &["graph", "--format", format]);
        assert_eq!(run.code, 0, "{format}: {}", run.err);
        assert_eq!(run.out, golden, "{format}");
        assert_eq!(run.err, "");
    }
}

// ADR 0004, invariant 23: the diagram of a trace is the selection, its edges
// and their other ends.
#[test]
fn trace_matches_the_golden_file_of_each_format() {
    let goldens = [
        ("dot", include_str!("golden/trace.dot")),
        ("mermaid", include_str!("golden/trace.mmd")),
        ("graphml", include_str!("golden/trace.graphml")),
    ];
    for (format, golden) in goldens {
        let run = on("json", &["trace", "package=requests", "--format", format]);
        assert_eq!(run.code, 0, "{format}: {}", run.err);
        assert_eq!(run.out, golden, "{format}");
    }
}

#[test]
fn the_diagrams_follow_the_level_and_graphml_does_not() {
    let package = on("json", &["graph", "--format", "dot", "--level", "package"]);
    assert_eq!(
        package.out,
        "digraph \"modfence\" {\n  rankdir=LR;\n  n0 [id=\"ext:py:requests\", label=\"requests\", shape=diamond];\n  n1 [id=\"pkg:py:pkg\", label=\"pkg\", shape=box];\n  n1 -> n0 [style=solid, arrowhead=normal];\n}\n"
    );
    let package = on(
        "json",
        &["graph", "--format", "mermaid", "--level", "package"],
    );
    assert_eq!(
        package.out,
        "flowchart LR\n  n0{\"requests\"}\n  n1[\"pkg\"]\n  n1 --> n0\n"
    );
    let symbol = on("json", &["graph", "--format", "dot", "--level", "symbol"]);
    assert!(
        symbol
            .out
            .contains("\"sym:py:pkg/orders.py#place\" [label=\"place\", shape=triangle];")
    );
    let golden = include_str!("golden/graph.graphml");
    for level in ["package", "module", "symbol"] {
        let flags = ["--level", level, "--group-by", "layer"];
        let run = on(
            "json",
            &[&["graph", "--format", "graphml"][..], &flags].concat(),
        );
        assert_eq!(run.out, golden, "{level}");
    }
}

#[test]
fn diagrams_cluster_by_a_tag_key() {
    let run = on("json", &["graph", "--format", "dot", "--group-by", "layer"]);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(
        run.out
            .contains("  subgraph cluster_0 {\n    label=\"domain\";\n")
    );
    let run = on(
        "json",
        &["graph", "--format", "mermaid", "--group-by", "boundary"],
    );
    assert!(
        run.out
            .contains("  subgraph c0[\"network\"]\n    n0{\"requests\"}\n  end\n")
    );
}

#[test]
fn graph_and_trace_write_the_html_page() {
    let graph = on("app", &["graph", "--format", "html"]);
    assert_eq!(graph.code, 0);
    assert!(
        graph
            .out
            .starts_with("<!doctype html>\n<html lang=\"en\">\n")
    );
    assert!(graph.out.ends_with("</html>\n"));
    assert!(
        graph
            .out
            .contains("Showing packages, modules and external packages")
    );
    assert!(graph.out.contains("(grouped by package)."));
    let grouped = on(
        "app",
        &[
            "graph",
            "--format",
            "html",
            "--level",
            "package",
            "--group-by",
            "layer",
        ],
    );
    assert_eq!(grouped.code, 0);
    assert!(
        grouped
            .out
            .contains("Showing packages and external packages")
    );
    assert!(grouped.out.contains("(grouped by tag layer)."));
    let traced = on("app", &["trace", "layer=data", "--format", "html"]);
    assert_eq!(traced.code, 0);
    assert!(traced.out.starts_with("<!doctype html>"));
    assert!(traced.out.contains("data-id=\"mod:"));
    assert!(traced.out.contains("layer: data"));
    let directory = scratch("html-output");
    let file = directory.join("graph.html");
    let written = on(
        "app",
        &[
            "graph",
            "--format",
            "html",
            "--output",
            file.to_str().unwrap(),
        ],
    );
    assert_eq!((written.code, written.out.as_str()), (0, ""));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), graph.out);
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn level_and_group_by_are_validated_and_leave_the_text_alone() {
    let plain = on("app", &["graph"]);
    for level in ["package", "module", "symbol"] {
        let run = on("app", &["graph", "--level", level, "--group-by", "layer"]);
        assert_eq!(run.code, 0, "{level}");
        assert_eq!(run.out, plain.out);
    }
    let traced = on("app", &["trace", "layer=ui", "--level", "package"]);
    assert_eq!(traced.out, on("app", &["trace", "layer=ui"]).out);
    for args in [
        &["graph", "--level", "file"][..],
        &["graph", "--group-by", "Layer"],
        &["graph", "--group-by", "path"],
        &["trace", "layer=ui", "--group-by", ""],
    ] {
        let run = on("app", args);
        assert_eq!(run.code, 2, "{args:?}");
        assert!(run.err.starts_with("error:"), "{}", run.err);
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
        (&["layer="], "`layer=` has an empty value"),
        (&["layer=\"\""], "has an empty value"),
        (&["layer=a,"], "has an empty value"),
        (&["layer=,a"], "has an empty value"),
        (&["layer=a,,b"], "has an empty value"),
        (&["path="], "has an empty value"),
        (&["layer=a\"b\""], "does not wrap a whole value"),
        (&["layer=\"a\"b"], "does not wrap a whole value"),
        (&["layer=a,b\"c"], "does not wrap a whole value"),
    ] {
        let run = on("app", &[&["trace"][..], selector].concat());
        assert_eq!(run.code, 2, "{selector:?}");
        assert!(run.err.contains(message), "{selector:?}: {}", run.err);
    }
}

#[test]
fn an_unreadable_configuration_is_exit_74_and_an_invalid_one_exit_2() {
    let missing = exec(&["check", "--config", "/nonexistent/modfence.toml"]);
    assert_eq!(missing.code, 74);
    assert!(
        missing
            .err
            .contains("error: cannot read /nonexistent/modfence.toml: "),
        "{}",
        missing.err
    );
    let directory = scratch("bad-config");
    let file = directory.join("modfence.toml");
    std::fs::write(&file, "version = 1\nunknown = true\n").unwrap();
    let bad = exec(&["check", "--config", file.to_str().unwrap()]);
    assert_eq!(bad.code, 2);
    assert!(bad.err.contains("modfence.toml:2:1"), "{}", bad.err);
    // Not UTF-8: the file cannot be read as text, which is an I/O error.
    std::fs::write(&file, [0xff, 0xfe, 0x00]).unwrap();
    let unreadable = exec(&["check", "--config", file.to_str().unwrap()]);
    assert_eq!(unreadable.code, 74);
    assert!(unreadable.err.contains("cannot read"), "{}", unreadable.err);
    // The same for a configuration found by walking up.
    let found = binary(&["check"], &directory);
    assert_eq!(found.status.code(), Some(74));
    assert!(String::from_utf8_lossy(&found.stderr).contains("cannot read"));
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
    // A failed write to standard output names what failed.
    let mut stderr = Vec::new();
    let code = modfence::run(
        ["graph", "--config", config].map(OsString::from),
        &mut Broken,
        &mut stderr,
    );
    assert_eq!(code, broken);
    let shown = String::from_utf8(stderr).unwrap();
    assert!(shown.contains("error: cannot write output: "), "{shown}");
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

/// The JSON of `text` with the version, which is stamped at release time,
/// replaced.
fn unversioned(text: &str) -> serde_json::Value {
    let mut document: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        document["graph"]["metadata"]["tool"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    document["graph"]["metadata"]["tool"]["version"] = "0.0.0".into();
    document
}

// ADR 0003, "Output format": the graph document. The golden file is
// validated against schema/modfence-graph.schema.json by the `graph-json-schema`
// hook, so equality with it is schema validity.
#[test]
fn graph_json_matches_the_golden_file_and_ignores_the_view_flags() {
    let golden = unversioned(include_str!("golden/json.graph.json"));
    let run = on("json", &["graph", "--format", "json"]);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(run.out.ends_with("}\n"));
    assert_eq!(unversioned(&run.out), golden, "{}", run.out);
    for level in ["package", "module", "symbol"] {
        let flags = ["--level", level, "--group-by", "layer"];
        let narrowed = on(
            "json",
            &[&["graph", "--format", "json"][..], &flags].concat(),
        );
        assert_eq!(narrowed.out, run.out, "{level}");
    }
    assert_eq!(run.err, "");
}

// ADR 0004, invariant 23: the same document, restricted to the matching nodes,
// their edges and the edges' other ends.
#[test]
fn trace_json_is_the_graph_document_of_the_selection() {
    let golden = unversioned(include_str!("golden/json-trace.graph.json"));
    let run = on("json", &["trace", "package=requests", "--format", "json"]);
    assert_eq!(run.code, 0, "{}", run.err);
    assert_eq!(unversioned(&run.out), golden, "{}", run.out);
    let none = on("json", &["trace", "layer=nope", "--format", "json"]);
    let empty = unversioned(&none.out);
    assert_eq!(empty["graph"]["nodes"], serde_json::json!({}));
    assert_eq!(empty["graph"]["edges"], serde_json::json!([]));
}

#[test]
fn json_goes_to_a_file_when_asked() {
    let directory = scratch("json-output");
    let file = directory.join("graph.json");
    let target = file.to_str().unwrap();
    let run = on("json", &["graph", "--format", "json", "--output", target]);
    assert_eq!(run.code, 0);
    assert_eq!(run.out, "");
    let written = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        unversioned(&written),
        unversioned(include_str!("golden/json.graph.json"))
    );
    std::fs::remove_dir_all(&directory).unwrap();
}

// ADR 0003, invariant 6, and ADR 0004: `package` means the same in `trace` as
// in a rule of modfence.toml, so it never matches an import that could not be
// placed.
#[test]
fn trace_judges_a_node_like_a_rule_does() {
    let both = on("trace", &["trace", "kind=external"]);
    assert!(both.out.starts_with("nodes: 2\n"), "{}", both.out);
    assert!(both.out.contains("missing"), "{}", both.out);
    let packages = on("trace", &["trace", "kind=external", "package=**"]);
    assert!(packages.out.starts_with("nodes: 1\n"), "{}", packages.out);
    assert!(packages.out.contains("fs"), "{}", packages.out);
    assert!(!packages.out.contains("missing"), "{}", packages.out);
}

#[cfg(unix)]
mod unreadable {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::{binary, scratch};

    /// Makes `path` a directory nobody can list; `false` when the process can
    /// list it anyway (it runs as root), so the test has nothing to show.
    fn lock(path: &Path) -> bool {
        std::fs::create_dir_all(path).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o0)).unwrap();
        if std::fs::read_dir(path).is_ok() {
            unlock(path);
            return false;
        }
        true
    }

    fn unlock(path: &Path) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    // The walker does not read an excluded directory, and names the nested
    // directory it fails on.
    #[test]
    fn the_walker_prunes_excluded_directories_and_names_the_failing_one() {
        let root = scratch("walk");
        std::fs::write(
            root.join("modfence.toml"),
            "version = 1\nexclude = [\"vendor/\"]\n",
        )
        .unwrap();
        std::fs::write(root.join("a.py"), "x = 1\n").unwrap();
        let vendor = root.join("deep/vendor");
        if !lock(&vendor) {
            std::fs::remove_dir_all(&root).unwrap();
            return;
        }
        let pruned = binary(&["graph"], &root);
        assert_eq!(pruned.status.code(), Some(0), "{pruned:?}");
        unlock(&vendor);
        let hidden = root.join("deep/hidden");
        assert!(lock(&hidden));
        let failed = binary(&["graph"], &root);
        assert_eq!(failed.status.code(), Some(74));
        let message = String::from_utf8_lossy(&failed.stderr).into_owned();
        assert!(message.contains("cannot walk "), "{message}");
        assert!(message.contains("deep/hidden"), "{message}");
        unlock(&hidden);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
