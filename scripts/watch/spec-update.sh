#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# openEHR spec-update watcher (tracker issue #137; design dossier: recorded on tracker issue #137
# — every mechanism live-verified 2026-07-20; the actionable-only filtering is
# #5364). Three detection sources feed two kinds of issue: ONE rolling issue
# per pinned component repository, and ONE per-key issue for a change no
# rolling issue carries.
#
#   C. Commits-ahead cross-check, run FIRST because A and B route by it — per
#      pinned component repo (every spec-docs.sh pin, ITS-BMM from its codegen
#      provenance record, and adl-antlr from the grammar vendor scripts), the
#      commits on the default branch past the vendored pin. Each commit is
#      classified by the paths it touches (commit_class): machine-readable,
#      normative text or other. A master pin reports "ahead of the vendored
#      pin" (re-vendor triage, `spec-version:current`); a Release-* pin reports
#      "unreleased commits past the vendored release" (look-ahead triage,
#      `spec-version:next`). The rolling issue lists the delta's commits by
#      class and the Jira keys the delta carries, inside a marked block that a
#      re-run replaces; the rest of the body is the triager's. A delta of
#      "other" commits alone opens no issue.
#   A. Jira poll — anonymous search of openehr.atlassian.net for SPEC* issues
#      newly Resolved/Closed inside the window. Uses the enhanced
#      /rest/api/3/search/jql endpoint (token pagination; the classic
#      /rest/api/2/search is HTTP 410 GONE). `status in (Resolved, Closed)`
#      is the completion signal — statusCategory "done" over-captures
#      (In Review is green-category with resolution=null). A candidate is
#      skipped when its Jira component is `Documentation`, or when no commit
#      past our pin in any watched repository names its key (the change is
#      inside the vendored text, or never reached a repository).
#   B. Amendment-record cross-check — the key that heads each upstream
#      amendment row (row_own_keys: the row's own ticket, never one it cites
#      with "see", "see also" or "addresses") absent from the vendored copy
#      under docs/specs/openehr/ (paths mirror the upstream repos 1:1; pins
#      live in scripts/vendor/spec-docs.sh).
#
# Routing: a key from A or B whose change lands in a MASTER-pinned
# component's repository files no issue of its own, because that component's
# rolling issue is the work item (the pin follows master) and lists the key.
# A key that lands only in a release-pinned repository gets its own issue,
# labelled `spec-version:next`.
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
#      helpers offline over fixtures: pin rows, amendment-row keys, commit
#      classes and the marked rolling block)
# Env: DRY_RUN=1 (report, create nothing, print the skipped set) · WINDOW_DAYS (default 14 — the scheduled cadence; run manually with a wide window, e.g. 365, to catch any backlog: dedup + the vendored baseline make that safe) ·
#      GH_TOKEN/GITHUB_TOKEN for gh. Requires curl, jq, gh, perl.
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
# The markers around the part of a rolling issue body the watcher owns.
BLOCK_BEGIN="<!-- spec-update:count -->"
BLOCK_END="<!-- /spec-update:count -->"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# ── C helpers: the pinned rows, the pin kind and the rolling-issue text ─────
# ITS-BMM feeds codegen only, so its pin lives in the codegen-inputs
# provenance record rather than in spec-docs.sh.
BMM_PROVENANCE="tools/openehr-codegen/vendor/bmm/PROVENANCE.md"
ADL_GRAMMARS="scripts/vendor/adl-grammars.sh"
LANG_GRAMMARS="scripts/vendor/lang-grammars.sh"

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

# The triage label a pin kind starts an issue with.
version_label() {
  case "$1" in
    branch) echo "spec-version:current" ;;
    release) echo "spec-version:next" ;;
    *) return 1 ;;
  esac
}

# The adl-antlr commit a grammar vendor script pins (ADL_ANTLR_COMMIT="<sha>").
grammar_pin() {
  local line
  line=$(grep -E '^ADL_ANTLR_COMMIT="[0-9a-f]{40}"' "$1" | sed -n 1p) || return 1
  [[ -n "$line" ]] || return 1
  sed -E 's/^ADL_ANTLR_COMMIT="([0-9a-f]{40})".*/\1/' <<<"$line"
}

