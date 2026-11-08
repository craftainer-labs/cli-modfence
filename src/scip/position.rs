//! SCIP positions to code-point spans (ADR 0003, invariant 20).

use crate::extract::Span;

/// How a document counts the `character` of its ranges (`PositionEncoding` in
/// `scip.proto`). An unspecified encoding is read as UTF-8 by the reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PositionEncoding {
    /// Offsets in UTF-8 bytes from the start of the line.
    Utf8,
    /// Offsets in UTF-16 code units from the start of the line.
    Utf16,
    /// Offsets in UTF-32 code units (code points) from the start of the line.
    Utf32,
}

impl PositionEncoding {
    /// How many offset units `c` takes in this encoding.
    const fn width(self, c: char) -> usize {
        match self {
            Self::Utf8 => c.len_utf8(),
            Self::Utf16 => c.len_utf16(),
            Self::Utf32 => 1,
        }
    }
}

/// A range as SCIP states it: 0-based lines, `character` offsets in the
/// document's [`PositionEncoding`], the end exclusive. The reader only builds
/// ranges whose end is not before their start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawRange {
    /// First line, from 0.
    pub start_line: u32,
    /// First offset on that line, from 0.
    pub start_character: u32,
    /// Last line, from 0.
    pub end_line: u32,
    /// Offset after the last character on that line.
    pub end_character: u32,
}

impl RawRange {
    /// A range from the signed integers of the wire format, or `None` when
    /// one is negative or the end lies before the start.
    pub(crate) fn from_wire(
        start_line: i32,
        start_character: i32,
        end_line: i32,
        end_character: i32,
    ) -> Option<Self> {
        let range = Self {
            start_line: u32::try_from(start_line).ok()?,
            start_character: u32::try_from(start_character).ok()?,
            end_line: u32::try_from(end_line).ok()?,
            end_character: u32::try_from(end_character).ok()?,
        };
        let start = (range.start_line, range.start_character);
        let end = (range.end_line, range.end_character);
        (end >= start).then_some(range)
    }
}

/// The lines of one source file, for converting [`RawRange`]s. Lines end at
/// `\n` only, as SCIP defines (a lone `\r`, U+2028 and U+2029 do not end a
/// line); a `\r` before the `\n` is stripped and is not part of the line.
#[derive(Debug, Clone)]
pub struct LineIndex<'a> {
    lines: Vec<&'a str>,
}

impl<'a> LineIndex<'a> {
    /// Indexes `source`.
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        Self {
            lines: source
                .split('\n')
                .map(|line| line.strip_suffix('\r').unwrap_or(line))
                .collect(),
        }
    }

    /// `range` as a [`Span`] (1-based, columns in code points), or `None` when
    /// a line does not exist or an offset lies beyond its line or inside a
    /// character (a sign that the index does not match this source).
    #[must_use]
    pub fn span(&self, range: RawRange, encoding: PositionEncoding) -> Option<Span> {
        let first = self.lines.get(usize::try_from(range.start_line).ok()?)?;
        let last = self.lines.get(usize::try_from(range.end_line).ok()?)?;
        Span::new(
            range.start_line.checked_add(1)?,
            column(first, range.start_character, encoding)?,
            range.end_line.checked_add(1)?,
            column(last, range.end_character, encoding)?,
        )
    }
}

/// The 1-based code-point column of offset `units` in `line`.
fn column(line: &str, units: u32, encoding: PositionEncoding) -> Option<u32> {
    let units = usize::try_from(units).ok()?;
    let (mut seen, mut points) = (0_usize, 0_usize);
    for c in line.chars() {
        if seen == units {
            break;
        }
        seen += encoding.width(c);
        points += 1;
        if seen > units {
            return None;
        }
    }
    if seen == units {
        u32::try_from(points + 1).ok()
    } else {
        None
    }
}
