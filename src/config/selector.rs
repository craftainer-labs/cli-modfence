//! Selectors (ADR 0004) and the value parsers shared by the config model.
//!
//! Every parser reports its problem as a serde custom error from inside the
//! value's own `Deserialize` impl, so `toml` attaches the span of the key or
//! value that is wrong.

use std::collections::BTreeMap;
use std::fmt;
use std::marker::PhantomData;

use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};

use crate::extract::{Lang, NodeKind, Tags};

/// Compiles a path or package glob. `*` does not cross `/` (gitignore-style).
pub(crate) fn compile_glob(pattern: &str) -> Result<GlobMatcher, String> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|error| error.to_string())
}

/// Whether `text` is a lowercase name, `[a-z][a-z0-9_-]*`.
pub(crate) fn is_name(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// A way to read one string into a typed value.
pub(crate) trait Parse {
    /// What the string becomes.
    type Out;

    /// Reads `text`, or says what is wrong with it.
    fn parse(text: &str) -> Result<Self::Out, String>;
}

/// A non-empty string.
pub(crate) struct Plain;

impl Parse for Plain {
    type Out = String;

    fn parse(text: &str) -> Result<String, String> {
        if text.is_empty() {
            Err("a value must not be empty".to_owned())
        } else {
            Ok(text.to_owned())
        }
    }
}

/// A tag value: non-empty, no double quote, no control character.
pub(crate) struct TagValue;

impl Parse for TagValue {
    type Out = String;

    fn parse(text: &str) -> Result<String, String> {
        if text.is_empty() || text.contains('"') || text.chars().any(char::is_control) {
            Err("a tag value must be non-empty, without double quotes or control characters"
                .to_owned())
        } else {
            Ok(text.to_owned())
        }
    }
}

/// A path or package glob.
pub(crate) struct PathGlob;

impl Parse for PathGlob {
    type Out = GlobMatcher;

    fn parse(text: &str) -> Result<GlobMatcher, String> {
        if text.is_empty() {
            return Err("a glob must not be empty".to_owned());
        }
        compile_glob(text).map_err(|error| format!("invalid glob `{text}`: {error}"))
    }
}

/// A language code, `py`, `ts` or `rs`.
pub(crate) struct LangName;

impl Parse for LangName {
    type Out = Lang;

    fn parse(text: &str) -> Result<Lang, String> {
        [Lang::Py, Lang::Ts, Lang::Rs]
            .into_iter()
            .find(|lang| lang.as_str() == text)
            .ok_or_else(|| format!("unknown language `{text}`, expected py, ts or rs"))
    }
}

/// A node kind name.
pub(crate) struct KindName;

impl Parse for KindName {
    type Out = NodeKind;

    fn parse(text: &str) -> Result<NodeKind, String> {
        [
            NodeKind::Package,
            NodeKind::Module,
            NodeKind::Symbol,
            NodeKind::External,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == text)
        .ok_or_else(|| {
            format!("unknown kind `{text}`, expected package, module, symbol or external")
        })
    }
}

/// One string, parsed by `P`.
pub(crate) struct One<P: Parse>(pub P::Out);

impl<'de, P: Parse> Deserialize<'de> for One<P> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        P::parse(&text)
            .map(One)
            .map_err(<D::Error as de::Error>::custom)
    }
}

/// A string or a non-empty list of strings, each parsed by `P`.
pub(crate) struct Many<P: Parse>(pub Vec<P::Out>);

impl<'de, P: Parse> Deserialize<'de> for Many<P> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Items<P>(PhantomData<P>);

        impl<'de, P: Parse> Visitor<'de> for Items<P> {
            type Value = Many<P>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a string or a non-empty list of strings")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Many<P>, E> {
                Ok(Many(vec![P::parse(text).map_err(E::custom)?]))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Many<P>, A::Error> {
                let mut items = Vec::new();
                while let Some(text) = seq.next_element::<String>()? {
                    items.push(P::parse(&text).map_err(<A::Error as de::Error>::custom)?);
                }
                if items.is_empty() {
                    return Err(<A::Error as de::Error>::custom("a list must not be empty"));
                }
                Ok(Many(items))
            }
        }

        deserializer.deserialize_any(Items::<P>(PhantomData))
    }
}

