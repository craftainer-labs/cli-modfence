//! The pre-commit hook definition (`.pre-commit-hooks.yaml`): the file is the
//! shape pre-commit and prek read, and its `entry` is a real command that
//! behaves as a hook must (exit 0 on a clean repository, 1 on a violation).
//!
//! The file is a flat YAML list of one mapping with scalar values and `#`
//! comments, so a line reader checks it without a YAML crate; the `check-yaml`
//! hook of the repository's own `prek` run validates the full syntax.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Output};

const BINARY: &str = env!("CARGO_BIN_EXE_modfence");

/// The scalar `key: value` pairs of the single hook in the file.
fn hook() -> BTreeMap<String, String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(".pre-commit-hooks.yaml");
    let text = std::fs::read_to_string(path).unwrap();
    let mut fields = BTreeMap::new();
    let mut hooks = 0;
    for line in text.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        assert!(!line.contains('\t'), "tab in {line:?}");
        let rest = if let Some(rest) = line.strip_prefix("- ") {
            hooks += 1;
            rest
        } else {
            line.strip_prefix("  ").expect("a two-space indent")
        };
        let (key, value) = rest.split_once(": ").expect("a `key: value` line");
        assert!(!key.starts_with(' '), "odd indent in {line:?}");
        let before = fields.insert(key.to_owned(), value.to_owned());
        assert!(before.is_none(), "duplicate key {key}");
    }
    assert_eq!(hooks, 1, "the file defines exactly one hook");
    fields
}

#[test]
fn the_hook_has_the_fields_pre_commit_requires() {
    let hook = hook();
    for key in ["id", "name", "entry", "language"] {
        let value = hook.get(key).expect("a required field");
        assert!(!value.is_empty(), "empty {key}");
    }
    assert_eq!(hook["id"], "modfence");
    assert_eq!(hook["language"], "rust");
    // Whole-repository analysis: file names are not passed.
    assert_eq!(hook["pass_filenames"], "false");
    assert_eq!(hook["always_run"], "true");
}

/// The words of `entry`: the program, the subcommand and its fixed flags.
fn entry_words() -> Vec<String> {
    let entry = hook()["entry"].clone();
    entry.split_whitespace().map(str::to_owned).collect()
}

#[test]
fn the_entry_names_the_binary_and_a_real_subcommand() {
    let words = entry_words();
    assert_eq!(words.len(), 2, "entry is `modfence <subcommand>`");
    assert_eq!(words[0], env!("CARGO_PKG_NAME"));
    let subcommand = words[1].as_str();
    let help = Command::new(BINARY).args([subcommand, "--help"]).output();
    let help = help.expect("the binary runs");
    assert!(help.status.success(), "`{subcommand} --help` failed");
    let top = Command::new(BINARY).arg("--help").output().unwrap();
    let listing = String::from_utf8(top.stdout).unwrap();
    assert!(listing.contains(&format!("  {subcommand}")));
}

fn run_hook(fixture: &str) -> Output {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let directory = root.join("tests/fixtures/cli").join(fixture);
    let mut command = Command::new(BINARY);
    command.args(&entry_words()[1..]).current_dir(directory);
    command.output().expect("the binary runs")
}

#[test]
fn the_hook_passes_a_clean_repository_and_fails_a_violation() {
    assert_eq!(run_hook("clean").status.code(), Some(0));
    assert_eq!(run_hook("app").status.code(), Some(1));
}
