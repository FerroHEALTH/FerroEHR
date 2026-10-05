#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/gh/migrate-fields.sh: move every OPEN issue from the type and
# priority labels onto GitHub's native issue type and the organisation's
# Priority and Effort issue fields.
#
# Closed issues are out of scope (owner decision 2026-10-02): they keep no
# type or priority once scripts/gh/labels.sh deletes the labels.
#
# The mapping (.claude/rules/issue-workflow.md §Type, priority and labels):
#   * type: `bug` -> Bug, `enhancement` -> Feature; an issue with neither
#     label keeps a type it already has, and otherwise takes the one its title
#     prefix names (`fix` -> Bug, `feat` -> Feature), `upstream-report` ->
#     Bug, anything else -> Task;
#   * priority: P0 -> Urgent, P1 -> High, P2 -> Medium, P3 -> Low; an issue
#     with no P label keeps a priority it already has, or takes the one judged
#     for it in PRIORITY_JUDGED below;
#   * effort: every issue that has none gets the value judged for it in
#     EFFORT_JUDGED below;
#   * a Task with no work-kind label gets the one its title's conventional
#     prefix implies (`docs:` -> documentation, `build:` -> ci, `chore:`,
#     `refactor:`, `perf:`, `test:`, `ci:` as named), `chore` when it is a
#     `spec-update` triage issue, or the one judged for it in WORKKIND_JUDGED.
# Every write goes through scripts/gh/fields.sh or `gh issue edit
# --add-label`, and only where the issue differs from the mapping, so a re-run
# changes nothing. Run it before scripts/gh/labels.sh deletes the old labels:
# once they are gone, the type and priority they carried are lost.
#
# Usage:
#   scripts/gh/migrate-fields.sh plan
#       Prints every change `apply` would make and the totals; writes nothing.
#       Exits 1 when an issue cannot be mapped (a conflict, or an issue the
#       tables below do not cover).
#   scripts/gh/migrate-fields.sh apply
#       Makes those changes.
#   scripts/gh/migrate-fields.sh verify
#       Exits 0 only when no open issue differs from the mapping and none is
#       unmappable, and prints the totals.
#   scripts/gh/migrate-fields.sh --self-test
#       Drives the three commands against a stub gh and a stub fields.sh.

# shellcheck disable=SC2016 # $o/$n/$endCursor in the query are GraphQL variables, and $p is a jq binding
set -euo pipefail

die() {
  echo "gh-migrate-fields: $*" >&2
  exit 1
}

# The effort of each issue open on 2026-10-02, judged from its body and its
# acceptance criteria: Low is one sitting, Medium one pull request across
# crates or with a fixture, High more than one pull request or a held design.
EFFORT_JUDGED="${GH_MIGRATE_EFFORT:-
157 low
300 medium
1357 low
1399 low
1407 low
1420 low
1469 low
1917 high
1920 low
2236 medium
2522 medium
2537 low
2538 medium
2603 high
2793 medium
2807 medium
2998 low
2999 low
3057 low
3080 medium
3085 high
3086 high
3087 high
3088 high
3089 high
3105 medium
3337 high
3350 high
3377 high
3379 high
3380 high
3381 high
3382 medium
3383 high
3452 medium
3473 low
3476 medium
3481 medium
3490 low
3503 medium
3520 medium
3524 medium
}"

# The priority of each open issue that carried no P label, judged from what
# it waits on.
PRIORITY_JUDGED="${GH_MIGRATE_PRIORITY:-
157 low
1920 low
2807 low
}"

# The work kind of each Task whose labels name none, whose title carries no
# conventional prefix and which is no spec-update triage, judged from what the
# issue asks for.
WORKKIND_JUDGED="${GH_MIGRATE_WORKKIND:-}"

WORKKINDS="documentation chore refactor perf test ci"

