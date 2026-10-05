#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/gh/fields.sh: the issue type, the Priority and the Effort field, by
# issue number.
#
# WHY THIS EXISTS: since 2026-10-02 the type of an issue (Bug, Feature, Task)
# is GitHub's native issue type, its priority (Urgent, High, Medium, Low) is
# the organisation's Priority issue field and its effort (High, Medium, Low)
# the organisation's Effort issue field. None of the three has a `gh issue`
# subcommand: all are set through GraphQL mutations that take node ids for
# the issue, the type, the field and the option. This wrapper resolves every
# id from a name, so a call reads as the intent and fails loud on a typo.
#
# Official docs (durable references):
#   Issue types ......... https://docs.github.com/en/issues/tracking-your-work-with-issues/configuring-issues/managing-issue-types-in-an-organization
#   Issue fields ........ https://docs.github.com/en/issues/tracking-your-work-with-issues/configuring-issues/managing-issue-fields-in-an-organization
#   GraphQL mutations ... https://docs.github.com/en/graphql/reference/mutations
#                         (updateIssueIssueType, setIssueFieldValue)
#
# Policy: .claude/rules/issue-workflow.md §Type, priority and labels.
#
# Usage:
#   scripts/gh/fields.sh type     <n> <bug|feature|task>
#   scripts/gh/fields.sh priority <n> <urgent|high|medium|low>
#   scripts/gh/fields.sh effort   <n> <high|medium|low>
#   scripts/gh/fields.sh show     <n>
#   scripts/gh/fields.sh new <bug|feature|task> <urgent|high|medium|low> <high|medium|low> <gh issue create args...>
#       Creates the issue with `gh issue create` (pass --title, --body,
#       --milestone, --label as usual) and sets type, priority and effort on
#       it; prints the URL. A token that may not read the organisation's
#       issue types files the issue with its labels alone and says so on
#       stderr, so a scheduled lane reports its finding either way.
#   scripts/gh/fields.sh --self-test
#       Drives `new` against a stub gh, with the organisation answering and
#       with the empty answer a workflow token gets.

# shellcheck disable=SC2016 # every $o/$n/$i/$t/$f in a query is a GraphQL variable, and $w/$f/$fl in a jq filter is a jq binding
set -euo pipefail

die() {
  echo "gh-fields: $*" >&2
  exit 1
}

# The self-test stands before the preflight below, because it answers every
# gh call from a stub on PATH and the real `gh repo view` refuses a runner
# that holds no token for this repository.
self_test() {
  command -v jq >/dev/null 2>&1 || die "jq is not installed"
  local work stub created types fields create
  work="$(mktemp -d)"
  stub="$work/bin"
  created="$work/created"
  mkdir -p "$stub"
  cat > "$stub/gh" <<'STUB'
#!/usr/bin/env bash
# The gh a self-test run of scripts/gh/fields.sh speaks to. It reads the
# answer to give from the environment, applies a --jq filter when one is
# passed, and writes what `issue create` received where the test reads it.
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
case "${1:-} ${2:-}" in
  "repo view")
    emit '{"nameWithOwner":"Example-Org/Example"}'
    ;;
  "api graphql")
    case "$*" in
      *updateIssueIssueType*)
        emit '{"data":{"updateIssueIssueType":{"issue":{"issueType":{"name":"Bug"}}}}}'
        ;;
      *setIssueFieldValue*)
        emit '{"data":{"setIssueFieldValue":{"issue":{"issueFieldValues":{"nodes":[{"field":{"name":"Priority"},"value":"High"},{"field":{"name":"Effort"},"value":"Low"}]}}}}}'
        ;;
      *issueFields*)
        emit "$GH_STUB_FIELDS"
        ;;
      *issueTypes*)
        emit "$GH_STUB_TYPES"
        ;;
      *"issue(number"*)
        emit '{"data":{"repository":{"issue":{"id":"I_stub"}}}}'
        ;;
      *)
        echo "stub: an unexpected query: $*" >&2
        exit 90
        ;;
    esac
    ;;
  "issue create")
    printf '%s\n' "$*" > "$GH_STUB_CREATED"
    if [ "${GH_STUB_CREATE:-ok}" != ok ]; then
      echo "stub: gh issue create refused" >&2
      exit 1
    fi
    printf '%s\n' "https://github.com/Example-Org/Example/issues/4242"
    ;;
  *)
    echo "stub: an unexpected call: $*" >&2
    exit 90
    ;;
