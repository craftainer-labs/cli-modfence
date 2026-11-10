//! The command line: in-process through `modfence::run`, and through the built
//! binary for what depends on the working directory.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use protobuf::{EnumOrUnknown, Message, MessageField};
use scip::types::{Document, Index, Metadata, Occurrence, PositionEncoding, ToolInfo};

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

// ADR 0004, invariant 21: the exit status does not depend on the output format.
#[test]
fn the_exit_status_is_the_same_for_every_format() {
    for (fixture, extra, expected) in [
        ("app", &[][..], 1),
        ("clean", &[][..], 0),
        ("warn", &[][..], 0),
        ("warn", &["--fail-on", "warning"][..], 1),
    ] {
        for format in ["text", "sarif", "github"] {
            let mut args = vec!["check", "--format", format];
            args.extend_from_slice(extra);
            let run = on(fixture, &args);
            assert_eq!(run.code, expected, "{fixture} {args:?}");
        }
    }
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
        "digraph \"modfence\" {\n  rankdir=LR;\n  n0 [id=\"ext:py:requests\", label=\"requests\", shape=diamond, tooltip=\"boundary: network\"];\n  n1 [id=\"pkg:py:pkg\", label=\"pkg\", shape=box];\n  n1 -> n0 [style=solid, arrowhead=normal];\n}\n"
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
            .contains("[id=\"sym:py:pkg/orders.py#place\", label=\"place\", shape=triangle, tooltip=\"layer: domain\"];")
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

// `--precise` and `--scip` (NC6c): the indexes are written in code and the
// project is a scratch directory.

const LIB: &str = "def run():\n    pass\n\n\ndef other():\n    pass\n";
const APP: &str =
    "from lib import run, other\n\n\ndef main():\n    run()\n    other()\n    run()\n";
const RUN: &str = "scip-python python lib 0.1 `lib`/run().";
const CONFIG: &str = "version = 1\n\n[[rule]]\nid = \"no-calls\"\nfrom = { path = \"app.py\" }\nto = { path = \"lib.py\" }\neffect = \"forbid\"\nkinds = [\"calls\"]\n";

fn wire_occurrence(range: &[i32], symbol: &str, roles: i32) -> Occurrence {
    let mut occurrence = Occurrence::new();
    occurrence.range = range.to_vec();
    symbol.clone_into(&mut occurrence.symbol);
    occurrence.symbol_roles = roles;
    occurrence
}

fn wire_document(path: &str, occurrences: Vec<Occurrence>) -> Document {
    let mut document = Document::new();
    path.clone_into(&mut document.relative_path);
    document.position_encoding =
        EnumOrUnknown::new(PositionEncoding::UTF8CodeUnitOffsetFromLineStart);
    document.occurrences = occurrences;
    document
}

fn wire_index(root: &str, tool: Option<&str>, documents: Vec<Document>) -> Vec<u8> {
    let mut metadata = Metadata::new();
    root.clone_into(&mut metadata.project_root);
    if let Some(name) = tool {
        let mut info = ToolInfo::new();
        name.clone_into(&mut info.name);
        metadata.tool_info = MessageField::some(info);
    }
    let mut index = Index::new();
    index.metadata = MessageField::some(metadata);
    index.documents = documents;
    index.write_to_bytes().unwrap()
}

/// The documents that resolve `run()` in `app.py`, and one for a file that
/// does not exist.
fn resolving() -> Vec<Document> {
    vec![
        wire_document("lib.py", vec![wire_occurrence(&[0, 4, 7], RUN, 1)]),
        wire_document(
            "app.py",
            vec![
                wire_occurrence(&[4, 4, 7], RUN, 0),
                wire_occurrence(&[6, 4, 7], RUN, 0),
            ],
        ),
        wire_document("nope.py", vec![]),
    ]
}

/// A scratch project with `lib.py`, `app.py` and a rule against calls.
fn project(name: &str) -> PathBuf {
    let dir = scratch(name);
    std::fs::write(dir.join("modfence.toml"), CONFIG).unwrap();
    std::fs::write(dir.join("lib.py"), LIB).unwrap();
    std::fs::write(dir.join("app.py"), APP).unwrap();
    dir
}

/// `modfence <args> --config <dir>/modfence.toml`.
fn in_project(dir: &Path, args: &[&str]) -> Run {
    let config = dir.join("modfence.toml");
    let mut all: Vec<&str> = args.to_vec();
    all.push("--config");
    all.push(config.to_str().unwrap());
    exec(&all)
}

fn write_index(dir: &Path, file: &str, bytes: &[u8]) -> String {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, bytes).unwrap();
    path.to_str().unwrap().to_owned()
}