# One `comp|repo|ref|sha|record` row per spec-docs.sh pin, then the ITS-BMM
# row from its provenance record line "Pinned commit: `<sha>` (<ref>)", then
# the two adl-antlr rows: the AM grammars follow master, and the LANG
# grammars keep a deliberate pin of the LANG 1.1.0 era, which reports as
# look-ahead.
rolling_rows() {
  local spec_docs=$1 bmm_record=$2 adl_script=$3 lang_script=$4 line sha ref tick=$'\x60'
  grep -E '^  "[A-Z-]+\|' "$spec_docs" | sed -e 's/^  "//' -e 's/"$//' |
    while IFS='|' read -r comp repo ref sha; do
      printf '%s|%s|%s|%s|%s\n' "$comp" "$repo" "$ref" "$sha" "$spec_docs"
    done
  line=$(grep -E "^Pinned commit: ${tick}[0-9a-f]{40}${tick} \\([^)]+\\)" "$bmm_record" | sed -n 1p) || return 1
  [[ -n "$line" ]] || return 1
  sha=$(sed -E "s/^Pinned commit: ${tick}([0-9a-f]{40})${tick}.*/\\1/" <<<"$line")
  ref=$(sed -E "s/^Pinned commit: ${tick}[0-9a-f]{40}${tick} \\(([^)]+)\\).*/\\1/" <<<"$line")
  printf 'ITS-BMM|specifications-ITS-BMM|%s|%s|%s\n' "$ref" "$sha" "$bmm_record"
  sha=$(grammar_pin "$adl_script") || return 1
  printf 'ADL-ANTLR|adl-antlr|master (AM grammars)|%s|%s\n' "$sha" "$adl_script"
  sha=$(grammar_pin "$lang_script") || return 1
  printf 'LANG-ANTLR|adl-antlr|Release-1.1.0 era (LANG grammars)|%s|%s\n' "$sha" "$lang_script"
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

# The count line of a rolling issue's marked block.
# Args: kind ahead ref sha default head_sha.
rolling_count_line() {
  local kind=$1 ahead=$2 ref=$3 sha=$4 default=$5 head_sha=$6
  case "$kind" in
    branch) echo "**Commits ahead of the pin:** $ahead (pin \`${sha:0:9}\`, upstream $default @ \`${head_sha:0:9}\`)" ;;
    release) echo "**Commits past the release:** $ahead (release \`${ref%% *}\` @ \`${sha:0:9}\`, upstream $default @ \`${head_sha:0:9}\`)" ;;
    *) return 1 ;;
  esac
}

