//! The graph core (ADR 0003, ADR 0004): nodes, import resolution, heuristic
//! calls, cross-language binding edges, tags and annotation findings, built
//! from the real Python and TypeScript extractors plus hand-made Rust facts.

use modfence::config::{self, Config, Severity};
use modfence::extract::python::PythonExtractor;
use modfence::extract::rust::RustExtractor;
use modfence::extract::typescript::TsExtractor;
use modfence::extract::{
    Annotation, BindingExport, BindingFramework, DiagnosticCode, ExtractCtx, Extractor, FileFacts,
    ImportEdge, Lang, Node, NodeId, NodeKind, SourceFile, Span, SymbolKind, Tags,
};
use modfence::graph::{Binding, Builder, Confidence, Edge, Graph, INVALID_ANNOTATION, Relation};

fn extract(path: &str, text: &str) -> FileFacts {
    let file = SourceFile { path, text };
    match Lang::from_path(path) {
        Some(Lang::Py) => PythonExtractor.extract(&file, &ExtractCtx::default()),
        Some(Lang::Ts) => TsExtractor.extract(&file, &ExtractCtx::default()),
        Some(Lang::Rs) => RustExtractor.extract(&file, &ExtractCtx::default()),
        None => FileFacts::default(),
    }
}

fn add_all(builder: &mut Builder<'_>, files: &[(&str, &str)]) {
    for &(path, text) in files {
        builder.add(&SourceFile { path, text }, extract(path, text));
    }
}

fn build(files: &[(&str, &str)]) -> Graph {
    build_with(&Config::default(), files)
}

fn build_with(config: &Config, files: &[(&str, &str)]) -> Graph {
    let mut builder = Builder::new(config);
    add_all(&mut builder, files);
    builder.build()
}

fn span(line: u32) -> Span {
    Span::new(line, 1, line, 10).unwrap()
}

fn id(kind: NodeKind, lang: Lang, path: &str, qualname: Option<&str>) -> NodeId {
    NodeId::new(kind, lang, path, qualname).unwrap()
}

fn module(lang: Lang, path: &str) -> NodeId {
    id(NodeKind::Module, lang, path, None)
}

fn external(lang: Lang, name: &str) -> NodeId {
    id(NodeKind::External, lang, name, None)
}

fn symbol(lang: Lang, path: &str, qualname: &str) -> NodeId {
    id(NodeKind::Symbol, lang, path, Some(qualname))
}

fn edge<'a>(graph: &'a Graph, relation: Relation, from: &NodeId, to: &NodeId) -> &'a Edge {
    graph
        .edges
        .iter()
        .find(|e| e.relation == relation && &e.source == from && &e.target == to)
        .unwrap_or_else(|| panic!("no {relation:?} edge {from} -> {to}"))
}

/// The `relation` edges leaving `from`, as (target, confidence, site count),
/// in the graph's order.
fn outgoing(
    graph: &Graph,
    relation: Relation,
    from: &NodeId,
) -> Vec<(String, Option<Confidence>, usize)> {
    graph
        .edges
        .iter()
        .filter(|e| e.relation == relation && &e.source == from)
        .map(|e| (e.target.to_string(), e.confidence, e.count()))
        .collect()
}

fn row(target: &str, confidence: Confidence, count: usize) -> (String, Option<Confidence>, usize) {
    (target.to_owned(), Some(confidence), count)
}

/// The invariants of ADR 0003 that hold for any graph: every endpoint is a
/// node, and each relation joins the node kinds it should (the schema leaves
/// that to NC4).
fn assert_well_formed(graph: &Graph) {
    for e in &graph.edges {
        let source = &graph.nodes[&e.source];
        let target = &graph.nodes[&e.target];
        let kinds = (source.kind, target.kind);
        let fits = match e.relation {
            Relation::Contains => matches!(
                kinds,
                (NodeKind::Package, NodeKind::Module) | (NodeKind::Module, NodeKind::Symbol)
            ),
            Relation::Imports => {
                matches!(
                    kinds,
                    (NodeKind::Module, NodeKind::Module | NodeKind::External)
                )
            }
            Relation::Calls => {
                matches!(
                    kinds,
                    (NodeKind::Symbol | NodeKind::Module, NodeKind::Symbol)
                )
            }
            // A `#[pymodule] mod x` exports the file's module node.
            Relation::Binds => {
                matches!(
                    kinds,
                    (
                        NodeKind::Symbol | NodeKind::Module,
                        NodeKind::Symbol | NodeKind::Module
                    )
                )
            }
        };
        assert!(fits, "{:?} {} -> {}", e.relation, e.source, e.target);
        if e.relation == Relation::Contains {
            assert!(e.confidence.is_none() && e.sites.is_empty() && e.specifier.is_none());
        } else {
            assert!(e.confidence.is_some() && e.count() >= 1 && e.specifier.is_some());
            assert!(
                e.sites.windows(2).all(|pair| pair[0] <= pair[1]),
                "sorted sites"
            );
            assert_eq!(e.site(), e.sites.first());
        }
        if matches!(e.relation, Relation::Calls | Relation::Binds) {
            assert_eq!(e.confidence, Some(Confidence::Heuristic));
        }
    }
    let keys: Vec<_> = graph
        .edges
        .iter()
        .map(|e| (&e.source, &e.target, e.relation, e.confidence))
        .collect();
    assert!(
        keys.windows(2).all(|pair| pair[0] < pair[1]),
        "edges are ordered and unique"
    );
}