esac
STUB
  chmod 0755 "$stub/gh"

  # run NAME CODE ARGS…: this program through the stub, landing on CODE.
  run() {
    local name=$1 code=$2 rc=0
    shift 2
    PATH="$stub:$PATH" GH_STUB_CREATED="$created" \
      GH_STUB_TYPES="$types" GH_STUB_FIELDS="$fields" GH_STUB_CREATE="$create" \
      bash "$0" "$@" > "$work/out" 2> "$work/err" || rc=$?
    if [[ "$rc" -ne "$code" ]]; then
      echo "gh-fields: self-test failed: $name exited $rc, wanted $code." >&2
      cat "$work/out" "$work/err" >&2
      exit 1
    fi
  }
  # said NAME FILE NEEDLE: the case left that sentence where it belongs.
  said() {
    local name=$1 file=$2 needle=$3
    if ! grep -qF -- "$needle" "$file"; then
      echo "gh-fields: self-test failed: $name did not say '$needle'." >&2
      cat "$work/out" "$work/err" >&2
      exit 1
    fi
  }

  types='{"data":{"organization":{"issueTypes":{"nodes":[{"id":"IT_bug","name":"Bug","isEnabled":true}]}}}}'
  fields='{"data":{"organization":{"issueFields":{"nodes":[{"id":"IF_priority","name":"Priority","options":[{"id":"OPT_high","name":"High"}]},{"id":"IF_effort","name":"Effort","options":[{"id":"OPT_low","name":"Low"}]}]}}}}'
  create=ok

  run "an organisation that answers" 0 new bug high low --title t --body b
  said "an organisation that answers" "$work/out" "#4242 is a Bug"
  said "an organisation that answers" "$work/out" "#4242 has priority High"
  said "an organisation that answers" "$work/out" "#4242 has effort Low"
  said "an organisation that answers" "$work/out" "https://github.com/Example-Org/Example/issues/4242"

  run "a type the organisation does not carry" 1 new dragon high low --title t --body b
  said "a type the organisation does not carry" "$work/err" "no enabled issue type named 'Dragon'"

  # The answer the default GITHUB_TOKEN of a workflow gets: `organization` is
  # null rather than an error, so a lane that trusted it would stop before it
  # reached `gh issue create` and lose its finding.
  types='{"data":{"organization":null}}'
  fields='{"data":{"organization":null}}'

  run "a token that may not read the types" 0 new task medium low --title t --body b --label ci
  said "a token that may not read the types" "$work/err" "type, priority and effort"
  said "a token that may not read the types" "$work/out" "https://github.com/Example-Org/Example/issues/4242"
  said "the labels it was given" "$created" "--label ci"

  run "an @path issue number" 1 show @/etc/hosts
  said "an @path issue number" "$work/err" "not an issue number"
  run "an @path issue number to set" 1 type @/etc/hosts bug
  said "an @path issue number to set" "$work/err" "not an issue number"

  create=fail
  run "a refused gh issue create" 1 new bug high low --title t --body b
  said "a refused gh issue create" "$work/err" "gh issue create failed"

  rm -r "$work"
  echo "gh-fields: self-test OK."
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit 0
fi

command -v gh >/dev/null 2>&1 || die "the GitHub CLI (gh) is not installed"
command -v jq >/dev/null 2>&1 || die "jq is not installed"

REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner 2>/dev/null)" ||
  die "could not resolve the current repository (run inside a gh-authenticated clone)"
OWNER="${REPO%%/*}"
NAME="${REPO##*/}"

usage() {
  sed -n '/^# Usage:/,/^$/p' "$0" | sed 's/^# \{0,1\}//'
  exit 2
}

# Title-case the lower-case word the command line takes: bug -> Bug.
titled() {
  printf '%s%s' "$(printf '%s' "${1:0:1}" | tr '[:lower:]' '[:upper:]')" "${1:1}"
}

# An issue number is digits only: gh reads a -F value that starts with @ as a
# file and would send its contents to GitHub.
need_number() {
  [[ "$1" =~ ^[0-9]+$ ]] || die "not an issue number: '$1'"
}

issue_id() {
  local id
  need_number "$1"
  id="$(gh api graphql -f query='query($o:String!,$n:String!,$i:Int!){ repository(owner:$o,name:$n){ issue(number:$i){ id } } }' \
    -f o="$OWNER" -f n="$NAME" -F i="$1" --jq '.data.repository.issue.id' 2>/dev/null)" ||
    die "could not resolve issue #$1"
  [[ -n "$id" && "$id" != "null" ]] || die "no issue #$1 in $REPO"
  printf '%s' "$id"
}

type_id() {
  local want id
  want="$(titled "$1")"
  id="$(gh api graphql -f query='query($o:String!){ organization(login:$o){ issueTypes(first:20){ nodes{ id name isEnabled } } } }' \
    -f o="$OWNER" 2>/dev/null | jq -r --arg w "$want" '.data.organization.issueTypes.nodes[] | select(.name==$w and .isEnabled) | .id')" ||
    die "could not read the issue types of $OWNER"
  [[ -n "$id" ]] || die "$OWNER has no enabled issue type named '$want' (the set is Bug, Feature, Task)"
  printf '%s' "$id"
}

# Prints "<field id> <option id>" for the option named on the single-select
# field named: field_option Priority high -> "IFSS_... IFSSO_...".
field_option() {
  local field="$1" want out
  want="$(titled "$2")"
  out="$(gh api graphql -f query='query($o:String!){ organization(login:$o){ issueFields(first:20){ nodes{ ... on IssueFieldSingleSelect{ id name options{ id name } } } } } }' \
    -f o="$OWNER" 2>/dev/null | jq -r --arg fl "$field" --arg w "$want" '.data.organization.issueFields.nodes[] | select(.name==$fl) | .id as $f | .options[] | select(.name==$w) | "\($f) \(.id)"')" ||
    die "could not read the issue fields of $OWNER"
  [[ -n "$out" ]] || die "$OWNER has no $field option named '$want' (Priority: Urgent, High, Medium, Low; Effort: High, Medium, Low)"
  printf '%s' "$out"
}

