#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Render website/book/src/compliance/control-matrix.md FROM the tracker.
#
# A hand-maintained compliance table is wrong the first time a control ships.
# This script reads the tracker instead: every issue whose body carries a
# structured line
#
#     Control: <legal source> <article or clause>
#
# becomes a row, joined to its legal source URL from the registry declared
# below and to its current status (shipped with the closing pull request,
# planned, or not planned). An issue may declare several controls, one per
# line. The Control column carries the issue title, unless the body also
# carries one line
#
#     Control-text: <the control, stated as what the product does>
#
# which the generator prefers: an issue titled as the defect it fixes would
# otherwise render its defect beside Shipped as if the defect were the
# control (#3249).
#
# DETERMINISM. The rendered page is a pure function of tracker state: no
# clock, no HEAD commit, no run counter. The docs CI job re-runs this script
# with --check and fails when the committed page differs, so anything that
# changed between two runs of an unchanged tracker would fail the build every
# day. When the page was generated, and from which commit, is git history of
# the file itself.
#
# THE CLOSING PULL REQUEST. A control's row shows Shipped with its closing
# pull request once that PR has MERGED; an open PR that says `Closes #N`
# changes nothing, so opening one never stales the page. The one exception is
# deliberate: run with `--closing <pr>` and the issues that pull request closes
# render as Shipped with that PR already, so the PR that closes a control
# regenerates the page itself and the page is right the moment it merges. The
# CI job passes the pull request under check (#3253).
#
# Usage:
#   scripts/render/control-matrix.sh                     write the page
#   scripts/render/control-matrix.sh --closing <pr>      write it as it will read once <pr> merges
#   scripts/render/control-matrix.sh --check [--closing <pr>]
#                                                        fail when the committed page is stale
set -euo pipefail
cd "$(dirname "$0")/../.."

PAGE="website/book/src/compliance/control-matrix.md"
SCRIPT="scripts/render/control-matrix.sh"

# Fetch ceiling. The tracker holds ~2100 issues, so a low limit silently drops
# the older half of the matrix; the truncation guard below turns a tracker that
# outgrows this number into a loud failure rather than a short page.
FETCH_LIMIT=10000

# The legal-source registry: short name, jurisdiction, then the official
# publisher URL. A control naming a short name that is not declared here is a
# hard failure, so no page cell can ever carry free text where a citation
# belongs. Order here is the order the table groups by.
#
# The jurisdiction column is not decoration. openEHR is not a Dutch standard,
# and FerroEHR is deployed outside the Netherlands, so a page that lists Dutch
# law beside EU law without saying which is which reads as though every
# deployment answers to both (#3185). EU and INT apply everywhere; a
# two-letter code is national and applies to deployments in that country.
#
# Every URL here is checked BY HAND when it is added or changed, and the date
# recorded below. Nothing in CI can do it: the link checker runs with
# --offline deliberately, because a rate-limited publisher says nothing about
# the change under review (#2287). That is exactly how the NEN 7510 entry sat
# here returning 404 (#3177).
#
# Last checked, all 200: 2026-09-13 (the DE and CH entries added that day;
# the NL, EU and INT ones re-checked 2026-09-09). The EPDV entry was added and
# checked 200 on 2026-09-16; its URL is the "whatever is current" form
# `docs/law/ch/epdv/PROVENANCE.md` records beside the consolidation the corpus
# vendors, the same shape the EPDG entry above carries. The EDPB 01/2025 URL
# moved to the EDPB's public-consultations page (the old one redirects there)
# and was checked 200 on 2026-10-06 (#3597). The CRA entry was added on
# 2026-10-06 (#3606); its ELI is the one the vendored OJ text in
# docs/law/eu/cra/ carries, since EUR-Lex answers a scripted request with a
# 202 bot challenge.
LEGAL_SOURCES=(
  "GDPR|EU|https://eur-lex.europa.eu/eli/reg/2016/679/oj"
  "EHDS|EU|https://eur-lex.europa.eu/eli/reg/2025/327/oj"
  "CRA|EU|https://eur-lex.europa.eu/eli/reg/2024/2847/oj"
  "EDPB 01/2025|EU|https://www.edpb.europa.eu/public-consultations/guidelines-012025-on-pseudonymisation_en"
  "EDPB 2/2023|EU|https://www.edpb.europa.eu/documents/guideline/guidelines-22023-on-technical-scope-of-art-53-of-eprivacy-directive_en"
  "UAVG|NL|https://wetten.overheid.nl/BWBR0040940"
  "Wabvpz|NL|https://wetten.overheid.nl/BWBR0023864"
  "NEN 7510|NL|https://www.nen.nl/nen-7510-1-2024-nl-331311"
  "NEN 7512|NL|https://www.nen.nl/nen-7512-2022-nl-297137"
  "NEN 7513|NL|https://www.nen.nl/nen-7513-2018-nl-245399"
  "BDSG|DE|https://www.gesetze-im-internet.de/bdsg_2018/"
  "SGB V|DE|https://www.gesetze-im-internet.de/sgb_5/"
  "GDNG|DE|https://www.gesetze-im-internet.de/gdng/"
  "StGB|DE|https://www.gesetze-im-internet.de/stgb/"
  "DSG|CH|https://www.fedlex.admin.ch/eli/cc/2022/491/de"
  "DSV|CH|https://www.fedlex.admin.ch/eli/cc/2022/568/de"
  "EPDG|CH|https://www.fedlex.admin.ch/eli/cc/2017/203/de"
  "EPDV|CH|https://www.fedlex.admin.ch/eli/cc/2017/204/de"
  "IHE ATNA|INT|https://profiles.ihe.net/ITI/TF/Volume1/ch-9.html"
)

