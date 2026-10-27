#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# openEHR spec-update watcher (tracker issue #137; design dossier: recorded on tracker issue #137
# — every mechanism live-verified 2026-07-20). Three detection sources; A and B
# file ONE GitHub issue per completed upstream spec change, C maintains ONE
# rolling issue per component:
#
#   A. Jira poll — anonymous search of openehr.atlassian.net for SPEC* issues
#      newly Resolved/Closed inside the window. Uses the enhanced
#      /rest/api/3/search/jql endpoint (token pagination; the classic
#      /rest/api/2/search is HTTP 410 GONE). `status in (Resolved, Closed)`
#      is the completion signal — statusCategory "done" over-captures
#      (In Review is green-category with resolution=null).
#   B. Amendment-record cross-check — the Jira keys present in each upstream
#      amendment record (default branch) but absent from the vendored copy
#      under docs/specs/openehr/ (paths mirror the upstream repos 1:1; pins
#      live in scripts/vendor/spec-docs.sh).
#   C. Commits-ahead cross-check — per pinned component repo (every
#      spec-docs.sh pin, plus ITS-BMM from its codegen provenance record), the
#      default branch's distance ahead of the vendored pin commit. A and B are
#      key-granular and blind to raw commits (a key adopted once dedups away
#      every later commit under it; the amendment diff sees only NEW keys and
#      never computable/BMM/**) — which is how 24 BASE commits accumulated
#      silently (issue #341). A master pin reports "ahead of the vendored pin"
#      (re-vendor triage); a Release-* tag pin reports "unreleased commits past
#      the vendored release" (look-ahead triage: the pin waits for a release).
#      One rolling issue per component: created when ahead, body updated in
#      place as the delta grows, auto-closed when a re-vendor catches the pin up.
#
# Dedup / watermark: the issue board itself — a key already present in ANY
# issue title (open or closed, searched with `--state all`) is never
# re-filed; a closed issue means "already triaged/implemented". No state
# file, no repo variables. (C dedups by its stable per-component title, open
# issues only — closed means caught-up, and a new delta reopens a fresh one.)
#
# Filing goes through the watcher family's one engine (#2778),
# .github/actions/file-watcher-issue/file-issue.sh, which owns the single dedup
# search idiom (an exact title for the rolling per-component issues of C, a key
# that must appear in the title for A/B/D); this script keeps the probes.
#
# Failure honesty: any non-2xx (one bounded retry on 429), unexpected JSON
# shape, or missing amendment path exits non-zero — the run goes RED. Only a
# successful poll that genuinely matches nothing is green-with-zero. A change
# FOUND is a green run that files work, per the family's run-colour rule.
#
# Usage: scripts/watch/spec-update.sh [--self-test]  (--self-test checks the
#      C helpers offline over fixture records)
# Env: DRY_RUN=1 (report, create nothing) · WINDOW_DAYS (default 14 — the scheduled cadence; run manually with a wide window, e.g. 365, to catch any backlog: dedup + the vendored baseline make that safe) ·
#      GH_TOKEN/GITHUB_TOKEN for gh. Requires curl, jq, gh.
set -euo pipefail
cd "$(dirname "$0")/../.."

JIRA="https://openehr.atlassian.net"
WINDOW_DAYS="${WINDOW_DAYS:-14}"
DRY_RUN="${DRY_RUN:-0}"
FILE_ISSUE=".github/actions/file-watcher-issue/file-issue.sh"
engine_dry=()
if [[ "$DRY_RUN" = "1" ]]; then
  engine_dry=(--dry-run)
fi
[[ "$WINDOW_DAYS" =~ ^[0-9]{1,4}$ ]] ||
  { echo "spec-update-watcher: WINDOW_DAYS must be a number of days, got '$WINDOW_DAYS'" >&2; exit 1; }
# Field separator for the candidate rows: ASCII unit separator — non-whitespace,
# so empty fields survive `read` (tab-IFS collapses consecutive tabs).
US=$'\x1f'
# The load-bearing SPEC* projects: the ones with a vendored pin + a spec:*
# label, plus SPECPR (cross-cutting problem reports). SPECPROC/CDS/INTG/PUB
# have neither a pin nor a label and are deliberately excluded.
PROJECTS="SPECRM,SPECBASE,SPECAM,SPECLANG,SPECQUERY,SPECITS,SPECTERM,SPECSM,SPECCNF,SPECPR"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# ── C helpers: the pinned rows, the pin kind and the rolling-issue text ─────
# ITS-BMM feeds codegen only, so its pin lives in the codegen-inputs
# provenance record rather than in spec-docs.sh.
BMM_PROVENANCE="tools/openehr-codegen/vendor/bmm/PROVENANCE.md"

