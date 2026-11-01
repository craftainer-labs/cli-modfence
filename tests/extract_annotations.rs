//! The `modfence:` annotation parser (ADR 0004): table tests for every grammar
//! rule, and generated inputs for the round trip and the no-panic guarantee.

use std::collections::BTreeSet;

use modfence::extract::{
    AnnotationError, ErrorKind, PREFIX, Parsed, RESERVED_KEYS, Suppression, Tags, comment_body,
    merge_tags, parse_comment,
};

/// A small deterministic generator (xorshift64*), so failures reproduce.
struct Rng(u64);

impl Rng {
    fn step(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        let n = u64::try_from(n).unwrap();
        usize::try_from(self.step() % n).unwrap()
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    fn blanks(&mut self, min: usize) -> String {
        let count = min + self.below(3);
        (0..count)
            .map(|_| if self.below(2) == 0 { ' ' } else { '\t' })
            .collect()
    }
}

fn tags(pairs: &[(&str, &[&str])]) -> Tags {
    pairs
        .iter()
        .map(|(key, values)| {
            let set: BTreeSet<String> = values.iter().map(|v| (*v).to_owned()).collect();
            ((*key).to_owned(), set)
        })
        .collect()
}

fn parse(body: &str) -> Result<Parsed, AnnotationError> {
    parse_comment(body).expect("the body is an annotation")
}

fn ignore(rules: &[&str], reason: Option<&str>) -> Parsed {
    Parsed::Ignore(Suppression {
        rules: rules.iter().map(|r| (*r).to_owned()).collect(),
        reason: reason.map(str::to_owned),
    })
}

#[test]
fn other_comments_are_not_annotations() {
    for body in [
        "",
        "   ",
        "hello",
        "modfence",
        "modfence :",
        "Modfence: a=1",
        "x modfence: a=1",
        "/ modfence: a=1",
        "!/usr/bin/env python",
        " -*- coding: utf-8 -*-",
        "arch: layer=domain",
    ] {
        assert!(parse_comment(body).is_none(), "{body:?}");
    }
}

#[test]
fn parses_key_value_pairs() {
    let expected = Ok(Parsed::Tags(tags(&[
        ("layer", &["domain"]),
        ("owner", &["team-a"]),
    ])));
    for body in [
        " modfence: layer=domain owner=team-a",
        "modfence:layer=domain owner=team-a",
        "\t modfence: \t layer=domain\t\towner=team-a \t",
        "modfence: layer=domain owner=team-a\r",
    ] {
        assert_eq!(parse(body), expected, "{body:?}");
    }
}

#[test]
fn a_bare_key_means_true() {
    assert_eq!(
        parse("modfence: deprecated"),
        Ok(Parsed::Tags(tags(&[("deprecated", &["true"])])))
    );
    assert_eq!(
        parse("modfence: a=true a"),
        Ok(Parsed::Tags(tags(&[("a", &["true"])])))
    );
}

#[test]
fn repeated_keys_accumulate_sorted_and_unique() {
    assert_eq!(
        parse("modfence: adr=0029 owner=x adr=0028 adr=0029"),
        Ok(Parsed::Tags(tags(&[
            ("adr", &["0028", "0029"]),
            ("owner", &["x"]),
        ])))
    );
}

#[test]
fn quoted_values_may_hold_spaces_commas_and_equals() {
    assert_eq!(
        parse(r#"modfence: owner="team a,team b" x="a=b" y=c=d z="é ü""#),
        Ok(Parsed::Tags(tags(&[
            ("owner", &["team a,team b"]),
            ("x", &["a=b"]),
            ("y", &["c=d"]),
            ("z", &["é ü"]),
        ])))
    );
}

#[test]
fn a_comment_after_ignore_open_is_only_a_suppression() {
    assert_eq!(
        parse("modfence: ignore=true"),
        Ok(Parsed::Tags(tags(&[("ignore", &["true"])])))
    );
    assert_eq!(
        parse("modfence: ignore[layers]"),
        Ok(ignore(&["layers"], None))
    );
}

#[test]
fn parses_suppressions() {
    let body = r#" modfence:ignore[ layers , modfence/unused-ignore ,layers]  reason="old, #12" "#;
    assert_eq!(
        parse(body),
        Ok(ignore(
            &["layers", "modfence/unused-ignore"],
            Some("old, #12")
        ))
    );
    assert_eq!(
        parse(r#"modfence: ignore[a.b_c-d/e] reason="r""#),
        Ok(ignore(&["a.b_c-d/e"], Some("r")))
    );
    assert_eq!(parse("modfence: ignore[a]   \t"), Ok(ignore(&["a"], None)));
}

#[test]
fn rejects_malformed_annotations_with_the_position() {
    let cases = [
        ("modfence:", ErrorKind::EmptyAnnotation, 9),
        ("  modfence:   ", ErrorKind::EmptyAnnotation, 11),
        ("modfence: Layer=x", ErrorKind::InvalidKey, 10),
        ("modfence: =x", ErrorKind::InvalidKey, 10),
        ("modfence: a=1 1b", ErrorKind::InvalidKey, 14),
        ("modfence: a=é B=2", ErrorKind::InvalidKey, 14),
        ("modfence: ignore [a]", ErrorKind::InvalidKey, 17),
        ("modfence: path=x", ErrorKind::ReservedKey, 10),
        ("modfence: ok lang", ErrorKind::ReservedKey, 13),
        ("modfence: kind=x", ErrorKind::ReservedKey, 10),
        ("modfence: package", ErrorKind::ReservedKey, 10),
        ("modfence: a=", ErrorKind::EmptyValue, 12),
        ("modfence: a= b=1", ErrorKind::EmptyValue, 12),
        ("modfence: a=\"\"", ErrorKind::EmptyValue, 12),
        ("modfence: a=\"x", ErrorKind::UnterminatedQuote, 12),
        ("modfence: a=\"x\"y", ErrorKind::JunkAfterQuote, 15),
        ("modfence: a=x\"y", ErrorKind::QuoteInValue, 12),
        ("modfence: a=x\u{1}y", ErrorKind::ControlCharacter, 12),
        ("modfence: a=\"x\ty\"", ErrorKind::ControlCharacter, 12),
        ("modfence: ignore[", ErrorKind::UnterminatedIgnore, 17),
        ("modfence: ignore[a", ErrorKind::UnterminatedIgnore, 17),
        ("modfence: ignore[]", ErrorKind::EmptyRuleId, 17),
        ("modfence: ignore[a,,b]", ErrorKind::EmptyRuleId, 19),
        ("modfence: ignore[a, ]", ErrorKind::EmptyRuleId, 19),
        ("modfence: ignore[a b]", ErrorKind::InvalidRuleId, 17),
        ("modfence: ignore[ok,bä]", ErrorKind::InvalidRuleId, 20),
        ("modfence: ignore[ a, b c ]", ErrorKind::InvalidRuleId, 21),
        (
            "modfence: ignore[a]reason=\"x\"",
            ErrorKind::UnexpectedAfterIgnore,
            19,
        ),
        (
            "modfence: ignore[a] reason=\"  \"",
            ErrorKind::EmptyValue,
            20,
        ),
        (
            "modfence: ignore[a] reason=x",
            ErrorKind::UnexpectedAfterIgnore,
            20,
        ),
        (
            "modfence: ignore[a] x",
            ErrorKind::UnexpectedAfterIgnore,
            20,
        ),
        (
            "modfence: ignore[a] reason=\"x\" more",
            ErrorKind::UnexpectedAfterIgnore,
            31,
        ),
        (
            "modfence: ignore[a] reason=\"x",
            ErrorKind::UnterminatedQuote,
            27,
        ),
        ("modfence: ignore[a] reason=\"\"", ErrorKind::EmptyValue, 20),
        (
            "modfence: ignore[a] reason=\"x\u{7}\"",
            ErrorKind::ControlCharacter,
            20,
        ),
    ];
    for (body, kind, offset) in cases {
        assert_eq!(
            parse(body),
            Err(AnnotationError { kind, offset }),
            "{body:?}"
        );
    }
}

#[test]
fn reserved_keys_are_the_selector_keys() {
    assert_eq!(RESERVED_KEYS, ["path", "package", "lang", "kind"]);
    for key in RESERVED_KEYS {
        let body = format!("{PREFIX} {key}=x");
        assert_eq!(
            parse(&body),
            Err(AnnotationError {
                kind: ErrorKind::ReservedKey,
                offset: 10
            })
        );
    }
}

#[test]
fn errors_explain_themselves() {
    let kinds = [
        ErrorKind::EmptyAnnotation,
        ErrorKind::InvalidKey,
        ErrorKind::ReservedKey,
        ErrorKind::EmptyValue,
        ErrorKind::QuoteInValue,
        ErrorKind::ControlCharacter,
        ErrorKind::UnterminatedQuote,
        ErrorKind::JunkAfterQuote,
        ErrorKind::UnterminatedIgnore,
        ErrorKind::EmptyRuleId,
        ErrorKind::InvalidRuleId,
        ErrorKind::UnexpectedAfterIgnore,
    ];
    let messages: BTreeSet<&str> = kinds.iter().map(|kind| kind.message()).collect();
    assert_eq!(messages.len(), kinds.len());
    assert!(messages.iter().all(|message| !message.is_empty()));
    let error = AnnotationError {
        kind: ErrorKind::ReservedKey,
        offset: 7,
    };
    let text = error.to_string();
    assert!(text.contains("character 7"), "{text}");
    assert!(text.contains(ErrorKind::ReservedKey.message()), "{text}");
    let boxed: Box<dyn std::error::Error> = Box::new(error);
    assert_eq!(boxed.to_string(), text);
}

#[test]
fn comment_markers_are_stripped() {
    assert_eq!(comment_body("# modfence: a"), Some(" modfence: a"));
    assert_eq!(comment_body("// modfence: a"), Some(" modfence: a"));
    assert_eq!(comment_body("/* modfence: a */"), Some(" modfence: a "));
    assert_eq!(comment_body("/**/"), Some(""));
    assert_eq!(comment_body("/*a\nb*/"), None);
    assert_eq!(comment_body("/*a\rb*/"), None);
    assert_eq!(comment_body("/*/"), None);
    assert_eq!(comment_body("/* open"), None);
    assert_eq!(comment_body("x = 1"), None);
    assert_eq!(comment_body(""), None);
}

#[test]
fn doc_comments_and_shebangs_are_never_annotations() {
    for raw in [
        "/// modfence: a=1",
        "//! modfence: a=1",
        "#!/usr/bin/env python",
        "/** modfence: a=1 */",
    ] {
        let body = comment_body(raw).expect("a comment");
        assert!(parse_comment(body).is_none(), "{raw:?}");
    }
    let body = comment_body("/* modfence: layer=x */").expect("a comment");
    assert!(parse_comment(body).is_some());
}

#[test]
fn merging_unions_the_values_per_key() {
    let mut into = tags(&[("adr", &["0028"]), ("layer", &["domain"])]);
    merge_tags(
        &mut into,
        &tags(&[("adr", &["0029", "0028"]), ("owner", &["x"])]),
    );
    assert_eq!(
        into,
        tags(&[
            ("adr", &["0028", "0029"]),
            ("layer", &["domain"]),
            ("owner", &["x"])
        ])
    );
}

#[test]
fn generated_tag_comments_round_trip() {
    const KEYS: [&str; 7] = ["layer", "owner", "adr", "req", "deprecated", "a1", "x-y_z"];
    const VALUES: [&str; 8] = [
        "domain",
        "0028",
        "team-a",
        "a,b",
        "x=y",
        "team a",
        "two  spaces",
        "é",
    ];
    let mut rng = Rng(0x1234_5678_9ABC_DEF1);
    for _ in 0..500 {
        let mut body = rng.blanks(0);
        body.push_str(PREFIX);
        body.push_str(&rng.blanks(0));
        let mut expected = Tags::new();
        for index in 0..=rng.below(6) {
            if index > 0 {
                body.push_str(&rng.blanks(1));
            }
            let key = *rng.pick(&KEYS);
            body.push_str(key);
            let value = if rng.below(5) == 0 {
                "true"
            } else {
                let value = *rng.pick(&VALUES);
                body.push('=');
                if value.contains(' ') || rng.below(2) == 0 {
                    body.push('"');
                    body.push_str(value);
                    body.push('"');
                } else {
                    body.push_str(value);
                }
                value
            };
            expected
                .entry(key.to_owned())
                .or_default()
                .insert(value.to_owned());
        }
        body.push_str(&rng.blanks(0));
        assert_eq!(parse(&body), Ok(Parsed::Tags(expected)), "{body:?}");
    }
}

fn is_valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn check_invariants(body: &str, result: &Result<Parsed, AnnotationError>) {
    match result {
        Ok(Parsed::Tags(tags)) => {
            assert!(!tags.is_empty(), "{body:?}");
            for (key, values) in tags {
                assert!(is_valid_key(key), "{body:?}");
                assert!(!RESERVED_KEYS.contains(&key.as_str()), "{body:?}");
                assert!(!values.is_empty(), "{body:?}");
                for value in values {
                    assert!(!value.is_empty(), "{body:?}");
                    assert!(!value.contains('"'), "{body:?}");
                    assert!(!value.chars().any(char::is_control), "{body:?}");
                }
            }
        }
        Ok(Parsed::Ignore(suppression)) => {
            assert!(!suppression.rules.is_empty(), "{body:?}");
            let unique: BTreeSet<&String> = suppression.rules.iter().collect();
            assert_eq!(unique.len(), suppression.rules.len(), "{body:?}");
            for rule in &suppression.rules {
                assert!(!rule.is_empty(), "{body:?}");
                assert!(
                    rule.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_./-".contains(c)),
                    "{body:?}"
                );
            }
            if let Some(reason) = &suppression.reason {
                assert!(!reason.is_empty(), "{body:?}");
                assert!(!reason.contains('"'), "{body:?}");
                assert!(!reason.chars().any(char::is_control), "{body:?}");
            }
        }
        Err(error) => assert!(error.offset <= body.chars().count(), "{body:?}"),
    }
}

#[test]
fn arbitrary_text_never_panics_and_results_are_well_formed() {
    let alphabet: Vec<char> = "modfence:ignore[]=,\"' \t\n\u{1}a1-_/.#é€🦀rsn"
        .chars()
        .collect();
    let starts = [
        "",
        "modfence:",
        "modfence: ",
        "modfence: ignore[",
        "modfence: a=",
    ];
    let mut rng = Rng(0xDEAD_BEEF_0BAD_F00D);
    for _ in 0..5000 {
        let mut body = (*rng.pick(&starts)).to_owned();
        for _ in 0..rng.below(30) {
            body.push(*rng.pick(&alphabet));
        }
        if let Some(result) = parse_comment(&body) {
            check_invariants(&body, &result);
        }
    }
}
