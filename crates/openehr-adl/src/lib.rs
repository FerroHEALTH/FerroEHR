// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR ADL 2.4.0 — the hand-written ADL2 engine: ADL2/cADL/ODIN source
//! parser, the AOM2 validation catalogue, specialisation flattening, OPT2
//! generation, and ADL 1.4→2 conversion, built over the generated
//! `openehr_am::v2_4` object model.
//!
//! Spec oracle: `docs/specs/openehr/AM/docs/{ADL2,AOM2,OPT2}/` (+ the ODIN
//! spec at `docs/specs/openehr/LANG/docs/odin/`). The normative ANTLR4
//! grammars are vendored under `vendor/grammar/` (version-scoped by AM
//! generation: `v1_4/`, `v2_4/`) as reference input for the
//! hand-written `logos`/`chumsky` implementation — no ANTLR runtime.
//!
//! Components: the outer artefact parser ([`source`]), the shared typed
//! [`error`] catalogue, and the cADL definition-section parser ([`parse`]) that
//! builds the `openehr_am::v2_4::aom2` constraint model. The substrate under
//! them is [`aom`] (field accessors, constructors, interval arithmetic),
//! [`artefact`] (artefact views + the parent/supplier repository), [`hrid`] (the
//! archetype-id grammar) and [`odin`] (the ODIN reading bridge). Above them:
//! [`codes`]/[`paths`], [`validate`], [`flatten`] + [`opt`], the ADL2 serializer
//! [`mod@print`], and [`adl14`] (1.4→2 conversion — our own design/extension, no
//! openEHR spec).
//!
//! The crate has NO lexer of its own: the cADL token stream is the shared
//! workspace lexical layer under its ADL reading
//! ([`openehr_lang::v1_1::lexer::lex_adl`]), and ODIN sections go through the
//! `openehr_lang::v1_1::odin` reader. The `rules` body and the slot
//! include/exclude assertions are BEL expression trees ([`rules`]) whose string
//! form is rendered back from the tree ([`print::assertion_text`]), never parsed
//! out of it.

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]
pub mod adl14;
pub mod aom;
pub mod artefact;
pub mod assemble;
pub mod codes;
pub mod error;
pub mod flatten;
pub mod hrid;
pub mod meta;
pub mod odin;
pub mod opt;
pub mod parse;
pub mod paths;
pub mod print;
pub mod rules;
pub mod source;
pub mod validate;

/// The openEHR specification version this crate implements.
///
/// The pin is deliberately independent of the crates.io package version,
/// which is the crate's own `SemVer` line and moves only with this
/// implementation's code.
pub const SPEC_VERSION: &str = "2.4.0";
