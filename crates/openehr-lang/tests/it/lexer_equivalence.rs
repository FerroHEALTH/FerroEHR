// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The per-language lexical surface battery: the pinned oracle for
//! [`openehr_lang::v1_1::lexer`].
//!
//! One shared `logos` DFA serves ADL2/cADL, ODIN and BEL, and each language's
//! reading is produced by a reclassification pass over it. This battery pins
//! all three readings token-for-token, span-for-span and payload-for-payload
//! against `tests/fixtures/lexer_equivalence_{adl,odin,bel}.txt`, which were
//! **captured from the three separate hand-written lexers** the shared layer
//! replaced — so the fixtures are an external oracle, not a snapshot of the
//! current implementation.
//!
//! The inputs (`tests/fixtures/lexer_battery.txt`) cover every token class of
//! the union plus every point at which the three readings are known to
//! diverge: keyword reservation and case folding, the ADL-only code / HRID /
//! version / GUID / constraint-pattern classes, the ODIN-only local term code
//! and space-separated date-time, the three `ADL_PATH` shapes, BEL's narrower
//! `INTEGER`/`ISO8601_*` forms and its `in`/`<>` additions, the BOM, string
//! and character escapes, and the lexical-error positions.
//!
//! Editing a fixture line therefore changes an accepted (or refused) lexical
//! surface and needs an adjudicated, spec-cited reason — never a
//! "the output moved" update.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_lang::v1_1::lexer::{LexError, Spanned, lex_adl, lex_bel, lex_odin};

/// One per-language entry point of the shared lexer.
type Reading = fn(&str) -> Result<Vec<Spanned>, LexError>;

/// Render one lexing outcome in the fixture's canonical form.
fn render(outcome: Result<Vec<Spanned>, LexError>) -> String {
    match outcome {
        Ok(tokens) => tokens
            .iter()
            .map(|s| format!("{:?}@{}..{}", s.token, s.span.start, s.span.end))
            .collect::<Vec<_>>()
            .join(" "),
        Err(failure) => format!("ERROR@{}..{}", failure.span.start, failure.span.end),
    }
}

/// The battery inputs, in fixture-record order.
fn battery() -> Vec<String> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/lexer_battery.txt"
    );
    let text = std::fs::read_to_string(path).expect("the lexer battery fixture should be readable");
    text.split("\n%%\n").map(str::to_owned).collect()
}

/// The expected `index|dump` lines for one language.
fn expectations(language: &str) -> Vec<(usize, String)> {
    let path = format!(
        "{}/tests/fixtures/lexer_equivalence_{language}.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    let text = std::fs::read_to_string(&path).expect("the expectation fixture should be readable");
    text.lines()
        .map(|line| {
            let (index, dump) = line
                .split_once('|')
                .expect("every expectation line is `index|dump`");
            (
                index.parse::<usize>().expect("a decimal record index"),
                dump.to_owned(),
            )
        })
        .collect()
}

/// Every battery input, under every reading, reproduces the surface the
/// language's own lexer produced before the three were unified.
#[test]
fn every_reading_reproduces_its_pre_unification_surface() {
    let inputs = battery();
    let mut divergences = Vec::new();
    let readings: [(&str, Reading); 3] = [("adl", lex_adl), ("odin", lex_odin), ("bel", lex_bel)];
    let mut compared = 0usize;
    for (language, lex) in readings {
        for (index, expected) in expectations(language) {
            let input = inputs
                .get(index)
                .unwrap_or_else(|| panic!("expectation #{index} has no battery record"));
            let produced = render(lex(input));
            compared += 1;
            if produced != expected {
                divergences.push(format!(
                    "[{language} #{index}] {input:?}\n  expected {expected}\n  produced {produced}"
                ));
            }
        }
    }
    assert_eq!(
        compared,
        inputs.len() * 3,
        "every battery record must be pinned under all three readings"
    );
    assert!(
        divergences.is_empty(),
        "{} lexical-surface divergences:\n{}",
        divergences.len(),
        divergences.join("\n")
    );
}
