//! The rules engine (ADR 0004): evaluation per site, layers, the default
//! effect, suppressions, the built-in rules and the notices.

use modfence::config::{self, Config, Severity};
use modfence::extract::python::PythonExtractor;
use modfence::extract::{ExtractCtx, Extractor, Lang, NodeId, NodeKind, SourceFile, Span, Tags};
use modfence::graph::{Builder, Confidence, Edge, Graph, INVALID_ANNOTATION, Node, Relation, Site};
use modfence::rules::{
    self, CALLS_NOT_CHECKED, DEFAULT_DENY, GITHUB_RESULT_LIMIT, LAYERS, Notice, Outcome,
    UNUSED_IGNORE,
};

fn cfg(text: &str) -> Config {
    config::parse(&format!("version = 1\n{text}")).expect("a valid config")
}

fn build(config: &Config, files: &[(&str, &str)]) -> Graph {
    let mut builder = Builder::new(config);
    for &(path, text) in files {
        let file = SourceFile { path, text };
        let facts = PythonExtractor.extract(&file, &ExtractCtx::default());
        builder.add(&file, facts);
    }
    builder.build()
}

fn run(config: &Config, files: &[(&str, &str)]) -> Outcome {
    rules::check(&build(config, files), config, true)
}

/// Every violation as `rule@path:line`, in the outcome's order.
fn found(outcome: &Outcome) -> String {
    let mut items = Vec::new();
    for v in &outcome.violations {
        items.push(format!("{}@{}:{}", v.rule, v.path, v.span.start_line));
    }
    items.join(" ")
}

/// A forbid rule with two selectors and extra lines.
fn rule(id: &str, from: &str, to: &str, extra: &str) -> String {
    let head = format!("[[rule]]\nid = \"{id}\"\nfrom = {from}\nto = {to}\n");
    format!("{head}effect = \"forbid\"\n{extra}")
}

/// The files of the `app` fixture, with `ui` as `app/ui.py`.
fn app(ui: &str) -> Vec<(&str, &str)> {
    vec![
        ("app/__init__.py", ""),
        ("app/db.py", ""),
        ("app/ui.py", ui),
    ]
}

const WHY: &str = "reason=\"r\"";
const UNUSED: &str = "modfence/unused-ignore@app/ui.py";
const INVALID: &str = "modfence/invalid-annotation@app/ui.py";

const NO_DB: &str = r#"
[[rule]]
id = "no-db"
from = { path = "app/ui.py" }
to = { path = "app/db.py" }
effect = "forbid"
"#;

const ALLOW_DB: &str = r#"
[[rule]]
id = "ok-db"
from = { path = "app/**" }
to = { path = "app/db.py" }
effect = "allow"
"#;

// ADR 0004, "Evaluation" step 2 and invariant 5: a forbid rule reports every
// site at its own position, with the rule's severity and the edge it judged.
#[test]
fn a_forbid_rule_reports_each_site() {
    let outcome = run(&cfg(NO_DB), &app("import app.db\nx = 1\nimport app.db\n"));
    assert_eq!(found(&outcome), "no-db@app/ui.py:1 no-db@app/ui.py:3");
    let first = &outcome.violations[0];
    assert_eq!(first.severity, Severity::Error);
    assert_eq!(
        first.message,
        "app/ui.py must not depend on app/db.py (rule `no-db`)"
    );
    assert_eq!((first.span.start_line, first.span.start_column), (1, 1));
    let edge = first.dependency.as_ref().expect("an edge violation");
    assert_eq!(edge.from.id.as_str(), "mod:py:app/ui.py");
    assert_eq!(edge.from.path.as_deref(), Some("app/ui.py"));
    assert_eq!(edge.to.id.as_str(), "mod:py:app/db.py");
    assert_eq!(edge.relation, Relation::Imports);
    assert_eq!(edge.confidence, Confidence::Resolved);
}

#[test]
fn a_rule_sets_the_severity_and_the_message() {
    let text = format!("{NO_DB}severity = \"warning\"\nmessage = \"use the service\"\n");
    let outcome = run(&cfg(&text), &app("import app.db\n"));
    assert_eq!(outcome.violations[0].severity, Severity::Warning);
    assert_eq!(outcome.violations[0].message, "use the service");
    assert!(!outcome.fails(Severity::Error));
    assert!(outcome.fails(Severity::Warning));
    assert!(outcome.fails(Severity::Note));
}

