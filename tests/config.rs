//! `modfence.toml` (ADR 0004): parsing, the errors with their spans, selectors
//! and tag precedence.

use std::collections::BTreeSet;

use modfence::config::{
    self, Config, ConfidenceReq, DefaultEffect, Effect, ParseError, Relation, Severity, Subject,
    resolve_tags, validate_tag_key,
};
use modfence::extract::{Lang, NodeKind, Tags};

const EXAMPLE: &str = include_str!("../schema/examples/modfence.example.toml");

/// A test case: its name, the levels lowest first, the expected result.
type Row<'a> = (&'a str, Vec<Tags>, Tags);

fn tags(pairs: &[(&str, &[&str])]) -> Tags {
    pairs
        .iter()
        .map(|(key, values)| {
            (
                (*key).to_owned(),
                values.iter().map(|v| (*v).to_owned()).collect(),
            )
        })
        .collect()
}

fn fail(text: &str) -> ParseError {
    config::parse(text).expect_err("the config must be rejected")
}

/// The text of the file the error points at.
fn culprit<'a>(text: &'a str, error: &ParseError) -> &'a str {
    &text[error.span.clone().expect("the error has a span")]
}

fn module<'a>(path: &'a str, tags: &'a Tags) -> Subject<'a> {
    Subject {
        kind: NodeKind::Module,
        lang: Lang::Py,
        path: Some(path),
        package: None,
        tags,
    }
}

// ADR 0004, "Config file": the example is valid and fully read.
#[test]
fn the_schema_example_parses() {
    let config = config::parse(EXAMPLE).expect("the example is valid");
    assert_eq!(config.exclude.len(), 1);
    assert_eq!(config.layers, ["app", "infra", "domain"]);
    assert_eq!(config.default, DefaultEffect::Deny);
    assert_eq!(
        config.accumulate,
        BTreeSet::from(["adr".to_owned(), "req".to_owned()])
    );
    assert!(config.require_reason);
    assert_eq!(config.nodes.len(), 5);
    assert_eq!(
        config.nodes[1].tags,
        tags(&[
            ("layer", &["domain"]),
            ("adr", &["0028", "0029"]),
            ("req", &["REQ-12"])
        ])
    );
    let ids: Vec<&str> = config.rules.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        ["domain-no-network", "app-uses-legacy-reader", "no-new-legacy"]
    );
    let last = &config.rules[2];
    assert_eq!(last.kinds, [Relation::Imports, Relation::Calls]);
    assert_eq!(last.confidence, Some(ConfidenceReq::Any));
    assert_eq!(last.severity, Severity::Warning);
    assert_eq!(
        last.message.as_deref(),
        Some("Do not add new uses of deprecated modules.")
    );
    assert_eq!(last.effect, Effect::Forbid);
}

// ADR 0004: defaults of every optional key.
#[test]
fn a_minimal_config_has_the_defaults() {
    let parsed = config::parse("version = 1\n").expect("valid");
    let default = Config::default();
    for config in [&parsed, &default] {
        assert!(config.exclude.is_empty());
        assert!(config.layers.is_empty());
        assert_eq!(config.default, DefaultEffect::Allow);
        assert!(config.accumulate.is_empty());
        assert!(config.require_reason);
        assert!(config.nodes.is_empty());
        assert!(config.rules.is_empty());
    }
}

#[test]
fn require_reason_can_be_turned_off() {
    let config = config::parse("version = 1\n[annotations]\nrequire_reason = false\n").unwrap();
    assert!(!config.require_reason);
}

#[test]
fn rule_defaults_are_all_kinds_error_and_no_confidence() {
    let text = "version = 1\n[[rule]]\nid = \"r\"\nfrom = { layer = \"a\" }\nto = { layer = \"b\" }\neffect = \"allow\"\n";
    let rule = &config::parse(text).unwrap().rules[0];
    assert_eq!(
        rule.kinds,
        [Relation::Imports, Relation::Binds, Relation::Calls]
    );
    assert_eq!(rule.confidence, None);
    assert_eq!(rule.severity, Severity::Error);
    assert_eq!(rule.message, None);
    assert_eq!(rule.effect, Effect::Allow);
}

#[test]
fn rule_kinds_are_sorted_and_deduplicated() {
    let text = "version = 1\n[[rule]]\nid = \"r\"\nfrom = { layer = \"a\" }\nto = { layer = \"b\" }\neffect = \"forbid\"\nkinds = [\"calls\", \"imports\", \"calls\"]\nconfidence = \"resolved\"\n";
    let rule = &config::parse(text).unwrap().rules[0];
    assert_eq!(rule.kinds, [Relation::Imports, Relation::Calls]);
    assert_eq!(rule.confidence, Some(ConfidenceReq::Resolved));
}