fn root_uri(dir: &Path) -> String {
    format!("file://{}", dir.canonicalize().unwrap().display())
}

#[test]
fn scip_resolves_calls_so_check_finds_them() {
    let dir = project("precise-check");
    let plain = in_project(&dir, &["check"]);
    assert_eq!(plain.code, 0, "{}", plain.err);
    assert!(plain.err.contains("call edges were not checked"));

    let bytes = wire_index(&root_uri(&dir), Some("scip-python 0.6"), resolving());
    let index = write_index(&dir, "index.scip", &bytes);
    let run = in_project(&dir, &["check", "--scip", &index]);
    assert_eq!(run.code, 1, "{}", run.err);
    assert!(run.out.contains("no-calls"), "{}", run.out);
    assert!(!run.err.contains("were not checked"), "{}", run.err);
    assert!(run.err.contains("note: read SCIP index"), "{}", run.err);
    assert!(run.err.contains("from scip-python 0.6"), "{}", run.err);
    assert!(run.err.contains("1 documents match no analysed file"));

    let sarif = in_project(&dir, &["check", "--format", "sarif", "--scip", &index]);
    let value: serde_json::Value = serde_json::from_str(&sarif.out).unwrap();
    let properties = &value["runs"][0]["properties"]["modfence"];
    assert_eq!(properties["callEdgesChecked"], true);
    assert_eq!(properties["precise"], true);
}

#[test]
fn graph_and_trace_take_the_flags() {
    let dir = project("precise-graph");
    let bytes = wire_index("", None, resolving());
    let index = write_index(&dir, "index.scip", &bytes);
    let graph = in_project(&dir, &["graph", "--format", "json", "--scip", &index]);
    assert_eq!(graph.code, 0, "{}", graph.err);
    let value: serde_json::Value = serde_json::from_str(&graph.out).unwrap();
    assert_eq!(value["graph"]["metadata"]["backend"], "tree-sitter+scip");
    let args = [
        "trace",
        "--format",
        "json",
        "--precise",
        "--scip",
        &index,
        "path=lib.py",
    ];
    let trace = in_project(&dir, &args);
    assert_eq!(trace.code, 0, "{}", trace.err);
    let value: serde_json::Value = serde_json::from_str(&trace.out).unwrap();
    assert_eq!(value["graph"]["metadata"]["backend"], "tree-sitter+scip");
}

#[test]
fn a_scip_file_that_cannot_be_read_exits_74() {
    let dir = project("precise-missing");
    let missing = dir.join("none.scip");
    let run = in_project(&dir, &["check", "--scip", missing.to_str().unwrap()]);
    assert_eq!(run.code, 74, "{}", run.err);
    assert!(run.err.contains("none.scip"), "{}", run.err);
    assert!(run.err.contains("cannot read the index"), "{}", run.err);
    let folder = dir.to_str().unwrap();
    let run = in_project(&dir, &["check", "--scip", folder]);
    assert_eq!(run.code, 74, "{}", run.err);
    assert!(run.err.contains("not a regular file"), "{}", run.err);
}

#[test]
fn a_scip_file_that_is_not_an_index_exits_2() {
    let dir = project("precise-invalid");
    let garbage = write_index(&dir, "garbage.scip", &[0xff; 8]);
    let run = in_project(&dir, &["check", "--scip", &garbage]);
    assert_eq!(run.code, 2, "{}", run.err);
    assert!(run.err.contains("not a SCIP index"), "{}", run.err);
    // One byte over the size cap: refused without being read.
    let big = dir.join("big.scip");
    let handle = std::fs::File::create(&big).unwrap();
    handle.set_len(512 * 1024 * 1024 + 1).unwrap();
    let run = in_project(&dir, &["check", "--scip", big.to_str().unwrap()]);
    assert_eq!(run.code, 2, "{}", run.err);
    assert!(run.err.contains("more than the limit"), "{}", run.err);
}

