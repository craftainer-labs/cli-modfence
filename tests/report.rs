//! The reports (ADR 0004, "Reports"): SARIF 2.1.0 and GitHub annotations.

use serde_json::{Value, json};

use modfence::config::{self, Config, Severity};
use modfence::extract::python::PythonExtractor;
use modfence::extract::{ExtractCtx, Extractor, Lang, NodeId, NodeKind, SourceFile, Span, Tags};
use modfence::graph::{Builder, Confidence, INVALID_ANNOTATION, Relation};
use modfence::report::{github, sarif};
use modfence::rules::{
    self, CALLS_NOT_CHECKED, Dependency, Endpoint, Notice, Outcome, UNUSED_IGNORE, Violation,
};

const GOLDEN: &str = include_str!("golden/report.sarif.json");

const NO_B: &str = r#"
[[rule]]
id = "no-b"
from = { path = "a.py" }
to = { path = "b.py" }
effect = "forbid"
severity = "warning"
message = "no b"
"#;

fn cfg(text: &str) -> Config {
    config::parse(&format!("version = 1\n{text}")).expect("a valid config")
}

fn span(line: u32, column: u32, end_column: u32) -> Span {
    Span::new(line, column, line, end_column).unwrap()
}

fn endpoint(path: &str, tags: &[(&str, &[&str])]) -> Endpoint {
    let mut map = Tags::new();
    for (key, values) in tags {
        let values = values.iter().map(|value| (*value).to_owned()).collect();
        map.insert((*key).to_owned(), values);
    }
    Endpoint {
        id: NodeId::new(NodeKind::Module, Lang::Py, path, None).unwrap(),
        path: Some(path.to_owned()),
        tags: map,
    }
}

fn violation(severity: Severity, rule: &str, path: &str, span: Span, message: &str) -> Violation {
    Violation {
        severity,
        rule: rule.to_owned(),
        path: path.to_owned(),
        span,
        dependency: None,
        message: message.to_owned(),
    }
}

fn edge_violation(span: Span) -> Violation {
    let dependency = Dependency {
        from: endpoint("a.py", &[("layer", &["app"]), ("owner", &["web"])]),
        to: endpoint("b.py", &[]),
        relation: Relation::Imports,
        confidence: Confidence::Resolved,
    };
    Violation {
        dependency: Some(dependency),
        ..violation(Severity::Warning, "no-b", "a.py", span, "no b")
    }
}

fn outcome(violations: Vec<Violation>) -> Outcome {
    Outcome {
        violations,
        notices: Vec::new(),
        precise: true,
    }
}

fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("valid JSON")
}

/// The one run of a SARIF document.
fn run_of(document: &Value) -> &Value {
    &document["runs"][0]
}

/// Drops the version, which is stamped at release time.
fn without_version(mut document: Value) -> Value {
    let driver = document["runs"][0]["tool"]["driver"]
        .as_object_mut()
        .unwrap();
    driver.remove("version");
    document
}