// ADR 0004, invariant 1: an unknown key is an error that points at the key.
#[test]
fn an_unknown_key_is_reported_at_the_key() {
    let text = "version = 1\ntypo = 2\n";
    let error = fail(text);
    assert_eq!(culprit(text, &error), "typo");
    assert_eq!((error.line, error.column), (Some(2), Some(1)));
    assert!(error.to_string().starts_with("2:1: "), "{error}");
}

#[test]
fn unknown_keys_are_reported_in_nested_tables_too() {
    let text = "version = 1\n\n[[rule]]\nid = \"r\"\nfrm = {}\n";
    let error = fail(text);
    assert_eq!(culprit(text, &error), "frm");
    assert_eq!(error.line, Some(5));
    let text = "version = 1\n[tags]\nacumulate = []\n";
    assert_eq!(culprit(text, &fail(text)), "acumulate");
    let text = "version = 1\n[annotations]\nreason = true\n";
    assert_eq!(culprit(text, &fail(text)), "reason");
    let text = "version = 1\n[[node]]\nmatch = \"a\"\ntags = { a = \"b\" }\nextra = 1\n";
    assert_eq!(culprit(text, &fail(text)), "extra");
}

#[test]
fn an_error_without_a_location_displays_its_message() {
    let error = ParseError {
        message: "boom".to_owned(),
        span: None,
        line: None,
        column: None,
    };
    assert_eq!(error.to_string(), "boom");
}

#[test]
fn malformed_toml_and_a_missing_version_are_errors() {
    assert!(fail("version = \n").span.is_some());
    assert!(fail("").message.contains("version"));
}

#[test]
fn only_version_one_is_accepted() {
    let text = "version = 2\n";
    let error = fail(text);
    assert_eq!(culprit(text, &error), "2");
    assert!(error.message.contains("unsupported version 2"));
    assert!(fail("version = -1\n").message.contains("-1"));
}

#[test]
fn the_error_column_counts_code_points_not_bytes() {
    let text = "version = 1\nlayers = [\"é\", \"é\"]\n";
    let error = fail(text);
    let start = error.span.clone().expect("a span").start;
    let line_start = text[..start].rfind('\n').map_or(0, |at| at + 1);
    let characters = text[line_start..start].chars().count();
    assert_eq!(error.column, Some(u32::try_from(characters + 1).unwrap()));
    assert!(start - line_start > characters, "the line holds a wide character");
}

// ADR 0004, invariant 1: duplicate layer names; schema: at least two layers.
#[test]
fn layers_are_checked() {
    let text = "version = 1\nlayers = [\"app\", \"app\"]\n";
    let error = fail(text);
    assert!(error.message.contains("duplicate layer `app`"));
    assert_eq!(error.line, Some(2));
    assert_eq!(culprit(text, &error).trim_matches('"'), "app");
    assert!(fail("version = 1\nlayers = [\"a\"]\n").message.contains("at least two"));
    assert!(fail("version = 1\nlayers = []\n").message.contains("at least two"));
    assert!(fail("version = 1\nlayers = [\"a\", \"b c\"]\n").message.contains("b c"));
    assert!(fail("version = 1\nlayers = [\"a\", \"\"]\n").message.contains("layer name"));
    let config = config::parse("version = 1\nlayers = [\"a.b\", \"c_d\", \"e-f\"]\n").unwrap();
    assert_eq!(config.layers, ["a.b", "c_d", "e-f"]);
}

// ADR 0004, invariant 13: `[tags] accumulate` lists tag keys only, once.
#[test]
fn accumulate_lists_valid_unique_tag_keys() {
    let text = "version = 1\n[tags]\naccumulate = [\"adr\", \"path\"]\n";
    let error = fail(text);
    assert!(error.message.contains("selector key"));
    assert_eq!(culprit(text, &error).trim_matches('"'), "path");
    let text = "version = 1\n[tags]\naccumulate = [\"adr\", \"adr\"]\n";
    assert!(fail(text).message.contains("listed twice"));
    let text = "version = 1\n[tags]\naccumulate = [\"Adr\"]\n";
    assert!(fail(text).message.contains("[a-z][a-z0-9_-]*"));
}

#[test]
fn tag_keys_are_validated_in_one_place() {
    for key in ["layer", "a", "adr-1", "x_y"] {
        assert_eq!(validate_tag_key(key), Ok(()), "{key}");
    }
    for key in ["path", "package", "lang", "kind"] {
        assert!(validate_tag_key(key).unwrap_err().contains("selector key"));
    }
    for key in ["", "Layer", "1a", "a b", "a.b"] {
        assert!(validate_tag_key(key).is_err(), "{key}");
    }
}

