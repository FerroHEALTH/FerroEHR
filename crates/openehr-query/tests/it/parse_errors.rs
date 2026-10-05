// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The parser's error contract: a caller branches on the failure, and reads
//! the located diagnostic only to show it to a human.

use openehr_query::lexer::{LexError, lex, lex_spanned};
use openehr_query::parser::{ParseError, SyntaxFault, parse, parse_str};

/// A lex failure and a syntax failure are DIFFERENT variants, so telling them
/// apart never means matching a substring — the property the typed error
/// exists for.
#[test]
fn the_two_passes_are_distinguishable_without_reading_the_message() {
    // `#` is in no AQL token, so the lexer refuses before the parser runs.
    let lexed = parse_str("SELECT # FROM EHR e").expect_err("`#` lexes as nothing");
    assert!(
        matches!(lexed, ParseError::Lex(_)),
        "a character outside every token is a lex failure, got {lexed:?}"
    );

    // Every token here is valid AQL; the sequence is not.
    let syntax = parse_str("SELECT FROM WHERE").expect_err("keywords in that order are no query");
    assert!(
        matches!(syntax, ParseError::Syntax { .. }),
        "well-formed tokens in an invalid order are a syntax failure, got {syntax:?}"
    );
}

/// The lex variant carries the lexer's own error as its `source`, so the cause
/// chain is walkable rather than flattened into text (RFC 0201).
#[test]
fn a_lex_failure_carries_the_lexer_error_as_its_source() {
    let err = parse_str("SELECT # FROM EHR e").expect_err("`#` lexes as nothing");
    let source = std::error::Error::source(&err).expect("the lex variant carries its cause");
    let lex: &LexError = source
        .downcast_ref()
        .expect("and that cause is the lexer's own error type");
    assert_eq!(lex.slice, "#", "the offending slice survives the hop");

    // Display is unchanged by the wrapping: the message a client sees is still
    // the lexer's located diagnostic verbatim.
    assert_eq!(err.to_string(), lex.to_string());
}

/// A syntax failure carries the position and the token found, so a caller can
/// act on WHERE the query broke instead of parsing a sentence.
#[test]
fn a_syntax_failure_carries_its_position_and_the_token_found() {
    let err = parse_str("SELECT FROM WHERE").expect_err("keywords in that order are no query");
    let ParseError::Syntax { faults } = err else {
        panic!("expected a syntax failure, got {err:?}");
    };
    let SyntaxFault {
        tokens,
        bytes,
        found,
    } = faults.first().expect("at least one reported position");
    assert!(
        tokens.start < tokens.end,
        "the reported token range is non-empty: {tokens:?}"
    );
    assert!(
        bytes.is_some(),
        "parse_str lexes with spans, so the source position survives"
    );
    assert!(
        found.is_some(),
        "the input did not end early, so a token was found"
    );
}

/// An empty source ends before any token, which the parser reports as a
/// position with no token found rather than as a lex failure.
#[test]
fn end_of_input_is_reported_with_no_token_found() {
    let err = parse_str("").expect_err("an empty source is not a query");
    let ParseError::Syntax { faults } = err else {
        panic!("an empty source lexes cleanly and fails the grammar, got {err:?}");
    };
    assert!(
        faults.iter().any(|f| f.found.is_none()),
        "the end-of-input position is reported: {faults:?}"
    );
}

/// A query whose token indices and byte offsets cannot coincide: a string
/// literal and runs of whitespace push the offending token's byte offset far
/// past its index in the stream, so slicing the source with the reported range
/// proves the range is a real source position and not the token index.
const MISPLACED_KEYWORD: &str =
    "SELECT c FROM COMPOSITION c WHERE c/name/value = 'a string, with   spaces'   SELECT";

/// The reported byte range covers the offending text itself, so a caller can
/// underline it without re-lexing.
#[test]
fn the_reported_byte_range_covers_the_offending_text() {
    let err = parse_str(MISPLACED_KEYWORD).expect_err("a trailing SELECT ends no query");
    let ParseError::Syntax { faults } = err else {
        panic!("a well-lexing query in an invalid order is a syntax failure, got {err:?}");
    };
    let fault = faults.first().expect("at least one reported position");
    let bytes = fault
        .bytes
        .clone()
        .expect("parse_str lexes with spans, so the byte range is present");

    assert_eq!(
        MISPLACED_KEYWORD.get(bytes.clone()),
        Some("SELECT"),
        "the range underlines a keyword: {fault:?}"
    );
    assert_eq!(
        bytes.start,
        MISPLACED_KEYWORD
            .rfind("SELECT")
            .expect("the source ends with the misplaced keyword"),
        "and it is the TRAILING one, not the leading keyword at offset 0: {fault:?}"
    );
    assert_ne!(
        bytes.start, fault.tokens.start,
        "a byte offset behind a string literal and irregular whitespace cannot \
         equal the token index: {fault:?}"
    );
}

/// Every lexed span slices back to exactly the text its token came from, which
/// is the property the parser's byte ranges are derived from.
#[test]
fn every_lexed_span_slices_back_to_its_own_token() {
    let stream = lex_spanned(MISPLACED_KEYWORD).expect("the source lexes cleanly");
    assert_eq!(
        stream.tokens().len(),
        stream.spans().len(),
        "tokens and spans stay index-aligned"
    );
    for (index, span) in stream.spans().iter().enumerate() {
        let text = MISPLACED_KEYWORD
            .get(span.clone())
            .unwrap_or_else(|| panic!("span {span:?} is a character boundary of the source"));
        assert!(
            !text.is_empty(),
            "token {index} spans a non-empty slice of the source"
        );
        assert_eq!(
            text.trim(),
            text,
            "a span covers the token's own text, never the whitespace around it"
        );
    }

    // The whole string literal, quotes and inner spaces included, is one span.
    assert!(
        stream
            .spans()
            .iter()
            .any(|span| MISPLACED_KEYWORD.get(span.clone()) == Some("'a string, with   spaces'")),
        "the string literal is spanned as one token"
    );
}

/// The spanned entry point is additive: it lexes the same tokens as [`lex`],
/// and parsing bare tokens still works — reporting no source position, because
/// a bare token slice carries none.
#[test]
fn the_spanned_entry_point_is_additive() {
    let plain = lex(MISPLACED_KEYWORD).expect("the source lexes cleanly");
    let spanned = lex_spanned(MISPLACED_KEYWORD).expect("the source lexes cleanly");
    assert_eq!(plain, spanned.tokens(), "the token sequence is unchanged");

    let err = parse(&plain).expect_err("a trailing SELECT ends no query");
    let ParseError::Syntax { faults } = err else {
        panic!("expected a syntax failure, got {err:?}");
    };
    assert!(
        faults.iter().all(|f| f.bytes.is_none()),
        "bare tokens carry no source positions: {faults:?}"
    );
}
