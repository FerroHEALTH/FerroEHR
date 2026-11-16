<!-- Layout note (2026-08-05, #1942/#1946): grammars are VERSION-SCOPED —
each LANG component version keeps its grammar set under its own `vX_Y/`
directory, mirroring the crate's generation modules. `v1_1/` = the
1.1.0-line normative grammars pinned below; `v1_0/` = the LANG 1.0.0
release's own normative ODIN set (pinned in its section below). The ONE
vendoring script is `scripts/vendor/lang-grammars.sh` (replacing
vendor-el-grammar.sh); it writes every version directory — never hand-copy
between them. -->

# Provenance — the normative ODIN + BEL ANTLR4 grammars

- Source: https://github.com/openEHR/adl-antlr (Apache-2.0; `LICENSE`
  vendored alongside)
- Commit: `d64ffeb6f2e2c0325558532bfd820dff9a7d8efc` (2026-11-06, the merge of
  adl-antlr PR 35 after PR 37); `--check` on the vendoring script proves the
  files still match it
- Files: `src/main/antlr/adl/{odin.g4, odin_values.g4, base_lexer.g4,
  base_expressions.g4}`, copied verbatim.

Why here: ODIN **and** the Basic Expression Language (BEL) are
LANG-component specifications (`docs/specs/openehr/LANG/docs/{odin,BEL}/`);
these grammars are the normative syntax those specs' syntax sections
reference (the BEL spec's `masterAppB`/`masterAppA` appendix is `include::`
pointers at `base_expressions.g4`), and the reference input for the
hand-written `openehr_lang::{odin, bel}` parsers (no ANTLR runtime).
`odin_values.g4` imports `base_lexer.g4`; `base_expressions.g4` imports
`cadl2_primitives` (the cADL primitive right-hand side of a `matches`
constraint — the AOM extension point that `openehr-adl` supplies over the
`openehr_lang::bel` seam) and `odin_values.g4`. The shared files (same
upstream commit) are also vendored at `crates/openehr-adl/vendor/grammar/`
for the ADL2/cADL grammar set — each crate's vendor dir stays
self-contained.

## The Expression Language (EL) grammars

- Source: https://github.com/openEHR/openEHR-antlr4 (Apache-2.0;
  `LICENSE-openEHR-antlr4` vendored alongside — the ONLY home of the EL
  grammars: `openEHR/adl-antlr` carries none, verified 2026-08-04).
- Commit: `3494da942f3ed35963279837447b3039dd098e20` (2025-12-15), via
  `scripts/vendor/lang-grammars.sh`.
- Files: `reader_common/src/main/antlr/{ElLexer.g4, ElParser.g4}` plus their
  transitive grammar imports `{Cadl2Lexer, Cadl2Parser, SymbolsLexer,
  GeneralIdsLexer}.g4`, copied verbatim (the appendix includes only the two EL
  files; the imports are vendored so the syntax resolves standalone).

Why here: the LANG EL syntax appendix
(`docs/specs/openehr/LANG/docs/EL/masterAppA-syntax.adoc`) is an `include::`
of exactly these two files — they are the normative EL syntax, and the
reference input for the hand-written `openehr_lang` EL parser (no ANTLR
runtime).

## The `v1_0/` set — LANG Release-1.0.0's normative ODIN grammar

- Source: https://github.com/openEHR/adl-antlr (same Apache-2.0 `LICENSE`).
- Commit: `7e0131ade4bcb94ee8c312e3905fa0c4343e785d` (2021-01-21 — the last
  adl-antlr commit before the LANG Release-1.0.0 tag was placed, 2021-03-01),
  via `scripts/vendor/lang-grammars.sh`.
- Files: `src/main/antlr/adl2/{odin.g4, odin_values.g4, base_patterns.g4,
  base_lexer.g4}`, copied verbatim (that era's layout kept the grammars
  under `adl2/`; the appendix includes the first three, and `base_lexer.g4`
  is `base_patterns.g4`'s transitive import — it defines
  `INCLUDED_LANGUAGE_FRAGMENT` among others — vendored so the set resolves
  standalone, the same treatment the v1_1 set gives its imports).

Why this set: the Release-1.0.0 ODIN syntax appendix
(`docs/odin/masterAppB-syntax_spec.adoc`, verified first-hand 2026-08-05) is
an `include::` of exactly these three files. The 1.0.0 Expression Language
(DEVELOPMENT) publishes NO grammar — no syntax appendix, zero grammar
includes across its five chapters — so no EL files exist here and
`openehr_lang::v1_0` carries no EL parser; BEL likewise first appears in
1.1.0.

Never hand-edit; re-vendor from upstream and update the commit pin here.
