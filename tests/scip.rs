//! The SCIP reader (NC6a): ranges and positions in every encoding, paths and
//! roots, the size cap and malformed input. Indexes are built in code.

use std::path::PathBuf;

use modfence::extract::Span;
use modfence::scip::{
    DEFINITION, Error as ScipError, IMPORT, Index as ScipIndex, LineIndex, MAX_INDEX_BYTES,
    PositionEncoding, RawRange, ReadStats, check_size, load, read_index,
};
use protobuf::{EnumOrUnknown, Message, MessageField};
use scip::types::occurrence::{Typed_enclosing_range, Typed_range};
use scip::types::{
    Document, Index, Metadata, MultiLineRange, Occurrence, PositionEncoding as Wire,
    SingleLineRange, ToolInfo,
};

const ROOT: &str = "/repo";

fn occurrence(range: &[i32], symbol: &str, roles: i32) -> Occurrence {
    let mut occurrence = Occurrence::new();
    occurrence.range = range.to_vec();
    symbol.clone_into(&mut occurrence.symbol);
    occurrence.symbol_roles = roles;
    occurrence
}

fn single(line: i32, start: i32, end: i32) -> SingleLineRange {
    let mut range = SingleLineRange::new();
    range.line = line;
    range.start_character = start;
    range.end_character = end;
    range
}

fn multi(start_line: i32, start: i32, end_line: i32, end: i32) -> MultiLineRange {
    let mut range = MultiLineRange::new();
    range.start_line = start_line;
    range.start_character = start;
    range.end_line = end_line;
    range.end_character = end;
    range
}

fn document(path: &str, encoding: Wire, occurrences: Vec<Occurrence>) -> Document {
    let mut document = Document::new();
    path.clone_into(&mut document.relative_path);
    document.position_encoding = EnumOrUnknown::new(encoding);
    document.occurrences = occurrences;
    document
}

fn index_with(project_root: &str, tool: Option<(&str, &str)>, documents: Vec<Document>) -> Vec<u8> {
    let mut metadata = Metadata::new();
    project_root.clone_into(&mut metadata.project_root);
    if let Some((name, version)) = tool {
        let mut info = ToolInfo::new();
        name.clone_into(&mut info.name);
        version.clone_into(&mut info.version);
        metadata.tool_info = MessageField::some(info);
    }
    let mut index = Index::new();
    index.metadata = MessageField::some(metadata);
    index.documents = documents;
    index.write_to_bytes().expect("serialises")
}

fn read(documents: Vec<Document>) -> (ScipIndex, ReadStats) {
    read_index(&index_with("", None, documents), ROOT).expect("reads")
}

fn one_document(occurrences: Vec<Occurrence>) -> (ScipIndex, ReadStats) {
    read(vec![document(
        "a.py",
        Wire::UTF8CodeUnitOffsetFromLineStart,
        occurrences,
    )])
}

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("modfence-scip-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("scratch directory");
    path
}

fn range(start_line: u32, start: u32, end_line: u32, end: u32) -> RawRange {
    RawRange {
        start_line,
        start_character: start,
        end_line,
        end_character: end,
    }
}

#[test]
fn integer_list_ranges_have_three_or_four_elements() {
    let (index, stats) = one_document(vec![
        occurrence(&[1, 2, 5], "s1", 0),
        occurrence(&[1, 2, 3, 4], "s2", 0),
        occurrence(&[3, 0, 1, 0], "backwards", 0),
        occurrence(&[-1, 0, 1], "negative", 0),
        occurrence(&[1, 2], "short", 0),
        occurrence(&[], "empty", 0),
        occurrence(&[1, 2, 5], "", 0),
    ]);
    let found = &index.documents[0].occurrences;
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].symbol, "s1");
    assert_eq!(found[0].range, range(1, 2, 1, 5));
    assert_eq!(found[1].range, range(1, 2, 3, 4));
    assert_eq!(stats.malformed_occurrences, 5);
    assert!(stats.warnings("x.scip")[0].contains("5 occurrences"));
}