#[test]
fn the_names_in_the_json_output() {
    assert_eq!(Relation::Contains.as_str(), "contains");
    assert_eq!(Relation::Imports.as_str(), "imports");
    assert_eq!(Relation::Calls.as_str(), "calls");
    assert_eq!(Relation::Binds.as_str(), "binds");
    assert_eq!(Confidence::Resolved.as_str(), "resolved");
    assert_eq!(Confidence::Heuristic.as_str(), "heuristic");
    assert_eq!(INVALID_ANNOTATION, "modfence/invalid-annotation");
}

// ADR 0003, "Node kinds" and "Edge relations": packages, modules, symbols and
// the `contains` edges between them; invariants 3 and 4.
#[test]
fn nodes_packages_and_contains_edges() {
    let graph = build(&[
        ("top.py", "def top():\n    pass\n"),
        (
            "pkg/core.py",
            "class Engine:\n    def run(self):\n        pass\n",
        ),
        ("web/app.ts", "export function boot() {}\n"),
        ("notes.txt", "not source"),
    ]);
    assert_well_formed(&graph);
    let root = id(NodeKind::Package, Lang::Py, ".", None);
    let pkg = id(NodeKind::Package, Lang::Py, "pkg", None);
    let web = id(NodeKind::Package, Lang::Ts, "web", None);
    let core = module(Lang::Py, "pkg/core.py");
    let run = symbol(Lang::Py, "pkg/core.py", "Engine.run");
    let ids: Vec<String> = graph.nodes.keys().map(ToString::to_string).collect();
    assert_eq!(
        ids,
        [
            "mod:py:pkg/core.py",
            "mod:py:top.py",
            "mod:ts:web/app.ts",
            "pkg:py:.",
            "pkg:py:pkg",
            "pkg:ts:web",
            "sym:py:pkg/core.py#Engine",
            "sym:py:pkg/core.py#Engine.run",
            "sym:py:top.py#top",
            "sym:ts:web/app.ts#boot",
        ]
    );
    edge(
        &graph,
        Relation::Contains,
        &root,
        &module(Lang::Py, "top.py"),
    );
    edge(&graph, Relation::Contains, &pkg, &core);
    edge(
        &graph,
        Relation::Contains,
        &web,
        &module(Lang::Ts, "web/app.ts"),
    );
    edge(&graph, Relation::Contains, &core, &run);
    assert_eq!(graph.nodes[&pkg].path.as_deref(), Some("pkg"));
    assert_eq!(graph.nodes[&run].symbol_kind, Some(SymbolKind::Method));
    assert_eq!(graph.nodes[&run].module.as_ref(), Some(&core));
    assert_eq!(graph.nodes[&core].module, None);
    assert_eq!(
        graph.languages.iter().copied().collect::<Vec<_>>(),
        [Lang::Py, Lang::Ts]
    );
    assert!(graph.diagnostics.is_empty() && graph.findings.is_empty());
}

#[test]
fn an_empty_analysis_is_an_empty_graph() {
    let config = Config::default();
    assert_eq!(Builder::new(&config).build(), Graph::default());
}

// ADR 0003, "Rules see module-level edges": a symbol rolls up to its module.
#[test]
fn symbols_roll_up_to_their_module() {
    let graph = build(&[("a.py", "def f():\n    pass\n")]);
    let (f, a) = (symbol(Lang::Py, "a.py", "f"), module(Lang::Py, "a.py"));
    let unknown = symbol(Lang::Py, "gone.py", "g");
    assert_eq!(graph.rollup(&f), &a);
    assert_eq!(graph.rollup(&a), &a);
    assert_eq!(graph.rollup(&unknown), &unknown);
}

// ADR 0003, invariants 6 and 4: Python imports are placed in the analysis or
// become external edges; a relative import that cannot be placed also gets an
// `unresolved-import` diagnostic.
#[test]
fn python_imports_are_resolved() {
    let main = r"import os
import app.util
from app import helpers, Thing
from .util import thing
from . import util
from .missing import x
from ..toofar import y
import requests.adapters
from ... import deep
";
    let graph = build(&[
        ("app/__init__.py", ""),
        ("app/util.py", ""),
        ("app/helpers.py", ""),
        ("app/main.py", main),
    ]);
    assert_well_formed(&graph);
    let from = module(Lang::Py, "app/main.py");
    let imports = outgoing(&graph, Relation::Imports, &from);
    let expected = vec![
        row("ext:py:...", Confidence::Heuristic, 1),
        row("ext:py:..toofar", Confidence::Heuristic, 1),
        row("ext:py:.missing", Confidence::Heuristic, 1),
        row("ext:py:os", Confidence::Resolved, 1),
        row("ext:py:requests", Confidence::Resolved, 1),
        row("mod:py:app/__init__.py", Confidence::Resolved, 1),
        row("mod:py:app/helpers.py", Confidence::Resolved, 1),
        row("mod:py:app/util.py", Confidence::Resolved, 3),
    ];
    assert_eq!(imports, expected);
    let util = edge(
        &graph,
        Relation::Imports,
        &from,
        &module(Lang::Py, "app/util.py"),
    );
    let lines: Vec<u32> = util.sites.iter().map(|s| s.span.start_line).collect();
    assert_eq!(lines, [2, 4, 5]);
    assert_eq!(util.site().unwrap().path, "app/main.py");
    assert_eq!(util.specifier.as_deref(), Some("app.util"));
    let diagnostics: Vec<_> = graph
        .diagnostics
        .iter()
        .map(|d| (d.path.as_str(), d.code, d.span.unwrap().start_line))
        .collect();
    let code = DiagnosticCode::UnresolvedImport;
    let path = "app/main.py";
    assert_eq!(
        diagnostics,
        [(path, code, 6), (path, code, 7), (path, code, 9)]
    );
    assert!(graph.diagnostics[0].message.contains(".missing"));
}

