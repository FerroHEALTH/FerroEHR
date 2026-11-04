// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The delimited free-text tokens lex in bounded stack whatever their length.
//!
//! Each case lexes one token of at least 1 MiB on a thread with a 256 KiB
//! stack. In a debug build the `logos` tail-call rendering of the DFA pushed
//! one frame per state transition its fast self-loop did not absorb (each
//! multi-byte character, each escape), so every one of these tokens
//! overflowed that stack before their text was scanned in a loop (#4961).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and thread plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_lang::v1_1::lexer::{Language, Spanned, Token, lex};

/// The smallest token any case builds.
const MIB: usize = 1 << 20;

/// The stack each case lexes on, a quarter of the token length.
const STACK: usize = 256 * 1024;

/// `unit` repeated until the text reaches at least 1 MiB.
fn repeated(unit: &str) -> String {
    unit.repeat(MIB.div_ceil(unit.len()))
}

/// Lex `src` under `language` on a [`STACK`]-byte thread.
fn lex_on_small_stack(language: Language, src: String) -> Vec<Spanned> {
    std::thread::Builder::new()
        .stack_size(STACK)
        .spawn(move || lex(language, &src).expect("the long token should lex"))
        .expect("the lexing thread should spawn")
        .join()
        .expect("lexing should not overflow a 256 KiB stack")
}

/// Assert `src` lexes as exactly one token spanning all of it.
fn assert_one_token(language: Language, src: String, matches: fn(&Token) -> bool) {
    let length = src.len();
    assert!(length >= MIB, "the token should be at least 1 MiB");
    let tokens = lex_on_small_stack(language, src);
    let [only] = tokens.as_slice() else {
        panic!("expected one token, got {}", tokens.len());
    };
    assert!(matches(&only.token), "unexpected token class");
    assert_eq!(
        only.span,
        0..length,
        "the token should span the whole input"
    );
}

#[test]
fn a_long_string_of_multi_byte_text_and_escapes_lexes() {
    let src = format!("\"{}\"", repeated("é∈ \\n\\\" ab"));
    assert_one_token(Language::Adl, src, |token| {
        matches!(token, Token::String(_))
    });
}

#[test]
fn a_long_comment_of_multi_byte_text_lexes_to_nothing() {
    let src = format!("-- {}\n", repeated("é∈ ab"));
    assert_eq!(
        lex_on_small_stack(Language::Adl, src),
        Vec::new(),
        "a comment should lex to no token"
    );
}

#[test]
fn a_long_contained_regexp_lexes() {
    let src = format!("{{/{}/}}", repeated("é\\/[a-z]+"));
    assert_one_token(Language::Adl, src, |token| {
        matches!(token, Token::ContainedRegexp(_))
    });
}

#[test]
fn a_long_plug_in_block_lexes() {
    let src = format!("<#{}#>", repeated("é# >"));
    assert_one_token(Language::Odin, src, |token| {
        matches!(token, Token::PlugInBlock(_))
    });
}

#[test]
fn a_long_embedded_uri_lexes() {
    let src = format!("<https://example.org/{}>", repeated("é/ab?c=d\n"));
    assert_one_token(Language::Odin, src, |token| {
        matches!(token, Token::EmbeddedUri(_))
    });
}
