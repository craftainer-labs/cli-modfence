//! The shared parser for inline `modfence:` comments (ADR 0004).
//!
//! It is language-agnostic: an extractor finds comment nodes in its syntax
//! tree, strips the comment markers (see [`comment_body`]), and passes the body
//! to [`parse_comment`]. Where the comment is, and what it is attached to,
//! stays the extractor's job.
//!
//! Grammar, after the prefix `modfence:` (case-sensitive, leading blanks
//! allowed, blanks after the colon optional; a blank is a space or a tab):
//!
//! ```text
//! annotation = ignore | tags
//! tags       = tag { blank+ tag }          (at least one tag)
//! tag        = key [ "=" value ]           (a bare key means the value "true")
//! key        = [a-z][a-z0-9_-]*            (not path, package, lang or kind)
//! value      = quoted | bare               (non-empty, no control characters)
//! quoted     = '"' { any but '"' } '"'     (followed by a blank or the end)
//! bare       = { any but blank and '"' }
//! ignore     = "ignore[" id { "," id } "]" [ blanks "reason=" quoted ]
//! id         = [A-Za-z0-9_./-]+            (blanks around an id are skipped)
//! ```
//!
//! A body whose text after the prefix starts with `ignore[` is only ever a
//! suppression. Keys may repeat; their values accumulate in a sorted set.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// The prefix that makes a comment an annotation.
pub const PREFIX: &str = "modfence:";

/// Selector keys, which may not be used as tag keys.
pub const RESERVED_KEYS: [&str; 4] = ["path", "package", "lang", "kind"];

const IGNORE_OPEN: &str = "ignore[";
const REASON_OPEN: &str = "reason=\"";

/// Tag keys with their values: sorted by key, and each value set sorted in byte
/// order and free of duplicates (ADR 0003, invariant 19).
pub type Tags = BTreeMap<String, BTreeSet<String>>;

/// Adds every value of `from` to `into` (the union used when several
/// annotations sit at the same level).
pub fn merge_tags(into: &mut Tags, from: &Tags) {
    for (key, values) in from {
        into.entry(key.clone())
            .or_default()
            .extend(values.iter().cloned());
    }
}

/// An `ignore[...]` suppression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suppression {
    /// The rule ids, in the order written, without duplicates.
    pub rules: Vec<String>,
    /// The `reason="..."` text. Whether it is required is a configuration
    /// matter (`[annotations] require_reason`), decided by the caller.
    pub reason: Option<String>,
}

/// A well-formed annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    /// Tags for a node.
    Tags(Tags),
    /// A suppression of rule violations at a site.
    Ignore(Suppression),
}

/// What is wrong with a malformed annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Nothing follows the prefix.
    EmptyAnnotation,
    /// A key is empty or does not match `[a-z][a-z0-9_-]*`.
    InvalidKey,
    /// A key is `path`, `package`, `lang` or `kind`.
    ReservedKey,
    /// A value or a reason is empty.
    EmptyValue,
    /// An unquoted value holds a double quote.
    QuoteInValue,
    /// A value or a reason holds a control character.
    ControlCharacter,
    /// A quoted string is not closed.
    UnterminatedQuote,
    /// A quoted value is followed by something other than a blank.
    JunkAfterQuote,
    /// `ignore[` has no closing bracket.
    UnterminatedIgnore,
    /// A rule id in `ignore[...]` is empty.
    EmptyRuleId,
    /// A rule id in `ignore[...]` holds a character outside `[A-Za-z0-9_./-]`.
    InvalidRuleId,
    /// Something other than `reason="..."` follows `ignore[...]`, or text
    /// follows the reason.
    UnexpectedAfterIgnore,
}