// ADR 0004, invariant 3: explicit rules are tried in file order and the first
// match decides, so an allow ahead of a forbid permits the site.
#[test]
fn the_first_matching_rule_decides() {
    let source = app("import app.db\n");
    let allowed = run(&cfg(&format!("{ALLOW_DB}{NO_DB}")), &source);
    assert_eq!(found(&allowed), "");
    let forbidden = run(&cfg(&format!("{NO_DB}{ALLOW_DB}")), &source);
    assert_eq!(found(&forbidden), "no-db@app/ui.py:1");
}

// ADR 0004, "Rules": a rule looks at the relations in `kinds`.
#[test]
fn kinds_limit_the_relations_a_rule_sees() {
    let ui = "from app.db import query\n\n\ndef run():\n    query()\n";
    let mut files = app(ui);
    files[1] = ("app/db.py", "def query():\n    pass\n");
    let any = format!("{NO_DB}confidence = \"any\"\n");
    let both = run(&cfg(&any), &files);
    assert_eq!(found(&both), "no-db@app/ui.py:1 no-db@app/ui.py:5");
    let call = both.violations[1].dependency.as_ref().unwrap();
    assert_eq!(call.relation, Relation::Calls);
    assert_eq!(call.confidence, Confidence::Heuristic);
    assert_eq!(call.from.id.as_str(), "mod:py:app/ui.py");
    assert_eq!(call.to.id.as_str(), "mod:py:app/db.py");
    let imports = format!("{any}kinds = [\"imports\"]\n");
    assert_eq!(found(&run(&cfg(&imports), &files)), "no-db@app/ui.py:1");
    let calls = format!("{any}kinds = [\"calls\"]\n");
    assert_eq!(found(&run(&cfg(&calls), &files)), "no-db@app/ui.py:5");
}

// ADR 0004, "Call edges in rules" and invariant 7: with `confidence` omitted a
// heuristic call never matches and an import always can.
#[test]
fn heuristic_calls_are_ignored_unless_a_rule_opts_in() {
    let ui = "from app.db import query\n\n\ndef run():\n    query()\n";
    let mut files = app(ui);
    files[1] = ("app/db.py", "def query():\n    pass\n");
    assert_eq!(found(&run(&cfg(NO_DB), &files)), "no-db@app/ui.py:1");
    let resolved = format!("{NO_DB}confidence = \"resolved\"\n");
    assert_eq!(found(&run(&cfg(&resolved), &files)), "no-db@app/ui.py:1");
}

fn module_node(path: &str, layers: &[&str]) -> Node {
    let mut tags = Tags::new();
    if !layers.is_empty() {
        let values = layers.iter().map(|layer| (*layer).to_owned()).collect();
        tags.insert("layer".to_owned(), values);
    }
    Node {
        id: NodeId::new(NodeKind::Module, Lang::Py, path, None).unwrap(),
        kind: NodeKind::Module,
        lang: Lang::Py,
        label: path.to_owned(),
        path: Some(path.to_owned()),
        symbol_kind: None,
        span: None,
        module: None,
        binding: None,
        tags,
    }
}

fn edge(from: &Node, to: &Node, relation: Relation, confidence: Confidence, lines: &[u32]) -> Edge {
    let site = |line: &u32| Site {
        path: from.path.clone().unwrap(),
        span: Span::new(*line, 1, *line, 5).unwrap(),
    };
    Edge {
        source: from.id.clone(),
        target: to.id.clone(),
        relation,
        confidence: Some(confidence),
        specifier: Some("x".to_owned()),
        sites: lines.iter().map(site).collect(),
    }
}

fn graph_of(nodes: &[&Node], edges: Vec<Edge>) -> Graph {
    let mut graph = Graph::default();
    for node in nodes {
        graph.nodes.insert(node.id.clone(), (*node).clone());
    }
    graph.edges = edges;
    graph
}

/// Edges `a -> b` of one relation and each confidence, checked by `config`.
fn judge(config: &str, relation: Relation, confidence: Confidence) -> String {
    let (a, b) = (module_node("a.py", &[]), module_node("b.py", &[]));
    let graph = graph_of(&[&a, &b], vec![edge(&a, &b, relation, confidence, &[1])]);
    let config = cfg(config);
    found(&rules::check(&graph, &config, true))
}

