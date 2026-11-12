//! Qualified names of TypeScript and JavaScript symbols (ADR 0003,
//! "TypeScript qualified names").
//!
//! A qualified name is a path of segments. Each segment is a name encoded with
//! [`encode_segment`]: the id escapes of [`encode_component`] plus `.` and `~`,
//! so an unescaped `.` or `~` is always a separator. `.` joins a namespace
//! member, an enum member or a static member to its parent; `~` joins an
//! instance member.

use crate::extract::ident::{decode_component, encode_component};

/// How a segment attaches to the segment before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sep {
    /// `.`: a namespace member or a static member.
    Dot,
    /// `~`: an instance member.
    Tilde,
}

impl Sep {
    /// The separator character.
    #[must_use]
    pub const fn as_char(self) -> char {
        match self {
            Self::Dot => '.',
            Self::Tilde => '~',
        }
    }
}

/// One decoded segment of a qualified name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    /// How it attaches to the previous segment; `None` for the first.
    pub sep: Option<Sep>,
    /// The name, decoded.
    pub name: String,
}

/// Encodes one name for use as a segment: the escapes of
/// [`encode_component`], plus `.` as `%2E` and `~` as `%7E`.
#[must_use]
pub fn encode_segment(raw: &str) -> String {
    encode_component(raw)
        .replace('.', "%2E")
        .replace('~', "%7E")
}

/// Appends the segment `raw` to `parent` with `sep`. An empty `parent` gives
/// the first segment, which has no separator.
#[must_use]
pub fn join(parent: &str, sep: Sep, raw: &str) -> String {
    let segment = encode_segment(raw);
    if parent.is_empty() {
        segment
    } else {
        format!("{parent}{}{segment}", sep.as_char())
    }
}

/// Splits an encoded qualified name into decoded segments. `None` when a
/// segment is empty or is not validly percent-encoded.
#[must_use]
pub fn split(qualname: &str) -> Option<Vec<Segment>> {
    let mut segments = Vec::new();
    let mut sep = None;
    let mut tail = qualname;
    loop {
        let end = tail.find(['.', '~']).unwrap_or(tail.len());
        let (head, rest) = tail.split_at(end);
        if head.is_empty() {
            return None;
        }
        segments.push(Segment {
            sep,
            name: decode_component(head)?,
        });
        let mut chars = rest.chars();
        sep = match chars.next() {
            None => return Some(segments),
            Some('.') => Some(Sep::Dot),
            Some(_) => Some(Sep::Tilde),
        };
        tail = chars.as_str();
    }
}

/// Builds the encoded qualified name of `segments`; the inverse of [`split`].
#[must_use]
pub fn render(segments: &[Segment]) -> String {
    let mut out = String::new();
    for segment in segments {
        out = join(&out, segment.sep.unwrap_or(Sep::Dot), &segment.name);
    }
    out
}
