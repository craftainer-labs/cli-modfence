//! `modfence.toml` (ADR 0004): the configuration model and its parser.
//!
//! - [`parse`] reads the text of a config file into a [`Config`]. Unknown keys,
//!   reserved or duplicate ids, bad globs and the other mistakes of ADR 0004's
//!   invariants 1, 2, 6 and 13 are [`ParseError`]s that carry the byte span, the
//!   line and the column of the key or value that is wrong.
//! - [`Selector`] and [`Subject`] select nodes for rules and `[[node]]` entries.
//! - [`resolve_tags`] computes a node's effective tags from its levels.
//!
//! Path globs match the whole repository-relative path, and `*` does not cross
//! `/` (gitignore-style, `**` does).
//!
//! The file is the input of the rules engine (NC4b) and of the tag computation
//! in the `graph` module; this module reads no files and runs nothing.

mod precedence;
mod selector;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::ops::Range;

use globset::GlobMatcher;
use serde::Deserialize;
use toml::Spanned;

use crate::extract::Tags;

use self::precedence::TagKey;
use self::selector::{Many, One, PathGlob, TagValue};

pub use self::precedence::{resolve_tags, validate_tag_key};
pub use self::selector::{Selector, Subject};

/// A configuration problem, located in the file when the parser knows where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// What is wrong.
    pub message: String,
    /// The byte range of the offending key or value in the file.
    pub span: Option<Range<usize>>,
    /// 1-based line of the start of `span`.
    pub line: Option<u32>,
    /// 1-based column, in code points, of the start of `span`.
    pub column: Option<u32>,
}

impl ParseError {
    fn new(text: &str, message: String, span: Option<Range<usize>>) -> Self {
        let (line, column) = span
            .as_ref()
            .map(|span| line_column(text, span.start))
            .unzip();
        Self {
            message,
            span,
            line,
            column,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.column) {
            (Some(line), Some(column)) => write!(f, "{line}:{column}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ParseError {}

/// The 1-based line and column (in code points) of byte offset `offset`.
fn line_column(text: &str, offset: usize) -> (u32, u32) {
    let before = text.get(..offset).unwrap_or(text);
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().unwrap_or_default().chars().count() + 1;
    (
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(column).unwrap_or(u32::MAX),
    )
}

/// What happens to a dependency no rule decided (`default = ...`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DefaultEffect {
    /// It is permitted.
    #[default]
    Allow,
    /// Between two nodes of the repository it is a violation of `default-deny`.
    Deny,
}

/// What a rule does with the sites it matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    /// Report a violation.
    Forbid,
    /// Permit the site and stop.
    Allow,
}

/// The edge relations a rule can apply to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Relation {
    /// `imports` edges.
    Imports,
    /// `binds` edges.
    Binds,
    /// `calls` edges.
    Calls,
}

/// Which edges a rule counts, by confidence (ADR 0004, "Call edges in rules").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfidenceReq {
    /// Every edge, heuristic calls included.
    Any,
    /// Only resolved edges.
    Resolved,
}

/// How serious a violation is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Fails the check at the default `--fail-on`.
    Error,
    /// A warning.
    Warning,
    /// A note.
    Note,
}

/// Which nodes a `[[node]]` entry tags.
#[derive(Debug, Clone)]
pub enum EntryTarget {
    /// A glob over repository-relative paths (`match`).
    Path(GlobMatcher),
    /// A glob over external package names (`package`).
    Package(GlobMatcher),
}

/// A `[[node]]` entry: tags for the nodes it matches.
#[derive(Debug, Clone)]
pub struct NodeEntry {
    /// Which nodes.
    pub target: EntryTarget,
    /// The tags, sorted and de-duplicated.
    pub tags: Tags,
}

impl NodeEntry {
    /// Whether the entry applies to `subject`.
    #[must_use]
    pub fn matches(&self, subject: &Subject<'_>) -> bool {
        match &self.target {
            EntryTarget::Path(glob) => subject.path.is_some_and(|path| glob.is_match(path)),
            EntryTarget::Package(glob) => {
                subject.package.is_some_and(|name| glob.is_match(name))
            }
        }
    }
}