#[test]
fn typed_ranges_win_over_the_integer_list() {
    let mut first = occurrence(&[9, 9, 9], "s", DEFINITION);
    first.typed_range = Some(Typed_range::SingleLineRange(single(1, 2, 5)));
    let enclosing = single(1, 0, 9);
    first.typed_enclosing_range = Some(Typed_enclosing_range::SingleLineEnclosingRange(enclosing));
    let mut second = occurrence(&[9, 9, 9], "t", DEFINITION);
    second.typed_range = Some(Typed_range::MultiLineRange(multi(1, 2, 3, 4)));
    let enclosing = multi(0, 0, 4, 1);
    second.typed_enclosing_range = Some(Typed_enclosing_range::MultiLineEnclosingRange(enclosing));
    let mut third = occurrence(&[1, 0, 2], "u", DEFINITION);
    third.enclosing_range = vec![0, 0, 3];
    let mut fourth = occurrence(&[1, 0, 2], "v", DEFINITION);
    fourth.enclosing_range = vec![0, 0, 4, 1];
    let mut fifth = occurrence(&[1, 0, 2], "w", DEFINITION);
    fifth.enclosing_range = vec![4, 0];
    let (index, stats) = one_document(vec![first, second, third, fourth, fifth]);
    let found = &index.documents[0].occurrences;
    assert_eq!(stats.malformed_occurrences, 0);
    assert_eq!(found[0].range, range(1, 2, 1, 5));
    assert_eq!(found[0].enclosing, Some(range(1, 0, 1, 9)));
    assert_eq!(found[1].range, range(1, 2, 3, 4));
    assert_eq!(found[1].enclosing, Some(range(0, 0, 4, 1)));
    assert_eq!(found[2].enclosing, Some(range(0, 0, 0, 3)));
    assert_eq!(found[3].enclosing, Some(range(0, 0, 4, 1)));
    assert_eq!(found[4].enclosing, None);
}

#[test]
fn roles_are_a_bitset() {
    let (index, _) = one_document(vec![
        occurrence(&[0, 0, 1], "d", DEFINITION),
        occurrence(&[0, 0, 1], "r", 8),
        occurrence(&[0, 0, 1], "di", DEFINITION | IMPORT),
    ]);
    let found = &index.documents[0].occurrences;
    assert_eq!(IMPORT, 2);
    assert!(found[0].is_definition());
    assert!(!found[1].is_definition());
    assert!(found[2].is_definition());
    assert_eq!(found[2].roles, 3);
}

#[test]
fn encodings_are_read_and_unspecified_ones_warn() {
    let mut unknown = document("e.py", Wire::UTF8CodeUnitOffsetFromLineStart, vec![]);
    unknown.position_encoding = EnumOrUnknown::from_i32(99);
    let (index, stats) = read(vec![
        document("a.py", Wire::UTF8CodeUnitOffsetFromLineStart, vec![]),
        document("b.py", Wire::UTF16CodeUnitOffsetFromLineStart, vec![]),
        document("c.py", Wire::UTF32CodeUnitOffsetFromLineStart, vec![]),
        document("d.py", Wire::UnspecifiedPositionEncoding, vec![]),
        unknown,
    ]);
    let encodings: Vec<_> = index.documents.iter().map(|d| d.encoding).collect();
    assert_eq!(
        encodings,
        [
            PositionEncoding::Utf8,
            PositionEncoding::Utf16,
            PositionEncoding::Utf32,
            PositionEncoding::Utf8,
            PositionEncoding::Utf8,
        ]
    );
    assert_eq!(stats.unspecified_encoding, 2);
    let warnings = stats.warnings("x.scip");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("2 documents state no position encoding"));
}

#[test]
fn unsafe_document_paths_are_dropped_and_counted() {
    let unsafe_paths = [
        "../x.py",
        "/abs.py",
        "a\\b.py",
        "a/../b.py",
        "",
        "a//b.py",
        "./a.py",
        "a/.",
        "a\0b.py",
    ];
    let mut documents: Vec<Document> = unsafe_paths
        .iter()
        .map(|path| document(path, Wire::UTF8CodeUnitOffsetFromLineStart, vec![]))
        .collect();
    documents.push(document(
        "pkg/mod.py",
        Wire::UTF8CodeUnitOffsetFromLineStart,
        vec![],
    ));
    documents.push(document(
        "dir with space/x.py",
        Wire::UTF8CodeUnitOffsetFromLineStart,
        vec![],
    ));
    let (index, stats) = read(documents);
    let kept: Vec<_> = index.documents.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(kept, ["pkg/mod.py", "dir with space/x.py"]);
    assert_eq!(stats.rejected_documents, unsafe_paths.len());
    assert!(stats.foreign_root.is_none());
    assert!(stats.warnings("x.scip")[0].contains("unsafe path"));
}

