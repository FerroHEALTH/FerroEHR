#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The technical documentation tree is complete, its links resolve, and a
# release that changes the logging component updates it.
#
# docs/technical-documentation/ is the technical documentation of EHDS Art. 37
# and Annex III (docs/law/eu/ehds/text.html) and CRA Annex VII
# (docs/law/eu/cra/text.html). EHDS Art. 30(2) has a change to a harmonised
# software component "reflected in the technical documentation", and the
# logging component's modules are the ones docs/architecture.md names under
# "Regulatory position (EHDS and CRA)". The documents that accompany a release
# (EHDS Art. 30(1)(d)) are checked here too, because the release lane attaches
# them and a missing one would ship a release without them.
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
SHEET="website/book/src/compliance/information-sheet.md"
IFU="website/book/src/compliance/instructions-for-use.md"
# The logging component, as docs/architecture.md lists it. Keep the two lists
# equal: a module added there is added here in the same change.
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

# The accompanying documents exist, and the information sheet carries every
# point of EHDS Art. 38(2).
[[ -f "$IFU" ]] || report "$IFU does not exist"
if [[ -f "$SHEET" ]]; then
  for point in a b c d e; do
    grep -qE "^## \(${point}\) " "$SHEET" || report "$SHEET has no section for Art. 38(2)(${point})"
  done
else
  report "$SHEET does not exist"
fi

# Change control: a change to the logging component since <ref> comes with a
# change to the tree.
if [[ -n "$since" ]]; then
  git rev-parse --verify --quiet "${since}^{commit}" >/dev/null \
    || { report "--since $since is not a commit"; exit 1; }
  changed="$(git diff --name-only "$since" HEAD -- "${LOGGING_PATHS[@]}")"
  if [[ -n "$changed" ]]; then
    if [[ -z "$(git diff --name-only "$since" HEAD -- "$TREE")" ]]; then
      report "the logging component changed since $since and $TREE did not (EHDS Art. 30(2)):"
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