impl ErrorKind {
    /// A short explanation, for the `modfence/invalid-annotation` message.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::EmptyAnnotation => "no tags after the prefix",
            Self::InvalidKey => "a tag key must match [a-z][a-z0-9_-]*",
            Self::ReservedKey => "path, package, lang and kind are selector keys, not tag keys",
            Self::EmptyValue => "a value must not be empty",
            Self::QuoteInValue => "an unquoted value must not contain a double quote",
            Self::ControlCharacter => "a value must not contain control characters",
            Self::UnterminatedQuote => "a double-quoted value is not closed",
            Self::JunkAfterQuote => "a quoted value must be followed by a space or the end",
            Self::UnterminatedIgnore => "ignore[ is not closed by ]",
            Self::EmptyRuleId => "a rule id in ignore[...] is empty",
            Self::InvalidRuleId => "a rule id may only hold letters, digits and _ . / -",
            Self::UnexpectedAfterIgnore => {
                "only reason=\"...\" may follow ignore[...], and nothing may follow the reason"
            }
        }
    }
}

/// A malformed annotation. The whole annotation is rejected, with no partial
/// result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnnotationError {
    /// What is wrong.
    pub kind: ErrorKind,
    /// Where, as a 0-based count of Unicode code points from the start of the
    /// body that was parsed.
    pub offset: usize,
}

impl fmt::Display for AnnotationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid modfence annotation at character {}: {}",
            self.offset,
            self.kind.message()
        )
    }
}

impl std::error::Error for AnnotationError {}

/// Strips the comment markers from one comment's source text: a leading `#`
/// (Python), a leading `//` (TypeScript, Rust), or `/* ... */` on a single
/// line. Returns `None` for anything else, including a block comment that
/// spans lines.
///
/// Only the marker itself is removed, so a doc comment (`/// ...`, `//! ...`),
/// a shebang (`#!...`) and similar keep their extra character and never start
/// with the prefix.
#[must_use]
pub fn comment_body(raw: &str) -> Option<&str> {
    if let Some(rest) = raw.strip_prefix("//").or_else(|| raw.strip_prefix('#')) {
        return Some(rest);
    }
    let inner = raw.strip_prefix("/*")?.strip_suffix("*/")?;
    if inner.contains(['\n', '\r']) {
        None
    } else {
        Some(inner)
    }
}

/// Parses a comment body (markers already stripped). `None` when the comment
/// is not an annotation at all (it does not start, after blanks, with
/// `modfence:`); `Some(Err(..))` when it is one but malformed.
#[must_use]
pub fn parse_comment(body: &str) -> Option<Result<Parsed, AnnotationError>> {
    let body = body.trim_end();
    let rest = body.trim_start_matches(is_blank).strip_prefix(PREFIX)?;
    let mut cur = Cursor {
        body,
        pos: body.len() - rest.len(),
    };
    cur.skip_blanks();
    Some(if cur.rest().starts_with(IGNORE_OPEN) {
        parse_ignore(&mut cur)
    } else {
        parse_tags(&mut cur)
    })
}

fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

fn is_key(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn is_rule_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-')
}

struct Cursor<'a> {
    body: &'a str,
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn rest(&self) -> &'a str {
        &self.body[self.pos..]
    }

    fn at_end(&self) -> bool {
        self.pos >= self.body.len()
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn eat(&mut self, c: char) -> bool {
        let found = self.peek() == Some(c);
        if found {
            self.pos += c.len_utf8();
        }
        found
    }

    fn skip_blanks(&mut self) {
        let rest = self.rest();
        self.pos += rest.len() - rest.trim_start_matches(is_blank).len();
    }

    fn take_while(&mut self, keep: impl Fn(char) -> bool) -> &'a str {
        let rest = self.rest();
        let len = rest.find(|c: char| !keep(c)).unwrap_or(rest.len());
        self.pos += len;
        &rest[..len]
    }

    /// Reads a double-quoted string with the cursor on its opening quote.
    fn quoted(&mut self) -> Result<&'a str, AnnotationError> {
        let open = self.pos;
        self.pos += 1;
        let inner = self.take_while(|c| c != '"');
        if self.eat('"') {
            Ok(inner)
        } else {
            Err(self.error(ErrorKind::UnterminatedQuote, open))
        }
    }

    /// An error at byte position `at`, reported in code points.
    fn error(&self, kind: ErrorKind, at: usize) -> AnnotationError {
        AnnotationError {
            kind,
            offset: self.body[..at].chars().count(),
        }
    }

    fn check_text(&self, text: &str, at: usize) -> Result<(), AnnotationError> {
        if text.is_empty() {
            Err(self.error(ErrorKind::EmptyValue, at))
        } else if text.chars().any(char::is_control) {
            Err(self.error(ErrorKind::ControlCharacter, at))
        } else {
            Ok(())
        }
    }
}