#[test]
fn exclude_globs_are_checked() {
    let config = config::parse("version = 1\nexclude = [\"a/*.py\", \"b/**\"]\n").unwrap();
    assert!(config.exclude[0].is_match("a/x.py"));
    assert!(!config.exclude[0].is_match("a/b/x.py"));
    assert!(config.exclude[1].is_match("b/c/d"));
    assert!(fail("version = 1\nexclude = [\"[\"]\n").message.contains("invalid glob"));
    assert!(fail("version = 1\nexclude = [\"\"]\n").message.contains("must not be empty"));
}

// ADR 0004, `[[node]]`.
#[test]
fn node_entries_need_one_target_and_tags() {
    let text = "version = 1\n[[node]]\ntags = { a = \"b\" }\n";
    assert!(fail(text).message.contains("exactly one"));
    let text = "version = 1\n[[node]]\nmatch = \"a\"\npackage = \"b\"\ntags = { a = \"b\" }\n";
    assert!(fail(text).message.contains("exactly one"));
    let text = "version = 1\n[[node]]\nmatch = \"a\"\ntags = {}\n";
    let error = fail(text);
    assert!(error.message.contains("at least one tag"));
    assert!(fail("version = 1\n[[node]]\nmatch = \"a\"\n").message.contains("tags"));
}

#[test]
fn node_tags_are_validated_at_the_key_or_value() {
    let text = "version = 1\n[[node]]\nmatch = \"a\"\ntags = { path = \"x\" }\n";
    let error = fail(text);
    assert_eq!(culprit(text, &error), "path");
    assert_eq!(error.line, Some(4));
    let text = "version = 1\n[[node]]\nmatch = \"a\"\ntags = { Layer = \"x\" }\n";
    assert_eq!(culprit(text, &fail(text)), "Layer");
    let text = "version = 1\n[[node]]\nmatch = \"a\"\ntags = { a = \"x\\\"y\" }\n";
    assert!(fail(text).message.contains("double quotes"));
    let text = "version = 1\n[[node]]\nmatch = \"a\"\ntags = { a = \"\" }\n";
    assert!(fail(text).message.contains("non-empty"));
    let text = "version = 1\n[[node]]\nmatch = \"a\"\ntags = { a = [] }\n";
    assert!(fail(text).message.contains("must not be empty"));
    let text = "version = 1\n[[node]]\nmatch = \"a\"\ntags = { a = [\"b\", \"\"] }\n";
    assert!(fail(text).message.contains("non-empty"));
    let text = "version = 1\n[[node]]\nmatch = \"a\"\ntags = { a = 1 }\n";
    assert!(fail(text).message.contains("non-empty list of strings"));
}

#[test]
fn node_entries_apply_by_path_or_package() {
    let text = "version = 1\n[[node]]\nmatch = \"src/*.py\"\ntags = { layer = \"app\" }\n\
                [[node]]\npackage = \"@org/*\"\ntags = { boundary = [\"net\", \"io\", \"net\"] }\n";
    let config = config::parse(text).unwrap();
    let none = Tags::new();
    let entry = &config.nodes[0];
    assert!(entry.matches(&module("src/a.py", &none)));
    assert!(!entry.matches(&module("src/x/a.py", &none)));
    let external = Subject {
        kind: NodeKind::External,
        lang: Lang::Ts,
        path: None,
        package: Some("@org/lib"),
        tags: &none,
    };
    assert!(!entry.matches(&external));
    let entry = &config.nodes[1];
    assert!(entry.matches(&external));
    assert!(!entry.matches(&module("@org/lib", &none)));
    assert_eq!(entry.tags, tags(&[("boundary", &["io", "net"])]));
}

// ADR 0004, "Rules" and invariants 1 and 6.
#[test]
fn rule_ids_are_checked() {
    let rule = |id: &str| {
        format!(
            "version = 1\n[[rule]]\nid = \"{id}\"\nfrom = {{ a = \"b\" }}\nto = {{ a = \"b\" }}\neffect = \"forbid\"\n"
        )
    };
    for id in ["layers", "default-deny", "modfence/unused-ignore"] {
        let text = rule(id);
        let error = fail(&text);
        assert!(error.message.contains("reserved"), "{id}");
        assert_eq!(culprit(&text, &error).trim_matches('"'), id);
        assert_eq!(error.line, Some(3));
    }
    for id in ["Bad", "-x", "a_b", ""] {
        assert!(fail(&rule(id)).message.contains("[a-z0-9][a-z0-9-]*"), "{id}");
    }
    assert!(config::parse(&rule("ok-1")).is_ok());
    let twice = format!("{}{}", rule("same"), &rule("same")["version = 1\n".len()..]);
    let error = fail(&twice);
    assert!(error.message.contains("duplicate rule id `same`"));
    assert_eq!(error.line, Some(8));
}

