#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The CRA risk assessment declares the security-relevant paths, every declared
# path exists, and a release that changes one of them revises the assessment.
#
# CRA Art. 13(3) has the assessment "documented and updated as appropriate
# during a support period", and Art. 13(14) has procedures "for products that
# are part of a series of production to remain in conformity", with changes
# in their design "adequately taken into account" (docs/law/eu/cra/text.html).
# The declared paths are the assets and controls of the assessment's register;
# a release that changes one adds a row to its Revisions table.
#
# Usage:
#   scripts/checks/cra-risk-assessment.sh                 declaration only
#   scripts/checks/cra-risk-assessment.sh --since <ref>   also change control
#   scripts/checks/cra-risk-assessment.sh --self-test     prove the detectors
#
# --since compares the trees of <ref> and HEAD, never their ancestry, so it
# works across the rewritten history (the previous release tag is resolved by
# version order by the caller).
set -euo pipefail
# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

PAGE="website/book/src/compliance/cra-risk-assessment.md"
START="<!-- security-relevant-paths:start -->"
END="<!-- security-relevant-paths:end -->"

# The declared paths: one backticked path per list item between the markers.
declared_paths() {
  # shellcheck disable=SC2016 # the backticks are literal Markdown in the pattern, not an expansion
  awk -v s="$START" -v e="$END" '$0==s{f=1;next} $0==e{f=0} f' \
    | sed -nE 's/^- `([^`]+)`.*$/\1/p'
}

# The number of data rows of the table under "## Revisions".
revision_rows() {
  awk '/^## /{f=($0=="## Revisions")} f && /^\| *[0-9]+ *\|/{n++} END{print n+0}'
}

self_test() {
  local page paths rows
  # shellcheck disable=SC2016 # the backticks are literal Markdown in the fixture, not an expansion
  page="$(printf '%s\n' "x" "$START" '- `a/b` reason' 'not a path' '- `c`' "$END" '- `d`' \
    "## Revisions" "| Version | Date |" "|---|---|" "| 1 | 2026-10-06 |" "| 2 | 2026-10-07 |" \
    "## Other" "| 9 | x |")"
  paths="$(declared_paths <<<"$page" | tr '\n' ' ')"
  [[ "$paths" == "a/b c " ]] || { echo "self-test: declared paths read '$paths'" >&2; exit 1; }
  rows="$(revision_rows <<<"$page")"
  [[ "$rows" == "2" ]] || { echo "self-test: revision rows read $rows, not 2" >&2; exit 1; }
  rows="$(revision_rows <<<"# no table")"
  [[ "$rows" == "0" ]] || { echo "self-test: an absent table reads $rows, not 0" >&2; exit 1; }
  echo "cra-risk-assessment: self-test OK (declared paths, revision rows, absent table)."
}

since=""
case "${1:-}" in
  --self-test)
    shift
    guard_no_args "$@"
    self_test
    exit 0
    ;;
  --since)
    [[ -n "${2:-}" ]] || guard_usage "[--since <ref> | --self-test]"
    since="$2"
    shift 2
    ;;
esac
guard_no_args "$@"

fail=0
report() { echo "cra-risk-assessment: $1" >&2; fail=1; }

[[ -f "$PAGE" ]] || { report "$PAGE does not exist"; exit 1; }
PATHS=()
while IFS= read -r p; do PATHS+=("$p"); done < <(declared_paths <"$PAGE")
[[ "${#PATHS[@]}" -gt 0 ]] || { report "$PAGE declares no security-relevant paths between $START and $END"; exit 1; }
for p in "${PATHS[@]}"; do
  [[ -e "$p" ]] || report "$PAGE declares $p, which does not exist"
done

if [[ -n "$since" ]]; then
  git rev-parse --verify --quiet "${since}^{commit}" >/dev/null \
    || { report "--since $since is not a commit"; exit 1; }
  changed="$(git diff --name-only "$since" HEAD -- "${PATHS[@]}")"
  if [[ -n "$changed" ]]; then
    before="$({ git show "$since:$PAGE" 2>/dev/null || true; } | revision_rows)"
    after="$(revision_rows <"$PAGE")"
    if [[ "$after" -le "$before" ]]; then
      report "security-relevant paths changed since $since and $PAGE has no new revision row (CRA Art. 13(3), 13(14)):"
      while IFS= read -r f; do echo "  $f" >&2; done <<<"$changed"
    fi
  fi
fi

[[ "$fail" -eq 0 ]] || exit 1
if [[ -n "$since" ]]; then
  echo "cra-risk-assessment: ${#PATHS[@]} declared paths exist, change control holds since $since"
else
  echo "cra-risk-assessment: ${#PATHS[@]} declared paths exist"
fi