fn parse_tags(cur: &mut Cursor<'_>) -> Result<Parsed, AnnotationError> {
    let mut tags = Tags::new();
    while !cur.at_end() {
        let start = cur.pos;
        let key = cur.take_while(|c| c != '=' && !is_blank(c));
        if !is_key(key) {
            return Err(cur.error(ErrorKind::InvalidKey, start));
        }
        if RESERVED_KEYS.contains(&key) {
            return Err(cur.error(ErrorKind::ReservedKey, start));
        }
        let value = if cur.eat('=') {
            parse_value(cur)?
        } else {
            String::from("true")
        };
        tags.entry(key.to_owned()).or_default().insert(value);
        cur.skip_blanks();
    }
    if tags.is_empty() {
        Err(cur.error(ErrorKind::EmptyAnnotation, cur.pos))
    } else {
        Ok(Parsed::Tags(tags))
    }
}

fn parse_value(cur: &mut Cursor<'_>) -> Result<String, AnnotationError> {
    let start = cur.pos;
    let value = if cur.peek() == Some('"') {
        let inner = cur.quoted()?;
        if !cur.at_end() && !cur.peek().is_some_and(is_blank) {
            return Err(cur.error(ErrorKind::JunkAfterQuote, cur.pos));
        }
        inner
    } else {
        let bare = cur.take_while(|c| !is_blank(c));
        if bare.contains('"') {
            return Err(cur.error(ErrorKind::QuoteInValue, start));
        }
        bare
    };
    cur.check_text(value, start)?;
    Ok(value.to_owned())
}

fn parse_ignore(cur: &mut Cursor<'_>) -> Result<Parsed, AnnotationError> {
    cur.pos += IGNORE_OPEN.len();
    let list_start = cur.pos;
    let list = cur.take_while(|c| c != ']');
    if !cur.eat(']') {
        return Err(cur.error(ErrorKind::UnterminatedIgnore, list_start));
    }
    let mut rules: Vec<String> = Vec::new();
    let mut at = list_start;
    for item in list.split(',') {
        let id = item.trim_matches(is_blank);
        if id.is_empty() {
            return Err(cur.error(ErrorKind::EmptyRuleId, at));
        }
        if !id.chars().all(is_rule_id_char) {
            return Err(cur.error(ErrorKind::InvalidRuleId, at));
        }
        if !rules.iter().any(|known| known == id) {
            rules.push(id.to_owned());
        }
        at += item.len() + 1;
    }
    cur.skip_blanks();
    let reason = if cur.at_end() {
        None
    } else {
        Some(parse_reason(cur)?)
    };
    Ok(Parsed::Ignore(Suppression { rules, reason }))
}

fn parse_reason(cur: &mut Cursor<'_>) -> Result<String, AnnotationError> {
    let start = cur.pos;
    if !cur.rest().starts_with(REASON_OPEN) {
        return Err(cur.error(ErrorKind::UnexpectedAfterIgnore, start));
    }
    cur.pos += REASON_OPEN.len() - 1;
    let reason = cur.quoted()?;
    cur.skip_blanks();
    if !cur.at_end() {
        return Err(cur.error(ErrorKind::UnexpectedAfterIgnore, cur.pos));
    }
    cur.check_text(reason, start)?;
    Ok(reason.to_owned())
}