# The self-test stands before the preflight below, because it answers every
# gh call from a stub on PATH and the real `gh repo view` refuses a runner
# that holds no token for this repository.
self_test() {
  command -v jq >/dev/null 2>&1 || die "jq is not installed"
  local work stub calls
  work="$(mktemp -d)"
  stub="$work/bin"
  calls="$work/calls"
  mkdir -p "$stub"
  cat > "$stub/gh" <<'STUB'
#!/usr/bin/env bash
# The gh a self-test run of scripts/gh/migrate-fields.sh speaks to. It writes
# every call where the test reads it, applies a --jq filter when one is
# passed, and answers the issue listing with the fixture it is given.
set -euo pipefail
filter=""
previous=""
for argument in "$@"; do
  if [ "$previous" = --jq ]; then
    filter="$argument"
  fi
  previous="$argument"
done
emit() {
  if [ -n "$filter" ]; then
    printf '%s' "$1" | jq -r "$filter"
  else
    printf '%s\n' "$1"
  fi
}
printf 'gh %s\n' "$*" >> "$GH_STUB_CALLS"
case "${1:-} ${2:-}" in
  "repo view") emit '{"nameWithOwner":"Example-Org/Example"}' ;;
  "api graphql") emit "$GH_STUB_ISSUES" ;;
  "issue edit") ;;
  *)
    echo "stub: an unexpected call: $*" >&2
    exit 90
    ;;
esac
STUB
  cat > "$stub/fields.sh" <<'STUB'
