#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Assemble the full Pages site tree into ./_site, exactly as CI does — so
# "works on my machine" == the deployed layout (URL scheme, frozen versions,
# and workflow all match the Docs CI job).
#
#   --dev-only   landing + /docs/dev/ + versions.json + 404 + robots
#                (the PR/build-job path — no frozen versions, no deploy)
#   --full       everything above PLUS the frozen /docs/vX.Y.Z/ trees from the
#                docs-dist branch, a fresh /docs/latest/ from the newest tag,
#                and a generated sitemap.xml (the deploy-job path)
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$PWD"
OUT="$ROOT/_site"
# The site is served from the ferroehr.eu apex (GitHub Pages custom domain),
# so it lives at the domain ROOT: SITE_BASE is empty. Both stay overridable so
# a sub-path build (e.g. a fork's project-pages URL) is still one env var away.
SITE_ORIGIN="${SITE_ORIGIN:-https://ferroehr.eu}"
SITE_BASE="${SITE_BASE:-}"
MODE="${1:---dev-only}"

log() { printf '\033[1;33m[build-site]\033[0m %s\n' "$*"; }

build_book() {
  # $1 = site-url (with trailing slash), $2 = absolute dest dir
  log "book -> $2  (site-url $1)"
  MDBOOK_OUTPUT__HTML__SITE_URL="$1" mdbook build "$ROOT/website/book" -d "$2" >/dev/null
}

# 1. Conformance claims are derived from the committed runner artifacts. The
#    book include is generated before mdbook runs, and the landing's data-cnf
#    markers are filled after the copy (step 3). Sources carry no numbers
#    (CI: scripts/checks/conformance-numbers.sh).
bash "$ROOT/scripts/render/conformance-stats.sh" includes
bash "$ROOT/scripts/render/comparison.sh"
bash "$ROOT/scripts/render/perf-assets.sh"
bash "$ROOT/scripts/render/cohort-bench.sh"
bash "$ROOT/scripts/render/storage-bench.sh"

# 2. Clean + recreate _site.
rm -rf "$OUT"
mkdir -p "$OUT"

# 3. Landing at the site root (relative-URL HTML, so no base-path rewriting);
#    conformance markers filled from the committed artifacts.
cp -R "$ROOT/website/landing/." "$OUT/"
bash "$ROOT/scripts/render/conformance-stats.sh" fill-html "$OUT/index.html"

# 4. The dev book at /docs/dev/.
build_book "$SITE_BASE/docs/dev/" "$OUT/docs/dev"

# 5. Version manifest (docs-dist copy wins in --full; else the in-repo dev stub).
VERSIONS_SRC="$ROOT/website/versions.json"
if [[ "$MODE" == "--full" && -f "$ROOT/docs-dist/versions.json" ]]; then
  VERSIONS_SRC="$ROOT/docs-dist/versions.json"
fi
#    Entries in docs-dist were written under whatever base the site used at the
#    time — `/ehrbase-rs` before the rename, `/ferroehr` before the ferroehr.eu
#    cutover — so their `path` values go stale on every move. Re-anchor each one
#    to the CURRENT SITE_BASE by keeping only the `/docs/<id>/` tail, which is
#    invariant. The frozen trees themselves are NOT rebuilt ("generate once"):
#    their internal links are relative, so only this manifest needs re-anchoring.
#    `sub` anchors on the first `/docs/` and keeps its tail, so a path already
#    written under the current base re-anchors to itself: idempotent.
jq --indent 2 --arg base "$SITE_BASE" '
  .versions |= map(
    if (.path | type) == "string" and (.path | test("/docs/"))
    then .path = $base + (.path | sub("^.*?(?=/docs/)"; ""))
    else . end
  )
' "$VERSIONS_SRC" > "$OUT/versions.json"

if [[ "$MODE" != "--full" ]]; then
  # The landing page links to /docs/latest/; alias it to the dev book so the
  # dev-only tree is self-consistent and the link gate stays meaningful.
  mkdir -p "$OUT/docs/latest"
  cp -R "$OUT/docs/dev/." "$OUT/docs/latest/"
  log "injecting per-page search metadata"
  cargo run -q -p docs-meta -- "$OUT" "$SITE_ORIGIN" "$SITE_BASE"
  log "dev-only site assembled at $OUT (latest aliased to dev)"
  exit 0
fi

# ── --full: frozen versions + latest + sitemap ──────────────────────────────