die() {
  echo "control-matrix: $*" >&2
  exit 1
}

command -v gh >/dev/null 2>&1 || die "the GitHub CLI (gh) is not installed"
command -v jq >/dev/null 2>&1 || die "jq is required (brew install jq / preinstalled on CI runners)"

CHECK=0
CLOSING=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) CHECK=1 ;;
    --closing)
      [[ -n "${2:-}" ]] || die "--closing needs a pull request number"
      CLOSING="$2"
      shift
      ;;
    -h | --help)
      sed -n '/^# Usage:/,/fail when the committed page is stale/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) die "unknown argument '$1' (expected --check or --closing <pr>)" ;;
  esac
  shift
done
[[ -z "$CLOSING" || "$CLOSING" =~ ^[0-9]+$ ]] || die "--closing takes a pull request number, got '$CLOSING'"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# ── the registry, as JSON ────────────────────────────────────────────────────
printf '%s\n' "${LEGAL_SOURCES[@]}" |
  jq -R -s 'split("\n") | map(select(length > 0) | split("|")
              | {name: .[0], jurisdiction: .[1], url: .[2]})
            | to_entries | map(.value + {rank: .key})' > "$WORK/sources.json"

# ── the tracker ──────────────────────────────────────────────────────────────
# One call, and everything the page needs, readable with a repository token.
gh issue list --state all --limit "$FETCH_LIMIT" \
  --json number,title,body,state,stateReason,url \
  > "$WORK/issues.json" || die "could not read the tracker (is gh authenticated?)"

fetched="$(jq 'length' "$WORK/issues.json")"
[[ "$fetched" -lt "$FETCH_LIMIT" ]] ||
  die "the tracker returned $fetched issues, the fetch ceiling — raise FETCH_LIMIT in $SCRIPT"