#!/usr/bin/env bash
# The fields.sh a self-test run speaks to: it records the call and agrees.
printf 'fields %s\n' "$*" >> "$GH_STUB_CALLS"
echo "ok: $*"
STUB
  chmod 0755 "$stub/gh" "$stub/fields.sh"

  local issues rc
  # run NAME CODE ARGS…: this program through the stubs, landing on CODE.
  run() {
    local name=$1 code=$2
    shift 2
    rc=0
    : > "$calls"
    PATH="$stub:$PATH" GH_STUB_CALLS="$calls" GH_STUB_ISSUES="$issues" \
      GH_FIELDS_SH="$stub/fields.sh" \
      GH_MIGRATE_EFFORT=$'1 low\n2 medium\n3 low\n4 high\n5 high\n6 low\n10 low' \
      GH_MIGRATE_PRIORITY=$'6 low\n10 medium' GH_MIGRATE_WORKKIND='6 chore' \
      bash "$0" "$@" > "$work/out" 2> "$work/err" || rc=$?
    if [[ "$rc" -ne "$code" ]]; then
      echo "gh-migrate-fields: self-test failed: $name exited $rc, wanted $code." >&2
      cat "$work/out" "$work/err" >&2
      exit 1
    fi
  }
  # said NAME FILE NEEDLE: the case left that sentence where it belongs.
  said() {
    local name=$1 file=$2 needle=$3
    if ! grep -qF -- "$needle" "$file"; then
      echo "gh-migrate-fields: self-test failed: $name did not say '$needle'." >&2
      cat "$work/out" "$work/err" "$calls" >&2
      exit 1
    fi
  }
  # never NAME FILE NEEDLE: and never that one.
  never() {
    local name=$1 file=$2 needle=$3
    if grep -qF -- "$needle" "$file"; then
      echo "gh-migrate-fields: self-test failed: $name said '$needle'." >&2
      cat "$work/out" "$work/err" "$calls" >&2
      exit 1
    fi
  }
  # node NUMBER STATE TYPE PRIORITY EFFORT LABELS TITLE: one issue as GraphQL
  # answers it, with "" for an absent type, priority or effort.
  node() {
    jq -cn --argjson n "$1" --arg s "$2" --arg t "$3" --arg p "$4" --arg e "$5" \
      --arg l "$6" --arg title "$7" '{number:$n, state:$s, title:$title,
        issueType: (if $t == "" then null else {name:$t} end),
        issueFieldValues:{nodes:(
          (if $p == "" then [] else [{field:{name:"Priority"}, value:$p}] end)
          + (if $e == "" then [] else [{field:{name:"Effort"}, value:$e}] end) + [{}])},
        labels:{nodes:($l | split(",") | map(select(. != "")) | map({name:.}))}}'
  }
  # page NODE…: the listing GraphQL answers.
  page() {
    printf '%s\n' "$@" | jq -cs '{data:{repository:{issues:{pageInfo:{hasNextPage:false,endCursor:null},nodes:.}}}}'
  }

  # Labels still on the issues: every kind of change, and one issue that is
  # already where the mapping puts it.
  issues="$(page \
    "$(node 1 OPEN "" "" "" "bug,P0,spec:RM" "A defect")" \
    "$(node 2 OPEN Task "" "" "enhancement,P2" "A capability")" \
    "$(node 3 OPEN "" "" "" "P3" "docs(readme): say it")" \
    "$(node 4 OPEN Task High High "chore" "Housekeeping")" \
    "$(node 5 OPEN "" "" "" "spec-update,P3,blocked-upstream" "spec-update(AM): a grammar change")" \
    "$(node 6 OPEN "" "" "" "upstream-report" "upstream(RM): a gap")" \
    "$(node 10 OPEN "" "" "" "" "feat(viewer): a page")")"
  run "a plan over labelled issues" 0 plan
  said "a plan over labelled issues" "$work/out" "#1  type Bug"
  said "a plan over labelled issues" "$work/out" "#1  priority Urgent"
  said "a plan over labelled issues" "$work/out" "#1  effort Low"
  said "a plan over labelled issues" "$work/out" "#2  type Feature (was Task)"
  said "a plan over labelled issues" "$work/out" "#3  label +documentation"
  said "a plan over labelled issues" "$work/out" "#5  label +chore"
  said "a plan over labelled issues" "$work/out" "#6  type Bug"
  said "a plan over labelled issues" "$work/out" "#6  priority Low"
  said "a plan over labelled issues" "$work/out" "#10  type Feature"
  said "a plan over labelled issues" "$work/out" "#10  priority Medium"
  said "a plan over labelled issues" "$work/out" "types: Bug 2, Feature 2, Task 3"
  said "a plan over labelled issues" "$work/out" "priorities: Urgent 1, High 1, Medium 2, Low 3, none 0"
  never "a plan over labelled issues" "$work/out" "#4  "
  never "a plan over labelled issues" "$work/out" "#6  label"
  never "a plan over labelled issues" "$calls" "fields "
  never "a plan over labelled issues" "$calls" "issue edit"

  run "an apply over labelled issues" 0 apply
  said "an apply over labelled issues" "$calls" "fields type 1 bug"
  said "an apply over labelled issues" "$calls" "fields priority 1 urgent"
  said "an apply over labelled issues" "$calls" "fields effort 1 low"
  said "an apply over labelled issues" "$calls" "fields type 2 feature"
  said "an apply over labelled issues" "$calls" "fields priority 3 low"
  said "an apply over labelled issues" "$calls" "gh issue edit 3 --add-label documentation"
  said "an apply over labelled issues" "$calls" "gh issue edit 5 --add-label chore"
  never "an apply over labelled issues" "$calls" "fields type 4"
  never "an apply over labelled issues" "$calls" "fields effort 4"

  run "a verify before the apply took" 1 verify
  said "a verify before the apply took" "$work/err" "changes still to make"

  # The same issues once migrated and with the old labels deleted.
  issues="$(page \
    "$(node 1 OPEN Bug Urgent Low "spec:RM" "A defect")" \
    "$(node 2 OPEN Feature Medium Medium "" "A capability")" \
    "$(node 3 OPEN Task Low Low "documentation" "docs(readme): say it")" \
    "$(node 4 OPEN Task High High "chore" "Housekeeping")" \
    "$(node 5 OPEN Task Low High "chore,spec-update,blocked-upstream" "spec-update(AM): a grammar change")" \
    "$(node 6 OPEN Bug Low Low "upstream-report" "upstream(RM): a gap")" \
    "$(node 10 OPEN Feature Medium Low "" "feat(viewer): a page")")"
  run "a verify after the migration" 0 verify
  said "a verify after the migration" "$work/out" "types: Bug 2, Feature 2, Task 3"
  run "an apply after the migration" 0 apply
  never "an apply after the migration" "$calls" "fields "

  # An issue the mapping cannot place stops the plan and names it.
  issues="$(page \
    "$(node 7 OPEN "" "" "" "bug,enhancement,P1" "Both")" \
    "$(node 8 OPEN "" "" "" "P1" "No kind")" \
    "$(node 9 OPEN Task "" "" "test,P1,P2" "Two priorities")" \
    "$(node 11 OPEN Task "" "" "chore" "No priority")")"
  run "issues the mapping cannot place" 1 plan
  said "issues the mapping cannot place" "$work/err" "#7 carries both bug and enhancement"
  said "issues the mapping cannot place" "$work/err" "#8 is open with no judged effort"
  said "issues the mapping cannot place" "$work/err" "#8 is a Task with no work-kind label"
  said "issues the mapping cannot place" "$work/err" "#9 carries more than one priority label"
  said "issues the mapping cannot place" "$work/err" "#11 has no priority label and no judged priority"
  run "an apply over issues it cannot place" 1 apply
  never "an apply over issues it cannot place" "$calls" "fields "

  rm -r "$work"
  echo "gh-migrate-fields: self-test OK."
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit 0
fi