#[test]
fn python_imports_from_the_root_and_from_source_roots() {
    let files = [
        ("__init__.py", ""),
        ("util.py", ""),
        ("top.py", "from . import util\nfrom . import Thing\n"),
        ("src/lib/__init__.py", ""),
        ("src/lib/x.py", ""),
        ("user.py", "import lib.x\nimport lib\nimport other\n"),
    ];
    let graph = build(&files);
    let top = outgoing(&graph, Relation::Imports, &module(Lang::Py, "top.py"));
    assert_eq!(
        top,
        [
            row("mod:py:__init__.py", Confidence::Resolved, 1),
            row("mod:py:util.py", Confidence::Resolved, 1),
        ]
    );
    let user = module(Lang::Py, "user.py");
    let plain = outgoing(&graph, Relation::Imports, &user);
    assert_eq!(
        plain,
        [
            row("ext:py:lib", Confidence::Resolved, 2),
            row("ext:py:other", Confidence::Resolved, 1),
        ]
    );
    let config = Config::default();
    let mut builder = Builder::new(&config).with_python_roots(["", "src"]);
    add_all(&mut builder, &files);
    let graph = builder.build();
    let rooted = outgoing(&graph, Relation::Imports, &user);
    assert_eq!(
        rooted,
        [
            row("ext:py:other", Confidence::Resolved, 1),
            row("mod:py:src/lib/__init__.py", Confidence::Resolved, 1),
            row("mod:py:src/lib/x.py", Confidence::Resolved, 1),
        ]
    );
    assert_well_formed(&graph);
}

#[test]
fn a_bare_relative_import_names_the_package_not_a_sibling_module() {
    let graph = build(&[
        ("a.py", "x = 1\n"),
        ("a/__init__.py", "helper = 1\n"),
        ("a/b.py", "from . import helper\n"),
    ]);
    let from = module(Lang::Py, "a/b.py");
    let package = module(Lang::Py, "a/__init__.py").to_string();
    let targets = outgoing(&graph, Relation::Imports, &from);
    assert_eq!(targets, [row(&package, Confidence::Resolved, 1)]);
}

// An import of an analysed package that does not resolve is unplaced, not an
// external package; unplaced targets are not matched by `package` selectors.
#[test]
fn unplaced_imports_are_not_packages() {
    let config =
        config::parse("version = 1\n[[node]]\npackage = \"**\"\ntags = { boundary = \"net\" }\n")
            .unwrap();
    let user = "import requests\nimport app.missing\nfrom app.sub import x\nfrom .gone import y\n";
    let graph = build_with(&config, &[("app/__init__.py", ""), ("user.py", user)]);
    assert_well_formed(&graph);
    let imports = outgoing(&graph, Relation::Imports, &module(Lang::Py, "user.py"));
    assert_eq!(
        imports,
        [
            row("ext:py:.gone", Confidence::Heuristic, 1),
            row("ext:py:app.missing", Confidence::Heuristic, 1),
            row("ext:py:app.sub", Confidence::Heuristic, 1),
            row("ext:py:requests", Confidence::Resolved, 1),
        ]
    );
    let lines: Vec<u32> = graph
        .diagnostics
        .iter()
        .map(|d| d.span.unwrap().start_line)
        .collect();
    assert_eq!(lines, [2, 3, 4]);
    let boundary = |name: &str| graph.nodes[&external(Lang::Py, name)].tags.len();
    assert_eq!(boundary("requests"), 1);
    for name in [".gone", "app.missing", "app.sub"] {
        assert_eq!(boundary(name), 0, "{name}");
    }
}

// A call is linked once per file in scope, even when a file imports itself.
#[test]
fn a_self_import_does_not_double_a_call() {
    let init = "from . import gone\n\n\ndef f():\n    pass\n\n\nf()\n";
    let graph = build(&[("a/__init__.py", init)]);
    let at = module(Lang::Py, "a/__init__.py");
    let calls = outgoing(&graph, Relation::Calls, &at);
    assert_eq!(
        calls,
        [row("sym:py:a/__init__.py#f", Confidence::Heuristic, 1)]
    );
}

// A trailing-tag check needs the comment's line; a line the text lacks is not
// an own-line comment.
#[test]
fn an_annotation_on_a_missing_line_is_not_own_line() {
    let mut facts = extract("a.py", "x = 1\n");
    let at = Span::new(9, 1, 9, 20).unwrap();
    facts
        .annotations
        .push(Annotation::from_comment("modfence: layer=a", at, None).unwrap());
    let config = Config::default();
    let mut builder = Builder::new(&config);
    builder.add(
        &SourceFile {
            path: "a.py",
            text: "x = 1\n",
        },
        facts,
    );
    let graph = builder.build();
    assert_eq!(graph.findings.len(), 1);
    assert_eq!(graph.findings[0].severity, Severity::Warning);
}

