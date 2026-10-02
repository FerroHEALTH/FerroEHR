#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# The watcher family's filing engine (#2778) — ONE dedup search and ONE
# file-or-comment loop for all six watchers. Before this file there were three
# dedup idioms and four label conventions across six lanes, and one lane
# (latest-deps) silently skipped an existing issue instead of commenting on it,
# so a recurring finding vanished after its first report.
#
# Callers: the sibling `action.yml` (the workflow-facing interface, used by
# image-scan, base-image-watcher, latest-deps and link-check-external) and the
# two probe shells `scripts/watch/spec-{update,release}.sh`, which file N issues
# per run and therefore call this script directly. The engine lives in the
# action directory because the action must be self-contained — a composite
# resolves its own files through `$GITHUB_ACTION_PATH`
# (https://docs.github.com/en/actions/reference/workflows-and-actions/metadata-syntax#runsstepsrun)
# — and the scripts reach into it rather than the reverse.
#
# RUN COLOUR. This engine only runs after a probe has produced an answer, so
# reaching it is never a failure: it exits 0 for every outcome (created,
# commented, updated, skipped, none) and non-zero only when its own `gh` call
# fails, which is a broken filing path rather than a finding. A probe that
# cannot answer — transport error, tool crash, unparseable output — fails in the
# probe step and never gets here. That is the family's uniform rule: the run is
# RED only when the PROBE fails; a finding files or updates an issue and the run
# stays GREEN, because the issue is the alert.
#
# DEDUP MATCH RULE. GitHub's `in:title "phrase"` is a phrase search, not an
# exact-title match, so the search alone can return a title that merely CONTAINS
# the phrase. Both modes therefore post-filter the search result:
#
#   * default (no `--dedup-key`) — the title IS the key: a match is an issue
#     whose title is byte-equal. Keep the title constant across runs (a run
#     number, date or version in it would open a new issue every run) and put
#     the varying detail in the body.
#   * `--dedup-key K` (repeatable) — the key is a token that must appear in the
#     title: a match is an issue whose title contains EVERY key as a whole
#     WORD (regex word boundaries, case-insensitive — #2796: RM/AM/SM occur
#     inside "the ARM build" and "transform", so substring containment could
#     dedup a genuine release against an unrelated issue). This is what the
#     spec watchers need, where the Jira key or the component-plus-version
#     pair identifies the work and the rest of the title is free text.
#
# BODY BY FILE ONLY. Watcher bodies carry remote content — upstream release
# notes, scanner output, third-party server messages — which must never be
# parsed as script or as a flag.
#
# Usage:
#   file-issue.sh file --title T --body-file F [--type Bug|Feature|Task]
#                      [--labels a,b] [--dedup-key K]...
#                      [--state open|all] [--on-existing comment|update|skip]
#                      [--repo R] [--dry-run]
#   file-issue.sh find [--title T] [--dedup-key K]... [--state open|all] [--repo R]
#
# ISSUE TYPE. `--type` sets GitHub's native issue type on a created issue
# (.claude/rules/issue-workflow.md §Type, priority and labels). It is set
# through the REST issue update after the create, because `gh issue create
# --type` resolves the name through an organisation query the workflow token
# may not read. A refused type is a warning, never a lost finding: the issue
# stands, and whoever picks it up sets the type with the priority and the
# effort, which no lane sets.
#
# `file` prints one `file-issue: <outcome> …` line and, when `$GITHUB_OUTPUT` is
# set, writes `issue=` and `outcome=` to it. `find` prints the matching issue
# number, or nothing.
#
# Env: GH_TOKEN (or GITHUB_TOKEN) for gh · GITHUB_REPOSITORY as the `--repo`
# default · GITHUB_OUTPUT to receive the outputs.
set -euo pipefail

verb="${1:-}"
case "$verb" in
  file | find) shift ;;
  *)
    echo "file-issue: expected verb 'file' or 'find', got '${verb:-<none>}'" >&2
    exit 2
    ;;
esac

title=""
body_file=""
labels=""
issue_type=""
state="open"
on_existing="comment"
repo="${GITHUB_REPOSITORY:-}"
dry_run=0
dedup_keys=()

while [ "$#" -gt 0 ]; do
  case "$1" in
    --title) title="${2:?--title needs a value}"; shift 2 ;;
    --body-file) body_file="${2:?--body-file needs a value}"; shift 2 ;;
    --labels) labels="${2-}"; shift 2 ;;
    --type) issue_type="${2:?--type needs a value}"; shift 2 ;;
    --dedup-key) dedup_keys+=("${2:?--dedup-key needs a value}"); shift 2 ;;
    --state) state="${2:?--state needs a value}"; shift 2 ;;
    --on-existing) on_existing="${2:?--on-existing needs a value}"; shift 2 ;;
    --repo) repo="${2:?--repo needs a value}"; shift 2 ;;
    --dry-run) dry_run=1; shift ;;
    *) echo "file-issue: unknown argument '$1'" >&2; exit 2 ;;
  esac