usage() {
  sed -n '/^# Usage:/,/^$/p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
}

mode="${1:-}"
case "$mode" in
  plan | apply | verify) [[ $# -eq 1 ]] || usage ;;
  *) usage ;;
esac

command -v gh >/dev/null 2>&1 || die "the GitHub CLI (gh) is not installed"
command -v jq >/dev/null 2>&1 || die "jq is not installed"
REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null)" ||
  die "could not resolve the current repository (run inside a gh-authenticated clone)"
FIELDS="${GH_FIELDS_SH:-$(dirname "$0")/fields.sh}"

# One line per open issue: number, state, type, priority, effort, labels,
# title, with "-" for an absent value so no tab-separated column is ever empty.
ROWS="$(gh api graphql --paginate \
  -f query='query($o:String!,$n:String!,$endCursor:String){ repository(owner:$o,name:$n){ issues(first:100, after:$endCursor, states:OPEN, orderBy:{field:CREATED_AT, direction:ASC}){ pageInfo{ hasNextPage endCursor } nodes{ number state title issueType{ name } issueFieldValues(first:10){ nodes{ ... on IssueFieldSingleSelectValue{ field{ ... on IssueFieldSingleSelect{ name } } value } } } labels(first:30){ nodes{ name } } } } } }' \
  -f o="${REPO%%/*}" -f n="${REPO##*/}" \
  --jq '.data.repository.issues.nodes[]
    | def field($p): [.issueFieldValues.nodes[] | select(.field.name? == $p) | .value] | first // "-";
      [(.number | tostring), .state, (.issueType.name // "-"), field("Priority"), field("Effort"),
       ([.labels.nodes[].name] | join(",") | if . == "" then "-" else . end), .title]
    | @tsv')" || die "could not list the issues of $REPO"

# judged TABLE NUMBER: the value a table above holds for an issue, or nothing.
judged() {
  awk -v n="$2" '$1 == n { print $2 }' <<<"$1"
}

# has LABELS NAME: whether the comma-joined label list carries NAME.
has() {
  [[ ",$1," == *",$2,"* ]]
}

lower() {
  printf '%s' "$1" | tr '[:upper:]' '[:lower:]'
}

titled() {
  printf '%s%s' "$(printf '%s' "${1:0:1}" | tr '[:lower:]' '[:upper:]')" "${1:1}"
}

changes=0
problems=0
bugs=0 features=0 tasks=0
urgent=0 high=0 medium=0 low=0 unprioritised=0
efforts=0 kinds=0

# change NUMBER WHAT: say a change, and make it when applying.
change() {
  local n="$1"
  shift
  changes=$((changes + 1))
  case "$mode" in
    apply)
      case "$1" in
        type | priority | effort) "$FIELDS" "$1" "$n" "$(lower "$2")" ;;
        label) gh issue edit "$n" --add-label "$2" >/dev/null && echo "ok: #$n carries $2" ;;
      esac
      ;;
    *) ;;
  esac
}

# was VALUE: the "(was …)" suffix for a value that is replaced.
was() {
  if [[ "$1" == "-" ]]; then printf ''; else printf ' (was %s)' "$1"; fi
}

problem() {
  echo "gh-migrate-fields: $*" >&2
  problems=$((problems + 1))
}

