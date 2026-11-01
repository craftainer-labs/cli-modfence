//! Node ids (ADR 0003): `pkg|mod|sym|ext:<py|ts|rs>:<path>[#<qualified name>]`.
//!
//! The path (or package name) is given raw and percent-encoded here. The
//! qualified name is language-specific (each extractor amends ADR 0003 with
//! its encoding), so the extractor supplies it already encoded and this module
//! only validates it against the id pattern of `schema/modfence-graph.schema.json`:
//!
//! ```text
//! ^(pkg|mod|sym|ext):(py|ts|rs):([^\s#%]|%[0-9A-Fa-f]{2})+(#([^\s#%]|%[0-9A-Fa-f]{2})+)?$
//! ```

use std::fmt;

use super::model::{Lang, NodeKind};

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Why a node could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeError {
    /// The path or package name was empty.
    EmptyPath,
    /// The label was empty.
    EmptyLabel,
    /// A symbol id needs a qualified name and none was given.
    MissingQualName,
    /// Only symbol ids carry a qualified name.
    UnexpectedQualName,
    /// The qualified name is empty or is not valid already-encoded text (it
    /// holds whitespace, `#`, or a `%` not followed by two hex digits).
    InvalidQualName,
}

impl NodeError {
    const fn message(self) -> &'static str {
        match self {
            Self::EmptyPath => "the path or package name is empty",
            Self::EmptyLabel => "the label is empty",
            Self::MissingQualName => "a symbol id needs a qualified name",
            Self::UnexpectedQualName => "only symbol ids carry a qualified name",
            Self::InvalidQualName => "the qualified name is empty or not validly percent-encoded",
        }
    }
}

impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for NodeError {}

/// A validated node id; its text always matches the schema's `nodeId` pattern.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(String);

impl NodeId {
    /// Builds the id for `kind` in `lang` from a raw `path` (a directory, a
    /// file or an external package name, relative to the analysis root, with
    /// `/` separators) and, for symbols only, an already-encoded `qualname`.
    ///
    /// # Errors
    ///
    /// [`NodeError`] when the path is empty, when a symbol has no qualified
    /// name, when another kind has one, or when the qualified name is not
    /// valid encoded text.
    pub fn new(
        kind: NodeKind,
        lang: Lang,
        path: &str,
        qualname: Option<&str>,
    ) -> Result<Self, NodeError> {
        if path.is_empty() {
            return Err(NodeError::EmptyPath);
        }
        let mut id = String::new();
        id.push_str(kind.prefix());
        id.push(':');
        id.push_str(lang.as_str());
        id.push(':');
        id.push_str(&encode_component(path));
        match (kind == NodeKind::Symbol, qualname) {
            (true, Some(name)) if is_encoded(name) => {
                id.push('#');
                id.push_str(name);
            }
            (true, Some(_)) => return Err(NodeError::InvalidQualName),
            (true, None) => return Err(NodeError::MissingQualName),
            (false, Some(_)) => return Err(NodeError::UnexpectedQualName),
            (false, None) => {}
        }
        Ok(Self(id))
    }

    /// The id text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The original (decoded) path or package name.
    #[must_use]
    pub fn path(&self) -> String {
        let rest = self.0.splitn(3, ':').nth(2).unwrap_or_default();
        let encoded = rest.split('#').next().unwrap_or_default();
        decode_component(encoded).unwrap_or_default()
    }

    /// The qualified name exactly as the extractor encoded it, for symbols.
    #[must_use]
    pub fn qualname(&self) -> Option<&str> {
        self.0.split_once('#').map(|(_, name)| name)
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// True for the characters the id pattern's `\s` can match under any common
/// regex engine: Unicode white space, control characters and U+FEFF.
fn needs_encoding(c: char) -> bool {
    c.is_whitespace() || c.is_control() || c == '\u{feff}' || c == '#' || c == '%'
}

/// Percent-encodes whitespace, control characters, U+FEFF, `#` and `%` in
/// `raw`, byte by byte in UTF-8 with upper-case hex digits. Every other
/// character is kept as it is.
#[must_use]
pub fn encode_component(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        if needs_encoding(c) {
            let mut buf = [0_u8; 4];
            for byte in c.encode_utf8(&mut buf).bytes() {
                out.push('%');
                out.push(char::from(HEX[usize::from(byte >> 4)]));
                out.push(char::from(HEX[usize::from(byte & 0x0F)]));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Reverses [`encode_component`]. `None` when a `%` is not followed by two
/// hex digits or the decoded bytes are not UTF-8.
#[must_use]
pub fn decode_component(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let high = hex_value(*bytes.get(i + 1)?)?;
            let low = hex_value(*bytes.get(i + 2)?)?;
            out.push((high << 4) | low);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

/// True when `text` is non-empty and holds no whitespace or `#`, and every `%`
/// starts a valid `%XX` escape: the pattern's `([^\s#%]|%[0-9A-Fa-f]{2})+`.
#[must_use]
pub fn is_encoded(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let valid = matches!(
                (chars.next(), chars.next()),
                (Some(a), Some(b)) if a.is_ascii_hexdigit() && b.is_ascii_hexdigit()
            );
            if !valid {
                return false;
            }
        } else if needs_encoding(c) {
            return false;
        }
    }
    true
}