# 6. Materialize frozen /docs/vX.Y.Z/ trees from the docs-dist worktree.
if [[ -d "$ROOT/docs-dist/docs" ]]; then
  log "copying frozen versions from docs-dist"
  cp -R "$ROOT/docs-dist/docs/." "$OUT/docs/"
  # Each frozen tree carries mdBook's generated 404.html, whose asset and home
  # links are absolute and were baked with the base path in force when that
  # version was cut (`/ehrbase-rs/...`, then `/ferroehr/...`). Re-anchor those
  # to the CURRENT base in the ASSEMBLED OUTPUT only — docs-dist is untouched,
  # so "generate once, never rebuilt" still holds for the frozen trees.
  log "re-anchoring absolute links in frozen trees"
  # A legacy base is exactly one path segment between the quote and `/docs/`; an
  # already-correct `"/docs/..."` has no such segment and is left alone, and a
  # path already under the current base rewrites to itself — so this is
  # idempotent. Two expressions rather than a `["']` class so the pattern stays
  # readable under shell quoting, and BSD and GNU sed agree on it.
  patched=0
  while IFS= read -r html; do
    sed -E -e 's#"/[A-Za-z0-9._-]+/docs/#"'"$SITE_BASE"'/docs/#g' \
           -e "s#'/[A-Za-z0-9._-]+/docs/#'${SITE_BASE}/docs/#g" \
           "$html" > "$html.reanchored"
    if cmp -s "$html" "$html.reanchored"; then
      rm -f "$html.reanchored"
    else
      mv "$html.reanchored" "$html"
      patched=$((patched + 1))
    fi
  done < <(find "$OUT/docs" -name '*.html')
  echo "  re-anchored $patched file(s)"
else
  log "no docs-dist worktree — skipping frozen versions (dev-only content only)"
fi

# 7. Build /docs/latest/ fresh from the newest tag's sources (deep links work).
# An unreadable manifest, an absent `latest`, or `latest: "dev"` all yield the
# empty string, which the guard below reads as "no frozen tag to build from" —
# so a malformed file skips this step rather than failing the whole site build.
LATEST_TAG="$(jq -r 'if (.latest // "") == "dev" then "" else (.latest // "") end' \
  "$OUT/versions.json" 2>/dev/null || true)"
if [[ -n "$LATEST_TAG" ]] && git rev-parse -q --verify "refs/tags/$LATEST_TAG" >/dev/null 2>&1; then
  log "building /docs/latest/ from tag $LATEST_TAG"
  WT="$ROOT/.latest-src"
  rm -rf "$WT"
  git worktree add --force --detach "$WT" "$LATEST_TAG" >/dev/null
  MDBOOK_OUTPUT__HTML__SITE_URL="$SITE_BASE/docs/latest/" mdbook build "$WT/website/book" -d "$OUT/docs/latest" >/dev/null
  git worktree remove --force "$WT" >/dev/null 2>&1 || rm -rf "$WT"
else
  log "no resolvable latest tag — aliasing /docs/latest/ to the dev book"
  mkdir -p "$OUT/docs/latest"
  cp -R "$OUT/docs/dev/." "$OUT/docs/latest/"
fi

# Per-page search metadata (canonical + description + Open Graph +
# BreadcrumbList) — see tools/docs-meta for why this runs over the built HTML
# rather than the mdBook theme. Idempotent.
log "injecting per-page search metadata"
cargo run -q -p docs-meta -- "$OUT" "$SITE_ORIGIN" "$SITE_BASE"

# 8. sitemap.xml — landing + the pages of /docs/latest/ only (§2d).
log "generating sitemap.xml"
{
  echo '<?xml version="1.0" encoding="UTF-8"?>'
  echo '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">'
  host="${SITE_ORIGIN}${SITE_BASE}"
  echo "  <url><loc>${host}/</loc></url>"
  if [[ -d "$OUT/docs/latest" ]]; then
    find "$OUT/docs/latest" -name '*.html' | sort | while IFS= read -r f; do
      rel="${f#"$OUT"}"
      echo "  <url><loc>${host}${rel}</loc></url>"
    done
  fi
  echo '</urlset>'
} > "$OUT/sitemap.xml"

# 9. robots.txt — disallow dev + every archived non-latest version (§2d).
log "writing robots.txt disallows"
{
  echo '# Generated by scripts/site/build.sh --full.'
  echo 'User-agent: *'
  echo 'Allow: /'
  echo "Disallow: ${SITE_BASE}/docs/dev/"
  # Every archived non-latest version. An unreadable or malformed manifest emits
  # nothing rather than failing the build — the same tolerance the `latest` read
  # above has, since robots.txt disallows are an SEO nicety, not correctness.
  jq -r --arg base "$SITE_BASE" '
    .latest as $latest
    | .versions[]?
    | select(.aliasOf == null and .id != "dev" and .id != "latest" and .id != $latest)
    | "Disallow: \($base)/docs/\(.id)/"
  ' "$OUT/versions.json" 2>/dev/null || true
  echo ""
  echo "Sitemap: ${SITE_ORIGIN}${SITE_BASE}/sitemap.xml"
} > "$OUT/robots.txt"

log "full site assembled at $OUT"