# The kind of a human pin ref: `master…` is a branch pin, `Release-…` a
# release-tag pin; any other ref word fails, so a mislabelled pin cannot land
# silently in either class.
pin_kind() {
  case "$1" in
    master*) echo "branch" ;;
    Release-*) echo "release" ;;
    *) return 1 ;;
  esac
}

# One `comp|repo|ref|sha|record` row per spec-docs.sh pin, then the ITS-BMM
# row from its provenance record line "Pinned commit: `<sha>` (<ref>)".
rolling_rows() {
  local spec_docs=$1 bmm_record=$2 line sha ref tick=$'\x60'
  grep -E '^  "[A-Z-]+\|' "$spec_docs" | sed -e 's/^  "//' -e 's/"$//' |
    while IFS='|' read -r comp repo ref sha; do
      printf '%s|%s|%s|%s|%s\n' "$comp" "$repo" "$ref" "$sha" "$spec_docs"
    done
  line=$(grep -E "^Pinned commit: ${tick}[0-9a-f]{40}${tick} \\([^)]+\\)" "$bmm_record" | sed -n 1p) || return 1
  [[ -n "$line" ]] || return 1
  sha=$(sed -E "s/^Pinned commit: ${tick}([0-9a-f]{40})${tick}.*/\\1/" <<<"$line")
  ref=$(sed -E "s/^Pinned commit: ${tick}[0-9a-f]{40}${tick} \\(([^)]+)\\).*/\\1/" <<<"$line")
  printf 'ITS-BMM|specifications-ITS-BMM|%s|%s|%s\n' "$ref" "$sha" "$bmm_record"
}

# The stable title of a component's rolling issue. The branch-pin title is the
# one this watcher has always used, so existing issues keep deduping.
rolling_title() {
  local kind="$1" component="$2"
  case "$kind" in
    branch) echo "[spec-update] $component spec repo is ahead of the vendored pin" ;;
    release) echo "[spec-update] $component has unreleased commits past the vendored release" ;;
    *) return 1 ;;
  esac
}

# The count line a re-run compares against the open issue's body.
# Args: kind ahead ref sha default head_sha.
rolling_count_line() {
  local kind=$1 ahead=$2 ref=$3 sha=$4 default=$5 head_sha=$6
  case "$kind" in
    branch) echo "**Commits ahead of the pin:** $ahead (pin \`${sha:0:9}\`, upstream $default @ \`${head_sha:0:9}\`)" ;;
    release) echo "**Commits past the release:** $ahead (release \`${ref%% *}\` @ \`${sha:0:9}\`, upstream $default @ \`${head_sha:0:9}\`)" ;;
    *) return 1 ;;
  esac
}