// ADR 0004, "Reports" (SARIF) and invariants 18 and 19: the whole document for
// a fixed outcome, including fingerprints (`:2` for the second result with the
// same hash) and the tool-execution notification.
#[test]
fn the_sarif_document_matches_the_golden_file() {
    let violations = vec![
        violation(
            Severity::Error,
            INVALID_ANNOTATION,
            "c.py",
            span(2, 1, 30),
            "invalid modfence annotation",
        ),
        violation(
            Severity::Warning,
            UNUSED_IGNORE,
            "c.py",
            span(5, 1, 40),
            "ignore[x] suppressed nothing",
        ),
        edge_violation(span(3, 5, 20)),
        edge_violation(span(8, 5, 20)),
    ];
    let notice = Notice {
        level: Severity::Note,
        message: CALLS_NOT_CHECKED.to_owned(),
    };
    let noted = Outcome {
        notices: vec![notice],
        precise: false,
        ..outcome(violations)
    };
    let text = sarif::render(&noted, &cfg(NO_B));
    assert!(text.ends_with("}\n"));
    let document = parse(&text);
    let driver = &run_of(&document)["tool"]["driver"];
    assert_eq!(driver["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(without_version(document), without_version(parse(GOLDEN)));
}

// ADR 0004, invariant 18: every result has a location with the four region
// fields, a fingerprint and properties; every rule has its texts.
#[test]
fn every_result_and_rule_has_what_github_requires() {
    let head = "[[rule]]\nid = \"ok\"\nfrom = { path = \"a\" }\nto = { path = \"b\" }\n";
    let allow = format!("{head}effect = \"allow\"\n");
    let forbid = allow
        .replace("allow", "forbid")
        .replace("\"ok\"", "\"bad\"");
    let config = cfg(&format!("{allow}{forbid}{NO_B}"));
    let text = sarif::render(&outcome(vec![edge_violation(span(1, 1, 5))]), &config);
    let document = parse(&text);
    let run = run_of(&document);
    assert_eq!(document["version"], "2.1.0");
    assert_eq!(run["columnKind"], "unicodeCodePoints");
    let rules = run["tool"]["driver"]["rules"].as_array().unwrap();
    let ids: Vec<&str> = rules.iter().map(|r| r["id"].as_str().unwrap()).collect();
    let builtins = ["layers", "default-deny", INVALID_ANNOTATION, UNUSED_IGNORE];
    let mut expected: Vec<&str> = builtins.to_vec();
    expected.extend(["ok", "bad", "no-b"]);
    assert_eq!(ids, expected);
    for rule in rules {
        for key in ["shortDescription", "fullDescription", "help"] {
            assert!(!rule[key]["text"].as_str().unwrap().is_empty(), "{rule}");
        }
    }
    assert_eq!(
        rules[4]["fullDescription"]["text"],
        "Allows the dependencies that rule `ok` selects."
    );
    assert_eq!(
        rules[5]["fullDescription"]["text"],
        "Forbids the dependencies that rule `bad` selects."
    );
    let result = &run["results"][0];
    let region = &result["locations"][0]["physicalLocation"]["region"];
    for key in ["startLine", "startColumn", "endLine", "endColumn"] {
        assert!(region[key].is_u64(), "{key}");
    }
    let fingerprint = &result["partialFingerprints"]["primaryLocationLineHash"];
    assert!(fingerprint.as_str().unwrap().ends_with(":1"));
    assert_eq!(
        result["properties"]["from"]["tags"]["owner"],
        json!(["web"])
    );
}

#[test]
fn a_precise_run_records_that_call_edges_were_checked() {
    let text = sarif::render(&outcome(Vec::new()), &Config::default());
    let document = parse(&text);
    let run = run_of(&document);
    let modfence = json!({ "precise": true, "callEdgesChecked": true });
    assert_eq!(run["properties"]["modfence"], modfence);
    assert_eq!(
        run["invocations"][0]["toolExecutionNotifications"],
        json!([])
    );
    assert_eq!(run["results"], json!([]));
}

// SARIF locations are URI references: odd characters are percent-encoded.
#[test]
fn artifact_paths_are_uri_references() {
    let odd = "app\\my module/\u{fc}~.py";
    let found = violation(Severity::Error, "r", odd, span(1, 1, 2), "m");
    let text = sarif::render(&outcome(vec![found]), &Config::default());
    let document = parse(&text);
    let location = &run_of(&document)["results"][0]["locations"][0];
    let uri = &location["physicalLocation"]["artifactLocation"]["uri"];
    assert_eq!(uri, "app/my%20module/%C3%BC~.py");
}

// ADR 0004: the target is a related location, which has no file for an
// external package.
#[test]
fn an_external_target_has_no_file_location() {
    let rule = "[[rule]]\nid = \"r\"\nfrom = { path = \"**\" }\nto = { package = \"req*\" }\n";
    let config = cfg(&format!("{rule}effect = \"forbid\"\n"));
    let files = [("a.py", "import requests\n")];
    let graph = {
        let mut builder = Builder::new(&config);
        for (path, text) in files {
            let file = SourceFile { path, text };
            builder.add(
                &file,
                PythonExtractor.extract(&file, &ExtractCtx::default()),
            );
        }
        builder.build()
    };
    let checked = rules::check(&graph, &config, true);
    let document = parse(&sarif::render(&checked, &config));
    let result = &run_of(&document)["results"][0];
    assert_eq!(
        result["relatedLocations"][0]["message"]["text"],
        "ext:py:requests"
    );
    assert!(
        result["relatedLocations"][0]
            .get("physicalLocation")
            .is_none()
    );
    assert_eq!(result["properties"]["to"]["node"], "ext:py:requests");
}

// ADR 0003, invariant 1: the same inputs give the same bytes, whatever order
// the files were added in.
#[test]
fn the_output_does_not_depend_on_the_order_of_the_files() {
    let rule = "[[rule]]\nid = \"r\"\nfrom = { path = \"**\" }\nto = { path = \"app/db.py\" }\n";
    let config = cfg(&format!("{rule}effect = \"forbid\"\n"));
    let files = [
        ("app/__init__.py", ""),
        ("app/db.py", ""),
        ("app/a.py", "import app.db\n"),
        ("app/b.py", "import app.db\nimport app.db\n"),
    ];
    let render = |order: &[usize]| {
        let mut builder = Builder::new(&config);
        for &index in order {
            let (path, text) = files[index];
            let file = SourceFile { path, text };
            builder.add(
                &file,
                PythonExtractor.extract(&file, &ExtractCtx::default()),
            );
        }
        let checked = rules::check(&builder.build(), &config, true);
        (sarif::render(&checked, &config), github::render(&checked))
    };
    let forward = render(&[0, 1, 2, 3]);
    assert_eq!(forward, render(&[3, 2, 1, 0]));
    assert!(forward.0.contains("app/b.py"));
    assert_eq!(forward.1.lines().count(), 4);
}

fn at(severity: Severity, line: u32) -> Violation {
    violation(severity, "r", "a.py", span(line, 2, 9), "m")
}

// ADR 0004, "Reports" (GitHub annotations): one command per violation with
// file, line and column, and a summary line.
#[test]
fn annotations_are_workflow_commands() {
    let violations = vec![
        at(Severity::Error, 1),
        at(Severity::Warning, 2),
        at(Severity::Note, 3),
    ];
    let text = github::render(&outcome(violations));
    let expected = [
        "::error file=a.py,line=1,endLine=1,col=2,endColumn=9,title=r::m",
        "::warning file=a.py,line=2,endLine=2,col=2,endColumn=9,title=r::m",
        "::notice file=a.py,line=3,endLine=3,col=2,endColumn=9,title=r::m",
        "modfence: errors: 1, warnings: 1, notes: 1",
        "",
    ];
    assert_eq!(text, expected.join("\n"));
    let empty = github::render(&outcome(Vec::new()));
    assert_eq!(empty, "modfence: errors: 0, warnings: 0, notes: 0\n");
}

// ADR 0004, invariant 20: `%`, CR and LF are escaped in messages, and `:` and
// `,` in properties too.
#[test]
fn annotations_escape_messages_and_properties() {
    let (rule, path, message) = ("r:1,2", "a:b,c%.py", "50% done\r\nnext");
    let tricky = violation(Severity::Error, rule, path, span(1, 1, 2), message);
    let text = github::render(&outcome(vec![tricky]));
    let first = text.lines().next().unwrap();
    let head = "::error file=a%3Ab%2Cc%25.py,line=1,endLine=1,col=1,endColumn=2,title=r%3A1%2C2";
    assert_eq!(first, format!("{head}::50%25 done%0D%0Anext"));
}

// ADR 0004, invariant 20: at most 10 annotations of each level.
#[test]
fn annotations_are_capped_per_level() {
    let mut violations = Vec::new();
    for line in 1..=12 {
        violations.push(at(Severity::Error, line));
    }
    for line in 1..=3 {
        violations.push(at(Severity::Warning, line));
    }
    let text = github::render(&outcome(violations));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 10 + 3 + 1);
    let tenth = "::error file=a.py,line=10,endLine=10,col=2,endColumn=9,title=r::m";
    assert_eq!(lines[9], tenth);
    let first = "::warning file=a.py,line=1,endLine=1,col=2,endColumn=9,title=r::m";
    assert_eq!(lines[10], first);
    let summary = "modfence: errors: 12, warnings: 3, notes: 0";
    assert_eq!(
        lines[13],
        format!("{summary}; annotated the first 10 of each level")
    );
    assert_eq!(github::ANNOTATIONS_PER_LEVEL, 10);
}
