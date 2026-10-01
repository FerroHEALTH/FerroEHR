# `openehr-query` — AQL 1.1 lexer/parser/AST (hand-written)

The AQL front end: `logos` lexer + `chumsky` parser + the typed AST.
Grammar-driven (the official `AqlLexer/AqlParser .g4` grammars +
`docs/specs/openehr/QUERY/docs/AQL/`), not BMM — this crate is
hand-written and corpus-validated.

- **The AQL spec text is the grammar authority** (`/spec-lookup` → QUERY).
  Accept exactly what the grammar accepts; parser-level divergences are
  spec-citable decisions, never silent. Where the spec is wider than the
  engine implements, the *parser* still parses — rejection happens as a
  typed error at semantic analysis/engine level (in `app/ferroehr`), so the
  reject message can cite the construct.
- This crate stops at the AST: no SQL, no storage, no RM knowledge beyond
  what path syntax needs. The engine (IR → sea-query SQL) lives in
  `app/ferroehr`; keep the boundary clean.
- The parser corpus tests are the regression net — extend them with every
  grammar-touching change; never delete a corpus case to get green.
- Spec pin: QUERY 1.1.0 via `SPEC_VERSION`; the package version is the
  crate's own SemVer line (`.claude/rules/crates-publishing.md`).
- Gates: `cargo clippy -p openehr-query --all-targets` +
  `cargo nextest run -p openehr-query`.
- Modules beyond the front end: `visit` (`Visit`/`VisitMut`, exhaustive
  walk functions that destructure every node — a new AST field or variant
  must be added there or the crate stops compiling), `bind` (ITS-REST
  `query_parameters` → typed literals; faults carry names, never values),
  and `federation` behind the opt-in `federation` feature (a token-level
  split of the `FROM ENDPOINT`/`ORGANISATION` directive; the strict parser
  keeps refusing it). `ast::Span` rides on `IdentifiedPath` and on each
  `WhereExpr::Identified` leaf and is excluded from equality, so the
  printer round-trip invariant stays a statement about syntax.