# The class of one commit from the paths it touches, read one per line on
# stdin: `machine` (computable/**, BMM JSON, ANTLR grammars, XSD, OpenAPI,
# terminology XML) outranks `normative` (the docs/** chapters other than the
# amendment records), which outranks `other` (rendered HTML, repository
# tooling, amendment records alone).
commit_class() {
  local path class=other
  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    case "$path" in
      computable/*|*/computable/*|*.bmm.json|*.g4|*.xsd|*.openapi.yaml|*.openapi.yml)
        echo machine
        return 0 ;;
      *.xml)
        if [[ "$(tr '[:upper:]' '[:lower:]' <<<"$path")" == *terminology* ]]; then
          echo machine
          return 0
        fi ;;
      *master00-amendment_record.adoc|*Amendment_record.md) ;;
      docs/*.adoc|docs/*.md|*/docs/*.adoc|*/docs/*.md) class=normative ;;
      *) ;;
    esac
  done
  echo "$class"
}

# One line per row of an amendment record (stdin), the row's text joined.
# An AsciiDoc row opens at its issue-number cell (`|2.4.3`, optionally
# anchored `|[[latest_issue]]2.4.5`); a release header or a table edge closes
# it. A Markdown record (ITS-REST) keeps each row in one `<tr>` element.
amendment_rows() {
  awk '
    function flush() { if (row != "") print row; row = "" }
    /^<tr>|^[[:space:]]*<tr>/ { flush(); inrow = 1; html = 1; next }
    /<\/tr>/ { if (html) { flush(); inrow = 0 }; next }
    /^\|(\[\[[a-z_]+\]\])?[0-9][0-9.]*[[:space:]]*$/ { flush(); inrow = 1; html = 0; row = $0; next }
    /^\|===/ || /^[0-9]+\+/ { flush(); inrow = 0; next }
    inrow { row = row " " $0 }
    END { flush() }
  '
}

# The key that heads each amendment row on stdin (one row per line), one per
# line: the first SPEC* key once every key the row only cites ("see", "see
# also", "addresses", "addressed by", with the keys listed after it) is
# removed. A row naming no key of its own prints nothing.
row_own_keys() {
  perl -ne '
    my $ref = qr{(?:\{spec_tickets\}/SPEC[A-Z]*-[0-9]+\[[^\]]*\]|<a\s[^>]*>SPEC[A-Z]*-[0-9]+</a>|SPEC[A-Z]*-[0-9]+)};
    s{\b(?:see(?:\s+also)?|addresse[sd](?:\s+by)?)\s*:?\s*$ref(?:\s*(?:,|and)\s*$ref)*}{}gi;
    print "$1\n" if /(SPEC[A-Z]*-[0-9]+)/;
  '
}

# The marked block of a rolling issue. Args: count_line commits_file keys_file.
# commits_file: `sha<US>class<US>subject` per classified commit, in order;
# keys_file: `key<US>source` per key the delta carries.
rolling_block() {
  local count_line=$1 commits=$2 keys=$3 n_machine n_normative n_other
  n_machine=$(awk -F"$US" '$2 == "machine"' "$commits" | wc -l | tr -d ' ')
  n_normative=$(awk -F"$US" '$2 == "normative"' "$commits" | wc -l | tr -d ' ')
  n_other=$(awk -F"$US" '$2 == "other"' "$commits" | wc -l | tr -d ' ')
  echo "$BLOCK_BEGIN"
  echo "$count_line"
  echo "**Delta by kind:** $n_machine machine-readable, $n_normative normative text, $n_other other (merge commits not counted)"
  if [[ -s "$keys" ]]; then
    echo "**Keys this delta carries:** $(sort -u "$keys" | awk -F"$US" '{ printf "%s%s (%s)", sep, $1, $2; sep = ", " }')"
  else
    echo "**Keys this delta carries:** none named"
  fi
  echo
  local class label
  for class in machine normative other; do
    case "$class" in
      machine) label="Machine-readable" ;;
      normative) label="Normative text" ;;
      *) label="Other (rendered HTML, tooling, amendment records)" ;;
    esac
    if awk -F"$US" -v c="$class" '$2 == c { found = 1 } END { exit !found }' "$commits"; then
      echo "$label:"
      awk -F"$US" -v c="$class" '$2 == c { printf "- `%s` %s\n", substr($1, 1, 9), $3 }' "$commits"
      echo
    fi
  done
  echo "$BLOCK_END"
}

# A rolling issue body (stdin) with its marked block replaced by the block in
# file $1; a body without the markers keeps its text and gains the block at
# its end, so text a triager wrote is never lost.
replace_block() {
  local block=$1
  awk -v begin="$BLOCK_BEGIN" -v end="$BLOCK_END" -v block="$block" '
    { lines[NR] = $0 }
    $0 == begin && !b { b = NR }
    $0 == end && b && !e { e = NR }
    END {
      if (b && e) {
        for (i = 1; i < b; i++) print lines[i]
        while ((getline line < block) > 0) print line
        for (i = e + 1; i <= NR; i++) print lines[i]
      } else {
        for (i = 1; i <= NR; i++) print lines[i]
        print ""
        while ((getline line < block) > 0) print line
      }
    }
  '
}

# The rolling issue body. Args: kind comp repo sha default record block_file.
rolling_body() {
  local kind=$1 comp=$2 repo=$3 sha=$4 default=$5 record=$6 block=$7
  if [[ "$kind" = "release" ]]; then
    cat <<BODY
Upstream \`openEHR/$repo\` ($comp) has commits on $default past the release we vendor. They are unreleased and the pin moves only to a released tag (\`docs/VERSIONS.md\`, spec version policy), so this is look-ahead triage, not a re-vendor.

- **Compare:** https://github.com/openEHR/$repo/compare/${sha}...${default}
- **Vendored pin:** \`$record\`

$(cat "$block")

### Triage checklist

- [ ] Triage every commit in the delta: wire-visible / prose / machine-readable codegen input
- [ ] File a tracker issue for each change the implementation will need once a release carries it
- [ ] Re-vendor when a release tag carries the delta (the spec-release watcher reports new tags)

_The spec-update watcher replaces only the block between its markers as the delta grows, and closes the issue when a re-vendor catches the pin up; the rest of this body is the triager's._
BODY
  else
    cat <<BODY
Upstream \`openEHR/$repo\` ($comp) has moved past our vendored pin — commit-delta triage needed (the per-commit triage pattern that closed #341).

- **Compare:** https://github.com/openEHR/$repo/compare/${sha}...${default}
- **Vendored pin:** \`$record\`; where this component feeds codegen, also the machine-readable vendor dirs (\`tools/openehr-codegen/vendor/bmm/\`, \`crates/openehr-its/{vendor,schemas}/\`, re-vendored by \`scripts/vendor/codegen-inputs.sh\`)

$(cat "$block")

### Triage checklist

- [ ] Baseline: full regen + drift check green against the CURRENT pin before changing inputs
- [ ] Triage every commit in the delta: prose / class-table / machine-readable codegen input
- [ ] Verify each behaviour-relevant item against the implementation (several may already be satisfied)
- [ ] Re-vendor + re-pin + regenerate; review the generated diff line by line
- [ ] CNF zero-drift + changelog + \`docs/VERSIONS.md\` if anything is wire-visible

_The spec-update watcher replaces only the block between its markers as the delta grows, and closes the issue when a re-vendor catches the pin up; the rest of this body is the triager's._
BODY
  fi
}

# Offline checks of the helpers over fixtures: no network, no gh.
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
  printf 'ADL_ANTLR_REPO="openEHR/adl-antlr"\nADL_ANTLR_COMMIT="%s" # 2024-04-06\n' 5555555555555555555555555555555555555555 > "$dir/adl.sh"
  printf 'ADL_ANTLR_COMMIT="%s" # 2024-04-06\n' 6666666666666666666666666666666666666666 > "$dir/lang.sh"
  check() {
    local what="$1" actual="$2" expected="$3"
    if [[ "$actual" != "$expected" ]]; then
      echo "self-test: $what: got '$actual', want '$expected'" >&2
      fail=1
    fi
  }
  rows=$(rolling_rows "$dir/spec-docs.sh" "$dir/PROVENANCE.md" "$dir/adl.sh" "$dir/lang.sh")
  check "rows" "$rows" "RM|specifications-RM|master (RM 1.2.0)|1111111111111111111111111111111111111111|$dir/spec-docs.sh
ITS-REST|specifications-ITS-REST|Release-1.1.0 (released 19-Jul-2026)|2222222222222222222222222222222222222222|$dir/spec-docs.sh
ITS-BMM|specifications-ITS-BMM|master|3333333333333333333333333333333333333333|$dir/PROVENANCE.md
ADL-ANTLR|adl-antlr|master (AM grammars)|5555555555555555555555555555555555555555|$dir/adl.sh
LANG-ANTLR|adl-antlr|Release-1.1.0 era (LANG grammars)|6666666666666666666666666666666666666666|$dir/lang.sh"
  if rolling_rows "$dir/spec-docs.sh" "$dir/BROKEN.md" "$dir/adl.sh" "$dir/lang.sh" >/dev/null; then
    echo "self-test: a provenance record without a pinned commit was accepted" >&2
    fail=1
  fi
  if rolling_rows "$dir/spec-docs.sh" "$dir/PROVENANCE.md" "$dir/BROKEN.md" "$dir/lang.sh" >/dev/null; then
    echo "self-test: a grammar script without an adl-antlr pin was accepted" >&2
    fail=1
  fi
  check "branch kind" "$(pin_kind "master (RM 1.2.0)")" "branch"
  check "release kind" "$(pin_kind "Release-1.1.0 (released)")" "release"
  check "LANG grammars kind" "$(pin_kind "Release-1.1.0 era (LANG grammars)")" "release"
  if pin_kind "development" >/dev/null; then
    echo "self-test: an unknown ref word was classified" >&2
    fail=1
  fi
  check "branch label" "$(version_label branch)" "spec-version:current"
  check "release label" "$(version_label release)" "spec-version:next"
  check "branch title" "$(rolling_title branch ITS-BMM)" "[spec-update] ITS-BMM spec repo is ahead of the vendored pin"
  check "release title" "$(rolling_title release ITS-REST)" "[spec-update] ITS-REST has unreleased commits past the vendored release"
  check "release count" \
    "$(rolling_count_line release 6 "Release-1.1.0 (released 19-Jul-2026)" 2222222222222222222222222222222222222222 master 4444444444444444444444444444444444444444)" \
    "**Commits past the release:** 6 (release \`Release-1.1.0\` @ \`222222222\`, upstream master @ \`444444444\`)"
  check "branch count" \
    "$(rolling_count_line branch 6 master 3333333333333333333333333333333333333333 master 4444444444444444444444444444444444444444)" \
    "**Commits ahead of the pin:** 6 (pin \`333333333\`, upstream master @ \`444444444\`)"

  # Commit classes, one fixture per path kind.
  check "class: computable" "$(printf 'computable/BMM/openehr_rm_1.2.0.bmm.json\nREADME.md\n' | commit_class)" "machine"
  check "class: OpenAPI" "$(printf 'specifications/computable/OAS/ehr.openapi.yaml\n' | commit_class)" "machine"
  check "class: grammar" "$(printf 'src/main/antlr/adl/adl2.g4\n' | commit_class)" "machine"
  check "class: XSD" "$(printf 'components/ITS-XML/Release-2.0.0/Archetype.xsd\n' | commit_class)" "machine"
  check "class: terminology XML" "$(printf 'docs/terminology/openehr_terminology.xml\n' | commit_class)" "machine"
  check "class: chapter" "$(printf 'docs/AOM2/master03-constraint_model.adoc\nAGENTS.md\n' | commit_class)" "normative"
  check "class: REST chapter" "$(printf 'specifications/docs/ehr/ehr.md\n' | commit_class)" "normative"
  check "class: amendment record only" "$(printf 'docs/AOM2/master00-amendment_record.adoc\n' | commit_class)" "other"
  check "class: rendered and tooling" "$(printf 'docs/AOM2/index.html\nAGENTS.md\n.github/workflows/x.yml\n' | commit_class)" "other"
  check "class: no paths" "$(printf '' | commit_class)" "other"

  # The row's own key, one fixture per form: a heading key, a trailing key
  # with a "see also" key, a heading key that cites with "see" and with
  # "Addresses", a Markdown row, a row naming no key.
  cat > "$dir/amendment.adoc" <<'FIXTURE'
= Amendment Record

[cols="1,6a,2,2", options="header"]
|===
|Issue|Details|Raiser, Implementer|Completed

4+^h|*AM Release 2.4.0 (unreleased)*

|[[latest_issue]]2.4.3
|Add conversion rules for translators. ({spec_tickets}/SPECAM-107[SPECAM-107^], see also {spec_tickets}/SPECRM-119[SPECRM-119^])
|S Kohler
|[[latest_issue_date]]02 Nov 2026

|2.4.2
|{spec_tickets}/SPECAM-109[SPECAM-109^]. Add `units_system` (see <<_c_quantity_item_class>>), as on `DV_QUANTITY` (see {spec_tickets}/SPECRM-65[SPECRM-65^]).
|S Kohler
|16 Oct 2026

|2.4.1
|{spec_tickets}/SPECRM-65[SPECRM-65^]: Add attributes to `DV_QUANTITY`. (Addresses {spec_tickets}/SPECPR-251[SPECPR-251^])
|S Kohler
|01 Oct 2026

4+^h|*AM Release 2.3.0*

|2.3.1
|Fix typos.
|S Kohler
|01 Sep 2026
|===
FIXTURE
  check "adoc own keys" "$(amendment_rows < "$dir/amendment.adoc" | row_own_keys)" "SPECAM-107
SPECAM-109
SPECRM-65"
  cat > "$dir/Amendment_record.md" <<'FIXTURE'
<table>
    <tbody>
    <tr>
        <th colspan="4"><a href="https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0">Release-1.1.0</a></th>
    </tr>
    <tr>
        <td>5.9</td>
        <td><a href="https://specifications.openehr.org/tickets/SPECITS-95" target="_blank" rel="noopener">SPECITS-95</a>:
            Fix typing (see also <a href="https://specifications.openehr.org/tickets/SPECPR-428" target="_blank">SPECPR-428</a>)</td>
        <td>H Heiser</td>
        <td>11 Jun 2026</td>
    </tr>
    </tbody>
</table>
FIXTURE
  check "md own keys" "$(amendment_rows < "$dir/Amendment_record.md" | row_own_keys)" "SPECITS-95"
  check "see also list" "$(echo '|1.0 |Text. (SPECPR-1, see also SPECPR-2, SPECPR-3 and SPECPR-4)' | row_own_keys)" "SPECPR-1"

  # The marked block: replaced in place with the triager's text kept, and
  # appended to a body that has no markers.
  printf 'sha1%smachine%sAdd a BMM attribute\nsha2%sother%sRegenerate HTML\n' "$US" "$US" "$US" "$US" > "$dir/commits"
  printf 'SPECAM-109%scommit\nSPECRM-65%samendment row\n' "$US" "$US" > "$dir/keys"
  rolling_block "COUNT 2" "$dir/commits" "$dir/keys" > "$dir/block"
  check "block keys" "$(grep -F 'Keys this delta carries' "$dir/block")" "**Keys this delta carries:** SPECAM-109 (commit), SPECRM-65 (amendment row)"
  check "block kinds" "$(grep -F 'Delta by kind' "$dir/block")" "**Delta by kind:** 1 machine-readable, 0 normative text, 1 other (merge commits not counted)"
  printf 'Intro.\n\n%s\nCOUNT 1\n%s\n\nTriaged: SPECAM-109 is #5298.\n' "$BLOCK_BEGIN" "$BLOCK_END" > "$dir/body"
  replace_block "$dir/block" < "$dir/body" > "$dir/replaced"
  check "replaced block" "$(grep -c '^COUNT' "$dir/replaced")/$(grep -c '^COUNT 2$' "$dir/replaced")" "1/1"
  check "kept intro" "$(sed -n 1p "$dir/replaced")" "Intro."
  check "kept triage" "$(tail -n 1 "$dir/replaced")" "Triaged: SPECAM-109 is #5298."
  printf 'A body a triager rewrote.\n' > "$dir/bare"
  replace_block "$dir/block" < "$dir/bare" > "$dir/appended"
  check "appended keeps text" "$(sed -n 1p "$dir/appended")" "A body a triager rewrote."
  check "appended block" "$(grep -c -xF "$BLOCK_BEGIN" "$dir/appended")" "1"
  rolling_body branch RM specifications-RM 1111 master "$dir/spec-docs.sh" "$dir/block" > "$dir/full"
  check "body carries block" "$(grep -c -xF "$BLOCK_END" "$dir/full")" "1"
  replace_block "$dir/block" < "$dir/full" > "$dir/again"
  check "replace is idempotent" "$(cmp -s "$dir/full" "$dir/again" && echo same)" "same"
  if [[ "$fail" -ne 0 ]]; then
    exit 1
  fi
  echo "spec-update-watcher: self-test OK (pin rows incl. ITS-BMM and adl-antlr, pin kinds, labels, titles, count lines, commit classes, amendment-row keys, the marked block)."
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

for bin in curl jq gh perl; do
  command -v "$bin" >/dev/null 2>&1 || { echo "spec-update-watcher: $bin is required" >&2; exit 1; }
done

# ── component maps ───────────────────────────────────────────────────────────
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
    RM) echo "spec:RM" ;; BASE) echo "spec:BASE" ;; AM|ADL-ANTLR) echo "spec:AM" ;;
    LANG|LANG-ANTLR) echo "spec:LANG" ;; QUERY) echo "spec:QUERY" ;; TERM) echo "spec:TERM" ;;
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

# ── shared sinks ─────────────────────────────────────────────────────────────
# Candidates: key comp summary source resolved fixv comps status resolution
# itype created fixv_plain route descr, US-separated; route is `file` or
# `rolling:<COMP>`.
CANDIDATES="$tmp/candidates.tsv"
: > "$CANDIDATES"
# Skips: key<US>reason, printed as the skipped set in a dry run.
SKIPPED="$tmp/skipped.tsv"
: > "$SKIPPED"
skip() {
  echo "  $1: $2 — skipped"
  printf "%s${US}%s\n" "$1" "$2" >> "$SKIPPED"
}
# Per watched component: its pin kind, and the keys its delta carries.
mkdir -p "$tmp/delta"
# Every key a commit past a pin names: key<US>comp<US>kind.
COMMIT_KEYS="$tmp/commit-keys.tsv"
: > "$COMMIT_KEYS"

# ── C, first half: read each delta, classify its commits ────────────────────
echo "spec-update-watcher: commits-ahead cross-check against the vendored pins"
rolling_rows scripts/vendor/spec-docs.sh "$BMM_PROVENANCE" "$ADL_GRAMMARS" "$LANG_GRAMMARS" > "$tmp/rows" ||
  { echo "spec-update-watcher: no pinned commit in $BMM_PROVENANCE, $ADL_GRAMMARS or $LANG_GRAMMARS" >&2; exit 1; }
while IFS='|' read -r comp repo ref sha record; do
  [[ -n "$comp" ]] || continue
  kind=$(pin_kind "$ref") ||
    { echo "spec-update-watcher: $comp ref '$ref' names neither master nor a Release-* tag ($record)" >&2; exit 1; }
  echo "$kind" > "$tmp/delta/$comp.kind"
  default=$(gh api "repos/openEHR/$repo" --jq '.default_branch') ||
    { echo "spec-update-watcher: failed to read openEHR/$repo default branch" >&2; exit 1; }
  gh api "repos/openEHR/$repo/compare/${sha}...${default}" > "$tmp/compare.json" ||
    { echo "spec-update-watcher: failed to compare ${sha}...${default} for openEHR/$repo" >&2; exit 1; }
  jq -e 'has("ahead_by") and (.commits | type == "array")' "$tmp/compare.json" >/dev/null ||
    { echo "spec-update-watcher: unexpected compare payload for openEHR/$repo — API changed?" >&2; exit 1; }
  ahead=$(jq -r '.ahead_by' "$tmp/compare.json")
  head_sha=$(gh api "repos/openEHR/$repo/branches/$default" --jq '.commit.sha') ||
    { echo "spec-update-watcher: failed to read openEHR/$repo $default head" >&2; exit 1; }
  printf '%s|%s|%s\n' "$ahead" "$default" "$head_sha" > "$tmp/delta/$comp.head"
  : > "$tmp/delta/$comp.commits"
  : > "$tmp/delta/$comp.keys"
  # Every key a commit message names, merge commits included (a pull
  # request's title often carries the only key).
  jq -r '.commits[].commit.message' "$tmp/compare.json" | { grep -oE 'SPEC[A-Z]*-[0-9]+' || true; } | sort -u |
    while IFS= read -r key; do
      printf "%s${US}commit\n" "$key" >> "$tmp/delta/$comp.keys"
      printf "%s${US}%s${US}%s\n" "$key" "$comp" "$kind" >> "$COMMIT_KEYS"
    done
  jq -r --arg us "$US" '.commits[] | select((.parents | length) < 2) |
      [.sha, (.commit.message | split("\n")[0])] | join($us)' "$tmp/compare.json" |
    while IFS="$US" read -r csha subject; do
      class=$(gh api "repos/openEHR/$repo/commits/$csha" --jq '.files[]?.filename' | commit_class) ||
        { echo "spec-update-watcher: failed to read openEHR/$repo commit $csha" >&2; exit 1; }
      printf "%s${US}%s${US}%s\n" "$csha" "$class" "$subject" >> "$tmp/delta/$comp.commits"
    done
done < "$tmp/rows"

# The pin kind of a watched component, empty when it has none.
kind_of() {
  if [[ -f "$tmp/delta/$1.kind" ]]; then
    cat "$tmp/delta/$1.kind"
  fi
}

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
    skip "$key" "already inside the vendored pin (amendment record carries it)"
    continue
  fi
  # A rejected/duplicate resolution means NOTHING changed in the spec — no
  # triage material. "Rejected" was missing from this list, which is how
  # SPECTERM-30 got filed as tracker work (issue #175 / bug #340); "Superseded"
  # likewise (SPECPR-124 → issue #347): the successor ticket is detected on its
  # own resolution, the superseded one itself lands nothing.
  resolution=$(echo "$issue" | jq -r '.fields.resolution.name // "unresolved"')
  case "$resolution" in
    "Rejected"|"Superseded"|"Won't Do"|"Won't Fix"|"Duplicate"|"Cannot Reproduce"|"Declined"|"Abandoned"|"Not a Bug")
      skip "$key" "resolution '$resolution', no spec change"
      continue ;;
    *) ;;
  esac
  component=$(component_for_project "$project")
  # A change whose fix versions are ALL strictly older than our pins is
  # already inside the vendored spec text (catches cross-project SPECPR
  # keys the amendment-record baseline cannot see).
  fixv_lines=$(echo "$issue" | jq -r '.fields.fixVersions[]?.name')
  if all_fixversions_inside_pin "$fixv_lines" "$component"; then
    skip "$key" "fix version(s) [$(echo "$fixv_lines" | paste -sd, -)] predate our pins"
    continue
  fi
  comps=$(echo "$issue" | jq -r '[.fields.components[]?.name] | join(", ") | if . == "" then "—" else . end')
  if echo "$issue" | jq -e '[.fields.components[]?.name] | any(. == "Documentation")' >/dev/null; then
    skip "$key" "Jira component Documentation"
    continue
  fi
  # The change must have reached a watched repository past our pin; where it
  # reached a master-pinned one, that component's rolling issue carries it.
  landed=$(awk -F"$US" -v k="$key" '$1 == k { print $2 "|" $3 }' "$COMMIT_KEYS" | sort -u)
  if [[ -z "$landed" ]]; then
    skip "$key" "no commit past our pin in a watched repository names it"
    continue
  fi
  route="file"
  rolling=$(grep -E '\|branch$' <<<"$landed" | sed -n 1p | cut -d'|' -f1 || true)
  [[ -n "$rolling" ]] && route="rolling:$rolling"
  summary=$(echo "$issue" | jq -r '.fields.summary')
  status=$(echo "$issue" | jq -r '.fields.status.name // "unknown"')
  itype=$(echo "$issue" | jq -r '.fields.issuetype.name // "unknown"')
  created=$(echo "$issue" | jq -r '.fields.created // "unknown" | split("T")[0]')
  resolved=$(echo "$issue" | jq -r '.fields.resolutiondate // "unknown" | split("T")[0]')
  # Fix versions with their release state: "Release-1.3.0 (unreleased)".
  fixv=$(echo "$issue" | jq -r '[.fields.fixVersions[]? | "\(.name) (\(if .released then "released" else "unreleased" end))"] | join(", ") | if . == "" then "—" else . end')
  fixv_plain=$(echo "$issue" | jq -r '[.fields.fixVersions[]?.name] | join(", ") | if . == "" then "—" else . end')
  # Description arrives as Atlassian Document Format — flatten the text
  # leaves and truncate to a triage-sized excerpt.
  descr=$(echo "$issue" | jq -r '[.fields.description // {} | .. | .text? // empty] | join(" ") | .[0:700]' | tr "$US" ' ' | tr '\n' ' ')
  printf "%s${US}%s${US}%s${US}jira${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s\n" \
    "$key" "$component" "$summary" "$resolved" "$fixv" "$comps" \
    "$status" "$resolution" "$itype" "$created" "$fixv_plain" "$route" "$descr" >> "$CANDIDATES"
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
    <(amendment_rows < "$tmp/upstream" | row_own_keys | sort -u) \
    <(grep -oE 'SPEC[A-Z]*-[0-9]+' "$local_path" | sort -u) || true)
  [[ -n "$new_keys" ]] || continue
  route="file"
  [[ "$(kind_of "$comp")" = "branch" ]] && route="rolling:$comp"
  while IFS= read -r key; do
    if [[ "$route" != file ]]; then
      printf "%s${US}amendment row\n" "$key" >> "$tmp/delta/$comp.keys"
    fi
    printf "%s${US}%s${US}%s${US}amendment${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s${US}%s\n" \
      "$key" "$comp" "new amendment-record row in openEHR/$repo/$rel" \
      "see amendment record" "—" "—" \
      "(amendment)" "(amendment)" "amendment row" "—" "—" "$route" \
      "New row headed by $key in the upstream amendment record openEHR/$repo/$rel — not present in our vendored copy." >> "$CANDIDATES"
  done <<< "$new_keys"
done <<< "$amendment_files"

# ── C, second half: one rolling issue per component and pin kind ────────────
# Created when the delta carries a machine-readable or normative commit,
# its marked block replaced in place as the delta grows, auto-closed when a
# re-vendor catches the pin up.
ahead_created=0 ahead_updated=0 ahead_closed=0
while IFS='|' read -r comp repo ref sha record; do
  [[ -n "$comp" ]] || continue
  kind=$(kind_of "$comp")
  IFS='|' read -r ahead default head_sha < "$tmp/delta/$comp.head"
  title=$(rolling_title "$kind" "$comp")
  num=$("$FILE_ISSUE" find --title "$title")
  if [[ "$ahead" -gt 0 ]]; then
    count_line=$(rolling_count_line "$kind" "$ahead" "$ref" "$sha" "$default" "$head_sha")
    rolling_block "$count_line" "$tmp/delta/$comp.commits" "$tmp/delta/$comp.keys" > "$tmp/block.md"
    if [[ -n "$num" ]]; then
      gh issue view "$num" --json body --jq '.body' > "$tmp/old-body.md"
      replace_block "$tmp/block.md" < "$tmp/old-body.md" > "$tmp/ahead-body.md"
      if [[ "$(cat "$tmp/old-body.md")" == "$(cat "$tmp/ahead-body.md")" ]]; then
        continue # unchanged since the last run
      fi
    else
      if ! awk -F"$US" '$2 != "other" { found = 1 } END { exit !found }' "$tmp/delta/$comp.commits"; then
        echo "  $comp is $ahead commit(s) ahead ($kind pin), none machine-readable or normative — nothing filed"
        continue
      fi
      rolling_body "$kind" "$comp" "$repo" "$sha" "$default" "$record" "$tmp/block.md" > "$tmp/ahead-body.md"
    fi
    labels="spec-update,chore,$(version_label "$kind")"
    lbl=$(label_for_component "$comp" || true)
    [[ -n "${lbl:-}" ]] && labels="$labels,$lbl"
    if [[ "$DRY_RUN" = "1" ]]; then
      sed 's/^/    │ /' "$tmp/ahead-body.md"
    fi
    # One engine call covers both branches: create the rolling issue when it
    # does not exist, write the body with only its block replaced when it does.
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

created=0 skipped=0 unblocked=0 routed=0
while IFS="$US" read -r key component summary source resolved fixv comps status resolution itype jcreated fixv_plain route descr; do
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
    echo "  $key: on the board as #$num"
    skipped=$((skipped + 1))
    continue
  fi
  if [[ "$route" == rolling:* ]]; then
    echo "  $key: lands in master-pinned ${route#rolling:}, listed on its rolling issue"
    routed=$((routed + 1))
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
  # A per-key issue is a change past a release pin: look-ahead triage.
  labels="spec-update,chore,$(version_label release)"
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

if [[ "$DRY_RUN" = "1" ]] && [[ -s "$SKIPPED" ]]; then
  echo "spec-update-watcher: the skipped set ($(wc -l < "$SKIPPED" | tr -d ' ')):"
  sort -t"$US" -k1,1 "$SKIPPED" | awk -F"$US" '{ printf "  %s  %s\n", $1, $2 }'
fi
echo "spec-update-watcher: done — $created new, $routed on a rolling issue, $unblocked unblocked, $skipped already on the board (dedup by key), $(wc -l < "$SKIPPED" | tr -d ' ') skipped by a filter."