const A_TO_B: &str = "[[rule]]\nid = \"r\"\nfrom = { path = \"a.py\" }\nto = { path = \"b.py\" }\n";

// ADR 0004, invariant 7: the matrix of relation, confidence and `confidence`.
#[test]
fn confidence_decides_which_edges_a_rule_counts() {
    let forbid = format!("{A_TO_B}effect = \"forbid\"\n");
    let resolved = format!("{forbid}confidence = \"resolved\"\n");
    let any = format!("{forbid}confidence = \"any\"\n");
    let (calls, imports) = (Relation::Calls, Relation::Imports);
    let (sure, guess) = (Confidence::Resolved, Confidence::Heuristic);
    let hit = "r@a.py:1";
    assert_eq!(judge(&forbid, calls, sure), hit);
    assert_eq!(judge(&forbid, calls, guess), "");
    assert_eq!(judge(&forbid, imports, guess), hit);
    assert_eq!(judge(&forbid, Relation::Binds, guess), hit);
    assert_eq!(judge(&resolved, calls, sure), hit);
    assert_eq!(judge(&resolved, calls, guess), "");
    assert_eq!(judge(&resolved, imports, guess), "");
    assert_eq!(judge(&any, calls, guess), hit);
    assert_eq!(judge(&any, imports, guess), hit);
    let kinds = "kinds = [\"binds\", \"imports\"]\n";
    let binds_only = format!("{forbid}{kinds}confidence = \"resolved\"\n");
    assert_eq!(judge(&binds_only, Relation::Binds, guess), "");
    assert_eq!(judge(&binds_only, imports, sure), hit);
    let calls_only = format!("{forbid}kinds = [\"calls\"]\n");
    assert_eq!(judge(&calls_only, imports, sure), "");
}

// ADR 0004, invariant 4: edges inside one module, edges to the module's own
// symbols, `contains` edges and edges to nodes the graph lacks are not judged.
#[test]
fn only_edges_between_two_modules_are_judged() {
    let (a, b) = (module_node("a.py", &[]), module_node("b.py", &[]));
    let ghost = module_node("ghost.py", &[]);
    let contains = Edge {
        relation: Relation::Contains,
        confidence: None,
        specifier: None,
        sites: Vec::new(),
        ..edge(&a, &b, Relation::Contains, Confidence::Resolved, &[])
    };
    let imports = Confidence::Resolved;
    let edges = vec![
        contains,
        edge(&a, &a, Relation::Imports, imports, &[1]),
        edge(&a, &ghost, Relation::Imports, imports, &[2]),
        edge(&ghost, &b, Relation::Imports, imports, &[3]),
        edge(&a, &b, Relation::Imports, imports, &[4]),
    ];
    let graph = graph_of(&[&a, &b], edges);
    let config = cfg(&format!("{A_TO_B}effect = \"forbid\"\n"));
    assert_eq!(found(&rules::check(&graph, &config, true)), "r@a.py:4");
}

#[test]
fn a_site_with_an_imports_and_a_binds_edge_is_reported_once() {
    let (a, b) = (module_node("a.py", &[]), module_node("b.py", &[]));
    let edges = vec![
        edge(&a, &b, Relation::Imports, Confidence::Resolved, &[1]),
        edge(&a, &b, Relation::Binds, Confidence::Heuristic, &[1]),
    ];
    let graph = graph_of(&[&a, &b], edges);
    let config = cfg(&format!("{A_TO_B}effect = \"forbid\"\n"));
    assert_eq!(found(&rules::check(&graph, &config, true)), "r@a.py:1");
}

#[test]
fn calls_between_symbols_of_one_module_are_not_judged() {
    let text = "def f():\n    pass\n\n\ndef g():\n    f()\n";
    let any = "{ path = \"**\" }";
    let config = cfg(&rule("r", any, any, "confidence = \"any\"\n"));
    assert_eq!(found(&run(&config, &[("a.py", text)])), "");
}