#[test]
fn typescript_imports_are_resolved() {
    let app = r#"import a from "./util";
import b from "./widgets";
import c from "./legacy.js";
import d from "../shared/x";
import e from "./missing";
import f from "../../escape";
import g from "react";
import h from "@scope/pkg/sub";
import i from "@scope";
import j from "lodash/fp";
import k from ".";
import l from "./a.mjs";
import m from "./b.jsx";
import n from "./c.cjs";
import o from "./exact.js";
import p from "./dir/./inner//deep";
import q from "..";
"#;
    let graph = build(&[
        ("web/app.ts", app),
        ("web/util.ts", ""),
        ("web/widgets/index.tsx", ""),
        ("web/legacy.ts", ""),
        ("shared/x.ts", ""),
        ("web/index.ts", ""),
        ("web/a.mts", ""),
        ("web/b.tsx", ""),
        ("web/c.cts", ""),
        ("web/exact.js", ""),
        ("web/dir/inner/deep.ts", ""),
        ("index.js", ""),
    ]);
    assert_well_formed(&graph);
    let from = module(Lang::Ts, "web/app.ts");
    let targets: Vec<(String, Option<Confidence>)> = outgoing(&graph, Relation::Imports, &from)
        .into_iter()
        .map(|(target, confidence, _)| (target, confidence))
        .collect();
    let resolved = |t: &str| (t.to_owned(), Some(Confidence::Resolved));
    let guessed = |t: &str| (t.to_owned(), Some(Confidence::Heuristic));
    assert_eq!(
        targets,
        [
            guessed("ext:ts:../../escape"),
            guessed("ext:ts:./missing"),
            resolved("ext:ts:@scope"),
            resolved("ext:ts:@scope/pkg"),
            resolved("ext:ts:lodash"),
            resolved("ext:ts:react"),
            resolved("mod:ts:index.js"),
            resolved("mod:ts:shared/x.ts"),
            resolved("mod:ts:web/a.mts"),
            resolved("mod:ts:web/b.tsx"),
            resolved("mod:ts:web/c.cts"),
            resolved("mod:ts:web/dir/inner/deep.ts"),
            resolved("mod:ts:web/exact.js"),
            resolved("mod:ts:web/index.ts"),
            resolved("mod:ts:web/legacy.ts"),
            resolved("mod:ts:web/util.ts"),
            resolved("mod:ts:web/widgets/index.tsx"),
        ]
    );
    let codes: Vec<_> = graph.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [DiagnosticCode::UnresolvedImport; 2]);
}

#[test]
fn rust_imports_are_external_unless_in_the_crate() {
    let config = Config::default();
    let mut builder = Builder::new(&config);
    let import = |specifier: &str, line| ImportEdge {
        specifier: specifier.to_owned(),
        names: Vec::new(),
        span: span(line),
    };
    let mut facts = FileFacts {
        nodes: vec![Node::module(Lang::Rs, "src/lib.rs").unwrap()],
        ..FileFacts::default()
    };
    facts.imports = vec![import("std::fmt", 1), import("crate::inner", 2)];
    builder.add(
        &SourceFile {
            path: "src/lib.rs",
            text: "",
        },
        facts,
    );
    let graph = builder.build();
    assert_well_formed(&graph);
    // `crate::inner` is the crate root's own module here: no edge.
    let imports = outgoing(&graph, Relation::Imports, &module(Lang::Rs, "src/lib.rs"));
    assert_eq!(imports, [row("ext:rs:std", Confidence::Resolved, 1)]);
    assert_eq!(graph.diagnostics.len(), 0);
}

/// The imports of a Rust file as (target, confidence, site count).
fn rust_imports(graph: &Graph, path: &str) -> Vec<(String, Option<Confidence>, usize)> {
    outgoing(graph, Relation::Imports, &module(Lang::Rs, path))
}

/// The paths of the files with an unresolved import, one entry per diagnostic.
fn unresolved(graph: &Graph) -> Vec<&str> {
    let found = graph.diagnostics.iter();
    let found = found.filter(|d| d.code == DiagnosticCode::UnresolvedImport);
    found.map(|d| d.path.as_str()).collect()
}