#[test]
fn a_rule_that_can_never_match_is_an_error() {
    let text = "version = 1\n[[rule]]\nid = \"r\"\nfrom = { a = \"b\" }\nto = { a = \"b\" }\neffect = \"forbid\"\nkinds = [\"binds\"]\nconfidence = \"resolved\"\n";
    let error = fail(text);
    assert_eq!(culprit(text, &error).trim_matches('"'), "resolved");
    assert!(error.message.contains("can never match"));
    let fine = text.replace("\"resolved\"", "\"any\"");
    assert!(config::parse(&fine).is_ok());
    let fine = text.replace("[\"binds\"]", "[\"binds\", \"imports\"]");
    assert!(config::parse(&fine).is_ok());
}

#[test]
fn rule_fields_are_checked() {
    let base = "version = 1\n[[rule]]\nid = \"r\"\nfrom = { a = \"b\" }\nto = { a = \"b\" }\neffect = \"forbid\"\n";
    assert!(fail(&format!("{base}kinds = []\n")).message.contains("kinds must not be empty"));
    assert!(fail(&format!("{base}kinds = [\"bogus\"]\n")).message.contains("bogus"));
    assert!(fail(&format!("{base}message = \"\"\n")).message.contains("message"));
    assert!(fail(&format!("{base}severity = \"fatal\"\n")).message.contains("fatal"));
    assert!(fail(&base.replace("forbid", "ban")).message.contains("ban"));
    assert!(fail(&base.replace("effect = \"forbid\"\n", "")).message.contains("effect"));
    let ok = format!("{base}message = \"m\"\nseverity = \"note\"\n");
    let rule = &config::parse(&ok).unwrap().rules[0];
    assert_eq!(rule.message.as_deref(), Some("m"));
    assert_eq!(rule.severity, Severity::Note);
}

// ADR 0004, invariant 2: selector keys and values.
#[test]
fn selectors_are_validated() {
    let with = |selector: &str| {
        format!(
            "version = 1\n[[rule]]\nid = \"r\"\nfrom = {selector}\nto = {{ a = \"b\" }}\neffect = \"forbid\"\n"
        )
    };
    let error = fail(&with("{ Layer = \"x\" }"));
    assert!(error.message.contains("selector key"));
    assert!(fail(&with("{}")).message.contains("at least one key"));
    assert!(fail(&with("\"x\"")).message.contains("selector table"));
    assert!(fail(&with("{ lang = \"go\" }")).message.contains("unknown language `go`"));
    assert!(fail(&with("{ kind = [\"module\", \"file\"] }")).message.contains("unknown kind `file`"));
    assert!(fail(&with("{ path = \"[\" }")).message.contains("invalid glob"));
    assert!(fail(&with("{ package = [] }")).message.contains("must not be empty"));
    assert!(fail(&with("{ layer = \"\" }")).message.contains("must not be empty"));
    assert!(fail(&with("{ layer = 3 }")).message.contains("non-empty list of strings"));
}

#[test]
fn selectors_match_with_and_across_keys_and_or_within_a_list() {
    let text = "version = 1\n[[rule]]\nid = \"r\"\neffect = \"forbid\"\n\
                from = { path = [\"src/app/**\", \"src/web/*.py\"], lang = [\"py\", \"ts\"], kind = \"module\", layer = [\"app\", \"web\"], adr = \"0028\" }\n\
                to = { package = \"requests\" }\n";
    let config = config::parse(text).unwrap();
    let rule = &config.rules[0];
    let node_tags = tags(&[("layer", &["web"]), ("adr", &["0001", "0028"])]);
    assert!(rule.from.matches(&module("src/app/a/b.py", &node_tags)));
    assert!(rule.from.matches(&module("src/web/b.py", &node_tags)));
    // `*` does not cross `/`.
    assert!(!rule.from.matches(&module("src/web/x/b.py", &node_tags)));
    // AND across keys.
    let other = tags(&[("layer", &["web"])]);
    assert!(!rule.from.matches(&module("src/app/a.py", &other)));
    let other = tags(&[("layer", &["db"]), ("adr", &["0028"])]);
    assert!(!rule.from.matches(&module("src/app/a.py", &other)));
    let mut subject = module("src/app/a.py", &node_tags);
    subject.lang = Lang::Rs;
    assert!(!rule.from.matches(&subject));
    subject.lang = Lang::Ts;
    assert!(rule.from.matches(&subject));
    subject.kind = NodeKind::Symbol;
    assert!(!rule.from.matches(&subject));
    // A node without a path never matches a path glob.
    subject.kind = NodeKind::Module;
    subject.path = None;
    assert!(!rule.from.matches(&subject));
    // `package` matches external package names only.
    let none = Tags::new();
    let external = Subject {
        kind: NodeKind::External,
        lang: Lang::Py,
        path: None,
        package: Some("requests"),
        tags: &none,
    };
    assert!(rule.to.matches(&external));
    assert!(!rule.to.matches(&module("requests", &none)));
}

