#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# openEHR RELEASE watcher (tracker issue #179) — the companion of
# scripts/watch/spec-update.sh: that one tracks individual completed spec
# CHANGES; this one makes sure a component RELEASE event is never missed
# again (ITS-REST 1.1.0 shipped 19-Jul-2026 and was discovered by hand).
#
# Mechanism (surface verified live 2026-07-20): the upstream
# openEHR/specifications-* repos tag releases as `Release-X.Y.Z` (with
# occasional `Release-X.Y.ZvN` re-cut suffixes — same release, normalized
# away; SM/CNF/ITS-JSON have never tagged and are skipped). Per component:
# take the HIGHEST Release-* version, and file ONE issue when it is newer
# than our vendored pin — routed per the version policy
# (docs/VERSIONS.md §Spec version policy): minor of a pinned line →
# re-vendor checklist; MAJOR → per-component generation decision; ITS-REST →
# always-latest adoption umbrella.
#
# Backfill guard + watermark: a release is filed only when NEWER than the
# pin (comparable pins), and the issue board is the dedup — the component and
# the version must both appear in the title, over open AND closed issues, which
# also matches hand-made adoption umbrellas (e.g. #178 "Adopt ITS-REST 1.1.0 …"),
# so covered releases are never re-filed. No state file.
#
# Filing goes through the watcher family's one engine (#2778),
# .github/actions/file-watcher-issue/file-issue.sh, which owns the single dedup
# search idiom; this script keeps the probe — the tag poll, the version
# comparison and the routing.
#
# Failure honesty: a tags-API transport failure or unexpected shape is a RED
# run; an empty tag list on a never-tagged repo is a logged skip. A release
# FOUND is a green run that files work, per the family's run-colour rule.
#
# Env: DRY_RUN=1 (report, create nothing) · GH_TOKEN/GITHUB_TOKEN for gh.
set -euo pipefail
cd "$(dirname "$0")/../.."

DRY_RUN="${DRY_RUN:-0}"
FILE_ISSUE=".github/actions/file-watcher-issue/file-issue.sh"
engine_dry=()
if [[ "$DRY_RUN" = "1" ]]; then
  engine_dry=(--dry-run)
fi
for bin in jq gh; do
  command -v "$bin" >/dev/null 2>&1 || { echo "spec-release-watcher: $bin is required" >&2; exit 1; }
done
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

label_for_component() {
  case "$1" in
    RM) echo "spec:RM" ;; BASE) echo "spec:BASE" ;; AM) echo "spec:AM" ;;
    LANG) echo "spec:LANG" ;; QUERY) echo "spec:QUERY" ;; TERM) echo "spec:TERM" ;;
    SM) echo "spec:SM" ;; CNF) echo "spec:CNF" ;; ITS-*) echo "spec:ITS" ;;
    *) echo "" ;;
  esac
}

