#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The vendored identity and provisioning documents are the bytes their records
# say they are.
#
# Each tree below is fetched by a scripts/vendor/*.sh that pins every file by
# sha256, and its PROVENANCE.md lists each file with that digest as the last
# two backticked cells of a table row. Nothing parses these documents, so this
# digest check is how the tree is exercised (.claude/rules/vendored-corpora.md):
#
#   1. every file a record lists exists and hashes to the recorded sha256, so
#      a hand edit or a partial re-fetch fails here;
#   2. every file in the tree is listed, so an unrecorded file cannot ride in.
#
# WHAT IT DOES NOT CHECK: whether a pin is still the newest upstream, and
# whether a record quotes its licence truthfully. Both are review-enforced.
#
# Usage: scripts/checks/pinned-docs.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

readonly TREES=(
  docs/specs/rfc
  docs/specs/openid-connect-core
  docs/specs/microsoft-entra
  docs/specs/scim-reference-code
  docs/specs/keycloak
)

fail=0
note() { echo "pinned-docs: $*" >&2; fail=1; }

for tree in "${TREES[@]}"; do
  record="$tree/PROVENANCE.md"
  [[ -f "$record" ]] || { note "$tree has no PROVENANCE.md"; continue; }
  listed=$(mktemp)
  # The last two backticked cells of a table row: the file, then its sha256.
  # shellcheck disable=SC2016 # the backticks are literal Markdown, not expansions
  sed -nE 's/^\|(.*\|)? `([^`]+)` \| `([0-9a-f]{64})` \|$/\2 \3/p' "$record" >"$listed"
  [[ -s "$listed" ]] || note "$record lists no file with its sha256"
  while read -r path sha; do
    file="$tree/$path"
    if [[ ! -f "$file" ]]; then
      note "$record lists $path, which $tree does not hold"
    elif [[ "$(shasum -a 256 "$file" | cut -d' ' -f1)" != "$sha" ]]; then
      note "$file does not hash to the sha256 $record records"
    fi
  done <"$listed"
  while IFS= read -r file; do
    path="${file#"$tree"/}"
    grep -qxF "$path" <(cut -d' ' -f1 "$listed") || note "$file is not listed in $record"
  done < <(find "$tree" -type f ! -name PROVENANCE.md | LC_ALL=C sort)
  rm -f "$listed"
done

[[ "$fail" -eq 0 ]] || exit 1
echo "ok: ${#TREES[@]} pinned document trees match their records"
