// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AQL lexer and parser: any authenticated caller can post arbitrary query
//! text to the query surface, so this is the widest attacker-controlled grammar
//! in the system. `parse_str` is the entry the service layer calls.
//!
//! Once a query parses, the harness also asserts the invariant
//! `openehr_query::printer` documents for itself — `parse(to_aql(ast)) == ast`
//! — over arbitrary input rather than only over the vendored worked-example
//! corpus that verifies it today.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(query) = openehr_query::parser::parse_str(text) else {
        return;
    };
    let printed = openehr_query::printer::to_aql(&query);
    match openehr_query::parser::parse_str(&printed) {
        Ok(reparsed) => assert!(
            reparsed == query,
            "printer round-trip drifted the AST via: {printed}"
        ),
        Err(error) => panic!("printed AQL failed to reparse: {printed}\n  {error}"),
    }
});