#[test]
fn bare_precise_reads_the_index_directory() {
    let dir = project("precise-directory");
    // No directory, then an empty one: a warning, and the run goes on.
    for create in [false, true] {
        if create {
            std::fs::create_dir_all(dir.join(".modfence/scip")).unwrap();
        }
        let run = in_project(&dir, &["check", "--precise"]);
        assert_eq!(run.code, 0, "{}", run.err);
        assert!(run.err.contains("no SCIP indexes in .modfence/scip"));
        assert!(run.err.contains("call edges were not checked"));
    }
    // A good index, a broken one and a file that is not an index.
    let bytes = wire_index("", None, resolving());
    write_index(&dir, ".modfence/scip/a.scip", &bytes);
    write_index(&dir, ".modfence/scip/z.scip", &[0xff; 8]);
    write_index(&dir, ".modfence/scip/notes.txt", b"not an index");
    let run = in_project(&dir, &["check", "--precise"]);
    assert_eq!(run.code, 1, "{}", run.err);
    assert!(run.err.contains("z.scip: not a SCIP index"), "{}", run.err);
    assert!(run.err.contains("skipped"), "{}", run.err);
    assert!(!run.err.contains("notes.txt"), "{}", run.err);
    assert!(!run.err.contains("no SCIP indexes"), "{}", run.err);
}

#[test]
fn an_index_with_nothing_usable_leaves_the_run_on_tree_sitter() {
    let dir = project("precise-unusable");
    // Its root is somewhere else, so all its documents are ignored; and the
    // only file in the directory is broken.
    let bytes = wire_index("file:///elsewhere", None, resolving());
    let index = write_index(&dir, "index.scip", &bytes);
    let run = in_project(&dir, &["check", "--scip", &index]);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(run.err.contains("outside the analysis root"), "{}", run.err);
    assert!(run.err.contains("no usable SCIP documents"), "{}", run.err);
    assert!(
        run.err.contains("call edges were not checked"),
        "{}",
        run.err
    );

    // Right root, but the only document is for a file the run never analysed.
    let stale = project("precise-stale");
    let bytes = wire_index(
        &root_uri(&stale),
        None,
        vec![wire_document("nope.py", vec![])],
    );
    let index = write_index(&stale, "index.scip", &bytes);
    let run = in_project(&stale, &["check", "--scip", &index]);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(run.err.contains("no usable SCIP documents"), "{}", run.err);
    assert!(
        run.err.contains("call edges were not checked"),
        "{}",
        run.err
    );

    let only_broken = project("precise-only-broken");
    write_index(&only_broken, ".modfence/scip/bad.scip", &[0xff; 8]);
    let run = in_project(&only_broken, &["check", "--precise"]);
    assert_eq!(run.code, 0, "{}", run.err);
    assert!(run.err.contains("no usable SCIP documents"), "{}", run.err);
}

#[test]
fn an_unreadable_index_directory_exits_74() {
    let dir = project("precise-dir-is-file");
    // `.modfence/scip` is a file, so listing it fails with something other
    // than "not found".
    write_index(&dir, ".modfence/scip", b"x");
    let run = in_project(&dir, &["check", "--precise"]);
    assert_eq!(run.code, 74, "{}", run.err);
    assert!(!run.err.contains("no SCIP indexes"), "{}", run.err);
}

#[test]
fn text_from_an_index_is_escaped_where_it_is_printed() {
    let dir = project("precise-escape");
    let bytes = wire_index("", Some("scip\u{1b}[31m\nfake: error"), resolving());
    let index = write_index(&dir, "index.scip", &bytes);
    let run = in_project(&dir, &["check", "--scip", &index]);
    assert!(
        run.err.contains("scip\\u{1b}[31m\\nfake: error"),
        "{:?}",
        run.err
    );
    assert!(!run.err.contains('\u{1b}'), "{:?}", run.err);
    assert!(!run.err.contains("\nfake: error"), "{:?}", run.err);
}
