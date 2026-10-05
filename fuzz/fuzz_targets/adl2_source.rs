// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL source parser (`openehr_adl::source`): archetype text arrives on the
//! definition surface as an upload, so the cADL/ODIN lexer and the outer
//! artefact parser both read attacker-controlled bytes.
//!
//! Both dialects run on every input: the same text is legal or illegal
//! differently under ADL 2 and under ADL 1.4, and each dialect takes its own
//! path through the outer parser.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let _ = openehr_adl::source::parse_source(text, openehr_adl::parse::Dialect::Adl2);
    let _ = openehr_adl::source::parse_source(text, openehr_adl::parse::Dialect::Adl14);
});
