#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the LANG component's normative ANTLR4 grammars, VERSION-SCOPED.
#
# Layout (the multi-generation foundation, #1936/#1942): each LANG component
# version keeps its complete grammar set under its own
# crates/openehr-lang/vendor/grammar/vX_Y/ directory, mirroring the crate's
# generation modules — so a spec bump vendors a NEW directory and no
# generation's reference input can silently drift under another's.
#
# v1_1 — the 1.1.0-line normative grammars, from two upstream repos:
#   - ODIN + BEL: https://github.com/openEHR/adl-antlr (the LANG odin/BEL
#     syntax appendices reference these files as the normative syntax).
#   - EL: https://github.com/openEHR/openEHR-antlr4 (the EL syntax appendix
#     masterAppA-syntax.adoc is an `include::` of ElParser.g4 + ElLexer.g4),
#     plus the transitive grammar imports so the set resolves standalone.
#
# v1_0 — the LANG 1.0.0 release's normative ODIN grammar: the release's own
#   syntax appendix includes odin.g4/odin_values.g4/base_patterns.g4 from the
#   adl-antlr adl2/ layout of its era (pin below). 1.0.0 EL is DEVELOPMENT
#   prose with no grammar; BEL does not exist in 1.0.0 — neither has files.
#
# Both repos are Apache-2.0 (LICENSE vendored alongside).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)/crates/openehr-lang/vendor/grammar"

ADL_ANTLR_REPO="openEHR/adl-antlr"
ADL_ANTLR_COMMIT="8db091ec3d810371cc41cd072fee81ce893fea47" # 2024-04-06
ANTLR4_REPO="openEHR/openEHR-antlr4"
ANTLR4_COMMIT="3494da942f3ed35963279837447b3039dd098e20" # master 2025-12-15

fetch() {
  local repo="$1" commit="$2" path="$3" out="$4"
  curl -fsSL --proto '=https' --proto-redir '=https' "https://raw.githubusercontent.com/${repo}/${commit}/${path}" -o "${out}"
  echo "vendored ${out} @ ${commit}"
}

# ── v1_1: the pinned normative set ──────────────────────────────────────────
DEST="${ROOT}/v1_1"
mkdir -p "${DEST}"

# ODIN + BEL (adl-antlr)
for f in odin.g4 odin_values.g4 base_lexer.g4 base_expressions.g4; do
  fetch "${ADL_ANTLR_REPO}" "${ADL_ANTLR_COMMIT}" "src/main/antlr/adl/${f}" "${DEST}/${f}"
done

# EL + transitive imports (openEHR-antlr4)
for f in ElLexer.g4 ElParser.g4 Cadl2Lexer.g4 Cadl2Parser.g4 SymbolsLexer.g4 GeneralIdsLexer.g4; do
  fetch "${ANTLR4_REPO}" "${ANTLR4_COMMIT}" "reader_common/src/main/antlr/${f}" "${DEST}/${f}"
done
fetch "${ANTLR4_REPO}" "${ANTLR4_COMMIT}" "LICENSE" "${ROOT}/LICENSE-openEHR-antlr4"

# ── v1_0: the LANG 1.0.0 release's own normative set (#1946) ────────────────
# The Release-1.0.0 ODIN syntax appendix (docs/odin/masterAppB-syntax_spec.adoc,
# verified first-hand 2026-08-05) is an `include::` of odin.g4 + odin_values.g4
# + base_patterns.g4 from the adl-antlr `adl2/` directory of its era; the pin
# is the last adl-antlr commit before the Release-1.0.0 tag was placed
# (2021-03-01). The 1.0.0 Expression Language (DEVELOPMENT) publishes NO
# grammar — no syntax appendix, zero grammar includes across its five
# chapters — so no EL files exist here and openehr_lang::v1_0 carries no EL
# parser (the adjudicated boundary; BEL likewise first appears in 1.1.0).
ADL_ANTLR_100_COMMIT="7e0131ade4bcb94ee8c312e3905fa0c4343e785d" # 2021-01-21
DEST10="${ROOT}/v1_0"
mkdir -p "${DEST10}"
# base_lexer.g4 is base_patterns.g4's transitive import (it defines
# INCLUDED_LANGUAGE_FRAGMENT among others) — vendored so the set resolves
# standalone, same treatment the v1_1 set gives its imports.
for f in odin.g4 odin_values.g4 base_patterns.g4 base_lexer.g4; do
  fetch "${ADL_ANTLR_REPO}" "${ADL_ANTLR_100_COMMIT}" "src/main/antlr/adl2/${f}" "${DEST10}/${f}"
done