#[test]
fn the_index_root_decides_the_path_prefix() {
    let cases = [
        ("", ""),
        ("file:///repo", ""),
        ("file:///repo/", ""),
        ("file:///repo/sub%20dir", "sub dir/"),
        ("file:///repo/pkg/api", "pkg/api/"),
    ];
    for (project_root, prefix) in cases {
        let bytes = index_with(
            project_root,
            None,
            vec![document(
                "m.py",
                Wire::UTF8CodeUnitOffsetFromLineStart,
                vec![],
            )],
        );
        let (index, stats) = read_index(&bytes, ROOT).expect("reads");
        assert_eq!(
            index.documents[0].path,
            format!("{prefix}m.py"),
            "{project_root}"
        );
        assert_eq!(stats, ReadStats::default(), "{project_root}");
    }
}

#[test]
fn a_root_outside_the_analysis_root_yields_no_documents() {
    let foreign = [
        "file:///other",
        "file:///repo2",
        "file:///",
        "file:///repo/../etc",
        "file:///repo/%2e%2e/etc",
        "file:///repo/a%5Cb",
        "file://host/repo",
        "https://example.com/repo",
        "file:///repo/bad%zz",
        "file:///repo/trunc%4",
        "file:///repo/%ff",
    ];
    for project_root in foreign {
        let bytes = index_with(
            project_root,
            Some(("scip-python", "0.6")),
            vec![
                document("a.py", Wire::UTF8CodeUnitOffsetFromLineStart, vec![]),
                document("b.py", Wire::UTF8CodeUnitOffsetFromLineStart, vec![]),
            ],
        );
        let (index, stats) = read_index(&bytes, ROOT).expect("reads");
        assert!(index.documents.is_empty(), "{project_root}");
        assert_eq!(index.tool.as_deref(), Some("scip-python 0.6"));
        assert_eq!(stats.rejected_documents, 2, "{project_root}");
        assert_eq!(stats.foreign_root.as_deref(), Some(project_root));
        let warnings = stats.warnings("x.scip");
        assert!(
            warnings[0].contains("outside the analysis root"),
            "{warnings:?}"
        );
    }
    // An index made at a parent of the analysis root is foreign too.
    let bytes = index_with(
        "file:///repo",
        None,
        vec![document(
            "pkg/a.py",
            Wire::UTF8CodeUnitOffsetFromLineStart,
            vec![],
        )],
    );
    let (index, stats) = read_index(&bytes, "/repo/pkg").expect("reads");
    assert_eq!(index.documents, Vec::new());
    assert!(stats.foreign_root.is_some());
}

#[test]
fn the_tool_is_its_name_and_version() {
    let documents = || {
        vec![document(
            "a.py",
            Wire::UTF8CodeUnitOffsetFromLineStart,
            vec![],
        )]
    };
    let cases = [
        (Some(("scip-python", "0.6")), Some("scip-python 0.6")),
        (Some(("rust-analyzer", "")), Some("rust-analyzer")),
        (Some(("", "1.0")), None),
        (None, None),
    ];
    for (tool, expected) in cases {
        let (index, _) = read_index(&index_with("", tool, documents()), ROOT).expect("reads");
        assert_eq!(index.tool.as_deref(), expected);
    }
}

#[test]
fn malformed_input_is_an_error_and_never_a_panic() {
    let valid = index_with(
        "file:///repo",
        Some(("scip-python", "0.6")),
        vec![document(
            "a.py",
            Wire::UTF16CodeUnitOffsetFromLineStart,
            vec![occurrence(
                &[1, 2, 5],
                "scip-python python pkg 1.0 `a`/f().",
                DEFINITION,
            )],
        )],
    );
    let truncated = read_index(&valid[..valid.len() - 1], ROOT);
    let error = truncated.expect_err("a truncated index is an error");
    assert!(matches!(error, ScipError::Parse(_)));
    assert!(error.to_string().starts_with("not a SCIP index: "));
    assert!(matches!(
        read_index(&[0xff; 8], ROOT),
        Err(ScipError::Parse(_))
    ));
    for end in 0..valid.len() {
        let _ = read_index(&valid[..end], ROOT);
    }
    let (empty, stats) = read_index(&[], ROOT).expect("an empty message is an empty index");
    assert_eq!(empty, ScipIndex::default());
    assert_eq!(stats, ReadStats::default());
    assert_eq!(stats.warnings("x.scip"), Vec::<String>::new());
}

