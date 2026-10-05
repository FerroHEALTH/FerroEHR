#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The licensing declarations, the licence texts and the licensing chapter must
# describe the same set of licences.
#
# The `licensing` CI job runs `reuse lint` in the step before this one, which
# proves that every file in the tree carries licensing information and that no
# licence text is orphaned. This script runs no linter of its own: it checks
# the thing this repository actually gets wrong over time, which `reuse lint`
# cannot see — the human-readable licensing chapter drifting away from the
# machine-readable declarations, so a compliance reviewer reading the site and
# a scanner reading `REUSE.toml` are told different things.
#
# Four assertions, all mechanical:
#   1. every SPDX identifier used in REUSE.toml has its text in LICENSES/;
#   2. every text in LICENSES/ is used by REUSE.toml (no orphan);
#   3. every SPDX identifier used in REUSE.toml is named in the licensing
#      chapter, so a licence cannot enter the tree without the page that
#      explains the tree acquiring it;
#   4. no first-party file still claims the licence the project LEFT. The
#      2026-09-03 move from MIT to the Business Source License 1.1 touched
#      headers, manifests, badges, image labels and pages at once; a claim that
#      survives that sweep, or comes back with a copied template, is a stale
#      statement about the project's own code. Third-party files keep their
#      own licences and are excluded, not exempted.
#
# What it deliberately does NOT check: whether the chapter's PROSE about a
# licence is correct. No tool can judge that, and a check that pretended to
# would be the kind of unenforced rule `.claude/rules/reliability.md` refuses.
#
# Usage: scripts/checks/licensing-declarations.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

readonly REUSE_TOML='REUSE.toml'
readonly LICENSES_DIR='LICENSES'
readonly CHAPTER='website/book/src/licensing.md'

for required in "$REUSE_TOML" "$CHAPTER"; do
  [[ -f "$required" ]] || { echo "error: missing $required" >&2; exit 1; }
done
[[ -d "$LICENSES_DIR" ]] || { echo "error: missing $LICENSES_DIR/" >&2; exit 1; }
command -v yq >/dev/null || { echo "error: yq is required" >&2; exit 1; }

# Every identifier named in an SPDX expression. Expressions here are `AND`/`OR`
# joins of plain identifiers — no parentheses, no `WITH` — so splitting on the
# operators is exact rather than an approximation of a parser.
declared=$(
  yq -p toml -o json '[.annotations[]."SPDX-License-Identifier"] | flatten' "$REUSE_TOML" \
    | jq -r '.[]' \
    | tr ' ' '\n' \
    | grep -vxE 'AND|OR' \
    | grep -v '^$' \
    | sort -u
)
texts=$(find "$LICENSES_DIR" -maxdepth 1 -type f -name '*.txt' -exec basename {} .txt \; | sort)

fail=0
note() { echo "licensing-declarations: $*" >&2; fail=1; }

while read -r id; do
  [[ -n "$id" ]] || continue
  note "REUSE.toml declares '$id' but $LICENSES_DIR/$id.txt does not exist — REUSE requires the full text of every licence the tree is offered under"
done < <(comm -23 <(echo "$declared") <(echo "$texts"))

while read -r id; do
  [[ -n "$id" ]] || continue
  note "$LICENSES_DIR/$id.txt is present but nothing in REUSE.toml is offered under '$id' — delete the text, or declare the files that need it"
done < <(comm -13 <(echo "$declared") <(echo "$texts"))

# The chapter names licences the way a reader does ("CC-BY-SA 3.0"), so both the
# SPDX spelling and the spaced human spelling count as a mention.
while read -r id; do
  [[ -n "$id" ]] || continue
  human=${id%-only}
  human=$(printf '%s' "$human" | sed -E 's/^(CC-BY-SA)-([0-9])/\1 \2/; s/^(MPL|AGPL|Apache|GPL|LGPL)-([0-9])/\1 \2/')
  if ! grep -qF -- "$id" "$CHAPTER" && ! grep -qF -- "$human" "$CHAPTER"; then
    note "'$id' is declared in REUSE.toml but named nowhere in $CHAPTER — the licensing chapter must account for every licence this tree redistributes under"
  fi
done < <(echo "$declared")

# 4. A stale claim of the superseded licence. The patterns are the SHAPES a
# project-own claim takes (a header identifier, a manifest field, a badge, an
# OCI label, a licence-file name), never the bare word: MIT stays a legitimate
# word for KaTeX, for dependencies, and for the history the changelog records.
# The pattern quotes the tag syntax it hunts for, so reuse lint must skip it.
# REUSE-IgnoreStart
readonly STALE='SPDX-License-Identifier: MIT( AND| -->|$)|LICENSE-MIT|^license = "MIT|^license: MIT|licenses="?MIT|artifacthub\.io/license: MIT|badge/[Ll]icense-MIT|MIT-licensed|"license": "mit"'
# REUSE-IgnoreEnd
readonly -a STALE_EXCLUDED=(
  ':(exclude)docs/specs/**'
  ':(exclude)corpus/**'
  ':(exclude)crates/*/vendor/**'
  ':(exclude)crates/*/tests/**'
  ':(exclude)crates/*/schemas/**'
  ':(exclude)crates/*/assets/**'
  ':(exclude)tools/openehr-codegen/vendor/**'
  ':(exclude)LICENSES/**'
  ':(exclude).github/cff-schema/**'
  ':(exclude)website/book/theme/mermaid.min.js'
  ':(exclude)website/book/src/katex/**'
  ':(exclude)security/fossa-adjudications.md'
  ':(exclude)CHANGELOG.md'
  ':(exclude)scripts/checks/licensing-declarations.sh'
)
if stale=$(git grep -InE "$STALE" -- . "${STALE_EXCLUDED[@]}"); then
  note "a first-party file still claims the MIT licence the project left on 2026-09-03:"
  echo "$stale" >&2
fi

[[ "$fail" -eq 0 ]] || {
  echo >&2
  echo "The machine-readable declarations (REUSE.toml + LICENSES/) and the" >&2
  echo "licensing chapter are the same statement in two registers; they may" >&2
  echo "never disagree about WHICH licences apply." >&2
  exit 1
}

echo "ok: $(echo "$declared" | wc -l | tr -d ' ') licences declared, texted and documented"