done

case "$state" in
  open | all) ;;
  *) echo "file-issue: --state must be open or all (got '$state')" >&2; exit 2 ;;
esac
case "$issue_type" in
  "" | Bug | Feature | Task) ;;
  *) echo "file-issue: --type must be Bug, Feature or Task (got '$issue_type')" >&2; exit 2 ;;
esac
case "$on_existing" in
  comment | update | skip) ;;
  *) echo "file-issue: --on-existing must be comment, update or skip (got '$on_existing')" >&2; exit 2 ;;
esac

# The dedup key defaults to the title, which is then matched exactly.
exact=0
if [ "${#dedup_keys[@]}" -eq 0 ]; then
  [ -n "$title" ] || { echo "file-issue: --title is required when no --dedup-key is given" >&2; exit 2; }
  dedup_keys=("$title")
  exact=1
fi

gh_args=()
if [ -n "$repo" ]; then
  gh_args=(--repo "$repo")
fi

# THE dedup search — the single implementation for the whole family.
find_existing() {
  local query="" key json
  for key in "${dedup_keys[@]}"; do
    query+="in:title \"${key}\" "
  done
  json="$(gh issue list "${gh_args[@]+"${gh_args[@]}"}" --state "$state" --search "$query" \
            --limit 100 --json number,title)"
  if [ "$exact" = 1 ]; then
    printf '%s' "$json" | jq -r --arg t "$title" \
      'map(select(.title == $t)) | .[0].number // empty'
  else
    # Word-boundary containment, not substring (#2796): the short component
    # tokens RM/AM/SM occur inside ordinary words ("the ARM build",
    # "transform"), so a plain contains() could dedup a genuine notification
    # against an unrelated issue. Each key is regex-escaped, then must match
    # between word boundaries, case-insensitively.
    printf '%s' "$json" | jq -r --args \
      'map(select(.title as $t
                  | all($ARGS.positional[];
                        gsub("(?<c>[.\\\\+*?()|\\[\\]{}^$-])"; "\\" + .c) as $rx
                        | $t | test("\\b" + $rx + "\\b"; "i"))))
       | .[0].number // empty' -- "${dedup_keys[@]}"
  fi
}

# Checked before the search, so a broken caller costs no API call.
if [ "$verb" = file ]; then
  [ -n "$title" ] || { echo "file-issue: --title is required" >&2; exit 2; }
  [ -n "$body_file" ] || { echo "file-issue: --body-file is required" >&2; exit 2; }
  [ -s "$body_file" ] || { echo "::error::body file '$body_file' is missing or empty"; exit 1; }
fi

existing="$(find_existing)"

if [ "$verb" = find ]; then
  printf '%s\n' "$existing"
  exit 0
fi

emit() { # $1 = issue number (may be empty), $2 = outcome
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
      echo "issue=$1"
      echo "outcome=$2"
    } >> "$GITHUB_OUTPUT"
  fi
  echo "file-issue: $2${1:+ #$1} — $title"
}

if [ "$dry_run" = 1 ]; then
  if [ -n "$existing" ]; then
    echo "file-issue: DRY-RUN would $on_existing on #$existing — $title"
  else
    echo "file-issue: DRY-RUN would create — $title <${issue_type:-no type}> [${labels:-no labels}]"
  fi
  emit "$existing" none
  exit 0
fi

if [ -n "$existing" ]; then
  case "$on_existing" in
    comment)
      gh issue comment "$existing" "${gh_args[@]+"${gh_args[@]}"}" --body-file "$body_file" >/dev/null
      emit "$existing" commented
      ;;
    update)
      gh issue edit "$existing" "${gh_args[@]+"${gh_args[@]}"}" --body-file "$body_file" >/dev/null
      emit "$existing" updated
      ;;
    skip)
      emit "$existing" skipped
      ;;
  esac
  exit 0
fi

# One --label per name; an empty list creates an unlabelled issue.
label_args=()
if [ -n "$labels" ]; then
  IFS=',' read -r -a names <<<"$labels"
  for name in "${names[@]}"; do
    name="${name#"${name%%[![:space:]]*}"}"
    name="${name%"${name##*[![:space:]]}"}"
    [ -n "$name" ] && label_args+=(--label "$name")
  done
fi
url="$(gh issue create "${gh_args[@]+"${gh_args[@]}"}" --title "$title" --body-file "$body_file" \
         "${label_args[@]+"${label_args[@]}"}")"
number="${url##*/}"
if [ -n "$issue_type" ]; then
  target="${repo:-$(gh repo view --json nameWithOwner --jq .nameWithOwner)}"
  if ! gh api --method PATCH "repos/$target/issues/$number" \
         -f type="$issue_type" --silent 2>/dev/null; then
    echo "::warning::#$number was filed without its issue type ($issue_type); set it at pickup with scripts/gh/fields.sh"
  fi
fi
emit "$number" created
