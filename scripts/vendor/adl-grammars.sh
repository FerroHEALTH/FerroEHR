#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Vendor the AM component's normative ADL/cADL ANTLR4 grammars, VERSION-SCOPED.
#
# Layout (the multi-generation foundation, #1936/#1946): the grammar files are
# grouped by the AM generation their productions target, mirroring the
# openehr-am generation modules —
#   v2_4/ — the ADL2 set: the ADL2 spec's normative syntax appendix
#     (docs/specs/openehr/AM/docs/ADL2/masterAppB-syntax_spec.adoc) is an
#     `include::` of these files, PLUS the support grammars upstream maintains
#     as part of the ADL2 family (adl_keywords, base_expressions, base_lexer).
#   v1_4/ — the ADL 1.4 grammar set (adl14/cadl14/cadl14_primitives), the
#     reference input for the 1.4→2 conversion front end. These IMPORT the
#     v2_4 support grammars (upstream keeps one flat directory serving both
#     generations), so v1_4/ is deliberately NOT a self-contained closure.
#   PCRE.g4 — generation-independent lexical support, top-level (upstream
#     keeps it outside the adl/ directory too).
#
# odin.g4/odin_values.g4 stay with the LANG component
# (crates/openehr-lang/vendor/grammar/ — ODIN is a LANG spec); base_lexer.g4
# is vendored in BOTH crates' dirs from the same commit.
#
# Apache-2.0 upstream (LICENSE vendored alongside).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)/crates/openehr-adl/vendor/grammar"

ADL_ANTLR_REPO="openEHR/adl-antlr"
ADL_ANTLR_COMMIT="8db091ec3d810371cc41cd072fee81ce893fea47" # 2024-04-06

fetch() {
  local path="$1" out="$2"
  curl -fsSL --proto '=https' --proto-redir '=https' "https://raw.githubusercontent.com/${ADL_ANTLR_REPO}/${ADL_ANTLR_COMMIT}/${path}" -o "${out}"
  echo "vendored ${out} @ ${ADL_ANTLR_COMMIT}"
}

mkdir -p "${ROOT}/v1_4" "${ROOT}/v2_4"

for f in adl2.g4 cadl2.g4 cadl2_primitives.g4 adl_keywords.g4 base_expressions.g4 base_lexer.g4; do
  fetch "src/main/antlr/adl/${f}" "${ROOT}/v2_4/${f}"
done

for f in adl14.g4 cadl14.g4 cadl14_primitives.g4; do
  fetch "src/main/antlr/adl/${f}" "${ROOT}/v1_4/${f}"
done

fetch "src/main/antlr/PCRE.g4" "${ROOT}/PCRE.g4"
fetch "LICENSE" "${ROOT}/LICENSE"