filed=0 skipped=0
# component|repo|ref|sha rows from the vendor script (the single pin source).
while IFS='|' read -r comp repo ref sha; do
  [[ -n "$comp" ]] || continue
  # Highest Release-X.Y.Z tag (vN re-cut suffixes collapse to their base).
  if ! gh api "repos/openEHR/$repo/tags?per_page=100" --jq '[.[].name]' > "$tmp/tags.json" 2>"$tmp/gh-err"; then
    echo "spec-release-watcher: tags fetch FAILED for openEHR/$repo:" >&2
    cat "$tmp/gh-err" >&2
    exit 1
  fi
  latest=$(jq -r '.[] | select(test("^Release-[0-9]+\\.[0-9]+\\.[0-9]+(v[0-9]+)?$"))
                  | sub("^Release-"; "") | sub("v[0-9]+$"; "")' "$tmp/tags.json" |
           sort -uV | tail -1)
  if [[ -z "$latest" ]]; then
    echo "  $comp (openEHR/$repo): no Release-* tags — skipped"
    continue
  fi
  pin_ver=$(echo "$ref" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1 || true)

  if [[ -n "$pin_ver" ]]; then
    # Comparable pin: only a strictly NEWER release is a missed event.
    if [[ "$latest" = "$pin_ver" ]] ||
      [[ "$(printf '%s\n%s\n' "$latest" "$pin_ver" | sort -V | tail -1)" = "$pin_ver" ]]; then
      skipped=$((skipped + 1))
      continue
    fi
  fi
  # Board dedup — matches [spec-release] issues AND hand-made adoption
  # umbrellas that already carry the component + version in the title.
  covered=$("$FILE_ISSUE" find --state all --dedup-key "$comp" --dedup-key "$latest")
  if [[ -n "$covered" ]]; then
    echo "  $comp Release-$latest: already on the board (#$covered) — skipped"
    skipped=$((skipped + 1))
    continue
  fi

  # Routing per the version policy.
  pin_major="${pin_ver%%.*}"
  rel_major="${latest%%.*}"
  if [[ "$comp" = "ITS-REST" ]]; then
    routing="ITS-REST is single-version (always the latest RELEASED API): open an adoption umbrella like #178 — re-vendor the released tag, regenerate, implement the delta, sweep the served identity."
  elif [[ -n "$pin_ver" ]] && [[ "$rel_major" != "$pin_major" ]]; then
    routing="**MAJOR release** — incompatible by openEHR's release strategy: a per-component generation decision is required (dual generation via the am14/am24 codegen pattern only if the ecosystem runs both; otherwise cutover). See docs/VERSIONS.md §Spec version policy."
  else
    routing="Minor/patch of the pinned line (compatible superset): re-vendor at the release tag, regenerate, implement any behaviour delta, update docs/VERSIONS.md."
  fi

  title="[spec-release] $comp Release-$latest published"
  cat > "$tmp/body.md" <<EOF
Upstream published a new **$comp** release: **Release-$latest**
(https://specifications.openehr.org/releases/$comp/Release-$latest · tag on openEHR/$repo).

- **Our vendored pin:** ${ref} @ ${sha:0:9}
- **Routing (docs/VERSIONS.md §Spec version policy):** $routing

### Checklist

- [ ] Read the release notes / amendment records for the delta vs our pin
- [ ] Re-vendor at the release tag (\`scripts/vendor/spec-docs.sh\` + codegen inputs where applicable) — or record the generation decision if this is a major
- [ ] Regenerate + implement behaviour deltas; update \`docs/VERSIONS.md\` (pin + release ladder)
- [ ] ECC zero-drift run at close

_Opened automatically by \`.github/workflows/spec-release-watcher.yml\`._
EOF
  lbl=$(label_for_component "$comp")
  up_label="upstream:${comp}-${latest}"
  labels="spec-update,chore,$up_label"
  [[ -n "$lbl" ]] && labels="$labels,$lbl"
  if [[ "$DRY_RUN" = "1" ]]; then
    sed 's/^/    │ /' "$tmp/body.md"
  else
    # Ensure the per-release collection label exists. --force makes an existing
    # label a success (it re-applies the deterministic description/colour), so
    # a real failure — permissions, transport — fails the probe loudly HERE
    # instead of surfacing later as a confusing `gh issue create` refusal of a
    # label that never got made (#2797).
    gh label create "$up_label" --force --description "Changes arriving with the upstream $comp Release-$latest" --color BFD4F2
  fi
  # `--on-existing skip` re-runs the dedup at filing time: a race that lands a
  # covering issue between the check above and here must not double-file, and
  # the counters below follow the engine's reported outcome rather than assuming.
  engine_out=$("$FILE_ISSUE" file --title "$title" --body-file "$tmp/body.md" --type Task --labels "$labels" \
    --dedup-key "$comp" --dedup-key "$latest" --state all --on-existing skip \
    "${engine_dry[@]+"${engine_dry[@]}"}")
  printf '%s\n' "$engine_out"
  case "${engine_out##*$'\n'}" in
    *"file-issue: skipped"*) skipped=$((skipped + 1)) ;;
    *) filed=$((filed + 1)) ;;
  esac
done < <(grep -E '^  "[A-Z-]+\|' scripts/vendor/spec-docs.sh | tr -d '"' | sed 's/^  //')

echo "spec-release-watcher: done — $filed new release(s) filed, $skipped already covered/current."