/// A `[[rule]]`.
#[derive(Debug, Clone)]
pub struct Rule {
    /// The unique id, which is also the SARIF rule id.
    pub id: String,
    /// The source selector.
    pub from: Selector,
    /// The target selector.
    pub to: Selector,
    /// Forbid or allow.
    pub effect: Effect,
    /// The relations the rule applies to, sorted; all three by default.
    pub kinds: Vec<Relation>,
    /// The confidence requirement; `None` when omitted.
    pub confidence: Option<ConfidenceReq>,
    /// The severity of a violation; `error` by default.
    pub severity: Severity,
    /// The message of a violation, if the rule has one.
    pub message: Option<String>,
}

/// The parsed `modfence.toml`.
#[derive(Debug, Clone)]
pub struct Config {
    /// `exclude` globs, added to the ignore files the walker honours.
    pub exclude: Vec<GlobMatcher>,
    /// Layer names, highest first; empty when `layers` is not set.
    pub layers: Vec<String>,
    /// The default effect.
    pub default: DefaultEffect,
    /// `[tags] accumulate`.
    pub accumulate: BTreeSet<String>,
    /// `[annotations] require_reason`; true by default.
    pub require_reason: bool,
    /// The `[[node]]` entries, in file order.
    pub nodes: Vec<NodeEntry>,
    /// The `[[rule]]`s, in file order.
    pub rules: Vec<Rule>,
}

impl Default for Config {
    /// The configuration of a repository without a `modfence.toml`.
    fn default() -> Self {
        Self {
            exclude: Vec::new(),
            layers: Vec::new(),
            default: DefaultEffect::Allow,
            accumulate: BTreeSet::new(),
            require_reason: true,
            nodes: Vec::new(),
            rules: Vec::new(),
        }
    }
}

impl Config {
    /// The effective tags of the node `subject`: the matching `[[node]]`
    /// entries in file order, then the `inline` levels (the module's, then the
    /// symbol's), resolved as in [`resolve_tags`].
    #[must_use]
    pub fn effective_tags<'a>(&'a self, subject: &Subject<'_>, inline: &[&'a Tags]) -> Tags {
        let entries = self
            .nodes
            .iter()
            .filter(|entry| entry.matches(subject))
            .map(|entry| &entry.tags);
        resolve_tags(entries.chain(inline.iter().copied()), &self.accumulate)
    }
}

/// Parses the text of a `modfence.toml`.
///
/// # Errors
///
/// A [`ParseError`] for malformed TOML, an unknown key, a wrong type or value,
/// or a violation of ADR 0004's invariants 1, 2, 6 and 13.
pub fn parse(text: &str) -> Result<Config, ParseError> {
    let raw: Raw = toml::from_str(text)
        .map_err(|error| ParseError::new(text, error.message().to_owned(), error.span()))?;
    raw.build()
        .map_err(|(message, span)| ParseError::new(text, message, Some(span)))
}