#[test]
fn the_size_cap_is_a_constant() {
    assert!(check_size(MAX_INDEX_BYTES).is_ok());
    let error = check_size(MAX_INDEX_BYTES + 1).expect_err("one byte over");
    assert_eq!(
        error,
        ScipError::TooLarge {
            bytes: MAX_INDEX_BYTES + 1
        }
    );
    assert!(error.to_string().contains("more than the limit"));
}

#[test]
fn load_reads_a_file_and_refuses_what_it_cannot() {
    let directory = scratch("load");
    let bytes = index_with(
        "",
        None,
        vec![document(
            "a.py",
            Wire::UTF8CodeUnitOffsetFromLineStart,
            vec![],
        )],
    );
    let file = directory.join("index.scip");
    std::fs::write(&file, &bytes).expect("writes");
    let (index, _) = load(&file, ROOT).expect("loads");
    assert_eq!(index.documents[0].path, "a.py");

    let missing = load(&directory.join("none.scip"), ROOT).expect_err("missing");
    assert!(matches!(missing, ScipError::Io(_)));
    assert!(missing.to_string().starts_with("cannot read the index: "));

    // A sparse file one byte over the cap is refused without being read.
    let big = directory.join("big.scip");
    let handle = std::fs::File::create(&big).expect("creates");
    handle
        .set_len(MAX_INDEX_BYTES + 1)
        .expect("sets the length");
    let refused = load(&big, ROOT).expect_err("too large");
    assert_eq!(
        refused,
        ScipError::TooLarge {
            bytes: MAX_INDEX_BYTES + 1
        }
    );
    std::fs::remove_dir_all(&directory).expect("cleans up");
}

/// One code point `a`, an emoji (4 UTF-8 bytes, 2 UTF-16 units), `e` with a
/// combining acute accent (2 code points, 3 UTF-8 bytes, 2 UTF-16 units), then
/// `foo()`: `foo` starts at code-point column 5 and ends at 8.
const LINE: &str = "a\u{1f600}e\u{301}foo()";

#[test]
fn offsets_become_code_points_in_every_encoding() {
    let lines = LineIndex::new(LINE);
    let expected = Span::new(1, 5, 1, 8);
    assert!(expected.is_some());
    let cases = [
        (PositionEncoding::Utf8, 8, 11),
        (PositionEncoding::Utf16, 5, 8),
        (PositionEncoding::Utf32, 4, 7),
    ];
    for (encoding, start, end) in cases {
        assert_eq!(
            lines.span(range(0, start, 0, end), encoding),
            expected,
            "{encoding:?}"
        );
    }
}

#[test]
fn offsets_inside_a_character_or_past_the_line_are_not_positions() {
    let lines = LineIndex::new(LINE);
    let cases = [
        (PositionEncoding::Utf8, range(0, 2, 0, 8)),
        (PositionEncoding::Utf8, range(0, 7, 0, 11)),
        (PositionEncoding::Utf16, range(0, 2, 0, 8)),
        (PositionEncoding::Utf32, range(0, 4, 0, 99)),
        (PositionEncoding::Utf32, range(5, 0, 5, 1)),
        (PositionEncoding::Utf32, range(0, 0, 5, 1)),
    ];
    for (encoding, raw) in cases {
        assert_eq!(lines.span(raw, encoding), None, "{encoding:?} {raw:?}");
    }
}

#[test]
fn spans_cover_empty_ranges_several_lines_and_crlf() {
    let utf32 = PositionEncoding::Utf32;
    assert_eq!(
        LineIndex::new("").span(range(0, 0, 0, 0), utf32),
        Span::new(1, 1, 1, 1)
    );
    assert_eq!(
        LineIndex::new("ab\ncd").span(range(0, 1, 1, 1), utf32),
        Span::new(1, 2, 2, 2)
    );
    let crlf = LineIndex::new("ab\r\ncd");
    assert_eq!(crlf.span(range(0, 0, 0, 2), utf32), Span::new(1, 1, 1, 3));
    assert_eq!(crlf.span(range(0, 0, 0, 3), utf32), None);
    assert_eq!(crlf.span(range(1, 0, 1, 2), utf32), Span::new(2, 1, 2, 3));
}