# The rolling issue body. Args: kind comp repo sha default record count_line.
rolling_body() {
  local kind=$1 comp=$2 repo=$3 sha=$4 default=$5 record=$6 count_line=$7
  if [[ "$kind" = "release" ]]; then
    cat <<BODY
Upstream \`openEHR/$repo\` ($comp) has commits on $default past the release we vendor. They are unreleased and the pin moves only to a released tag (\`docs/VERSIONS.md\`, spec version policy), so this is look-ahead triage, not a re-vendor.

$count_line
- **Compare:** https://github.com/openEHR/$repo/compare/${sha}...${default}
- **Vendored pin:** \`$record\`

### Triage checklist

- [ ] Triage every commit in the delta: wire-visible / prose / machine-readable codegen input
- [ ] File a tracker issue for each change the implementation will need once a release carries it
- [ ] Re-vendor when a release tag carries the delta (the spec-release watcher reports new tags)

_Maintained automatically by the spec-update watcher: the count above updates in place as the delta grows, and the issue closes when a re-vendor catches the pin up._
BODY
  else
    cat <<BODY
Upstream \`openEHR/$repo\` ($comp) has moved past our vendored pin — commit-delta triage needed (the per-commit triage pattern that closed #341).

$count_line
- **Compare:** https://github.com/openEHR/$repo/compare/${sha}...${default}
- **Vendored pin:** \`$record\`; where this component feeds codegen, also the machine-readable vendor dirs (\`tools/openehr-codegen/vendor/bmm/\`, \`crates/openehr-its/{vendor,schemas}/\`, re-vendored by \`scripts/vendor/codegen-inputs.sh\`)

### Triage checklist

- [ ] Baseline: full regen + drift check green against the CURRENT pin before changing inputs
- [ ] Triage every commit in the delta: prose / class-table / machine-readable codegen input
- [ ] Verify each behaviour-relevant item against the implementation (several may already be satisfied)
- [ ] Re-vendor + re-pin + regenerate; review the generated diff line by line
- [ ] CNF zero-drift + changelog + \`docs/VERSIONS.md\` if anything is wire-visible

_Maintained automatically by the spec-update watcher: the count above updates in place as the delta grows, and the issue closes when a re-vendor catches the pin up._
BODY
  fi
}

# Offline checks of the C helpers over fixture records: no network, no gh.
self_test() {
  local dir="$tmp/self-test" fail=0 rows
  mkdir -p "$dir"
  cat > "$dir/spec-docs.sh" <<'FIXTURE'
PINS=(
  "RM|specifications-RM|master (RM 1.2.0)|1111111111111111111111111111111111111111"
  "ITS-REST|specifications-ITS-REST|Release-1.1.0 (released 19-Jul-2026)|2222222222222222222222222222222222222222"
)
FIXTURE
  printf 'Source: x\nPinned commit: \x60%s\x60 (master)\n' 3333333333333333333333333333333333333333 > "$dir/PROVENANCE.md"
  printf 'Source: x\nPinned commit: 3333 (master)\n' > "$dir/BROKEN.md"
  check() {
    local what="$1" actual="$2" expected="$3"
    if [[ "$actual" != "$expected" ]]; then
      echo "self-test: $what: got '$actual', want '$expected'" >&2
      fail=1
    fi
  }
  rows=$(rolling_rows "$dir/spec-docs.sh" "$dir/PROVENANCE.md")
  check "rows" "$rows" "RM|specifications-RM|master (RM 1.2.0)|1111111111111111111111111111111111111111|$dir/spec-docs.sh
ITS-REST|specifications-ITS-REST|Release-1.1.0 (released 19-Jul-2026)|2222222222222222222222222222222222222222|$dir/spec-docs.sh
ITS-BMM|specifications-ITS-BMM|master|3333333333333333333333333333333333333333|$dir/PROVENANCE.md"
  if rolling_rows "$dir/spec-docs.sh" "$dir/BROKEN.md" >/dev/null; then
    echo "self-test: a provenance record without a pinned commit was accepted" >&2
    fail=1
  fi
  check "branch kind" "$(pin_kind "master (RM 1.2.0)")" "branch"
  check "release kind" "$(pin_kind "Release-1.1.0 (released)")" "release"
  if pin_kind "development" >/dev/null; then
    echo "self-test: an unknown ref word was classified" >&2
    fail=1
  fi
  check "branch title" "$(rolling_title branch ITS-BMM)" "[spec-update] ITS-BMM spec repo is ahead of the vendored pin"
  check "release title" "$(rolling_title release ITS-REST)" "[spec-update] ITS-REST has unreleased commits past the vendored release"
  check "release count" \
    "$(rolling_count_line release 6 "Release-1.1.0 (released 19-Jul-2026)" 2222222222222222222222222222222222222222 master 4444444444444444444444444444444444444444)" \
    "**Commits past the release:** 6 (release \`Release-1.1.0\` @ \`222222222\`, upstream master @ \`444444444\`)"
  check "branch count" \
    "$(rolling_count_line branch 6 master 3333333333333333333333333333333333333333 master 4444444444444444444444444444444444444444)" \
    "**Commits ahead of the pin:** 6 (pin \`333333333\`, upstream master @ \`444444444\`)"
  if ! grep -qxF "COUNT" <<<"$(rolling_body release ITS-REST specifications-ITS-REST 2222 master "$dir/spec-docs.sh" "COUNT")"; then
    echo "self-test: the release body lacks its count line" >&2
    fail=1
  fi
  if ! grep -qF "\`$dir/PROVENANCE.md\`" <<<"$(rolling_body branch ITS-BMM specifications-ITS-BMM 3333 master "$dir/PROVENANCE.md" "COUNT")"; then
    echo "self-test: the branch body does not name its pin record" >&2
    fail=1
  fi
  if [[ "$fail" -ne 0 ]]; then
    exit 1
  fi
  echo "spec-update-watcher: self-test OK (pin rows incl. ITS-BMM, pin kinds, rolling titles, count lines, bodies)."
}

case "${1:-}" in
  "") ;;
  --self-test)
    self_test
    exit 0
    ;;
  *)
    echo "usage: $0 [--self-test]" >&2
    exit 2
    ;;
esac

for bin in curl jq gh; do
  command -v "$bin" >/dev/null 2>&1 || { echo "spec-update-watcher: $bin is required" >&2; exit 1; }
done

# ── component maps ───────────────────────────────────────────────────────────
label_for_project() {
  case "$1" in
    SPECRM) echo "spec:RM" ;; SPECBASE) echo "spec:BASE" ;; SPECAM) echo "spec:AM" ;;
    SPECLANG) echo "spec:LANG" ;; SPECQUERY) echo "spec:QUERY" ;; SPECITS) echo "spec:ITS" ;;
    SPECTERM) echo "spec:TERM" ;; SPECSM) echo "spec:SM" ;; SPECCNF) echo "spec:CNF" ;;
    *) echo "" ;; # SPECPR & friends: cross-component, spec-update only
  esac
}
component_for_project() {
  case "$1" in
    SPECRM) echo "RM" ;; SPECBASE) echo "BASE" ;; SPECAM) echo "AM" ;;
    SPECLANG) echo "LANG" ;; SPECQUERY) echo "QUERY" ;; SPECITS) echo "ITS-REST" ;;
    SPECTERM) echo "TERM" ;; SPECSM) echo "SM" ;; SPECCNF) echo "CNF" ;;
    *) echo "" ;;
  esac
}
label_for_component() { # vendored component dir -> label
  case "$1" in
    RM) echo "spec:RM" ;; BASE) echo "spec:BASE" ;; AM) echo "spec:AM" ;;
    LANG) echo "spec:LANG" ;; QUERY) echo "spec:QUERY" ;; TERM) echo "spec:TERM" ;;
    SM) echo "spec:SM" ;; CNF) echo "spec:CNF" ;; ITS-*) echo "spec:ITS" ;;
    *) echo "" ;;
  esac
}
# Our vendored pin, human-readable: the spec VERSION we currently have
# (parsed out of the vendor ref) + the vendored commit. This is the BASELINE
# ("what we already have"), never a ceiling — upstream changes targeting any
# newer version still get an issue.
pin_for_component() {
  local comp="$1" line ref sha ver
  line=$(grep -E "^  \"${comp}\|" scripts/vendor/spec-docs.sh | sed -n 1p | tr -d '"') || true
  [[ -n "$line" ]] || { echo "unpinned"; return 0; }
  ref=$(echo "$line" | awk -F'|' '{print $3}')
  sha=$(echo "$line" | awk -F'|' '{printf "%.9s", $4}')
  # "master (BASE 1.3.0)" -> 1.3.0 · "Release-1.1.0" -> 1.1.0 · else the ref word
  ver=$(echo "$ref" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | sed -n 1p)
  [[ -n "$ver" ]] || ver=$(echo "$ref" | awk '{print $1}')
  echo "$ver @ $sha"
}
repo_for_component() {
  local comp="$1" line
  line=$(grep -E "^  \"${comp}\|" scripts/vendor/spec-docs.sh | sed -n 1p | tr -d '"') || true
  [[ -n "$line" ]] || return 1
  echo "$line" | awk -F'|' '{print $2}'
}
# The numeric version of our pin for a component, empty when the pin has no
# version number (development/master pins are not comparable).
pin_version_for_component() {
  local component="$1"
  pin_for_component "$component" | grep -oE '^[0-9]+\.[0-9]+\.[0-9]+' || true
}
# Map a Jira fixVersion NAME to the pinned component it belongs to:
# "RM Release 1.1.0" -> RM · "TERM Release 2.3.0" -> TERM · "AM version
# 2.4.0"/"ADL 2.4.0" -> AM · "Release REST 1.1.0"/"ITS-REST Release …" ->
# ITS-REST · bare "Release-1.3.0" -> the issue's own project component.
component_for_fixversion() {
  local name="$1" fallback="$2"
  case "$name" in
    RM\ *) echo "RM" ;; BASE\ *) echo "BASE" ;;
    AM\ *|ADL\ *|AOM\ *|OPT\ *) echo "AM" ;;
    LANG\ *) echo "LANG" ;; QUERY\ *|AQL\ *) echo "QUERY" ;;
    TERM\ *) echo "TERM" ;; SM\ *) echo "SM" ;; CNF\ *) echo "CNF" ;;
    ITS-REST*|*REST\ *) echo "ITS-REST" ;;
    *) echo "$fallback" ;;
  esac
}
# True (0) when EVERY fixVersion of the candidate is comparable and STRICTLY
# older than our pin for its component — the change is already inside the
# vendored spec text. Equal or unparseable versions keep the candidate
# (an equal-version target may postdate our vendored commit; triage decides).
all_fixversions_inside_pin() {
  local fixv_names="$1" fallback_comp="$2" name comp fv_ver pin_ver any=1
  [[ -n "$fixv_names" ]] || return 1
  while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    any=0
    comp=$(component_for_fixversion "$name" "$fallback_comp")
    [[ -n "$comp" ]] || return 1
    fv_ver=$(echo "$name" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | sed -n 1p)
    [[ -n "$fv_ver" ]] || return 1
    pin_ver=$(pin_version_for_component "$comp")
    [[ -n "$pin_ver" ]] || return 1
    [[ "$fv_ver" = "$pin_ver" ]] && return 1
    # fv_ver < pin_ver ⟺ the version-sorted max is the pin
    [[ "$(printf '%s\n%s\n' "$fv_ver" "$pin_ver" | sort -V | tail -1)" = "$pin_ver" ]] || return 1
  done <<< "$fixv_names"
  return $any
}

# ── shared: candidate sink (key<TAB>component<TAB>summary<TAB>extra) ─────────
CANDIDATES="$tmp/candidates.tsv"
: > "$CANDIDATES"

# ── A. Jira poll ─────────────────────────────────────────────────────────────
jira_fetch() { # $1 = full URL; body on stdout; one bounded retry on 429
  local url="$1" code attempt
  for attempt in 1 2; do
    code=$(curl -sS -o "$tmp/resp.json" -w '%{http_code}' "$url") ||
      { echo "spec-update-watcher: curl transport failure for $url" >&2; return 1; }
    case "$code" in
      200) cat "$tmp/resp.json"; return 0 ;;
      429) [[ "$attempt" = 1 ]] && { echo "Jira 429 — one bounded retry in 30s" >&2; sleep 30; continue; } ;;
      *) ;;
    esac
    echo "spec-update-watcher: Jira returned HTTP $code for $url" >&2
    head -c 500 "$tmp/resp.json" >&2 || true
    return 1
  done
}

jql="project in (${PROJECTS}) AND status in (Resolved, Closed) AND resolutiondate >= -${WINDOW_DAYS}d ORDER BY resolutiondate DESC"
jql_enc=$(jq -rn --arg s "$jql" '$s|@uri')
fields="summary,status,resolution,resolutiondate,created,issuetype,fixVersions,components,description"
echo "spec-update-watcher: Jira poll — window ${WINDOW_DAYS}d, projects ${PROJECTS}"

: > "$tmp/jira.jsonl"
token=""
while :; do
  url="$JIRA/rest/api/3/search/jql?maxResults=50&fields=$fields&jql=$jql_enc"
  [[ -n "$token" ]] && url="$url&nextPageToken=$(jq -rn --arg s "$token" '$s|@uri')"
  body=$(jira_fetch "$url")
  echo "$body" | jq -e 'has("issues") and has("isLast")' >/dev/null ||
    { echo "spec-update-watcher: unexpected Jira payload shape (no issues/isLast) — API changed?" >&2; exit 1; }
  echo "$body" | jq -c '.issues[]' >> "$tmp/jira.jsonl"
  [[ "$(echo "$body" | jq -r '.isLast')" = "true" ]] && break
  token=$(echo "$body" | jq -r '.nextPageToken // empty')
  [[ -n "$token" ]] || { echo "spec-update-watcher: isLast=false but no nextPageToken" >&2; exit 1; }
done

jira_count=$(wc -l < "$tmp/jira.jsonl" | tr -d ' ')
echo "spec-update-watcher: Jira poll matched $jira_count completed issue(s)"

# Keys already present in the VENDORED amendment records: the change is
# already inside our pinned spec text — no triage needed, skip with a note.
# (Explicit file list — grep --include on explicit args differs across
# GNU/BSD grep flavors.)
{ find docs/specs/openehr -name 'master00-amendment_record.adoc'
  echo docs/specs/openehr/ITS-REST/specifications/docs/overview/Amendment_record.md
} | xargs grep -hoE 'SPEC[A-Z]*-[0-9]+' | sort -u > "$tmp/vendored-keys"

while IFS= read -r issue; do
  key=$(echo "$issue" | jq -r '.key')
  project="${key%%-*}"
  if grep -qxF "$key" "$tmp/vendored-keys"; then
    echo "  $key: already inside the vendored pin (amendment record carries it) — skipped"
    continue
  fi
  # A rejected/duplicate resolution means NOTHING changed in the spec — no
  # triage material. Skip with a note (still deduped later if it ever flips).
  # "Rejected" was missing from this list, which is how SPECTERM-30
  # (resolution: Rejected) got filed as tracker work (issue #175 / bug #340);
  # "Superseded" likewise (SPECPR-124 → issue #347): the successor ticket is
  # detected on its own resolution, the superseded one itself lands nothing.
  resolution=$(echo "$issue" | jq -r '.fields.resolution.name // "unresolved"')
  case "$resolution" in
    "Rejected"|"Superseded"|"Won't Do"|"Won't Fix"|"Duplicate"|"Cannot Reproduce"|"Declined"|"Abandoned"|"Not a Bug")
      echo "  $key: resolution '$resolution' — no spec change, skipped"
      continue ;;
    *) ;;
  esac
  component=$(component_for_project "$project")
  # A change whose fix versions are ALL strictly older than our pins is
  # already inside the vendored spec text (catches cross-project SPECPR
  # keys the amendment-record baseline cannot see).
  fixv_lines=$(echo "$issue" | jq -r '.fields.fixVersions[]?.name')
  if all_fixversions_inside_pin "$fixv_lines" "$component"; then
    echo "  $key: fix version(s) [$(echo "$fixv_lines" | paste -sd, -)] predate our pins — already in the vendored text, skipped"
    continue
  fi
  summary=$(echo "$issue" | jq -r '.fields.summary')
  status=$(echo "$issue" | jq -r '.fields.status.name // "unknown"')
  itype=$(echo "$issue" | jq -r '.fields.issuetype.name // "unknown"')
  created=$(echo "$issue" | jq -r '.fields.created // "unknown" | split("T")[0]')
  resolved=$(echo "$issue" | jq -r '.fields.resolutiondate // "unknown" | split("T")[0]')
  # Fix versions with their release state: "Release-1.3.0 (unreleased)".
  fixv=$(echo "$issue" | jq -r '[.fields.fixVersions[]? | "\(.name) (\(if .released then "released" else "unreleased" end))"] | join(", ") | if . == "" then "—" else . end')
  fixv_plain=$(echo "$issue" | jq -r '[.fields.fixVersions[]?.name] | join(", ") | if . == "" then "—" else . end')
  comps=$(echo "$issue" | jq -r '[.fields.components[]?.name] | join(", ") | if . == "" then "—" else . end')
  # Description arrives as Atlassian Document Format — flatten the text
  # leaves and truncate to a triage-sized excerpt.
  descr=$(echo "$issue" | jq -r '[.fields.description // {} | .. | .text? // empty] | join(" ") | .[0:700]' | tr "$US" ' ' | tr '\n' ' ')
  printf "%s${US}%s${US}%s${US}jira${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s\n" \
    "$key" "$component" "$summary" "$resolved" "$fixv" "$comps" \
    "$status" "$resolution" "$itype" "$created" "$fixv_plain" "$descr" >> "$CANDIDATES"
done < "$tmp/jira.jsonl"

# ── B. Amendment-record cross-check ─────────────────────────────────────────
echo "spec-update-watcher: amendment-record cross-check against the vendored mirror"
amendment_files=$(find docs/specs/openehr -name "master00-amendment_record.adoc" | sort)
amendment_files="$amendment_files
docs/specs/openehr/ITS-REST/specifications/docs/overview/Amendment_record.md"

while IFS= read -r local_path; do
  [[ -f "$local_path" ]] || { echo "spec-update-watcher: vendored amendment file missing: $local_path" >&2; exit 1; }
  comp=${local_path#docs/specs/openehr/}; comp=${comp%%/*}
  rel=${local_path#docs/specs/openehr/"$comp"/}
  repo=$(repo_for_component "$comp") ||
    { echo "spec-update-watcher: no upstream repo mapping for component $comp" >&2; exit 1; }
  # Default-branch content via the contents API (gh handles auth + rate limits);
  # a 404 here means the upstream moved the file — that must be a RED run.
  if ! gh api "repos/openEHR/$repo/contents/$rel" --jq '.content' 2>"$tmp/gh-err" | base64 -d > "$tmp/upstream" 2>/dev/null; then
    echo "spec-update-watcher: failed to fetch openEHR/$repo/$rel (moved upstream?):" >&2
    cat "$tmp/gh-err" >&2
    exit 1
  fi
  new_keys=$(comm -23 \
    <(grep -oE 'SPEC[A-Z]*-[0-9]+' "$tmp/upstream" | sort -u) \
    <(grep -oE 'SPEC[A-Z]*-[0-9]+' "$local_path" | sort -u) || true)
  if [[ -n "$new_keys" ]]; then
    while IFS= read -r key; do
      printf "%s${US}%s${US}%s${US}amendment${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s\n" \
        "$key" "$comp" "new amendment-record row in openEHR/$repo/$rel" \
        "see amendment record" "—" "—" \
        "(amendment)" "(amendment)" "amendment row" "—" "—" \
        "New row referencing $key in the upstream amendment record openEHR/$repo/$rel — not present in our vendored copy." >> "$CANDIDATES"
    done <<< "$new_keys"
  fi
done <<< "$amendment_files"

# ── C. Commits-ahead cross-check (every pinned component repo) ──────────────
# One rolling issue per component and pin kind (rolling_title): created when
# the default branch is ahead of the pin, updated in place as the delta grows,
# auto-closed when a re-vendor catches the pin up.
echo "spec-update-watcher: commits-ahead cross-check against the vendored pins"
ahead_created=0 ahead_updated=0 ahead_closed=0
rolling_rows scripts/vendor/spec-docs.sh "$BMM_PROVENANCE" > "$tmp/rows" ||
  { echo "spec-update-watcher: no pinned commit in $BMM_PROVENANCE" >&2; exit 1; }
while IFS='|' read -r comp repo ref sha record; do
  [[ -n "$comp" ]] || continue
  kind=$(pin_kind "$ref") ||
    { echo "spec-update-watcher: $comp ref '$ref' names neither master nor a Release-* tag ($record)" >&2; exit 1; }
  default=$(gh api "repos/openEHR/$repo" --jq '.default_branch') ||
    { echo "spec-update-watcher: failed to read openEHR/$repo default branch" >&2; exit 1; }
  ahead=$(gh api "repos/openEHR/$repo/compare/${sha}...${default}" --jq '.ahead_by') ||
    { echo "spec-update-watcher: failed to compare ${sha}...${default} for openEHR/$repo" >&2; exit 1; }
  head_sha=$(gh api "repos/openEHR/$repo/branches/$default" --jq '.commit.sha') ||
    { echo "spec-update-watcher: failed to read openEHR/$repo $default head" >&2; exit 1; }
  title=$(rolling_title "$kind" "$comp")
  num=$("$FILE_ISSUE" find --title "$title")
  if [[ "$ahead" -gt 0 ]]; then
    count_line=$(rolling_count_line "$kind" "$ahead" "$ref" "$sha" "$default" "$head_sha")
    rolling_body "$kind" "$comp" "$repo" "$sha" "$default" "$record" "$count_line" > "$tmp/ahead-body.md"
    if [[ -n "$num" ]] && grep -qF "$count_line" <<<"$(gh issue view "$num" --json body --jq '.body')"; then
      continue # unchanged since the last run
    fi
    labels="spec-update,chore"
    lbl=$(label_for_component "$comp" || true)
    [[ -n "${lbl:-}" ]] && labels="$labels,$lbl"
    if [[ "$DRY_RUN" = "1" ]] && [[ -z "$num" ]]; then
      sed 's/^/    │ /' "$tmp/ahead-body.md"
    fi
    # One engine call covers both branches: create the rolling issue when it
    # does not exist, replace its body in place when it does (the count updates
    # as the delta grows).
    "$FILE_ISSUE" file --title "$title" --body-file "$tmp/ahead-body.md" \
      --type Task --labels "$labels" --on-existing update "${engine_dry[@]+"${engine_dry[@]}"}"
    echo "  $comp is $ahead commit(s) ahead ($kind pin)"
    if [[ -n "$num" ]]; then
      ahead_updated=$((ahead_updated + 1))
    else
      ahead_created=$((ahead_created + 1))
    fi
  elif [[ -n "$num" ]]; then
    if [[ "$DRY_RUN" = "1" ]]; then
      echo "DRY-RUN would close #$num: $title (pin caught up)"
    else
      gh issue comment "$num" --body "Auto-close (spec-update watcher): the vendored pin has caught up with upstream \`openEHR/$repo\` $default — 0 commits ahead." >/dev/null
      gh issue close "$num" >/dev/null
      echo "closed #$num: $comp pin caught up"
    fi
    ahead_closed=$((ahead_closed + 1))
  fi
done < "$tmp/rows"
echo "spec-update-watcher: commits-ahead — $ahead_created created, $ahead_updated updated, $ahead_closed closed"

# ── D. Dedup by key against the issue board (open AND closed), create ───────
# Sort so the richer Jira-sourced row wins over an amendment duplicate.
sort -t"$US" -k1,1 -k4,4r "$CANDIDATES" | awk -F"$US" '!seen[$1]++' > "$tmp/unique.tsv"

# Keys the AMENDMENT cross-check detected — the text-has-landed evidence for
# the auto-unblock below. Collected independently of the dedup's source
# priority (a Jira-window hit for the same key must not mask the landing).
amendment_keys=$(awk -F"$US" '$4 == "amendment" { print $1 }' "$CANDIDATES" | sort -u)

created=0 skipped=0 unblocked=0
while IFS="$US" read -r key component summary source resolved fixv comps status resolution itype jcreated fixv_plain descr; do
  [[ -n "$key" ]] || continue
  num=$("$FILE_ISSUE" find --state all --dedup-key "$key")
  if [[ -n "$num" ]]; then
    # AUTO-UNBLOCK: an amendment-diff hit means the key's normative text has
    # now LANDED upstream. If the board carries this key as an OPEN issue
    # labelled blocked-upstream (resolved in Jira before the text was
    # published), announce the landing and drop the label; every other
    # existing-issue hit keeps the silent dedup skip. The state and labels are
    # read only on that path — the dedup itself needs the number alone.
    match=""
    if grep -qxF "$key" <<<"$amendment_keys"; then
      match=$(gh issue view "$num" --json state,labels)
    fi
    if [[ -n "$match" ]] &&
      [[ "$(echo "$match" | jq -r '.state')" = "OPEN" ]] &&
      [[ "$(echo "$match" | jq -r '[.labels[].name] | any(. == "blocked-upstream")')" = "true" ]]; then
      if [[ "$DRY_RUN" = "1" ]]; then
        echo "DRY-RUN would unblock #$num ($key): $summary"
      else
        gh issue comment "$num" --body "Auto-unblock (spec-update watcher): the normative text for $key has landed upstream — $summary. Re-vendor the component at pickup; this issue is implementable now — move it to the current milestone when picked up." >/dev/null
        gh issue edit "$num" --remove-label blocked-upstream >/dev/null
        echo "unblocked #$num ($key)"
      fi
      unblocked=$((unblocked + 1))
    fi
    skipped=$((skipped + 1))
    continue
  fi
  # Title = key + summary, nothing else (owner ruling: component and target
  # version live in the BODY and the spec:* label, never as title clutter).
  # Long Jira summaries (some embed whole URLs) are capped for readability;
  # dedup only needs the key, which always leads.
  if [[ -n "$component" ]]; then
    pin=$(pin_for_component "$component")
  else
    pin="n/a"
  fi
  short_summary="$summary"
  if [[ "${#short_summary}" -gt 90 ]]; then
    short_summary="${short_summary:0:87}..."
  fi
  title="[spec-update] $key — $short_summary"
  labels="spec-update,chore"
  lbl=$([[ -n "$component" ]] && label_for_component "$component" || true)
  [[ -n "${lbl:-}" ]] && labels="$labels,$lbl"

  cat > "$tmp/body.md" <<EOF
Upstream openEHR spec change completed — conformance-impact triage needed.

- **Jira:** [$key]($JIRA/browse/$key) — $itype
- **Upstream state:** $status / resolution: $resolution
- **Created:** $jcreated · **Completed (resolved):** $resolved
- **Lands in upstream version:** $fixv
- **Jira component(s):** $comps
- **Detected via:** $source poll
- **What we currently have vendored (the baseline this is newer than):** ${component:-n/a} ${pin} (\`docs/VERSIONS.md\` / \`scripts/vendor/spec-docs.sh\`)

### Upstream summary

$summary

$([[ -n "$descr" ]] && printf '<details><summary>Upstream description (excerpt)</summary>\n\n%s\n\n</details>' "$descr")

### Triage checklist

- [ ] Re-vendor the affected spec (\`scripts/vendor/spec-docs.sh\` — bump the pin + SHA)
- [ ] Regenerate if BMM/OAS/XSD changed (\`/regen-codegen\`)
- [ ] Assess behaviour impact — add exactly one \`spec-impact:*\` label
- [ ] Implement where behaviour changes; update \`docs/VERSIONS.md\`

_Opened automatically by \`.github/workflows/spec-update-watcher.yml\`._
EOF

  if [[ "$DRY_RUN" = "1" ]]; then
    sed 's/^/    │ /' "$tmp/body.md"
  fi
  # `--on-existing skip` re-runs the dedup at filing time: a race that lands a
  # covering issue between the check above and here must not double-file, and
  # the counters below follow the engine's reported outcome rather than assuming.
  engine_out=$("$FILE_ISSUE" file --title "$title" --body-file "$tmp/body.md" --type Task --labels "$labels" \
    --dedup-key "$key" --state all --on-existing skip "${engine_dry[@]+"${engine_dry[@]}"}")
  printf '%s\n' "$engine_out"
  case "${engine_out##*$'\n'}" in
    *"file-issue: skipped"*) skipped=$((skipped + 1)) ;;
    *) created=$((created + 1)) ;;
  esac
done < "$tmp/unique.tsv"

echo "spec-update-watcher: done — $created new, $unblocked unblocked, $skipped already on the board (dedup by key)."
