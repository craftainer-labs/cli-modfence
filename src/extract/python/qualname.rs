//! Python qualified names (ADR 0003, "Python qualified names").
//!
//! A symbol's qualified name is the list of names from the module down to it,
//! as Python's `__qualname__` writes them: `Engine.run`, and
//! `Engine.run.<locals>.inner` for a definition inside a function body. Each
//! component is percent-encoded and the components are joined with `.`. A
//! later definition of a name that is already defined in the same scope of the
//! same file gets the suffix `@2`, `@3`, and so on on its own component, so the
//! names below it say which definition they belong to (`C@2.m`).

use crate::extract::ident::{decode_component, encode_component};

/// The component Python puts between a function and the names defined in its
/// body.
pub const LOCALS: &str = "<locals>";

const ORDINAL: char = '@';

/// One component of a qualified name: the raw name and its ordinal (1 for the
/// first definition of the name, counting up for later ones).
pub type Component<S = String> = (S, u32);

/// Percent-encodes one name. On top of [`encode_component`], `.` and `@` are
/// escaped (`%2E`, `%40`), so that they separate components and mark the
/// ordinal and nothing else.
fn encode_part(raw: &str) -> String {
    encode_component(raw)
        .replace('.', "%2E")
        .replace(ORDINAL, "%40")
}

/// Encodes a qualified name; the ordinal shows in the result only above 1.
/// `components` must not be empty, and no name may be empty, or the result is
/// not a valid qualified name.
#[must_use]
pub fn encode_qualname<S: AsRef<str>>(components: &[Component<S>]) -> String {
    let parts: Vec<String> = components
        .iter()
        .map(|(name, ordinal)| {
            let part = encode_part(name.as_ref());
            if *ordinal > 1 {
                format!("{part}{ORDINAL}{ordinal}")
            } else {
                part
            }
        })
        .collect();
    parts.join(".")
}

/// The ordinal of a later definition: a number above 1.
fn parse_ordinal(digits: &str) -> Option<u32> {
    digits.parse().ok().filter(|ordinal| *ordinal > 1)
}

/// Splits `part` at its ordinal suffix and decodes the name.
fn decode_part(part: &str) -> Option<Component> {
    let (name, ordinal) = match part.split_once(ORDINAL) {
        Some((name, digits)) => (name, parse_ordinal(digits)?),
        None => (part, 1),
    };
    Some((decode_component(name)?, ordinal))
}

/// Reverses [`encode_qualname`]. `None` when `encoded` is not exactly what
/// [`encode_qualname`] produces (bad escapes, lower-case hex, an empty name, a
/// non-canonical ordinal).
#[must_use]
pub fn decode_qualname(encoded: &str) -> Option<Vec<Component>> {
    let components = encoded
        .split('.')
        .map(decode_part)
        .collect::<Option<Vec<Component>>>()?;
    let canonical = !components.iter().any(|(name, _)| name.is_empty())
        && encode_qualname(&components) == encoded;
    canonical.then_some(components)
}