// ADR 0003, "Rust qualified names": `crate::`, `self::` and `super::` paths are
// resolved against the module tree of the crate, which `mod name;` declarations
// build from `src/lib.rs` (`name.rs` or `name/mod.rs`, below the directory of
// the module that declares them).
#[test]
fn rust_paths_resolve_through_the_module_tree() {
    let lib = "\
mod a;
mod b;
pub mod inline {
    pub fn deep() {}
    pub mod nested;
}
use crate::a::thing;
use self::b::sub;
use inline::deep;
use crate::inline::deep as again;
use crate::missing::x;
use serde::Serialize;
use ::tokio::spawn;
";
    let sub = "use crate::a::*;\nuse super::super::sub as nobody;\n";
    let files = [
        ("crates/app/src/lib.rs", lib),
        ("crates/app/src/a.rs", "use super::inline::deep;\nuse self::gone::f;\n"),
        ("crates/app/src/b/mod.rs", "pub mod sub;\nuse super::a::helper;\n"),
        ("crates/app/src/b/sub.rs", sub),
        ("crates/app/src/inline/nested.rs", "use super::deep;\n"),
    ];
    let graph = build(&files);
    assert_well_formed(&graph);
    let confident = |target: &str, count| row(target, Confidence::Resolved, count);
    assert_eq!(
        rust_imports(&graph, "crates/app/src/lib.rs"),
        [
            confident("ext:rs:serde", 1),
            confident("ext:rs:tokio", 1),
            confident("mod:rs:crates/app/src/a.rs", 2),
            confident("mod:rs:crates/app/src/b/mod.rs", 1),
            confident("mod:rs:crates/app/src/b/sub.rs", 1),
            confident("mod:rs:crates/app/src/inline/nested.rs", 1),
        ]
    );
    // `super` from `a` is the crate root; `self::gone` is `a` itself.
    assert_eq!(
        rust_imports(&graph, "crates/app/src/a.rs"),
        [confident("mod:rs:crates/app/src/lib.rs", 1)]
    );
    // `super` from `b` is the root, and `super::a` is `a`.
    assert_eq!(
        rust_imports(&graph, "crates/app/src/b/mod.rs"),
        [
            confident("mod:rs:crates/app/src/a.rs", 1),
            confident("mod:rs:crates/app/src/b/sub.rs", 1),
        ]
    );
    // A glob takes the module itself; two `super` from `b::sub` reach the root.
    assert_eq!(
        rust_imports(&graph, "crates/app/src/b/sub.rs"),
        [
            confident("mod:rs:crates/app/src/a.rs", 1),
            confident("mod:rs:crates/app/src/lib.rs", 1),
        ]
    );
    // `inline/nested.rs` is `inline::nested`, declared in an inline module.
    assert_eq!(
        rust_imports(&graph, "crates/app/src/inline/nested.rs"),
        [confident("mod:rs:crates/app/src/lib.rs", 1)]
    );
    assert_eq!(unresolved(&graph), Vec::<&str>::new());
}

// A path that climbs above the crate root, and a `crate::` path in a file no
// crate root reaches, cannot be placed: an `external` node and a diagnostic.
#[test]
fn rust_paths_that_leave_the_tree_are_unplaced() {
    let files = [
        ("src/lib.rs", "mod a;\n"),
        ("src/a.rs", "use super::super::up;\nuse std::fmt;\n"),
        ("src/orphan.rs", "use crate::a::f;\nuse self::a;\nuse super::a;\n"),
    ];
    let graph = build(&files);
    assert_well_formed(&graph);
    assert_eq!(
        rust_imports(&graph, "src/a.rs"),
        [
            row("ext:rs:std", Confidence::Resolved, 1),
            row("ext:rs:super::super", Confidence::Heuristic, 1),
        ]
    );
    assert_eq!(
        rust_imports(&graph, "src/orphan.rs"),
        [
            row("ext:rs:crate::a", Confidence::Heuristic, 1),
            row("ext:rs:self", Confidence::Heuristic, 1),
            row("ext:rs:super", Confidence::Heuristic, 1),
        ]
    );
    assert_eq!(unresolved(&graph), ["src/a.rs", "src/orphan.rs", "src/orphan.rs", "src/orphan.rs"]);
}

// Crates are found from Cargo's layout. Each file below imports `crate::x::y`,
// which lands on the file's own crate root when the file belongs to a crate and
// cannot be placed when it does not.
#[test]
fn rust_crate_roots_follow_the_cargo_layout() {
    let roots = [
        "src/lib.rs",
        "src/main.rs",
        "src/bin/one.rs",
        "src/bin/two/main.rs",
        "tests/it.rs",
        "examples/e.rs",
        "benches/b.rs",
        "pkg/src/lib.rs",
    ];
    let others = [
        "build.rs",
        "lib.rs",
        "src/bin/two/other.rs",
        "src/model.rs",
        "src/tests/x.rs",
    ];
    let text = "use crate::x::y;\n";
    let all: Vec<(&str, &str)> = roots.iter().chain(&others).map(|path| (*path, text)).collect();
    let graph = build(&all);
    assert_eq!(unresolved(&graph), others);
}

