//! SARIF 2.1.0, built by hand with `serde_json` (ADR 0004, "Reports").
//!
//! One run per invocation. Every result has the four region fields, a
//! `primaryLocationLineHash` fingerprint and `properties` with the edge's
//! confidence and both endpoints' tags; the rules include the built-in ones,
//! each with the descriptions and help text GitHub requires. Keys come out
//! sorted (`serde_json` keeps maps ordered) and results in the outcome's
//! order, so the text is deterministic.

use serde_json::{Value, json};

use crate::config::{Config, Effect, Severity};
use crate::extract::Span;
use crate::graph::INVALID_ANNOTATION;
use crate::rules::{DEFAULT_DENY, Endpoint, LAYERS, Notice, Outcome, UNUSED_IGNORE, Violation};

/// A built-in rule: id, level, short description, full description.
type Builtin = (&'static str, Severity, &'static str, &'static str);

const BUILTINS: [Builtin; 4] = [
    (
        LAYERS,
        Severity::Error,
        "A module depends on a higher layer",
        "A module must not depend on a module in a higher layer of `layers` in modfence.toml; \
         a layer may use itself and the layers below it.",
    ),
    (
        DEFAULT_DENY,
        Severity::Error,
        "No rule allows this dependency",
        "With `default = \"deny\"` in modfence.toml, a dependency between two modules of the \
         repository needs an `allow` rule, or layers that permit it.",
    ),
    (
        INVALID_ANNOTATION,
        Severity::Error,
        "A modfence annotation is malformed",
        "A `modfence:` comment could not be read, or is a tag that has no effect where it is.",
    ),
    (
        UNUSED_IGNORE,
        Severity::Warning,
        "An ignore suppressed nothing",
        "A `modfence: ignore[...]` comment names a rule that has no violation on its line or \
         the line below, so it can be removed.",
    ),
];

/// The SARIF `level` of a severity.
const fn level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Note => "note",
    }
}

/// The 64-bit FNV-1a hash of `parts`, each followed by a NUL byte. It is
/// defined here, not taken from the standard library's hashers, whose output
/// is not specified.
fn fnv(parts: &[&str]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.bytes().chain([0]) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
    }
    hash
}

/// The fingerprint hash of a violation: the rule, source, target and relation
/// of an edge violation; the rule and the path of an annotation finding.
fn fingerprint_hash(violation: &Violation) -> String {
    let hash = match &violation.dependency {
        Some(dependency) => fnv(&[
            &violation.rule,
            dependency.from.id.as_str(),
            dependency.to.id.as_str(),
            dependency.relation.as_str(),
        ]),
        None => fnv(&[&violation.rule, &violation.path]),
    };
    format!("{hash:016x}")
}

/// The `<hash>:<n>` fingerprints, where `n` is the 1-based position of the
/// violation among those with the same hash, ordered by path and position.
fn fingerprints(violations: &[Violation]) -> Vec<String> {
    let hashes: Vec<String> = violations.iter().map(fingerprint_hash).collect();
    let mut order: Vec<usize> = (0..violations.len()).collect();
    order.sort_by_key(|&i| (&hashes[i], &violations[i].path, violations[i].span, i));
    let mut numbers = vec![0_usize; violations.len()];
    let mut previous: Option<&String> = None;
    let mut count = 0;
    for index in order {
        count = if previous == Some(&hashes[index]) {
            count + 1
        } else {
            1
        };
        previous = Some(&hashes[index]);
        numbers[index] = count;
    }
    hashes
        .iter()
        .zip(numbers)
        .map(|(hash, number)| format!("{hash}:{number}"))
        .collect()
}

fn artifact(path: &str) -> Value {
    json!({ "uri": path, "uriBaseId": "%SRCROOT%" })
}

fn region(span: Span) -> Value {
    json!({
        "startLine": span.start_line,
        "startColumn": span.start_column,
        "endLine": span.end_line,
        "endColumn": span.end_column,
    })
}

fn node_properties(endpoint: &Endpoint) -> Value {
    json!({ "node": endpoint.id.as_str(), "tags": endpoint.tags })
}

/// The target of a violation as a related location.
fn related(target: &Endpoint) -> Value {
    let mut location = json!({ "id": 0, "message": { "text": target.id.as_str() } });
    if let Some(path) = &target.path {
        location["physicalLocation"] = json!({ "artifactLocation": artifact(path) });
    }
    location
}

fn result(violation: &Violation, fingerprint: &str) -> Value {
    let (properties, related_locations) = match &violation.dependency {
        Some(dependency) => (
            json!({
                "confidence": dependency.confidence.as_str(),
                "from": node_properties(&dependency.from),
                "to": node_properties(&dependency.to),
            }),
            json!([related(&dependency.to)]),
        ),
        None => (json!({}), json!([])),
    };
    json!({
        "ruleId": violation.rule,
        "level": level(violation.severity),
        "message": { "text": violation.message },
        "locations": [{
            "physicalLocation": {
                "artifactLocation": artifact(&violation.path),
                "region": region(violation.span),
            },
        }],
        "relatedLocations": related_locations,
        "partialFingerprints": { "primaryLocationLineHash": fingerprint },
        "properties": properties,
    })
}

fn descriptor(id: &str, severity: Severity, short: &str, full: &str) -> Value {
    json!({
        "id": id,
        "shortDescription": { "text": short },
        "fullDescription": { "text": full },
        "help": {
            "text": format!(
                "Fix the dependency, or suppress one site with \
                 `modfence: ignore[{id}] reason=\"...\"`."
            ),
        },
        "defaultConfiguration": { "level": level(severity) },
    })
}

/// The built-in rules, then the config's, in file order.
fn rules(config: &Config) -> Vec<Value> {
    let builtins = BUILTINS
        .iter()
        .map(|&(id, severity, short, full)| descriptor(id, severity, short, full));
    let configured = config.rules.iter().map(|rule| {
        let verb = match rule.effect {
            Effect::Forbid => "Forbids",
            Effect::Allow => "Allows",
        };
        let short = format!("Rule `{}` of modfence.toml", rule.id);
        let full = rule
            .message
            .clone()
            .unwrap_or_else(|| format!("{verb} the dependencies that rule `{}` selects.", rule.id));
        descriptor(&rule.id, rule.severity, &short, &full)
    });
    builtins.chain(configured).collect()
}

fn notification(notice: &Notice) -> Value {
    json!({ "level": level(notice.level), "message": { "text": notice.message } })
}

/// Renders `outcome` as a SARIF 2.1.0 document, with `config` supplying the
/// rule descriptions.
#[must_use]
pub fn render(outcome: &Outcome, config: &Config) -> String {
    let numbers = fingerprints(&outcome.violations);
    let results: Vec<Value> = outcome
        .violations
        .iter()
        .zip(numbers)
        .map(|(violation, fingerprint)| result(violation, &fingerprint))
        .collect();
    let notifications: Vec<Value> = outcome.notices.iter().map(notification).collect();
    let document = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "modfence",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": env!("CARGO_PKG_REPOSITORY"),
                    "rules": rules(config),
                },
            },
            "columnKind": "unicodeCodePoints",
            "invocations": [{
                "executionSuccessful": true,
                "toolExecutionNotifications": notifications,
            }],
            "results": results,
            "properties": {
                "modfence": {
                    "precise": outcome.precise,
                    "callEdgesChecked": outcome.precise,
                },
            },
        }],
    });
    format!("{document:#}\n")
}