// ADR 0004, "Tag precedence" and invariant 11: one row per combination.
#[test]
fn tag_precedence_table() {
    let accumulate = BTreeSet::from(["adr".to_owned()]);
    let rows: Vec<Row<'_>> = vec![
        ("no levels", vec![], tags(&[])),
        (
            "one entry",
            vec![tags(&[("layer", &["app"])])],
            tags(&[("layer", &["app"])]),
        ),
        (
            "a later entry replaces an earlier one",
            vec![tags(&[("layer", &["a"])]), tags(&[("layer", &["b"])])],
            tags(&[("layer", &["b"])]),
        ),
        (
            "module inline replaces the entry",
            vec![
                tags(&[("layer", &["a"]), ("owner", &["x"])]),
                tags(&[("layer", &["b"])]),
            ],
            tags(&[("layer", &["b"]), ("owner", &["x"])]),
        ),
        (
            "symbol inline replaces the module's whole list",
            vec![
                tags(&[("team", &["a", "b"])]),
                tags(&[("team", &["c"])]),
            ],
            tags(&[("team", &["c"])]),
        ),
        (
            "an accumulating key is the union, sorted",
            vec![
                tags(&[("adr", &["0028"])]),
                tags(&[("adr", &["0030"])]),
                tags(&[("adr", &["0029", "0028"])]),
            ],
            tags(&[("adr", &["0028", "0029", "0030"])]),
        ),
        (
            "accumulation does not touch other keys",
            vec![
                tags(&[("adr", &["1"]), ("layer", &["a"])]),
                tags(&[("adr", &["2"]), ("layer", &["b"])]),
            ],
            tags(&[("adr", &["1", "2"]), ("layer", &["b"])]),
        ),
        (
            "a symbol keeps the module tags it does not override",
            vec![
                tags(&[("layer", &["a"])]),
                tags(&[("owner", &["m"])]),
                tags(&[("deprecated", &["true"])]),
            ],
            tags(&[("deprecated", &["true"]), ("layer", &["a"]), ("owner", &["m"])]),
        ),
    ];
    for (name, levels, expected) in rows {
        assert_eq!(resolve_tags(&levels, &accumulate), expected, "{name}");
    }
}

#[test]
fn effective_tags_combine_entries_and_inline_levels() {
    let text = "version = 1\n[tags]\naccumulate = [\"adr\"]\n\
                [[node]]\nmatch = \"src/**\"\ntags = { layer = \"app\", adr = \"0001\" }\n\
                [[node]]\nmatch = \"src/domain/**\"\ntags = { layer = \"domain\", adr = \"0002\" }\n\
                [[node]]\npackage = \"requests\"\ntags = { boundary = \"net\" }\n";
    let config = config::parse(text).unwrap();
    let none = Tags::new();
    let module_inline = tags(&[("adr", &["0003"])]);
    let symbol_inline = tags(&[("layer", &["infra"])]);
    let subject = module("src/domain/a.py", &none);
    assert_eq!(
        config.effective_tags(&subject, &[]),
        tags(&[("layer", &["domain"]), ("adr", &["0001", "0002"])])
    );
    assert_eq!(
        config.effective_tags(&subject, &[&module_inline, &symbol_inline]),
        tags(&[("layer", &["infra"]), ("adr", &["0001", "0002", "0003"])])
    );
    assert_eq!(
        config.effective_tags(&module("lib/a.py", &none), &[]),
        Tags::new()
    );
    let external = Subject {
        kind: NodeKind::External,
        lang: Lang::Py,
        path: None,
        package: Some("requests"),
        tags: &none,
    };
    assert_eq!(
        config.effective_tags(&external, &[]),
        tags(&[("boundary", &["net"])])
    );
}