/// A selector key: a lowercase name.
struct SelectorKey(String);

impl<'de> Deserialize<'de> for SelectorKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if is_name(&text) {
            Ok(Self(text))
        } else {
            Err(<D::Error as de::Error>::custom(format!(
                "a selector key must match [a-z][a-z0-9_-]*, not `{text}`"
            )))
        }
    }
}

/// The facts about one node that a selector looks at.
#[derive(Debug, Clone, Copy)]
pub struct Subject<'a> {
    /// The node's kind.
    pub kind: NodeKind,
    /// The node's language.
    pub lang: Lang,
    /// The file or directory of a package, module or symbol; `None` for an
    /// external node.
    pub path: Option<&'a str>,
    /// The package name of an external node; `None` for any other.
    pub package: Option<&'a str>,
    /// The node's effective tags.
    pub tags: &'a Tags,
}

/// A rule selector. Keys are AND-ed, a list of values is an OR, and a key
/// matches a node if any of the node's values for it matches (ADR 0004). A
/// field that is empty was not given.
#[derive(Debug, Clone, Default)]
pub struct Selector {
    /// `path` globs, matched against a node's path.
    pub paths: Vec<GlobMatcher>,
    /// `package` globs, matched against an external node's package name.
    pub packages: Vec<GlobMatcher>,
    /// `lang` values.
    pub langs: Vec<Lang>,
    /// `kind` values.
    pub kinds: Vec<NodeKind>,
    /// Tag keys with the values that select a node.
    pub tags: BTreeMap<String, Vec<String>>,
}

impl Selector {
    /// Whether the selector names no key at all.
    fn is_empty(&self) -> bool {
        self.paths.is_empty()
            && self.packages.is_empty()
            && self.langs.is_empty()
            && self.kinds.is_empty()
            && self.tags.is_empty()
    }

    /// Whether `subject` is selected.
    #[must_use]
    pub fn matches(&self, subject: &Subject<'_>) -> bool {
        any_glob(&self.paths, subject.path)
            && any_glob(&self.packages, subject.package)
            && (self.langs.is_empty() || self.langs.contains(&subject.lang))
            && (self.kinds.is_empty() || self.kinds.contains(&subject.kind))
            && self.tags.iter().all(|(key, wanted)| {
                subject
                    .tags
                    .get(key)
                    .is_some_and(|have| wanted.iter().any(|value| have.contains(value)))
            })
    }
}

/// True when no glob was given, or `value` exists and one of them matches it.
fn any_glob(globs: &[GlobMatcher], value: Option<&str>) -> bool {
    globs.is_empty() || value.is_some_and(|value| globs.iter().any(|glob| glob.is_match(value)))
}

impl<'de> Deserialize<'de> for Selector {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Keys;

        impl<'de> Visitor<'de> for Keys {
            type Value = Selector;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a selector table")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Selector, A::Error> {
                let mut selector = Selector::default();
                while let Some(SelectorKey(key)) = map.next_key::<SelectorKey>()? {
                    match key.as_str() {
                        "path" => selector.paths = map.next_value::<Many<PathGlob>>()?.0,
                        "package" => selector.packages = map.next_value::<Many<PathGlob>>()?.0,
                        "lang" => selector.langs = map.next_value::<Many<LangName>>()?.0,
                        "kind" => selector.kinds = map.next_value::<Many<KindName>>()?.0,
                        _ => {
                            let values = map.next_value::<Many<Plain>>()?.0;
                            selector.tags.insert(key, values);
                        }
                    }
                }
                if selector.is_empty() {
                    return Err(<A::Error as de::Error>::custom(
                        "a selector needs at least one key",
                    ));
                }
                Ok(selector)
            }
        }

        deserializer.deserialize_map(Keys)
    }
}
