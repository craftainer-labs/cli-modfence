//! Tag keys and tag precedence (ADR 0004, "Tag precedence").

use std::collections::BTreeSet;

use serde::Deserialize;
use serde::de::{self, Deserializer};

use super::selector::is_name;
use crate::extract::{RESERVED_KEYS, Tags};

/// Checks a tag key: a lowercase name that is not a selector key.
///
/// # Errors
///
/// A message saying what is wrong with `key`.
pub fn validate_tag_key(key: &str) -> Result<(), String> {
    if RESERVED_KEYS.contains(&key) {
        Err(format!(
            "`{key}` is a selector key and cannot be used as a tag key"
        ))
    } else if is_name(key) {
        Ok(())
    } else {
        Err(format!("a tag key must match [a-z][a-z0-9_-]*, not `{key}`"))
    }
}

/// A tag key read from the configuration.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TagKey(pub String);

impl<'de> Deserialize<'de> for TagKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        validate_tag_key(&text)
            .map(|()| Self(text))
            .map_err(<D::Error as de::Error>::custom)
    }
}

/// The effective tags of a node, from its levels, lowest first: the matching
/// `[[node]]` entries in file order, then the module's inline tags, then the
/// symbol's own. A key listed in `accumulate` takes the union of its values
/// over all levels; any other key takes the values of the highest level that
/// sets it (ADR 0004, invariant 11).
#[must_use]
pub fn resolve_tags<'a>(
    levels: impl IntoIterator<Item = &'a Tags>,
    accumulate: &BTreeSet<String>,
) -> Tags {
    let mut effective = Tags::new();
    for level in levels {
        for (key, values) in level {
            if accumulate.contains(key) {
                effective
                    .entry(key.clone())
                    .or_default()
                    .extend(values.iter().cloned());
            } else {
                effective.insert(key.clone(), values.clone());
            }
        }
    }
    effective
}