// `mod name;` looks in the directory of a root or a `mod.rs`, and in the
// directory named after any other file.
#[test]
fn rust_module_files_are_found_below_the_declaring_module() {
    let files = [
        ("tests/it.rs", "mod common;\n"),
        ("tests/common/mod.rs", "mod inner;\n"),
        ("tests/common/inner.rs", "pub fn f() {}\n"),
        ("src/lib.rs", "mod flat;\nmod twice;\n"),
        ("src/flat.rs", "mod sub;\n"),
        ("src/flat/sub.rs", "use super::g;\n"),
        ("src/flat/lost.rs", "use super::g;\n"),
        ("src/bin/two/main.rs", "mod util;\nfn main() { u(); }\n"),
        ("src/bin/two/util.rs", "pub fn u() {}\n"),
        ("src/twice.rs", ""),
        ("src/twice/mod.rs", ""),
    ];
    let graph = build(&files);
    assert_well_formed(&graph);
    let to = |target: &str| row(&format!("mod:rs:{target}"), Confidence::Resolved, 1);
    assert_eq!(
        rust_imports(&graph, "tests/it.rs"),
        [to("tests/common/mod.rs")]
    );
    assert_eq!(
        rust_imports(&graph, "tests/common/mod.rs"),
        [to("tests/common/inner.rs")]
    );
    // `twice.rs` wins over `twice/mod.rs`, which nobody then declares.
    assert_eq!(
        rust_imports(&graph, "src/lib.rs"),
        [to("src/flat.rs"), to("src/twice.rs")]
    );
    assert_eq!(rust_imports(&graph, "src/flat.rs"), [to("src/flat/sub.rs")]);
    // `super` from `flat::sub` is `flat`; `lost.rs` is declared by nobody.
    assert_eq!(rust_imports(&graph, "src/flat/sub.rs"), [to("src/flat.rs")]);
    assert_eq!(unresolved(&graph), ["src/flat/lost.rs"]);
    assert_eq!(
        rust_imports(&graph, "src/bin/two/main.rs"),
        [to("src/bin/two/util.rs")]
    );
    // The call to `u` is found through the module the file declares.
    let main = symbol(Lang::Rs, "src/bin/two/main.rs", "main");
    assert_eq!(
        outgoing(&graph, Relation::Calls, &main),
        [row("sym:rs:src/bin/two/util.rs#u", Confidence::Heuristic, 1)]
    );
}

// Calls to Rust items are matched by the last name, which for a method is the
// method's name, not its path.
#[test]
fn rust_calls_match_symbols_by_their_last_name() {
    let lib = "\
mod m;
pub struct Config;
impl Config {
    pub fn new() -> Self { Self::make() }
    fn make() -> Self { Config }
}
pub fn run() { Config::new(); m::util(); }
";
    let graph = build(&[("src/lib.rs", lib), ("src/m.rs", "pub fn util() {}\n")]);
    assert_well_formed(&graph);
    let run = symbol(Lang::Rs, "src/lib.rs", "run");
    assert_eq!(
        outgoing(&graph, Relation::Calls, &run),
        [
            row("sym:rs:src/lib.rs#<Config>::new", Confidence::Heuristic, 1),
            row("sym:rs:src/m.rs#util", Confidence::Heuristic, 1),
        ]
    );
    let new = symbol(Lang::Rs, "src/lib.rs", "<Config>::new");
    assert_eq!(
        outgoing(&graph, Relation::Calls, &new),
        [row("sym:rs:src/lib.rs#<Config>::make", Confidence::Heuristic, 1)]
    );
}

// A `mod` is not a symbol, so `#[pymodule] mod x` exports the file's module
// node, which Python binds to by the name of the module.
#[test]
fn a_pymodule_mod_binds_the_rust_module_node() {
    let native = "#[pymodule]\nmod _core {\n    #[pyfunction]\n    pub fn add() {}\n}\n";
    let graph = build(&[
        ("native/src/lib.rs", native),
        ("app.py", "from native import _core\n_core.add()\n"),
    ]);
    assert_well_formed(&graph);
    let rust = module(Lang::Rs, "native/src/lib.rs");
    let app = module(Lang::Py, "app.py");
    let bound = edge(&graph, Relation::Binds, &app, &rust);
    assert_eq!(bound.confidence, Some(Confidence::Heuristic));
    assert_eq!(bound.count(), 1);
    assert_eq!(
        graph.nodes[&rust].binding,
        Some(Binding {
            framework: BindingFramework::Pyo3,
            exported_name: "_core".to_owned(),
        })
    );
    let add = symbol(Lang::Rs, "native/src/lib.rs", "_core::add");
    assert_eq!(edge(&graph, Relation::Binds, &app, &add).count(), 1);
}

// ADR 0003, "Edge relations" and invariant 7: call edges are heuristic, found
// by name in the module and in the modules it imports, and folded per target
// with every site kept.
#[test]
fn calls_are_heuristic_name_matches() {
    let a = r"from pkg import util


def helper():
    pass


class Engine:
    def run(self):
        helper()
        helper()
        util.go()
        size()
        unknown()

    @property
    def size(self):
        return 1

    @size.setter
    def size(self, v):
        pass


helper()
";
    let graph = build(&[("pkg/a.py", a), ("pkg/util.py", "def go():\n    pass\n")]);
    assert_well_formed(&graph);
    let run = symbol(Lang::Py, "pkg/a.py", "Engine.run");
    let calls = outgoing(&graph, Relation::Calls, &run);
    assert_eq!(
        calls,
        [
            row("sym:py:pkg/a.py#Engine.size", Confidence::Heuristic, 1),
            row("sym:py:pkg/a.py#Engine.size@2", Confidence::Heuristic, 1),
            row("sym:py:pkg/a.py#helper", Confidence::Heuristic, 2),
            row("sym:py:pkg/util.py#go", Confidence::Heuristic, 1),
        ]
    );
    let helper = symbol(Lang::Py, "pkg/a.py", "helper");
    let twice = edge(&graph, Relation::Calls, &run, &helper);
    assert_eq!(twice.specifier.as_deref(), Some("helper"));
    assert_eq!(twice.sites[0].span.start_line, 10);
    assert_eq!(twice.sites[1].span.start_line, 11);
    let at_module_level = module(Lang::Py, "pkg/a.py");
    let calls = outgoing(&graph, Relation::Calls, &at_module_level);
    assert_eq!(
        calls,
        [row("sym:py:pkg/a.py#helper", Confidence::Heuristic, 1)]
    );
}

