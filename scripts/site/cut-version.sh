#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Cut a frozen documentation version onto the docs-dist orphan branch.
# "generate once, never rebuild".
#
# Usage: scripts/site/cut-version.sh vX.Y.Z
#
# Builds the book once with site-url=/docs/vX.Y.Z/, rsyncs it into a
# docs-dist worktree under docs/vX.Y.Z/, prepends the version to versions.json,
# re-points the `latest` alias, then commits + pushes docs-dist.
#
# IDEMPOTENT (#2776): a version that is ALREADY frozen is reported and the
# script exits 0 without touching it — frozen trees are never rebuilt, and that
# is a no-op, not an error. It used to exit 1, which meant a release pipeline
# re-run after any later failure could never go green again; the recovery for
# every other leg is "re-run the pipeline", and this leg has to survive that.
#
# In a workflow the outcome is reported on $GITHUB_OUTPUT as `cut=true|false`
# (docs.github.com/actions/reference/workflows-and-actions/workflow-commands),
# so the caller knows whether a site rebuild is needed. Nothing is written when
# the variable is unset, which is every local run.
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$PWD"
# Served from the ferroehr.eu apex, so the book lives at the domain root.
SITE_BASE="${SITE_BASE:-}"

VER="${1:-}"
if [[ -z "$VER" ]]; then
  echo "usage: scripts/site/cut-version.sh vX.Y.Z" >&2
  exit 2
fi
if [[ ! "$VER" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
  echo "::error::'$VER' is not a vX.Y.Z tag." >&2
  exit 2
fi

WT="$ROOT/docs-dist"
log() { printf '\033[1;33m[cut-version]\033[0m %s\n' "$*"; }

# Report to the workflow, when there is one, whether a cut actually happened.
report() {
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    echo "cut=$1" >> "$GITHUB_OUTPUT"
  fi
}

# 1. Check out (or create) the docs-dist orphan branch as a worktree.
git worktree remove --force "$WT" >/dev/null 2>&1 || true
git fetch origin docs-dist >/dev/null 2>&1 || true
if git show-ref --verify --quiet refs/remotes/origin/docs-dist; then
  log "checking out existing docs-dist"
  git worktree add --force -B docs-dist "$WT" origin/docs-dist >/dev/null
else
  log "creating fresh docs-dist orphan branch"
  git worktree add --force --detach "$WT" HEAD >/dev/null
  git -C "$WT" checkout --orphan docs-dist >/dev/null 2>&1
  git -C "$WT" reset --hard >/dev/null 2>&1 || true
  git -C "$WT" rm -rf . >/dev/null 2>&1 || true
  printf '%s\n' '{ "latest": null, "versions": [] }' > "$WT/versions.json"
fi

# 2. Already frozen? Then there is nothing to do, and that is success.
# An unreadable or absent manifest is NOT an existing version — this must
# recognise only a version that is genuinely already frozen, so a malformed file
# is treated as absent here and replaced below rather than blocking the cut
# forever.
if [[ -d "$WT/docs/$VER" ]] \
  || jq -e --arg ver "$VER" 'any(.versions[]?; .id == $ver)' \
       "$WT/versions.json" >/dev/null 2>&1
then
  log "version $VER is already frozen in docs-dist — nothing to do (frozen trees are never rebuilt)"
  if [[ -d "$WT/docs/$VER" ]]; then
    log "  tree:     docs/$VER/ ($(find "$WT/docs/$VER" -type f | wc -l | tr -d ' ') files)"
  fi
  log "  manifest: latest -> $(jq -r '.latest // "none"' "$WT/versions.json" 2>/dev/null || echo unreadable)"
  report false
  exit 0
fi

# 3. Build the frozen book once, straight into docs-dist/docs/vX.Y.Z/.
log "building frozen book -> docs/$VER/"
rm -rf "$WT/docs/$VER"
mkdir -p "$WT/docs/$VER"
MDBOOK_OUTPUT__HTML__SITE_URL="$SITE_BASE/docs/$VER/" mdbook build "$ROOT/website/book" -d "$WT/docs/$VER" >/dev/null

# 4. Update versions.json: prepend the version, re-point `latest`.
log "updating versions.json"
# The manifest rewrite, in jq. An unreadable manifest starts from empty as it did
# before — the cut must not be blocked by a corrupt file it is about to replace.
# `dev`, `latest` and the version being cut are dropped from the carried-over list
# and re-added at the front, so the order is deterministic and re-cutting the same
# version is idempotent.
TODAY="$(date -u +%Y-%m-%d)"
if ! jq -e . "$WT/versions.json" >/dev/null 2>&1; then
  printf '%s\n' '{ "latest": null, "versions": [] }' > "$WT/versions.json"
fi
jq --arg ver "$VER" --arg base "$SITE_BASE" --arg today "$TODAY" '
  ((.versions // []) | map(select(.id != $ver and .id != "latest" and .id != "dev")))
  as $carried
  | {
      latest: $ver,
      versions: ([
        { id: "dev", label: "dev (main)", path: ($base + "/docs/dev/"),
          released: null, prerelease: true },
        { id: "latest", label: ("latest (" + $ver + ")"),
          path: ($base + "/docs/latest/"), released: $today, aliasOf: $ver },
        { id: $ver, label: $ver, path: ($base + "/docs/" + $ver + "/"),
          released: $today }
      ] + $carried)
    }
  ' "$WT/versions.json" > "$WT/versions.json.new"
mv "$WT/versions.json.new" "$WT/versions.json"

# 5. Commit + push docs-dist. CI runners have no git identity configured —
# without this the commit dies with `fatal: empty ident name` (v3.0.1 cut).
if ! git -C "$WT" config user.email >/dev/null 2>&1; then
  git -C "$WT" config user.name "github-actions[bot]"
  git -C "$WT" config user.email "41898282+github-actions[bot]@users.noreply.github.com"
fi
log "committing + pushing docs-dist"
git -C "$WT" add -A
if git -C "$WT" diff --cached --quiet; then
  log "nothing to commit"
else
  git -C "$WT" commit -m "docs: cut frozen documentation $VER" >/dev/null
  git -C "$WT" push origin docs-dist
fi

report true
log "done — /docs/$VER/ frozen; latest -> $VER"