/// A problem found after parsing: what, and the span of the culprit.
type Problem = (String, Range<usize>);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    version: Spanned<u32>,
    #[serde(default)]
    exclude: Vec<One<PathGlob>>,
    layers: Option<Spanned<Vec<Spanned<String>>>>,
    #[serde(default)]
    default: DefaultEffect,
    #[serde(default)]
    tags: RawTags,
    #[serde(default)]
    annotations: RawAnnotations,
    #[serde(default, rename = "node")]
    nodes: Vec<Spanned<RawNode>>,
    #[serde(default, rename = "rule")]
    rules: Vec<RawRule>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTags {
    #[serde(default)]
    accumulate: Vec<Spanned<String>>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAnnotations {
    require_reason: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNode {
    #[serde(rename = "match")]
    path: Option<One<PathGlob>>,
    package: Option<One<PathGlob>>,
    tags: BTreeMap<TagKey, Many<TagValue>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: Spanned<String>,
    from: Selector,
    to: Selector,
    effect: Effect,
    kinds: Option<Spanned<Vec<Relation>>>,
    confidence: Option<Spanned<ConfidenceReq>>,
    severity: Option<Severity>,
    message: Option<Spanned<String>>,
}

impl Raw {
    fn build(self) -> Result<Config, Problem> {
        let version = self.version;
        if *version.get_ref() != 1 {
            return Err((
                format!("unsupported version {}, expected 1", version.get_ref()),
                version.span(),
            ));
        }
        Ok(Config {
            exclude: self.exclude.into_iter().map(|glob| glob.0).collect(),
            layers: layers(self.layers)?,
            default: self.default,
            accumulate: accumulate(self.tags.accumulate)?,
            require_reason: self.annotations.require_reason.unwrap_or(true),
            nodes: self
                .nodes
                .into_iter()
                .map(node_entry)
                .collect::<Result<Vec<NodeEntry>, Problem>>()?,
            rules: rules(self.rules)?,
        })
    }
}

fn layers(raw: Option<Spanned<Vec<Spanned<String>>>>) -> Result<Vec<String>, Problem> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let span = raw.span();
    let mut seen = BTreeSet::new();
    let mut names = Vec::new();
    for item in raw.into_inner() {
        let name = item.get_ref();
        let valid = !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !valid {
            return Err((
                format!("a layer name may only hold letters, digits and . _ -, not `{name}`"),
                item.span(),
            ));
        }
        if !seen.insert(name.clone()) {
            return Err((format!("duplicate layer `{name}`"), item.span()));
        }
        names.push(name.clone());
    }
    if names.len() < 2 {
        return Err(("layers needs at least two names".to_owned(), span));
    }
    Ok(names)
}

fn accumulate(raw: Vec<Spanned<String>>) -> Result<BTreeSet<String>, Problem> {
    let mut keys = BTreeSet::new();
    for item in raw {
        validate_tag_key(item.get_ref()).map_err(|message| (message, item.span()))?;
        if !keys.insert(item.get_ref().clone()) {
            return Err((format!("`{}` is listed twice", item.get_ref()), item.span()));
        }
    }
    Ok(keys)
}

fn node_entry(raw: Spanned<RawNode>) -> Result<NodeEntry, Problem> {
    let span = raw.span();
    let raw = raw.into_inner();
    let target = match (raw.path, raw.package) {
        (Some(One(path)), None) => EntryTarget::Path(path),
        (None, Some(One(package))) => EntryTarget::Package(package),
        _ => {
            return Err((
                "a [[node]] needs exactly one of `match` and `package`".to_owned(),
                span,
            ));
        }
    };
    if raw.tags.is_empty() {
        return Err(("a [[node]] needs at least one tag".to_owned(), span));
    }
    let tags = raw
        .tags
        .into_iter()
        .map(|(TagKey(key), Many(values))| (key, values.into_iter().collect()))
        .collect();
    Ok(NodeEntry { target, tags })
}

/// Checks a rule id: not reserved, and `[a-z0-9][a-z0-9-]*`.
fn check_rule_id(id: &str) -> Result<(), String> {
    if id == "layers" || id == "default-deny" || id.starts_with("modfence/") {
        return Err(format!("`{id}` is a reserved rule id"));
    }
    let mut chars = id.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("a rule id must match [a-z0-9][a-z0-9-]*, not `{id}`"))
    }
}

fn rules(raw: Vec<RawRule>) -> Result<Vec<Rule>, Problem> {
    let mut ids = BTreeSet::new();
    let mut rules = Vec::new();
    for rule in raw {
        let id = rule.id.get_ref().clone();
        let id_span = rule.id.span();
        check_rule_id(&id).map_err(|message| (message, id_span.clone()))?;
        if !ids.insert(id.clone()) {
            return Err((format!("duplicate rule id `{id}`"), id_span));
        }
        let kinds = match rule.kinds {
            None => vec![Relation::Imports, Relation::Binds, Relation::Calls],
            Some(kinds) if kinds.get_ref().is_empty() => {
                return Err(("kinds must not be empty".to_owned(), kinds.span()));
            }
            Some(kinds) => {
                let mut kinds = kinds.into_inner();
                kinds.sort();
                kinds.dedup();
                kinds
            }
        };
        let never = rule.confidence.as_ref().filter(|confidence| {
            *confidence.get_ref() == ConfidenceReq::Resolved && kinds == [Relation::Binds]
        });
        if let Some(confidence) = never {
            return Err((
                format!(
                    "rule `{id}` can never match: binds edges are always heuristic, \
                     so confidence = \"resolved\" excludes them"
                ),
                confidence.span(),
            ));
        }
        let empty = rule
            .message
            .as_ref()
            .filter(|message| message.get_ref().is_empty());
        if let Some(message) = empty {
            return Err(("a message must not be empty".to_owned(), message.span()));
        }
        rules.push(Rule {
            id,
            from: rule.from,
            to: rule.to,
            effect: rule.effect,
            kinds,
            confidence: rule.confidence.map(Spanned::into_inner),
            severity: rule.severity.unwrap_or(Severity::Error),
            message: rule.message.map(Spanned::into_inner),
        });
    }
    Ok(rules)
}
