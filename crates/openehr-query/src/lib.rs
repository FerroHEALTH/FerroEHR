// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR QUERY (AQL 1.1.0): a hand-written lexer, parser, and AST over the
//! canonical ANTLR4 grammar vendored at `vendor/grammar/`, with no ANTLR
//! runtime dependency. AQL has no BMM meta-model, so this crate is written by
//! hand against the grammar and tested against `vendor/examples/`.
//!
//! The crate stops at the AST: [`lexer`] tokenizes `AqlLexer.g4` with `logos`,
//! [`ast`] carries one type per `AqlParser.g4` rule, and [`parser`] builds an
//! [`ast::SelectQuery`] with `chumsky`. [`visit`] walks and rewrites the tree,
//! [`bind`] substitutes ITS-REST `query_parameters` into it, and [`printer`]
//! renders it back to AQL text. Query execution lives in the application.
//!
//! The `federation` feature adds `federation`, which separates the
//! federation-only `FROM ENDPOINT` / `ORGANISATION` directive from a query
//! before the strict AQL 1.1.0 parse.

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]
pub mod ast;
pub mod bind;
#[cfg(feature = "federation")]
pub mod federation;
pub mod lexer;
pub mod parser;
pub mod printer;
pub mod visit;

/// The openEHR specification version this crate implements.
///
/// The pin is deliberately independent of the crates.io package version,
/// which is the crate's own `SemVer` line and moves only with this
/// implementation's code.
pub const SPEC_VERSION: &str = "1.1.0";
