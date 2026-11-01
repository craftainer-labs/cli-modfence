//! Python qualified names (ADR 0003, "Python qualified names").
//!
//! A symbol's qualified name is the list of names from the module down to it,
//! as Python's `__qualname__` writes them: `Engine.run`, and
//! `Engine.run.<locals>.inner` for a definition inside a function body. Each
//! component is percent-encoded and the components are joined with `.`. A
//! later definition of a name that is already defined in the same file gets
//! the suffix `@2`, `@3`, and so on.

use crate::extract::ident::{decode_component, encode_component};

/// The component Python puts between a function and the names defined in its
/// body.
pub const LOCALS: &str = "<locals>";

const ORDINAL: char = '@';

/// Percent-encodes one component. On top of [`encode_component`], `.` and `@`
/// are escaped (`%2E`, `%40`), so that they separate components and mark the
/// ordinal and nothing else.
fn encode_part(raw: &str) -> String {
    encode_component(raw)
        .replace('.', "%2E")
        .replace(ORDINAL, "%40")
}

/// Encodes a qualified name. `ordinal` is 1 for the first definition of the
/// name in the file and counts up for later ones; only values above 1 appear
/// in the result. `components` must not be empty, and no component may be
/// empty, or the result is not a valid qualified name.
#[must_use]
pub fn encode_qualname<S: AsRef<str>>(components: &[S], ordinal: u32) -> String {
    let parts: Vec<String> = components
        .iter()
        .map(|part| encode_part(part.as_ref()))
        .collect();
    let name = parts.join(".");
    if ordinal > 1 {
        format!("{name}{ORDINAL}{ordinal}")
    } else {
        name
    }
}

/// The ordinal of a later definition: a number above 1.
fn parse_ordinal(digits: &str) -> Option<u32> {
    digits.parse().ok().filter(|ordinal| *ordinal > 1)
}

/// Reverses [`encode_qualname`]: the raw components and the ordinal. `None`
/// when `encoded` is not exactly what [`encode_qualname`] produces (bad
/// escapes, lower-case hex, an empty component, a non-canonical ordinal).
#[must_use]
pub fn decode_qualname(encoded: &str) -> Option<(Vec<String>, u32)> {
    let (names, ordinal) = match encoded.split_once(ORDINAL) {
        Some((names, digits)) => (names, parse_ordinal(digits)?),
        None => (encoded, 1),
    };
    let components = names
        .split('.')
        .map(decode_component)
        .collect::<Option<Vec<String>>>()?;
    let canonical = !components.iter().any(String::is_empty)
        && encode_qualname(&components, ordinal) == encoded;
    canonical.then_some((components, ordinal))
}
