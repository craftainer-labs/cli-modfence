//! The real SCIP indexes committed in `tests/fixtures/scip/indexes/` (`NC6e`),
//! made by `scip-python`, `scip-typescript` and `rust-analyzer` from the
//! fixture projects next to them. Read offline: nothing here starts an indexer.
//!
//! Each index records the temporary directory of the job that made it as its
//! root, so a test points the root at the directory it copies the fixture to;
//! the committed files are never changed.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use modfence::scip::{DEFINITION, Index, LineIndex, PositionEncoding, read_index};
use protobuf::Message;
use scip::types::Index as WireIndex;
use serde_json::Value;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scip")
}

/// The committed index of `lang` with its root set to `root`.
fn rooted(lang: &str, root: &Path) -> Vec<u8> {
    let committed = fixtures().join("indexes").join(format!("{lang}.scip"));
    let mut index = WireIndex::parse_from_bytes(&std::fs::read(committed).unwrap()).unwrap();
    let metadata = index.metadata.mut_or_insert_default();
    metadata.project_root = format!("file://{}", root.display());
    index.write_to_bytes().unwrap()
}

/// A copy of the fixture project `lang` in a scratch directory, and the
/// committed index for it.
fn project(lang: &str, files: &[&str]) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("modfence-real-{lang}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let root = root.canonicalize().unwrap();
    for file in files {
        let target = root.join(file);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::copy(fixtures().join(lang).join(file), target).unwrap();
    }
    std::fs::write(root.join("modfence.toml"), "version = 1\n").unwrap();
    let index = root.join("index.scip");
    std::fs::write(&index, rooted(lang, &root)).unwrap();
    (root, index)
}

/// `modfence graph --format json --scip <index>` on a project: the document and
/// standard error.
fn graph(root: &Path, index: &Path) -> (Value, String) {
    let config = root.join("modfence.toml");
    let args: Vec<OsString> = vec![
        "graph".into(),
        "--format".into(),
        "json".into(),
        "--scip".into(),
        index.into(),
        "--config".into(),
        config.into(),
    ];
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = modfence::run(args, &mut out, &mut err);
    assert!(
        code == std::process::ExitCode::SUCCESS,
        "{}",
        String::from_utf8_lossy(&err)
    );
    let document = serde_json::from_slice(&out).unwrap();
    (document, String::from_utf8(err).unwrap())
}

/// The resolved `calls` edges the index gave the graph, as (caller, callee,
/// callee text at the call).
fn resolved_calls(document: &Value) -> Vec<(String, String, String)> {
    let edges = document["graph"]["edges"].as_array().unwrap();
    let mut found = Vec::new();
    for edge in edges.iter().filter(|e| e["relation"] == "calls") {
        let metadata = &edge["metadata"];
        assert_eq!(metadata["confidence"], "resolved", "{edge}");
        assert_eq!(metadata["origin"], "scip", "{edge}");
        let text = |value: &Value| value.as_str().unwrap().to_owned();
        found.push((
            text(&edge["source"]),
            text(&edge["target"]),
            text(&metadata["specifier"]),
        ));
    }
    found
}

fn has(found: &[(String, String, String)], callee: &str, written: &str) -> bool {
    found
        .iter()
        .any(|(_, target, text)| target == callee && text == written)
}

#[test]
fn scip_python_resolves_an_aliased_call_and_a_method_call() {
    let (root, index) = project("python", &["lib.py", "app.py"]);
    let (document, err) = graph(&root, &index);
    assert_eq!(document["graph"]["metadata"]["backend"], "tree-sitter+scip");
    let found = resolved_calls(&document);
    // `go()` is `run` imported under another name, and sits after non-ASCII
    // text on its line; `svc.run_it()` goes through an instance.
    assert!(has(&found, "sym:py:lib.py#run", "go"), "{found:?}");
    assert!(
        has(&found, "sym:py:lib.py#Svc.run_it", "run_it"),
        "{found:?}"
    );
    assert!(has(&found, "sym:py:lib.py#Svc", "Svc"), "{found:?}");
    // scip-python states no position encoding: that is counted, and the call
    // after the non-ASCII text still resolves because it is read as UTF-16.
    assert!(
        err.contains("state no position encoding; read as UTF-16"),
        "{err}"
    );
    assert!(!err.contains("may be stale"), "{err}");
}