// ADR 0004, "Rules": selectors match module tags, packages and kinds, and the
// unplaced import of ADR 0003 (invariant 6) is no package.
#[test]
fn selectors_see_tags_packages_and_unplaced_imports() {
    let nodes = "[[node]]\nmatch = \"app/**\"\ntags = { owner = \"web\" }\n";
    let files = app("import requests\nfrom .missing import x\n");
    let check = |to: &str| {
        let text = format!("{nodes}{}", rule("r", "{ owner = \"web\" }", to, ""));
        run(&cfg(&text), &files)
    };
    assert_eq!(found(&check("{ package = \"req*\" }")), "r@app/ui.py:1");
    assert_eq!(found(&check("{ package = \"**\" }")), "r@app/ui.py:1");
    assert_eq!(found(&check("{ owner = \"web\" }")), "");
    let external = check("{ kind = \"external\" }");
    assert_eq!(found(&external), "r@app/ui.py:1 r@app/ui.py:2");
    let heuristic = external.violations[1].dependency.as_ref().unwrap();
    assert_eq!(heuristic.confidence, Confidence::Heuristic);
    assert_eq!(heuristic.to.path, None);
}

// ADR 0004, "Layers" and invariants 3 and 10: a lower layer must not use a
// higher one, per site and with the reserved id `layers`.
const LAYERED: &str = r#"
layers = ["app", "infra", "domain"]
[[node]]
match = "app/**"
tags = { layer = "app" }
[[node]]
match = "infra/**"
tags = { layer = "infra" }
[[node]]
match = "domain/**"
tags = { layer = "domain" }
[[node]]
match = "mixed/**"
tags = { layer = ["app", "domain"] }
[[node]]
match = "odd/**"
tags = { layer = "other" }
"#;

fn layered_files<'a>(extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
    let mut files = vec![
        ("app/__init__.py", ""),
        (
            "app/ui.py",
            "import infra.db\nimport domain.model\nimport app.view\n",
        ),
        ("app/view.py", ""),
        ("infra/__init__.py", ""),
        ("infra/db.py", "import domain.model\nimport app.ui\n"),
        ("domain/__init__.py", ""),
        ("domain/model.py", "import app.ui\n"),
    ];
    files.extend_from_slice(extra);
    files
}

#[test]
fn layers_forbid_dependencies_on_a_higher_layer() {
    let outcome = run(&cfg(LAYERED), &layered_files(&[]));
    assert_eq!(
        found(&outcome),
        "layers@domain/model.py:1 layers@infra/db.py:2"
    );
    let first = &outcome.violations[0];
    assert_eq!(
        (first.severity, first.rule.as_str()),
        (Severity::Error, LAYERS)
    );
    assert_eq!(
        first.message,
        "layer `domain` (domain/model.py) must not depend on layer `app` (app/ui.py)"
    );
}

#[test]
fn layers_cannot_be_overridden_and_see_every_layer_value() {
    let (from, to) = ("{ layer = \"domain\" }", "{ layer = \"app\" }");
    let allow = rule("a", from, to, "").replace("forbid", "allow");
    let allowed = run(&cfg(&format!("{LAYERED}{allow}")), &layered_files(&[]));
    assert_eq!(
        found(&allowed),
        "layers@domain/model.py:1 layers@infra/db.py:2"
    );
    // A node with the layers app and domain, and nodes outside the list.
    let extra = [
        ("mixed/__init__.py", ""),
        ("mixed/m.py", "import infra.db\n"),
        ("odd/__init__.py", ""),
        ("odd/o.py", "import app.ui\nimport infra.db\n"),
        ("plain.py", "import app.ui\nimport mixed.m\n"),
        ("app/to_odd.py", "import odd.o\nimport plain\n"),
    ];
    let outcome = run(&cfg(LAYERED), &layered_files(&extra));
    let pairs = "layers@domain/model.py:1 layers@infra/db.py:2 layers@mixed/m.py:1";
    assert_eq!(found(&outcome), pairs);
}

#[test]
fn layers_count_calls_only_when_resolved() {
    let (high, low) = (
        module_node("hi.py", &["app"]),
        module_node("lo.py", &["domain"]),
    );
    let config = cfg("layers = [\"app\", \"domain\"]\n");
    let judge = |confidence| {
        let edges = vec![edge(&low, &high, Relation::Calls, confidence, &[7])];
        found(&rules::check(
            &graph_of(&[&high, &low], edges),
            &config,
            true,
        ))
    };
    assert_eq!(judge(Confidence::Resolved), "layers@lo.py:7");
    assert_eq!(judge(Confidence::Heuristic), "");
}

// ADR 0004, "Default effect" and invariants 9 and 10.
const DENY: &str = "default = \"deny\"\n";

