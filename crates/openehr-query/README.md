# openehr-query

**A hand-written openEHR AQL 1.1 front end for Rust**: lexer, parser, and
typed AST for the Archetype Query Language — native `logos` + `chumsky`, no
ANTLR runtime.

## What it provides

- A complete AQL 1.1 lexer and parser producing a typed AST (`SELECT`,
  `FROM`/`CONTAINS`, `WHERE`, `ORDER BY`, `LIMIT`, path expressions with
  predicates, parameters, functions), corpus-validated against the official
  grammar's example set.
- Typed parse errors: `ParseError` separates a lexing refusal (carrying the
  lexer's error as a real `source()`) from a grammar refusal (carrying every
  position the parser reported), so a caller branches on the failure instead
  of reading its message. Each grammar fault gives the position twice — as
  token-stream indices and as a byte range of the source, so a diagnostic can
  underline the offending text. `Display` is the located diagnostic to show a
  human; it names the token indices.
- `lexer::lex_spanned` — the token stream with each token's source byte span,
  additive beside `lexer::lex`. `parser::parse_spanned` consumes it and is
  what `parser::parse_str` runs; `parser::parse` over a bare `&[Token]` still
  works and reports token indices only.
- `printer::to_aql` — canonical AQL rendering of the AST, the parser's
  inverse, for programmatic query construction (corpus-verified fixed point:
  `parse(to_aql(ast)) == ast`).
- `visit::Visit` and `visit::VisitMut` — a read-only and a mutating
  traversal, exhaustive over every AST node, for finding, replacing or
  removing conditions without writing a recursive walk.
- Source spans: every `IdentifiedPath` and every `WHERE` condition carries an
  `ast::Span` (a byte range of the source), so a diagnostic can name a
  position without echoing the text there. Spans take no part in equality.
- `bind::bind` — binds ITS-REST `query_parameters` into the AST as typed
  literals the printer escapes, at every position the grammar admits a
  parameter; an unbound, unknown, duplicate or unbindable parameter is
  reported by name, never by value.
- The `federation` feature: `federation::parse_federated` lifts the
  federation-only `FROM ENDPOINT p [ … ]` / `ORGANISATION [ … ]` directive
  out of a query and parses the rest as strict AQL, which `to_aql` renders
  without it; `federation::to_federated_aql` renders it back.
- The AST is engine-agnostic: this crate deliberately stops at the syntax
  layer, so any execution engine (SQL generation, in-memory evaluation) can
  build on it.

## Versioning

The package version is the crate's **own independent SemVer line** — it
tracks this implementation's code and moves freely with fixes and
improvements, never with the vendored openEHR specification. The implemented
spec version is always available at runtime as the crate-level constant
`openehr_query::SPEC_VERSION` (`"1.1.0"` — QUERY/AQL), independent of the
package version.

## Minimum supported Rust version

Rust 1.97 (edition 2024).

## License

Business Source License 1.1 ([`LICENSE`](LICENSE), SPDX `BUSL-1.1`), the
licence of the FerroEHR application: all non-production use is free, production
use is free for Non-Commercial Purposes, and any other production use, hosting
for third parties or distribution for a fee needs a commercial licence from the
Licensor, Cadasto B.V. Each version becomes Apache License 2.0 four
years after it is published. The crate is hand-written; the openEHR
specification text it implements is the authority but is not embedded in the
package. Versions up to 0.0.59 were published under Apache-2.0 and keep it.

## Part of FerroEHR

This crate is the query-language layer of [FerroEHR](https://github.com/FerroHEALTH/FerroEHR), a pure-Rust,
openEHR-spec-conformant Clinical Data Repository (ITS-REST 1.1.0 + AQL 1.1 on
PostgreSQL 18). The crates are usable standalone; FerroEHR is the reference
consumer.