#[test]
fn scip_typescript_resolves_an_aliased_call_and_a_method_call() {
    let (root, index) = project("typescript", &["lib.ts", "app.ts", "package.json"]);
    let (document, err) = graph(&root, &index);
    assert_eq!(document["graph"]["metadata"]["backend"], "tree-sitter+scip");
    let found = resolved_calls(&document);
    assert!(has(&found, "sym:ts:lib.ts#run", "go"), "{found:?}");
    assert!(has(&found, "sym:ts:lib.ts#Svc~runIt", "runIt"), "{found:?}");
    assert!(!err.contains("may be stale"), "{err}");
    assert!(!err.contains("no position encoding"), "{err}");
}

#[test]
fn rust_analyzer_resolves_an_aliased_call_and_a_method_call() {
    let (root, index) = project("rust", &["src/lib.rs"]);
    let (document, err) = graph(&root, &index);
    assert_eq!(document["graph"]["metadata"]["backend"], "tree-sitter+scip");
    assert_eq!(
        document["graph"]["metadata"]["languages"],
        serde_json::json!(["rs"])
    );
    let found = resolved_calls(&document);
    // `go()` is `helper::run` imported under another name, and sits after
    // non-ASCII text on its line; `svc.run_it()` goes through an instance.
    assert!(
        has(&found, "sym:rs:src/lib.rs#helper::run", "go"),
        "{found:?}"
    );
    assert!(
        has(&found, "sym:rs:src/lib.rs#<Svc>::run_it", "run_it"),
        "{found:?}"
    );
    // rust-analyzer states UTF-8 offsets, so nothing is stale or unstated.
    assert!(!err.contains("may be stale"), "{err}");
    assert!(!err.contains("no position encoding"), "{err}");
}

/// The text each reference to `symbol_part` covers in the fixture source,
/// converted with the document's own encoding.
fn reference_texts(index: &Index, source: &str, path: &str, symbol_part: &str) -> Vec<String> {
    let lines = LineIndex::new(source);
    let document = index.documents.iter().find(|d| d.path == path).unwrap();
    document
        .occurrences
        .iter()
        .filter(|o| o.roles & DEFINITION == 0 && o.symbol.contains(symbol_part))
        .filter_map(|o| lines.span(o.range, document.encoding))
        .filter_map(|span| lines.slice(span))
        .map(str::to_owned)
        .collect()
}

#[test]
fn rust_analyzer_positions_convert_to_the_source_text() {
    let root = std::env::temp_dir().join(format!("modfence-real-rust-{}", std::process::id()));
    let bytes = rooted("rust", &root);
    let (index, stats) = read_index(&bytes, root.to_str().unwrap()).unwrap();
    assert_eq!(stats.warnings("rust.scip"), Vec::<String>::new());
    assert!(index.tool.as_deref().unwrap().starts_with("rust-analyzer"));
    let paths: Vec<&str> = index.documents.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, ["src/lib.rs"]);
    assert_eq!(index.documents[0].encoding, PositionEncoding::Utf8);
    let source = std::fs::read_to_string(fixtures().join("rust/src/lib.rs")).unwrap();
    // The `use` names `run` and then `go`, and the call after non-ASCII text
    // is `go`, read from UTF-8 offsets; the method call is found by name.
    let runs = reference_texts(&index, &source, "src/lib.rs", "helper/run().");
    assert_eq!(runs, ["run", "go", "go"]);
    let methods = reference_texts(&index, &source, "src/lib.rs", "impl#[Svc]run_it().");
    assert_eq!(methods, ["run_it"]);
    let defined = |part: &str| {
        index.documents[0]
            .occurrences
            .iter()
            .any(|o| o.is_definition() && o.symbol.contains(part))
    };
    assert!(defined("helper/run()."));
    assert!(defined("impl#[Svc]run_it()."));
}