#[test]
fn deny_mode_reports_undecided_internal_sites() {
    let outcome = run(
        &cfg(DENY),
        &app("import app.db\nimport requests\nfrom .gone import y\n"),
    );
    assert_eq!(found(&outcome), "default-deny@app/ui.py:1");
    let violation = &outcome.violations[0];
    assert_eq!(violation.rule, DEFAULT_DENY);
    assert_eq!(violation.severity, Severity::Error);
    assert_eq!(violation.message, "No rule allows this dependency.");
    let allowed = run(&cfg(&format!("{DENY}{ALLOW_DB}")), &app("import app.db\n"));
    assert_eq!(found(&allowed), "");
    let forbidden = run(&cfg(&format!("{DENY}{NO_DB}")), &app("import app.db\n"));
    assert_eq!(found(&forbidden), "no-db@app/ui.py:1");
}

#[test]
fn layers_permit_sites_in_deny_mode_but_never_add_default_deny() {
    let extra = [
        ("util.py", "import infra.db\n"),
        ("odd/o.py", "import app.ui\n"),
    ];
    let text = format!("{DENY}{LAYERED}");
    let outcome = run(&cfg(&text), &layered_files(&extra));
    let layers = "layers@domain/model.py:1 layers@infra/db.py:2";
    let denied = "default-deny@odd/o.py:1 default-deny@util.py:1";
    assert_eq!(found(&outcome), format!("{denied} {layers}"));
}

#[test]
fn deny_mode_ignores_heuristic_calls() {
    let (a, b) = (module_node("a.py", &[]), module_node("b.py", &[]));
    let config = cfg(DENY);
    let judge = |confidence| {
        let edges = vec![edge(&a, &b, Relation::Calls, confidence, &[3])];
        found(&rules::check(&graph_of(&[&a, &b], edges), &config, true))
    };
    assert_eq!(judge(Confidence::Resolved), "default-deny@a.py:3");
    assert_eq!(judge(Confidence::Heuristic), "");
}

// ADR 0004, "Suppressions" and invariants 16 and 17.
fn ignore(ids: &str) -> String {
    format!("# modfence: ignore[{ids}] {WHY}\n")
}

fn trailing(ids: &str) -> String {
    format!("  # modfence: ignore[{ids}] {WHY}\n")
}

fn ignored(ui: &str) -> Outcome {
    run(&cfg(NO_DB), &app(ui))
}

#[test]
fn an_ignore_on_the_line_or_the_line_above_suppresses_a_site() {
    let above = "# modfence: ignore[no-db] reason=\"legacy\"\nimport app.db\n";
    assert_eq!(found(&ignored(above)), "");
    let trailing = "import app.db  # modfence: ignore[no-db] reason=\"legacy\"\n";
    assert_eq!(found(&ignored(trailing)), "");
    let multiline = "# modfence: ignore[no-db] reason=\"legacy\"\nfrom app import (\n    db,\n)\n";
    assert_eq!(found(&ignored(multiline)), "");
}

#[test]
fn an_ignore_is_exact_about_rule_and_line() {
    let far = "# modfence: ignore[no-db] reason=\"r\"\nx = 1\nimport app.db\n";
    assert_eq!(
        found(&ignored(far)),
        "no-db@app/ui.py:3 modfence/unused-ignore@app/ui.py:1"
    );
    let other = "# modfence: ignore[other] reason=\"r\"\nimport app.db\n";
    assert_eq!(
        found(&ignored(other)),
        "no-db@app/ui.py:2 modfence/unused-ignore@app/ui.py:1"
    );
    let below = "import app.db\n# modfence: ignore[no-db] reason=\"r\"\n";
    assert_eq!(
        found(&ignored(below)),
        "no-db@app/ui.py:1 modfence/unused-ignore@app/ui.py:2"
    );
    let mut files = app("import app.db\n");
    files.push(("app/other.py", "# modfence: ignore[no-db] reason=\"r\"\n"));
    let outcome = run(&cfg(NO_DB), &files);
    assert_eq!(
        found(&outcome),
        "no-db@app/ui.py:1 modfence/unused-ignore@app/other.py:1"
    );
}

