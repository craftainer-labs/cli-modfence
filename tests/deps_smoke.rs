//! The NC4 dependencies are usable: TOML with spans, serde derive, JSON
//! output and glob matching.

use globset::Glob;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u32,
}

#[test]
fn toml_parses_and_rejects_unknown_keys() {
    let config: Config = toml::from_str("version = 1\n").expect("valid config");
    assert_eq!(config.version, 1);
    let error = toml::from_str::<Config>("version = 1\ntypo = 2\n")
        .err()
        .expect("unknown key is an error");
    assert!(error.span().is_some());
}

#[test]
fn json_is_written() {
    let value = serde_json::json!({ "a": [1, 2] });
    assert_eq!(value.to_string(), r#"{"a":[1,2]}"#);
}

#[test]
fn globs_match_paths() {
    let matcher = Glob::new("src/**/*.py")
        .expect("valid glob")
        .compile_matcher();
    assert!(matcher.is_match("src/a/b.py"));
    assert!(!matcher.is_match("src/a/b.rs"));
}

#[test]
fn scip_index_round_trips() {
    use protobuf::Message;
    use scip::types::{Document, Index, Occurrence};

    let mut occurrence = Occurrence::new();
    occurrence.range = vec![0, 4, 7];
    occurrence.symbol = "scip-python python pkg 1.0 `mod`/foo().".to_owned();
    occurrence.symbol_roles = 0x1;
    let mut document = Document::new();
    document.relative_path = "pkg/mod.py".to_owned();
    document.occurrences.push(occurrence);
    let mut index = Index::new();
    index.documents.push(document);

    let bytes = index.write_to_bytes().expect("serialises");
    let parsed = Index::parse_from_bytes(&bytes).expect("parses");
    assert_eq!(parsed.documents.len(), 1);
    assert_eq!(parsed.documents[0].relative_path, "pkg/mod.py");
    assert_eq!(parsed.documents[0].occurrences[0].range, vec![0, 4, 7]);
    assert_eq!(parsed.documents[0].occurrences[0].symbol_roles, 0x1);
}
