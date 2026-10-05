#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Derive every website conformance claim from the committed CNF artifacts
# (docs/conformance/ferroehr/: results.json + verdicts.json) at BUILD time —
# the site sources carry no hand-typed conformance numbers (enforced by
# scripts/checks/conformance-numbers.sh in CI), so the stale-numbers failure
# mode is impossible by construction.
#
#   render-conformance-stats.sh includes          write website/book/generated/*.md
#   render-conformance-stats.sh fill-html FILE    fill data-cnf markers in FILE in place
#
# Consumed by scripts/site/build.sh; runnable standalone for local previews.
set -euo pipefail
cd "$(dirname "$0")/../.."

ART="docs/conformance/ferroehr"
command -v jq >/dev/null 2>&1 || {
  echo "render-conformance-stats: jq is required (brew install jq / preinstalled on CI runners)" >&2
  exit 1
}
for f in "$ART/results.json" "$ART/verdicts.json"; do
  [[ -f "$f" ]] || {
    echo "render-conformance-stats: $f missing — run the conformance suite first" >&2
    exit 1
  }
done

# ── outcome counts (CNF §8.10 results schema) ────────────────────────────────
# Driven = a verdict-bearing execution; an N/A row carries a machine-readable
# citation (an unrealizable wire, an undeclared option branch, a ground the
# topology cannot establish) — a documented exclusion, never a skip.
total=$(jq '.outcomes | length' "$ART/results.json")
passed=$(jq '[.outcomes[] | select(.status == "passed")] | length' "$ART/results.json")
failed=$(jq '[.outcomes[] | select(.status == "failed")] | length' "$ART/results.json")
errored=$(jq '[.outcomes[] | select(.status == "errored")] | length' "$ART/results.json")
na=$(jq '[.outcomes[] | select(.status == "not_applicable" or .status == "skipped")] | length' "$ART/results.json")
driven=$((passed + failed + errored))

# The results artifact is deliberately clock-free (deterministic re-runs);
# the run date is the AUTHOR date of the artifact's last commit (fallback:
# file mtime). The committer date is restamped by a rebase or a re-signing
# of history, which is what moved the comparison page's date on 2026-09-16.
run_date=$(git log -1 --format=%as -- "$ART/results.json" 2>/dev/null || true)
[[ -n "$run_date" ]] || run_date=$(date -r "$ART/results.json" +%Y-%m-%d)

# ── profile verdicts, straight from the computed verdict report ──────────────
pverdict() {
  jq -r --arg t "$1" '.profiles[] | select(.[0]==$t) | .[1]' "$ART/verdicts.json" \
    | sed -e 's/_/ /g' -e 's/^$/—/'
}
core=$(pverdict CORE)
standard=$(pverdict STANDARD)
options=$(pverdict OPTIONS)
sec=$(jq -r '.security // "not claimed"' "$ART/verdicts.json" | sed 's/_/ /g')

# Capability satisfaction (passed or register-excused) across the matrix.
caps_ok=$(jq '[.capabilities[] | select(.[1] == "passed" or .[1] == "unrealized")] | length' "$ART/verdicts.json")
caps_total=$(jq '.capabilities | length' "$ART/verdicts.json")

# Sanity: refuse to render nonsense (a truncated artifact must fail the build).
for v in "$total" "$passed" "$failed" "$errored" "$na" "$driven" "$caps_ok" "$caps_total"; do
  [[ "$v" =~ ^[0-9]+$ ]] || { echo "render-conformance-stats: non-numeric stat '$v'" >&2; exit 1; }
done
for v in "$core" "$standard" "$options" "$sec"; do
  [[ -n "$v" ]] && [[ "$v" != "—" ]] || { echo "render-conformance-stats: empty profile verdict in verdicts.json" >&2; exit 1; }
done

upper() { printf '%s' "$1" | tr '[:lower:]' '[:upper:]'; }

case "${1:-includes}" in
  includes)
    mkdir -p website/book/generated
    cat > website/book/generated/conformance-stats.md <<EOF
- **${total} case-by-format executions: ${passed} passed, ${failed} failed,
  ${errored} inconclusive, ${na} not applicable with a machine-readable
  citation** (run of ${run_date}).
- **Profile verdicts — Core: $(upper "$core"). Standard: $(upper "$standard").
  Options: $(upper "$options"). Security (SEC-BASIC): $(upper "$sec").**
- **${caps_ok}/${caps_total} capabilities satisfied** (passed, or excused by
  a schedule-registered ambiguity — an unrealizable wire on this technology
  profile is an explicit scope exclusion, never a silent pass).
EOF
    echo "render-conformance-stats: wrote website/book/generated/conformance-stats.md (${passed}/${driven} driven, run ${run_date})"
    ;;
  fill-html)
    file="${2:?fill-html needs a file argument}"
    perl -pi -e "
      s/(data-cnf=\"executed\"[^>]*>)[^<]*/\${1}${driven}/g;
      s/(data-cnf=\"passed\"[^>]*>)[^<]*/\${1}${passed}/g;
      s/(data-cnf=\"failed\"[^>]*>)[^<]*/\${1}$((failed + errored))/g;
      s/(data-cnf=\"verdict-core\"[^>]*>)[^<]*/\${1}CORE · $(upper "$core")/g;
      s/(data-cnf=\"verdict-standard\"[^>]*>)[^<]*/\${1}STANDARD · $(upper "$standard")/g;
      s/(data-cnf=\"verdict-options\"[^>]*>)[^<]*/\${1}OPTIONS · $(upper "$options")/g;
      s/(data-cnf=\"verdict-sec\"[^>]*>)[^<]*/\${1}SEC-BASIC · $(upper "$sec")/g;
    " "$file"
    # A marker that survived filling means the HTML and this script drifted.
    if grep -qE 'data-cnf="[^"]*"[^>]*>—<' "$file"; then
      echo "render-conformance-stats: unfilled data-cnf marker left in $file" >&2
      exit 1
    fi
    echo "render-conformance-stats: filled $file"
    ;;
  *)
    echo "usage: $0 [includes | fill-html FILE]" >&2
    exit 2
    ;;
esac
