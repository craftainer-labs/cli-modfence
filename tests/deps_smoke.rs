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