// ADR 0003, "Cross-language edges" and invariant 9.
fn native() -> FileFacts {
    let path = "native/lib.rs";
    let symbols = ["add", "_core", "Counter"];
    let mut facts = FileFacts {
        nodes: vec![Node::module(Lang::Rs, path).unwrap()],
        ..FileFacts::default()
    };
    for (line, name) in (1..).zip(symbols) {
        let node = Node::symbol(Lang::Rs, path, name, name, SymbolKind::Function, span(line));
        facts.nodes.push(node.unwrap());
    }
    let export = |name: &str, framework, line| BindingExport {
        symbol: symbol(Lang::Rs, path, name),
        framework,
        exported_name: name.to_owned(),
        span: span(line),
    };
    facts.bindings = vec![
        export("add", BindingFramework::Pyo3, 1),
        export("_core", BindingFramework::Pyo3, 2),
        export("Counter", BindingFramework::Napi, 3),
        export("ghost", BindingFramework::WasmBindgen, 4),
    ];
    facts.imports = vec![ImportEdge {
        specifier: "super::add".to_owned(),
        names: vec!["add".to_owned()],
        span: span(5),
    }];
    facts
}

#[test]
fn python_and_typescript_bind_to_rust_exports_by_name() {
    let config = Config::default();
    let mut builder = Builder::new(&config);
    builder.add(
        &SourceFile {
            path: "native/lib.rs",
            text: "",
        },
        native(),
    );
    let source = "from native import add\nimport nativelib._core\n\n\ndef calc():\n    add(1, 2)\n    ping()\n\n\nadd(3, 4)\n";
    add_all(
        &mut builder,
        &[
            ("app/use.py", source),
            ("other/x.py", "from elsewhere import add\nadd()\n"),
            ("lonely.py", "def lonely():\n    add()\n"),
            (
                "web/x.ts",
                "import { Counter } from \"./native\";\nCounter();\n",
            ),
        ],
    );
    let graph = builder.build();
    assert_well_formed(&graph);
    let add = symbol(Lang::Rs, "native/lib.rs", "add");
    let counter = symbol(Lang::Rs, "native/lib.rs", "Counter");
    let user = module(Lang::Py, "app/use.py");
    let binds = outgoing(&graph, Relation::Binds, &user);
    assert_eq!(
        binds,
        [
            row("sym:rs:native/lib.rs#_core", Confidence::Heuristic, 1),
            row("sym:rs:native/lib.rs#add", Confidence::Heuristic, 2),
        ]
    );
    // The import comes first, then the module-level call.
    let both = edge(&graph, Relation::Binds, &user, &add);
    let lines: Vec<u32> = both.sites.iter().map(|s| s.span.start_line).collect();
    assert_eq!(lines, [1, 10]);
    let calc = symbol(Lang::Py, "app/use.py", "calc");
    let from_calc = outgoing(&graph, Relation::Binds, &calc);
    assert_eq!(
        from_calc,
        [row("sym:rs:native/lib.rs#add", Confidence::Heuristic, 1)]
    );
    // Matching is by exported name only: any module importing `add` binds.
    let other = module(Lang::Py, "other/x.py");
    assert_eq!(edge(&graph, Relation::Binds, &other, &add).count(), 2);
    // No import of an export, no edge; same-language imports never bind.
    let lonely = symbol(Lang::Py, "lonely.py", "lonely");
    assert_eq!(outgoing(&graph, Relation::Binds, &lonely), Vec::new());
    let rust = module(Lang::Rs, "native/lib.rs");
    assert_eq!(outgoing(&graph, Relation::Binds, &rust), Vec::new());
    let web = module(Lang::Ts, "web/x.ts");
    assert_eq!(edge(&graph, Relation::Binds, &web, &counter).count(), 2);
    // The ghost export names no node, and exporting symbols record it.
    assert!(
        !graph
            .nodes
            .contains_key(&symbol(Lang::Rs, "native/lib.rs", "ghost"))
    );
    assert_eq!(
        graph.nodes[&add].binding,
        Some(Binding {
            framework: BindingFramework::Pyo3,
            exported_name: "add".to_owned(),
        })
    );
    assert_eq!(graph.nodes[&user].binding, None);
}

// ADR 0004, "Tag precedence", through the graph: config entries, module
// inline, symbol inline, with `adr` accumulating.
#[test]
fn nodes_carry_their_effective_tags() {
    let config = config::parse(
        r#"
version = 1

[tags]
accumulate = ["adr"]

[[node]]
match = "src/**"
tags = { layer = "app", adr = "0001" }

[[node]]
package = "requests"
tags = { boundary = "net" }
"#,
    )
    .unwrap();
    let text = r"# modfence: layer=domain adr=0002

import requests


# modfence: adr=0003 owner=me
def f():
    pass


def g():
    pass
";
    let graph = build_with(
        &config,
        &[("src/a.py", text), ("lib/b.py", "def h():\n    pass\n")],
    );
    let tags = |id: &NodeId| -> Vec<String> {
        graph.nodes[id]
            .tags
            .iter()
            .map(|(key, values)| {
                format!(
                    "{key}={}",
                    values.iter().cloned().collect::<Vec<_>>().join(",")
                )
            })
            .collect()
    };
    assert_eq!(
        tags(&module(Lang::Py, "src/a.py")),
        ["adr=0001,0002", "layer=domain"]
    );
    assert_eq!(
        tags(&symbol(Lang::Py, "src/a.py", "f")),
        ["adr=0001,0002,0003", "layer=domain", "owner=me"]
    );
    assert_eq!(
        tags(&symbol(Lang::Py, "src/a.py", "g")),
        ["adr=0001,0002", "layer=domain"]
    );
    assert_eq!(tags(&external(Lang::Py, "requests")), ["boundary=net"]);
    assert_eq!(tags(&module(Lang::Py, "lib/b.py")), Vec::<String>::new());
    assert_eq!(
        tags(&id(NodeKind::Package, Lang::Py, "lib", None)),
        Vec::<String>::new()
    );
}