# ── controls ─────────────────────────────────────────────────────────────────
# One object per declared control. An unresolvable short name is emitted as an
# error row rather than a table cell.
jq --slurpfile sources "$WORK/sources.json" '
  ($sources[0]) as $reg
  | def source_for($text):
      [ $reg[] | select(.name as $n | $text == $n or ($text | startswith($n + " "))) ]
      | sort_by(-(.name | length)) | first;
    [ .[] as $issue
      | ( ($issue.body // "") | split("\n")
          | map(select(test("^[[:space:]]*(-[[:space:]]+|\\*[[:space:]]+)?Control-text:")))
          | first // ""
          | sub("^[[:space:]]*(-[[:space:]]+|\\*[[:space:]]+)?Control-text:[[:space:]]*"; "")
          | sub("[[:space:]]+$"; "")
        ) as $control_text
      | ($issue.body // "")
      | split("\n")[]
      | select(test("^[[:space:]]*(-[[:space:]]+|\\*[[:space:]]+)?Control:"))
      | sub("^[[:space:]]*(-[[:space:]]+|\\*[[:space:]]+)?Control:[[:space:]]*"; "")
      | sub("[[:space:]]+$"; "")
      | . as $text
      | source_for($text) as $src
      | if $src == null or ($text | length) == 0 then
          {error: true, issue: $issue.number, text: $text}
        else
          ($text[($src.name | length):] | sub("^[[:space:]]+"; "")) as $clause
          | {
              error: false,
              rank: $src.rank,
              source: $src.name,
              source_url: $src.url,
              jurisdiction: $src.jurisdiction,
              clause: $clause,
              title: (if ($control_text | length) > 0 then $control_text else $issue.title end),
              number: $issue.number,
              url: $issue.url,
              state: $issue.state,
              state_reason: ($issue.stateReason // "")
            }
        end
    ]
  | sort_by(.rank, .clause, .number)
' "$WORK/issues.json" > "$WORK/controls.json"

# A control naming an undeclared legal source stops the render, listing every
# offender so one run fixes them all.
if [[ "$(jq '[.[] | select(.error)] | length' "$WORK/controls.json")" -gt 0 ]]; then
  jq -r '.[] | select(.error)
         | "control-matrix: issue #\(.issue) declares legal source \"\(.text | split(" ")[0] // "")\" (from \"Control: \(.text)\"), which is not in the registry"' \
    "$WORK/controls.json" >&2
  die "add the source to LEGAL_SOURCES in $SCRIPT, or fix the issue body"
fi

# An empty matrix is a defect, not a valid state, once the compliance pages
# send readers here as the evidence surface (#3236): the page would render
# truthfully empty, the diff guard would stay green, and the accountability
# trail (GDPR Art. 5(2), Art. 24(1)) would land on a page saying the product
# ships no controls.
if [[ "$(jq 'length' "$WORK/controls.json")" -eq 0 ]] &&
   grep -rlq 'control-matrix.md' website/book/src/compliance/ --include='*.md' --exclude='control-matrix.md'; then
  die "no issue declares a Control: line while the compliance pages link to the matrix — declare the shipped controls on their issues (#3236)"
fi

# No board read. The roadmap board is the only source of "in progress"
# (.claude/rules/project-board.md), and reading it needs a Projects (v2) scope
# a repository token does not carry, so a page that rendered it could only be
# verified in CI by a standing personal access token. The page therefore says
# Planned for every open control and links the board for the live column
# (#3236); a repository token is enough to render and to check it.

# ── the closing pull requests ────────────────────────────────────────────────
# `gh issue list` reports the pull requests that reference an issue with a
# closing keyword but not whether they merged, and an OPEN one must not move a
# row (#3253). One GraphQL read per control-bearing issue fetches the state;
# ~30 issues, one call each, well inside the rate budget.
repo="$(gh repo view --json nameWithOwner --jq .nameWithOwner)" ||
  die "could not resolve the current repository (run inside a gh-authenticated clone)"
: > "$WORK/prs.jsonl"
for issue in $(jq -r '[ .[] | .number ] | unique | .[]' "$WORK/controls.json"); do
  # shellcheck disable=SC2016 # $owner/$name/$number are GraphQL variables, bound by the -f flags
  gh api graphql -f owner="${repo%%/*}" -f name="${repo##*/}" -F number="$issue" \
    -f query='query($owner:String!,$name:String!,$number:Int!){
      repository(owner:$owner,name:$name){issue(number:$number){
        closedByPullRequestsReferences(first:20){nodes{number url merged}}}}}' \
    > "$WORK/item.json" || die "could not read the closing pull requests of issue #$issue"
  jq -c --arg n "$issue" \
    '{key: $n, value: [ .data.repository.issue.closedByPullRequestsReferences.nodes[]?
                        | select(.merged) | {number, url} ]}' \
    "$WORK/item.json" >> "$WORK/prs.jsonl"
done
jq -s 'from_entries' "$WORK/prs.jsonl" > "$WORK/prs.json"

# The pull request under check, when the caller names one: the issues it will
# close render as Shipped with it, so the page committed on that PR is the page
# main carries after the merge.
echo '{"issues": [], "pr": null}' > "$WORK/closing.json"
if [[ -n "$CLOSING" ]]; then
  # shellcheck disable=SC2016 # GraphQL variables, bound by the -f flags
  gh api graphql -f owner="${repo%%/*}" -f name="${repo##*/}" -F number="$CLOSING" \
    -f query='query($owner:String!,$name:String!,$number:Int!){
      repository(owner:$owner,name:$name){pullRequest(number:$number){
        number url closingIssuesReferences(first:50){nodes{number}}}}}' \
    > "$WORK/item.json" || die "could not read pull request #$CLOSING"
  jq '{issues: [ .data.repository.pullRequest.closingIssuesReferences.nodes[]?.number ],
       pr: (.data.repository.pullRequest | {number, url})}' "$WORK/item.json" > "$WORK/closing.json"
fi

# ── render ───────────────────────────────────────────────────────────────────
OUT="$WORK/control-matrix.md"

cat > "$OUT" <<'HEADER'
# Control matrix

FerroEHR is software. It is not a controller, not a processor and not a
certified organisation, so this page makes no compliance claim on anyone's
behalf. It lists the technical controls the product ships or plans, and the
article or clause each one is designed to support. Whether a deployment
satisfies a legal obligation depends on how the deploying organisation runs
it.

