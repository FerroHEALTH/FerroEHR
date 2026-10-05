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
#   C. Commits-ahead cross-check — per MASTER-pinned component repo, the
#      default branch's distance ahead of the vendored pin commit. A and B are
#      key-granular and blind to raw commits (a key adopted once dedups away
#      every later commit under it; the amendment diff sees only NEW keys and
#      never computable/BMM/**) — which is how 24 BASE commits accumulated
#      silently (issue #341). One rolling issue per component: created when
#      ahead, body updated in place as the delta grows, auto-closed when a
#      re-vendor catches the pin up.
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
  line=$(grep -E "^  \"${comp}\|" scripts/vendor/spec-docs.sh | head -1 | tr -d '"') || true
  [[ -n "$line" ]] || { echo "unpinned"; return 0; }
  ref=$(echo "$line" | awk -F'|' '{print $3}')
  sha=$(echo "$line" | awk -F'|' '{printf "%.9s", $4}')
  # "master (BASE 1.3.0)" -> 1.3.0 · "Release-1.1.0" -> 1.1.0 · else the ref word
  ver=$(echo "$ref" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
  [[ -n "$ver" ]] || ver=$(echo "$ref" | awk '{print $1}')
  echo "$ver @ $sha"
}
repo_for_component() {
  local comp="$1" line
  line=$(grep -E "^  \"${comp}\|" scripts/vendor/spec-docs.sh | head -1 | tr -d '"') || true
  [[ -n "$line" ]] || return 1
  echo "$line" | awk -F'|' '{print $2}'
}
# The numeric version of our pin for a component, empty when the pin has no
# version number (development/master pins are not comparable).
pin_version_for_component() {
  pin_for_component "$1" | grep -oE '^[0-9]+\.[0-9]+\.[0-9]+' || true
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
    fv_ver=$(echo "$name" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)
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

# ── C. Commits-ahead cross-check (master-pinned components) ─────────────────
# Release-tag pins (e.g. QUERY, ITS-REST) are deliberately excluded: master
# being ahead of a release tag is the normal state between releases, and new
# releases are the release watcher's job (spec-release-watcher.yml).
echo "spec-update-watcher: commits-ahead cross-check against the vendored pins"
ahead_created=0 ahead_updated=0 ahead_closed=0
while IFS='|' read -r comp repo ref sha; do
  [[ -n "$comp" ]] || continue
  case "$ref" in master*) ;; *) continue ;; esac
  default=$(gh api "repos/openEHR/$repo" --jq '.default_branch') ||
    { echo "spec-update-watcher: failed to read openEHR/$repo default branch" >&2; exit 1; }
  ahead=$(gh api "repos/openEHR/$repo/compare/${sha}...${default}" --jq '.ahead_by') ||
    { echo "spec-update-watcher: failed to compare ${sha}...${default} for openEHR/$repo" >&2; exit 1; }
  head_sha=$(gh api "repos/openEHR/$repo/branches/$default" --jq '.commit.sha') ||
    { echo "spec-update-watcher: failed to read openEHR/$repo $default head" >&2; exit 1; }
  title="[spec-update] $comp spec repo is ahead of the vendored pin"
  num=$("$FILE_ISSUE" find --title "$title")
  if [[ "$ahead" -gt 0 ]]; then
    count_line="**Commits ahead of the pin:** $ahead (pin \`${sha:0:9}\`, upstream $default @ \`${head_sha:0:9}\`)"
    cat > "$tmp/ahead-body.md" <<EOF
Upstream \`openEHR/$repo\` ($comp) has moved past our vendored pin — commit-delta triage needed (the per-commit triage pattern that closed #341).

$count_line
- **Compare:** https://github.com/openEHR/$repo/compare/${sha}...${default}
- **Vendored pin:** \`scripts/vendor/spec-docs.sh\` (docs text); where this component feeds codegen, also the machine-readable vendor dirs (\`tools/openehr-codegen/vendor/bmm/\`, \`crates/openehr-its/{vendor,schemas}/\`)

### Triage checklist

- [ ] Baseline: full regen + drift check green against the CURRENT pin before changing inputs
- [ ] Triage every commit in the delta: prose / class-table / machine-readable codegen input
- [ ] Verify each behaviour-relevant item against the implementation (several may already be satisfied)
- [ ] Re-vendor + re-pin + regenerate; review the generated diff line by line
- [ ] CNF zero-drift + changelog + \`docs/VERSIONS.md\` if anything is wire-visible

_Maintained automatically by the spec-update watcher: the count above updates in place as the delta grows, and the issue closes when a re-vendor catches the pin up._
EOF
    if [[ -n "$num" ]] && gh issue view "$num" --json body --jq '.body' | grep -qF "$count_line"; then
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
    echo "  $comp is $ahead commit(s) ahead"
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
done < <(grep -E '^  "[A-Z-]+\|' scripts/vendor/spec-docs.sh | sed -e 's/^  "//' -e 's/"$//')
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
    if echo "$amendment_keys" | grep -qxF "$key"; then
      match=$(gh issue view "$num" --json state,labels)
    fi
    if [[ -n "$match" ]] &&
      [[ "$(echo "$match" | jq -r '.state')" = "OPEN" ]] &&
      [[ "$(echo "$match" | jq -r '[.labels[].name] | any(. == "blocked-upstream")')" = "true" ]]; then
      if [[ "$DRY_RUN" = "1" ]]; then
        echo "DRY-RUN would unblock #$num ($key): $summary"
      else
        gh issue comment "$num" --body "Auto-unblock (spec-update watcher): the normative text for $key has landed upstream — $summary. Re-vendor the component at pickup; this issue is implementable now — assign it to the current milestone when picked up (blocked issues carry no milestone)." >/dev/null
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