set_type() {
  local n="$1" kind="$2" iid tid got
  iid="$(issue_id "$n")"
  tid="$(type_id "$kind")"
  got="$(gh api graphql -f query='mutation($i:ID!,$t:ID!){ updateIssueIssueType(input:{issueId:$i,issueTypeId:$t}){ issue{ issueType{ name } } } }' \
    -f i="$iid" -f t="$tid" --jq '.data.updateIssueIssueType.issue.issueType.name')" ||
    die "setting the type of #$n failed"
  echo "ok: #$n is a $got"
}

# set_field <n> <Priority|Effort> <option word>
set_field() {
  local n="$1" field="$2" level="$3" iid fid oid got
  iid="$(issue_id "$n")"
  read -r fid oid <<<"$(field_option "$field" "$level")"
  got="$(gh api graphql -f query='mutation($i:ID!,$f:ID!,$o:ID!){ setIssueFieldValue(input:{issueId:$i,issueFields:[{fieldId:$f,singleSelectOptionId:$o}]}){ issue{ issueFieldValues(first:10){ nodes{ ... on IssueFieldSingleSelectValue{ field{ ... on IssueFieldSingleSelect{ name } } value } } } } } }' \
    -f i="$iid" -f f="$fid" -f o="$oid" 2>/dev/null | jq -r --arg fl "$field" '[.data.setIssueFieldValue.issue.issueFieldValues.nodes[] | select(.field.name==$fl) | .value] | first')" ||
    die "setting the $field of #$n failed"
  [[ -n "$got" && "$got" != "null" ]] || die "setting the $field of #$n failed"
  echo "ok: #$n has $(printf '%s' "$field" | tr '[:upper:]' '[:lower:]') $got"
}

show() {
  local n="$1"
  need_number "$n"
  gh api graphql -f query='query($o:String!,$n:String!,$i:Int!){ repository(owner:$o,name:$n){ issue(number:$i){ number title issueType{ name } issueFieldValues(first:10){ nodes{ ... on IssueFieldSingleSelectValue{ field{ ... on IssueFieldSingleSelect{ name } } value } } } labels(first:20){ nodes{ name } } milestone{ title } } } }' \
    -f o="$OWNER" -f n="$NAME" -F i="$n" \
    --jq '.data.repository.issue | "#\(.number)  \(.title)\n  type:      \(.issueType.name // "none")\n  priority:  \([.issueFieldValues.nodes[] | select(.field.name=="Priority") | .value] | first // "none")\n  effort:    \([.issueFieldValues.nodes[] | select(.field.name=="Effort") | .value] | first // "none")\n  labels:    \([.labels.nodes[].name] | join(", "))\n  milestone: \(.milestone.title // "none")"' ||
    die "could not read #$n"
}

# Whether this token may read the organisation's issue types at all. The
# default GITHUB_TOKEN of a workflow may not, and GraphQL answers
# `organization: null` rather than an error, so the read is a probe.
fields_readable() {
  local answer=no
  answer="$(gh api graphql -f query='query($o:String!){ organization(login:$o){ issueTypes(first:20){ nodes{ id } } } }' \
    -f o="$OWNER" 2>/dev/null |
    jq -r 'if (.data.organization.issueTypes.nodes | type) == "array" then "yes" else "no" end')" || answer=no
  [[ "$answer" == "yes" ]]
}

new() {
  local kind="$1" level="$2" effort="$3" url n
  shift 3
  if ! fields_readable; then
    # A lane that cannot set the three still has to file its finding, so the
    # issue lands with the labels it was given and the orchestrator sets type,
    # priority and effort at pickup (.claude/rules/issue-workflow.md).
    echo "gh-fields: this token cannot read the issue types of $OWNER, so the issue is filed without type, priority and effort; set them at pickup" >&2
    url="$(gh issue create "$@")" || die "gh issue create failed"
    echo "$url"
    return 0
  fi
  type_id "$kind" >/dev/null
  field_option Priority "$level" >/dev/null
  field_option Effort "$effort" >/dev/null
  url="$(gh issue create "$@")" || die "gh issue create failed"
  n="${url##*/}"
  set_type "$n" "$kind"
  set_field "$n" Priority "$level"
  set_field "$n" Effort "$effort"
  echo "$url"
}

cmd="${1:-}"
case "$cmd" in
  type)     [[ $# -eq 3 ]] || usage; set_type "$2" "$3" ;;
  priority) [[ $# -eq 3 ]] || usage; set_field "$2" Priority "$3" ;;
  effort)   [[ $# -eq 3 ]] || usage; set_field "$2" Effort "$3" ;;
  show)     [[ $# -eq 2 ]] || usage; show "$2" ;;
  new)      [[ $# -ge 5 ]] || usage; shift; new "$@" ;;
  *)        usage ;;
esac