Every row comes from the tracker. A control is declared on the issue that
delivers it, as a line in the issue body:

```text
Control: <legal source> <article or clause>
```

The short name resolves to an official publisher URL from a registry inside
the generator, so a legal citation on this page is never free text. An issue
may declare several controls, one per line. The Control column carries the
issue title, unless the body also states the control on its own line, which
the generator prefers so that an issue titled as the defect it fixed does not
read its defect as the control:

```text
Control-text: <the control, stated as what the product does>
```

## How this page is built

`scripts/render/control-matrix.sh` queries the tracker with the GitHub CLI,
joins each declared control to its legal source and to its current state, and
writes this file. A CI job re-runs the generator with `--check` and fails the
build when the committed page no longer matches the tracker, which is what
keeps a shipped control from sitting here as "planned".

- **Shipped:** the issue is closed as completed. The merged pull request that
  closed it is linked in the last column. The one pull request that closes a
  control regenerates this page as it will read after the merge, so the page
  never lags a shipped control.
- **Planned:** the issue is open. Whether work has started is the issue's
  column on the [public roadmap board](https://github.com/orgs/FerroHEALTH/projects/2),
  which this page does not copy: a status that lives in two places disagrees
  the day one of them moves.
- **Not planned:** the issue was closed without the control being built. The
  row stays visible so the record does not quietly lose it.

The page carries no generation timestamp and no build commit. Both change on
every run or every push while the tracker has not moved, which would make the
CI staleness check fail on days when nothing was wrong. When this page was
last regenerated, and from which commit, is the file's own git history.

## Controls

HEADER

rows="$(jq -r --slurpfile prs "$WORK/prs.json" --slurpfile closing "$WORK/closing.json" '
  ($prs[0]) as $merged | ($closing[0]) as $closing
  | def esc: gsub("\\|"; "\\|");
  def closing_here: ($closing.pr != null) and ([.number] | inside($closing.issues));
  def status:
    if .state == "CLOSED" and .state_reason == "NOT_PLANNED" then "Not planned"
    elif .state == "CLOSED" or closing_here then "Shipped"
    else "Planned" end;
  def prs:
    (($merged[.number | tostring] // []) + (if closing_here then [$closing.pr] else [] end)
     | unique_by(.number)) as $list
    | if ($list | length) == 0 then "—"
      else [ $list[] | "[#\(.number)](\(.url))" ] | join(", ") end;
  .[] | "| [\(.source)](\(.source_url)) | \(.jurisdiction) | \(.clause | esc) | \(.title | esc) | [#\(.number)](\(.url)) | \(status) | \(prs) |"
' "$WORK/controls.json")"

if [[ -n "$rows" ]]; then
  cat >> "$OUT" <<'TABLE'
| Legal source | Applies to | Article or clause | Control | Issue | Status | Closing PR |
|---|---|---|---|---|---|---|
TABLE
  printf '%s\n' "$rows" >> "$OUT"
else
  cat >> "$OUT" <<'EMPTY'
No issue in the tracker declares a control yet, so this table is empty. It
fills itself as the compliance program lands: the first issue to carry a
`Control:` line appears here on the next regeneration.
EMPTY
fi

{
  cat <<'FOOTER'

## Legal sources

The short names above resolve to these publishers. The linked text is the
authority; nothing on this page restates it.

| Short name | Applies to | Source |
|---|---|---|
FOOTER

  jq -r '.[] | "| \(.name) | \(.jurisdiction) | [\(.url)](\(.url)) |"' "$WORK/sources.json"

  cat <<'SCOPE'

`EU` and `INT` apply to every deployment. A two-letter country code is
national law or a national standard, and applies to a deployment in that
country: FerroEHR is an openEHR CDR, openEHR is not a Dutch standard, and a
deployment elsewhere answers to its own equivalents rather than to these.
Adding a jurisdiction is a registry entry plus the controls that cite it.
SCOPE
} >> "$OUT"

if [[ "$CHECK" -eq 1 ]]; then
  [[ -f "$PAGE" ]] || die "$PAGE does not exist — run $SCRIPT"
  if ! diff -u "$PAGE" "$OUT" > "$WORK/diff"; then
    cat "$WORK/diff" >&2
    die "$PAGE is stale against the tracker — run $SCRIPT and commit the result"
  fi
  echo "control-matrix: $PAGE matches the tracker ($(jq 'length' "$WORK/controls.json") controls)"
else
  mkdir -p "$(dirname "$PAGE")"
  cp "$OUT" "$PAGE"
  echo "control-matrix: wrote $PAGE ($(jq 'length' "$WORK/controls.json") controls from $fetched issues)"
fi
