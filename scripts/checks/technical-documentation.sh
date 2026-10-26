#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The technical documentation tree is complete, its links resolve, and a
# release that changes the logging component updates it.
#
# The technical documentation of EHDS Art. 37 and Annex III
# (docs/law/eu/ehds/text.html) is the EHR system's, FerroEHR with FerroBRIDGE,
# and lives in the FerroHEALTH book at the revision docs/ehds/system-documents.env
# pins; docs/technical-documentation/ keeps FerroEHR's CRA Annex VII points
# (docs/law/eu/cra/text.html) and the index. EHDS Art. 30(2) has a change to a
# harmonised software component "reflected in the technical documentation", and
# the logging component's modules are the ones docs/architecture.md names under
# "Regulatory position (EHDS and CRA)", so such a change moves the tree or the
# pin. The CRA user information is checked here too, because the release notes
# link it and a missing point would ship a release without it.
#
# Usage:
#   scripts/checks/technical-documentation.sh                 structure and links
#   scripts/checks/technical-documentation.sh --since <ref>   also change control
#
# --since compares the trees of <ref> and HEAD, never their ancestry, so it
# works across the rewritten history (the previous release tag is resolved by
# version order by the caller).
set -euo pipefail
# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

since=""
if [[ "${1:-}" == "--since" ]]; then
  [[ -n "${2:-}" ]] || guard_usage "[--since <ref>]"
  since="$2"
  shift 2
fi
guard_no_args "$@"

TREE="docs/technical-documentation"
PIN="docs/ehds/system-documents.env"
USERINFO="website/book/src/compliance/cra-user-information.md"
# The logging component, as docs/architecture.md lists it. Keep the two lists
# equal: a module added there is added here in the same change. The shared
# crates it is built on are read from their pins in the root Cargo.toml below.
LOGGING_PATHS=(
  "app/ferroehr/src/system_log"
  "app/ferroehr-rest/src/system_log"
  "app/ferroehr-rest/src/extensions/fhir.rs"
  "app/ferroehr/src/storage/marks.rs"
  "app/ferroehr/migrations/audit"
)

fail=0
report() { echo "technical-documentation: $1" >&2; fail=1; }

[[ -f "$TREE/README.md" ]] || { report "$TREE/README.md does not exist"; exit 1; }

# Every file the contents table names exists, and every file in the tree is
# named there, so the index and the tree cannot drift apart.
# shellcheck disable=SC2016 # the backticks are literal Markdown in the pattern, not an expansion
listed="$(grep -oE '^\| [^|]+ \| \[`[a-z0-9-]+\.md`\]' "$TREE/README.md" | grep -oE '[a-z0-9-]+\.md' | sort -u)"
[[ -n "$listed" ]] || report "$TREE/README.md has no contents table"
for f in $listed; do
  [[ -f "$TREE/$f" ]] || report "$TREE/README.md lists $f, which does not exist"
done
for path in "$TREE"/*.md; do
  f="${path##*/}"
  [[ "$f" == "README.md" ]] && continue
  grep -qxF "$f" <<<"$listed" || report "$TREE/$f is not listed in $TREE/README.md"
done

# Every relative link in the tree resolves to a file of this repository.
# Anchors are not checked; the book build checks the anchors of its own pages.
for path in "$TREE"/*.md; do
  while IFS= read -r target; do
    case "$target" in
      http://* | https://* | mailto:* | \#*) continue ;;
    esac
    file="${target%%#*}"
    [[ -e "$TREE/$file" ]] || report "$path links $target, which does not resolve"
  done < <(grep -oE '\]\([^)]+\)' "$path" | sed -E 's/^\]\((.*)\)$/\1/')
done

# The pin of the EHR system's documents names a full commit and a date for
# each page, and the CRA user information carries every point of Annex II
# (CRA Art. 13(18), Annex VII 1(d); docs/law/eu/cra/text.html), in order and
# with every lettered sub-point of point 8. The information sheet's points of
# EHDS Art. 38(2) are checked on the fetched page by
# scripts/release/accompanying-documents.sh.
if [[ -f "$PIN" ]]; then
  grep -qE '^FERROHEALTH_COMMIT=[0-9a-f]{40}$' "$PIN" || report "$PIN names no full FERROHEALTH_COMMIT"
  for key in INFORMATION_SHEET_REVISED INSTRUCTIONS_FOR_USE_REVISED READINESS_REVISED TECHNICAL_DOCUMENTATION_REVISED; do
    grep -qE "^${key}=[0-9]{4}-[0-9]{2}-[0-9]{2}$" "$PIN" || report "$PIN names no ${key} date"
  done
else
  report "$PIN does not exist"
fi
if [[ -f "$USERINFO" ]]; then
  # Headings in document order: "## 1. ", ..., "## 8. ", "### 8(a) ", ..., "## 9. ".
  found="$(grep -oE '^(## [1-9]\. |### 8\([a-f]\) )' "$USERINFO" | tr -d '# .' | paste -sd, -)"
  want="1,2,3,4,5,6,7,8,8(a),8(b),8(c),8(d),8(e),8(f),9"
  [[ "$found" == "$want" ]] \
    || report "$USERINFO must carry Annex II points $want as headings, in that order (found: ${found:-none})"
else
  report "$USERINFO does not exist"
fi

# Change control: a change to the logging component since <ref> comes with a
# change to the tree or to the pin of the EHR system's documents.
if [[ -n "$since" ]]; then
  git rev-parse --verify --quiet "${since}^{commit}" >/dev/null \
    || { report "--since $since is not a commit"; exit 1; }
  changed="$(git diff --name-only "$since" HEAD -- "${LOGGING_PATHS[@]}")"
  # The shared crates the component is built on (ehds-logging, ihe-atna) are
  # part of it too: a moved pin in the root Cargo.toml is a change to it.
  pins() { git show "$1:Cargo.toml" 2>/dev/null | grep -E '^(ehds-logging|ihe-atna) = ' || true; }
  if [[ "$(pins "$since")" != "$(pins HEAD)" ]]; then
    changed="$(printf '%s\n%s' "$changed" "Cargo.toml (the ehds-logging or ihe-atna pin)" | sed '/^$/d')"
  fi
  if [[ -n "$changed" ]]; then
    if [[ -z "$(git diff --name-only "$since" HEAD -- "$TREE" "$PIN")" ]]; then
      report "the logging component changed since $since and neither $TREE nor $PIN did (EHDS Art. 30(2)):"
      while IFS= read -r f; do echo "  $f" >&2; done <<<"$changed"
    fi
  fi
fi

[[ "$fail" -eq 0 ]] || exit 1
if [[ -n "$since" ]]; then
  echo "technical-documentation: tree complete, links resolve, change control holds since $since"
else
  echo "technical-documentation: tree complete, links resolve"
fi