#[test]
fn an_unused_ignore_is_a_warning_naming_the_rule() {
    let outcome = ignored("# modfence: ignore[no-db, layers] reason=\"r\"\nimport app.db\n");
    assert_eq!(found(&outcome), "modfence/unused-ignore@app/ui.py:1");
    let warning = &outcome.violations[0];
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(warning.message, "ignore[layers] suppressed nothing");
    assert_eq!(warning.dependency, None);
    assert_eq!(warning.rule, UNUSED_IGNORE);
}

#[test]
fn one_comment_may_suppress_a_site_from_two_places() {
    let both = format!("{}import app.db{}", ignore("no-db"), trailing("no-db"));
    assert_eq!(found(&ignored(&both)), "");
}

#[test]
fn built_in_rules_can_be_ignored_too() {
    let first = ignore("modfence/invalid-annotation");
    let invalid = format!("{first}# modfence: Bad=1\n");
    assert_eq!(found(&ignored(&invalid)), "");
    let unused = format!("{}{}", ignore(UNUSED_IGNORE), ignore("x"));
    assert_eq!(found(&ignored(&unused)), "");
    let layered = format!("import app.ui{}", trailing("layers"));
    let files = layered_files(&[("domain/z.py", layered.as_str())]);
    let outcome = run(&cfg(LAYERED), &files);
    assert_eq!(
        found(&outcome),
        "layers@domain/model.py:1 layers@infra/db.py:2"
    );
}

#[test]
fn an_unused_ignore_of_unused_ignore_is_reported_last() {
    let alone = "# modfence: ignore[modfence/unused-ignore] reason=\"r\"\n";
    assert_eq!(found(&ignored(alone)), "modfence/unused-ignore@app/ui.py:1");
    // A comment cannot silence the warning about itself.
    let itself = "# modfence: ignore[modfence/unused-ignore, x] reason=\"r\"\n";
    let outcome = ignored(itself);
    assert_eq!(found(&outcome), format!("{UNUSED}:1 {UNUSED}:1"));
    assert_eq!(
        outcome.violations[0].message,
        "ignore[modfence/unused-ignore] suppressed nothing"
    );
    assert_eq!(
        outcome.violations[1].message,
        "ignore[x] suppressed nothing"
    );
}

// ADR 0004, "Suppressions": a malformed annotation is an error and a trailing
// tag a warning, under `modfence/invalid-annotation`.
#[test]
fn annotation_findings_become_violations() {
    let ui = "x = 1  # modfence: layer=a\n# modfence: Bad=1\n";
    let outcome = ignored(ui);
    assert_eq!(found(&outcome), format!("{INVALID}:2 {INVALID}:1"));
    let (error, warning) = (&outcome.violations[0], &outcome.violations[1]);
    assert_eq!(
        (error.severity, warning.severity),
        (Severity::Error, Severity::Warning)
    );
    assert_eq!(error.rule, INVALID_ANNOTATION);
    assert_eq!(error.dependency, None);
    assert!(warning.message.contains("trailing"));
}

#[test]
fn violations_are_sorted_by_severity_rule_path_and_line() {
    let (a, b) = (module_node("a.py", &[]), module_node("b.py", &[]));
    let c = module_node("c.py", &[]);
    let sure = Confidence::Resolved;
    let edges = vec![
        edge(&c, &b, Relation::Imports, sure, &[9, 2]),
        edge(&a, &b, Relation::Imports, sure, &[5]),
        edge(&a, &c, Relation::Imports, sure, &[5]),
        edge(&b, &a, Relation::Imports, sure, &[1]),
    ];
    let graph = graph_of(&[&a, &b, &c], edges);
    let all = "{ path = \"**\" }";
    let text = [
        rule("aa", "{ path = \"a.py\" }", "{ path = \"c.py\" }", ""),
        rule(
            "zz",
            "{ path = \"a.py\" }",
            "{ path = \"b.py\" }",
            "severity = \"note\"\n",
        ),
        rule("mm", all, all, "severity = \"warning\"\n"),
    ]
    .concat();
    let config = cfg(&text);
    let outcome = rules::check(&graph, &config, true);
    assert_eq!(
        found(&outcome),
        "aa@a.py:5 mm@b.py:1 mm@c.py:2 mm@c.py:9 zz@a.py:5"
    );
}

// Notices.
fn notices(config: &str, files: &[(&str, &str)], precise: bool) -> Vec<Notice> {
    let config = cfg(config);
    rules::check(&build(&config, files), &config, precise).notices
}