# The plan, issue by issue: what the mapping wants against what is there.
plan_lines=""
while IFS=$'\t' read -r n state type priority effort labels title; do
  [[ -n "$n" && "$state" == OPEN ]] || continue

  prefix="$(printf '%s' "$title" | sed -nE 's/^(feat|fix|docs|chore|refactor|perf|test|ci|build)(\([^)]*\))?!?: .*/\1/p')"

  want_type="$type"
  if has "$labels" bug && has "$labels" enhancement; then
    problem "#$n carries both bug and enhancement"
    continue
  elif has "$labels" bug; then
    want_type=Bug
  elif has "$labels" enhancement; then
    want_type=Feature
  elif [[ "$type" == "-" ]]; then
    case "$prefix" in
      fix) want_type=Bug ;;
      feat) want_type=Feature ;;
      *)
        if has "$labels" upstream-report; then want_type=Bug; else want_type=Task; fi
        ;;
    esac
  fi

  want_priority="$priority"
  found=""
  for p in P0 P1 P2 P3; do
    if has "$labels" "$p"; then found="$found $p"; fi
  done
  case "$found" in
    "")
      if [[ "$priority" == "-" ]]; then
        want_priority="$(judged "$PRIORITY_JUDGED" "$n")"
        if [[ -z "$want_priority" ]]; then
          problem "#$n has no priority label and no judged priority (add it to PRIORITY_JUDGED)"
          want_priority="-"
        else
          want_priority="$(titled "$want_priority")"
        fi
      fi
      ;;
    " P0") want_priority=Urgent ;;
    " P1") want_priority=High ;;
    " P2") want_priority=Medium ;;
    " P3") want_priority=Low ;;
    *)
      problem "#$n carries more than one priority label:$found"
      continue
      ;;
  esac

  want_effort="$effort"
  if [[ "$effort" == "-" ]]; then
    want_effort="$(judged "$EFFORT_JUDGED" "$n")"
    if [[ -z "$want_effort" ]]; then
      problem "#$n is open with no judged effort (add it to EFFORT_JUDGED)"
      want_effort="-"
    else
      want_effort="$(titled "$want_effort")"
    fi
  fi

  want_kind=""
  if [[ "$want_type" == Task ]]; then
    carried=""
    for k in $WORKKINDS; do
      if has "$labels" "$k"; then carried="$k"; fi
    done
    if [[ -z "$carried" ]]; then
      case "$prefix" in
        docs) want_kind=documentation ;;
        build) want_kind=ci ;;
        chore | refactor | perf | test | ci) want_kind="$prefix" ;;
        *)
          if has "$labels" spec-update; then
            want_kind=chore
          else
            want_kind="$(judged "$WORKKIND_JUDGED" "$n")"
          fi
          ;;
      esac
      [[ -n "$want_kind" ]] || problem "#$n is a Task with no work-kind label and no conventional prefix (add it to WORKKIND_JUDGED)"
    fi
  fi

  case "$want_type" in
    Bug) bugs=$((bugs + 1)) ;;
    Feature) features=$((features + 1)) ;;
    *) tasks=$((tasks + 1)) ;;
  esac
  case "$want_priority" in
    Urgent) urgent=$((urgent + 1)) ;;
    High) high=$((high + 1)) ;;
    Medium) medium=$((medium + 1)) ;;
    Low) low=$((low + 1)) ;;
    *) unprioritised=$((unprioritised + 1)) ;;
  esac

  if [[ "$want_type" != "$type" ]]; then
    plan_lines+="#$n  type $want_type$(was "$type")"$'\n'
    plan_lines+="@$n type $want_type"$'\n'
  fi
  if [[ "$want_priority" != "$priority" ]]; then
    plan_lines+="#$n  priority $want_priority$(was "$priority")"$'\n'
    plan_lines+="@$n priority $want_priority"$'\n'
  fi
  if [[ "$want_effort" != "$effort" ]]; then
    efforts=$((efforts + 1))
    plan_lines+="#$n  effort $want_effort"$'\n'
    plan_lines+="@$n effort $want_effort"$'\n'
  fi
  if [[ -n "$want_kind" ]]; then
    kinds=$((kinds + 1))
    plan_lines+="#$n  label +$want_kind"$'\n'
    plan_lines+="@$n label $want_kind"$'\n'
  fi
done <<<"$ROWS"

if [[ "$problems" -gt 0 ]]; then
  die "$problems issues the mapping cannot place; nothing was written"
fi

while read -r line; do
  [[ -n "$line" ]] || continue
  case "$line" in
    "#"*) [[ "$mode" == apply ]] || echo "$line" ;;
    "@"*)
      read -r at what value <<<"$line"
      change "${at#@}" "$what" "$value"
      ;;
  esac
done <<<"$plan_lines"

echo "types: Bug $bugs, Feature $features, Task $tasks"
echo "priorities: Urgent $urgent, High $high, Medium $medium, Low $low, none $unprioritised"
echo "changes: $changes (effort set on $efforts open issues, a work-kind label on $kinds Tasks)"

if [[ "$mode" == verify && "$changes" -gt 0 ]]; then
  die "$changes changes still to make; run scripts/gh/migrate-fields.sh apply"
fi