// ADR 0004, "Inline annotations" and "Suppressions": a trailing tag is
// ignored with a warning, a malformed annotation and an ignore without a
// reason are errors, and well-formed ignores are kept for the rules engine.
#[test]
fn annotation_findings_and_suppressions() {
    let text = r#"# modfence: ignore[no-new-legacy] reason="migrating"
import legacy  # modfence: ignore[a, b] reason="x"
import other  # modfence: ignore[c]
x = 1  # modfence: layer=a
# modfence: Bad=1
"#;
    let strict = build(&[("a.py", text)]);
    let severities: Vec<(u32, Severity)> = strict
        .findings
        .iter()
        .map(|f| (f.span.start_line, f.severity))
        .collect();
    assert_eq!(
        severities,
        [
            (3, Severity::Error),
            (4, Severity::Warning),
            (5, Severity::Error)
        ]
    );
    assert!(strict.findings[0].message.contains("needs reason"));
    assert!(strict.findings[1].message.contains("trailing"));
    assert!(
        strict.findings[2]
            .message
            .contains("invalid modfence annotation")
    );
    assert!(strict.findings.iter().all(|f| f.path == "a.py"));
    let suppressed: Vec<(u32, Vec<&str>, Option<&str>)> = strict
        .suppressions
        .iter()
        .map(|s| {
            let rules = s.rules.iter().map(String::as_str).collect();
            (s.span.start_line, rules, s.reason.as_deref())
        })
        .collect();
    assert_eq!(
        suppressed,
        [
            (1, vec!["no-new-legacy"], Some("migrating")),
            (2, vec!["a", "b"], Some("x")),
        ]
    );
    // The trailing tag does not tag the module.
    assert_eq!(strict.nodes[&module(Lang::Py, "a.py")].tags, Tags::new());

    let relaxed = config::parse("version = 1\n[annotations]\nrequire_reason = false\n").unwrap();
    let graph = build_with(&relaxed, &[("a.py", text)]);
    assert_eq!(graph.findings.len(), 2);
    assert_eq!(graph.suppressions.len(), 3);
    assert_eq!(graph.suppressions[2].reason, None);
}

// The same findings for TypeScript and TSX: a trailing tag warns and does not
// tag the module, a malformed annotation is an error.
#[test]
fn annotation_findings_for_typescript() {
    let text = "const x = 1; // modfence: layer=a\n// modfence: Bad=1\nexport const y = 2;\n";
    let graph = build(&[("a.ts", text), ("b.tsx", text)]);
    let found: Vec<(&str, u32, Severity)> = graph
        .findings
        .iter()
        .map(|f| (f.path.as_str(), f.span.start_line, f.severity))
        .collect();
    assert_eq!(
        found,
        [
            ("a.ts", 1, Severity::Warning),
            ("a.ts", 2, Severity::Error),
            ("b.tsx", 1, Severity::Warning),
            ("b.tsx", 2, Severity::Error),
        ]
    );
    assert!(graph.findings[0].message.contains("trailing"));
    for path in ["a.ts", "b.tsx"] {
        assert_eq!(graph.nodes[&module(Lang::Ts, path)].tags, Tags::new());
    }
}

// ADR 0003, invariant 5: a file that fails to parse yields a diagnostic and
// the facts that could be recovered.
#[test]
fn file_diagnostics_are_kept_and_sorted() {
    let graph = build(&[
        ("b.py", "def (:\n"),
        ("a.ts", "const = ;\n"),
        ("c.py", "x = 1\n"),
    ]);
    let found: Vec<(&str, DiagnosticCode)> = graph
        .diagnostics
        .iter()
        .map(|d| (d.path.as_str(), d.code))
        .collect();
    assert_eq!(
        found,
        [
            ("a.ts", DiagnosticCode::ParseError),
            ("b.py", DiagnosticCode::ParseError)
        ]
    );
    assert!(graph.nodes.contains_key(&module(Lang::Py, "c.py")));
    assert_well_formed(&graph);
}

// ADR 0003, invariant 1: the same inputs give the same graph, whatever the
// order the files were added in.
#[test]
fn the_graph_does_not_depend_on_the_order_of_files() {
    let files = [
        ("pkg/a.py", "from pkg import b\nb.f()\n"),
        ("pkg/b.py", "def f():\n    pass\n"),
        ("web/x.ts", "import y from \"./y\";\ny();\n"),
        ("web/y.ts", "export default function y() {}\n"),
    ];
    let forward = build(&files);
    let mut reversed = files;
    reversed.reverse();
    assert_eq!(forward, build(&reversed));
    assert_well_formed(&forward);
}