#[test]
fn a_run_without_precise_says_call_edges_were_not_checked() {
    let imprecise = notices("", &[("a.py", "")], false);
    assert_eq!(imprecise.len(), 1);
    assert_eq!(imprecise[0].level, Severity::Note);
    assert_eq!(imprecise[0].message, CALLS_NOT_CHECKED);
    let config = Config::default();
    let precise = rules::check(&build(&config, &[("a.py", "")]), &config, true);
    assert!(precise.notices.is_empty() && precise.precise);
    assert!(!rules::check(&Graph::default(), &config, false).precise);
}

#[test]
fn rules_that_can_never_match_are_noticed() {
    let any = "{ path = \"**\" }";
    let calls = "kinds = [\"calls\"]\n";
    let text = [
        rule("ext-from", "{ kind = \"external\" }", any, ""),
        rule("pkg-from", "{ package = \"x\" }", any, ""),
        rule("sym-to", any, "{ kind = [\"symbol\", \"package\"] }", ""),
        rule("calls-pkg", any, "{ package = \"x\" }", calls),
        rule("path-ext", any, "{ path = \"a\", kind = \"external\" }", ""),
        rule("ok-pkg", any, "{ package = \"x\" }", ""),
        rule(
            "ok-ext",
            any,
            "{ kind = [\"external\", \"module\"] }",
            calls,
        ),
        rule("ok-tag", "{ owner = \"a\" }", "{ owner = \"b\" }", ""),
    ]
    .concat();
    let found = notices(&text, &[], true);
    let mut expected = Vec::new();
    let dead = [
        ("ext-from", "from"),
        ("pkg-from", "from"),
        ("sym-to", "to"),
        ("calls-pkg", "to"),
        ("path-ext", "to"),
    ];
    for (id, end) in dead {
        let tail = format!("no dependency has a node its `{end}` selector could select");
        expected.push(format!("rule `{id}` can never match: {tail}"));
    }
    let messages: Vec<String> = found.iter().map(|n| n.message.clone()).collect();
    assert_eq!(messages, expected);
    assert!(found.iter().all(|n| n.level == Severity::Warning));
}

#[test]
fn layers_without_a_layer_tag_are_noticed() {
    let layers = "layers = [\"a\", \"b\"]\n";
    let found = notices(layers, &[("x.py", "")], true);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].level, Severity::Warning);
    assert!(found[0].message.contains("no node has a `layer` tag"));
    let tagged = format!("{layers}[[node]]\nmatch = \"x.py\"\ntags = {{ layer = \"a\" }}\n");
    assert_eq!(
        notices(&tagged, &[("x.py", "")], true),
        Vec::<Notice>::new()
    );
    assert_eq!(notices("", &[("x.py", "")], true), Vec::<Notice>::new());
}

#[test]
fn more_results_than_github_keeps_are_noticed() {
    let (a, b) = (module_node("a.py", &[]), module_node("b.py", &[]));
    let lines: Vec<u32> = (1..=5001).collect();
    let edges = vec![edge(
        &a,
        &b,
        Relation::Imports,
        Confidence::Resolved,
        &lines,
    )];
    let graph = graph_of(&[&a, &b], edges);
    let config = cfg(&format!("{A_TO_B}effect = \"forbid\"\n"));
    let outcome = rules::check(&graph, &config, true);
    assert_eq!(outcome.violations.len(), GITHUB_RESULT_LIMIT + 1);
    assert_eq!(outcome.notices.len(), 1);
    let expected = "5001 results; GitHub code scanning keeps only the top 5000";
    assert_eq!(outcome.notices[0].message, expected);
    assert_eq!(outcome.notices[0].level, Severity::Warning);
}

// Follow-up of NC4a: a selector's tag values are checked like a node's.
#[test]
fn selector_tag_values_are_validated_like_node_tag_values() {
    for bad in ["\"\"", "\"a\\\"b\"", "[\"ok\", \"a\\nb\"]"] {
        let head = "version = 1\n[[rule]]\nid = \"r\"\n";
        let tail = "to = { path = \"x\" }\neffect = \"forbid\"\n";
        let text = format!("{head}from = {{ owner = {bad} }}\n{tail}");
        let error = config::parse(&text).expect_err("a bad tag value");
        assert!(
            error.message.contains("a tag value must not be empty"),
            "{error}"
        );
        assert_eq!(error.line, Some(4));
    }
}
