//! GitHub workflow commands (`--format github`, ADR 0004, invariant 20).

use std::collections::BTreeMap;

use crate::config::Severity;
use crate::rules::Outcome;

/// GitHub shows at most this many annotations of each level per step.
pub const ANNOTATIONS_PER_LEVEL: usize = 10;

/// Escapes the message of a workflow command.
fn escape_data(text: &str) -> String {
    text.replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

/// Escapes a property of a workflow command.
fn escape_property(text: &str) -> String {
    escape_data(text).replace(':', "%3A").replace(',', "%2C")
}

/// The workflow command of a severity.
const fn command(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Note => "notice",
    }
}

/// One `::error file=...::message` line per violation, at most
/// [`ANNOTATIONS_PER_LEVEL`] of each level, then one summary line.
#[must_use]
pub fn render(outcome: &Outcome) -> String {
    let mut out = String::new();
    let mut totals: BTreeMap<Severity, usize> = BTreeMap::new();
    let mut shown: BTreeMap<Severity, usize> = BTreeMap::new();
    for violation in &outcome.violations {
        *totals.entry(violation.severity).or_default() += 1;
        let count = shown.entry(violation.severity).or_default();
        if *count >= ANNOTATIONS_PER_LEVEL {
            continue;
        }
        *count += 1;
        out.push_str(&format!(
            "::{} file={},line={},col={},title={}::{}\n",
            command(violation.severity),
            escape_property(&violation.path),
            violation.span.start_line,
            violation.span.start_column,
            escape_property(&violation.rule),
            escape_data(&violation.message),
        ));
    }
    let total = |severity: Severity| totals.get(&severity).copied().unwrap_or_default();
    out.push_str(&format!(
        "modfence: errors: {}, warnings: {}, notes: {}",
        total(Severity::Error),
        total(Severity::Warning),
        total(Severity::Note),
    ));
    if outcome.violations.len() > shown.values().sum::<usize>() {
        out.push_str(&format!("; annotated the first {ANNOTATIONS_PER_LEVEL} of each level"));
    }
    out.push('\n');
    out
}
