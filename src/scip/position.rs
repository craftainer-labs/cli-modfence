//! SCIP positions to code-point spans (ADR 0003, invariant 20).

use std::cell::OnceCell;

use crate::extract::Span;

/// How a document counts the `character` of its ranges (`PositionEncoding` in
/// `scip.proto`). An unspecified encoding is read as UTF-16 by the reader.
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
    /// The index of this encoding in a line's offset tables.
    const fn slot(self) -> usize {
        match self {
            Self::Utf8 => 0,
            Self::Utf16 => 1,
            Self::Utf32 => 2,
        }
    }

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
///
/// Converting is cheap however long a line is: an ASCII line needs no table,
/// and any other line gets one table of its character boundaries per encoding,
/// built on first use and searched by bisection.
#[derive(Debug, Clone)]
pub struct LineIndex<'a> {
    lines: Vec<Line<'a>>,
}

/// One line and its offset tables.
#[derive(Debug, Clone)]
struct Line<'a> {
    text: &'a str,
    ascii: bool,
    /// Per encoding, the offset of every character boundary (the line's start,
    /// then the end of each character), built on first use.
    boundaries: [OnceCell<Vec<u32>>; 3],
}

impl<'a> LineIndex<'a> {
    /// Indexes `source`.
    #[must_use]
    pub fn new(source: &'a str) -> Self {
        Self {
            lines: source
                .split('\n')
                .map(|line| line.strip_suffix('\r').unwrap_or(line))
                .map(|text| Line {
                    text,
                    ascii: text.is_ascii(),
                    boundaries: [OnceCell::new(), OnceCell::new(), OnceCell::new()],
                })
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
            first.column(range.start_character, encoding)?,
            range.end_line.checked_add(1)?,
            last.column(range.end_character, encoding)?,
        )
    }

    /// The text a single-line `span` covers, or `None` for a span over several
    /// lines or outside the source.
    #[must_use]
    pub fn slice(&self, span: Span) -> Option<&'a str> {
        if span.start_line != span.end_line {
            return None;
        }
        let line = self
            .lines
            .get(usize::try_from(span.start_line.checked_sub(1)?).ok()?)?;
        line.slice(
            span.start_column.checked_sub(1)?,
            span.end_column.checked_sub(1)?,
        )
    }
}

impl<'a> Line<'a> {
    /// The offsets of the character boundaries in `encoding`, built once.
    fn table(&self, encoding: PositionEncoding) -> &[u32] {
        self.boundaries[encoding.slot()].get_or_init(|| {
            let mut seen = 0_u32;
            let mut table = vec![0];
            for c in self.text.chars() {
                seen = seen.saturating_add(u32::try_from(encoding.width(c)).unwrap_or(u32::MAX));
                table.push(seen);
            }
            table
        })
    }

    /// The 1-based code-point column of offset `units`, or `None` when it lies
    /// past the end of the line or inside a character.
    fn column(&self, units: u32, encoding: PositionEncoding) -> Option<u32> {
        let points = if self.ascii {
            if usize::try_from(units).ok()? > self.text.len() {
                return None;
            }
            usize::try_from(units).ok()?
        } else {
            self.table(encoding).binary_search(&units).ok()?
        };
        u32::try_from(points + 1).ok()
    }

    /// The text between the code-point indices `from` and `to` (0-based, the
    /// end exclusive).
    fn slice(&self, from: u32, to: u32) -> Option<&'a str> {
        if to < from {
            return None;
        }
        let (from, to) = (usize::try_from(from).ok()?, usize::try_from(to).ok()?);
        if self.ascii {
            return self.text.get(from..to);
        }
        let table = self.table(PositionEncoding::Utf8);
        let (begin, end) = (*table.get(from)?, *table.get(to)?);
        self.text
            .get(usize::try_from(begin).ok()?..usize::try_from(end).ok()?)
    }
}
