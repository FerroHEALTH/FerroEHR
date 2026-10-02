#!/usr/bin/env bash
# SPDX-FileCopyrightText: Vernum Projecten B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/gh/labels.sh: declare the FerroEHR issue-label taxonomy.
#
# Creates (idempotently) the labels the tracker workflow assumes:
#   * the WORK-KIND labels a Task carries to pick its commit type
#     (documentation/chore/refactor/perf/test/ci); a Bug and a Feature carry
#     none, because the native issue type already says fix or feat;
#   * the DOMAIN labels: one per openEHR component, the spec-update triage
#     set, the upstream-report pair, and the product areas;
#   * the workflow labels and the pull-request escape hatches CI reads.
# The type of an issue (Bug, Feature, Task) and its priority (Urgent, High,
# Medium, Low) are NOT labels since 2026-10-02: they are GitHub's native issue
# type and the organisation's Priority issue field, set with
# scripts/gh/fields.sh. The six labels that carried them before (bug,
# enhancement, P0 to P3) are deleted here, so a re-run converges a repository
# that still has them. Run scripts/gh/migrate-fields.sh first on a repository
# whose open issues still carry them, or the old values are lost with the
# labels. GitHub's default labels (duplicate, invalid, question, ...) and the
# ones Dependabot applies (dependencies, github_actions, rust, docker) are
# left as they are.
#
# `gh label create --force` updates an existing label in place, so re-running
# this is safe and converges the colours and descriptions to the values below.
#
# Taxonomy policy: .claude/rules/issue-workflow.md
# Official docs: https://cli.github.com/manual/gh_label_create
#
# Usage:
#   scripts/gh/labels.sh
#   scripts/gh/labels.sh --self-test
#       Drives this program against a stub gh on PATH: every label is created
#       with --force, a retired name is deleted only when the repository
#       carries it, and a refused gh call fails the run.

set -euo pipefail

die() {
  echo "gh-labels: $*" >&2
  exit 1
}

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
# The gh a self-test run of scripts/gh/labels.sh speaks to. It writes every
# call where the test reads it, applies a --jq filter when one is passed, and
# lists the labels the run is told the repository carries.
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
printf '%s\n' "$*" >> "$GH_STUB_CALLS"
case "${1:-} ${2:-}" in
  "repo view")
    emit '{"nameWithOwner":"Example-Org/Example"}'
    ;;
  "label list")
    emit "$GH_STUB_LABELS"
    ;;
  "label create")
    if [ "${GH_STUB_CREATE:-ok}" != ok ]; then
      echo "stub: gh label create refused" >&2
      exit 1
    fi
    ;;
  "label delete") ;;
  *)
    echo "stub: an unexpected call: $*" >&2
    exit 90
    ;;
esac
STUB
  chmod 0755 "$stub/gh"

  local labels create rc out
  # run NAME CODE: this program through the stub, landing on CODE.
  run() {
    local name=$1 code=$2
    rc=0
    : > "$calls"
    PATH="$stub:$PATH" GH_STUB_CALLS="$calls" \
      GH_STUB_LABELS="$labels" GH_STUB_CREATE="$create" \
      bash "$0" > "$work/out" 2> "$work/err" || rc=$?
    out="$work/out"
    if [[ "$rc" -ne "$code" ]]; then
      echo "gh-labels: self-test failed: $name exited $rc, wanted $code." >&2
      cat "$work/out" "$work/err" >&2
      exit 1
    fi
  }
  # said NAME FILE NEEDLE: the case left that sentence where it belongs.
  said() {
    local name=$1 file=$2 needle=$3
    if ! grep -qF -- "$needle" "$file"; then
      echo "gh-labels: self-test failed: $name did not say '$needle'." >&2
      cat "$work/out" "$work/err" "$calls" >&2
      exit 1
    fi
  }
  # never NAME FILE NEEDLE: and never that one.
  never() {
    local name=$1 file=$2 needle=$3
    if grep -qF -- "$needle" "$file"; then
      echo "gh-labels: self-test failed: $name said '$needle'." >&2
      cat "$work/out" "$work/err" "$calls" >&2
      exit 1
    fi
  }

  # A repository that carries none of the retired names: nothing is deleted,
  # and every label of the taxonomy is created in place with --force.
  labels='[{"name":"documentation"}]'
  create=ok
  run "a repository without the retired labels" 0
  said "a repository without the retired labels" "$out" "ok: documentation"
  said "a repository without the retired labels" "$out" "ok: spec:RM"
  said "a repository without the retired labels" "$out" "ok: upstream-report"
  said "a repository without the retired labels" "$out" "ok: no-crate-bump"
  said "a repository without the retired labels" "$calls" "label create spec:ITS-REST --color 0e8a16"
  said "a repository without the retired labels" "$calls" "label create documentation --color 0075ca"
  said "a repository without the retired labels" "$calls" "--force"
  never "a repository without the retired labels" "$out" "retired:"
  never "a repository without the retired labels" "$calls" "label delete"
  never "a repository without the retired labels" "$calls" "label create bug"
  never "a repository without the retired labels" "$calls" "label create P1"

  # The same repository with two of the six the native issue type and the
  # Priority field replaced on 2026-10-02: both go, and no other name does.
  labels='[{"name":"enhancement"},{"name":"P1"},{"name":"documentation"}]'
  run "a repository that still carries two retired labels" 0
  said "a repository that still carries two retired labels" "$out" "retired: enhancement"
  said "a repository that still carries two retired labels" "$out" "retired: P1"
  said "a repository that still carries two retired labels" "$calls" "label delete enhancement --yes"
  said "a repository that still carries two retired labels" "$calls" "label delete P1 --yes"
  never "a repository that still carries two retired labels" "$out" "retired: P0"
  never "a repository that still carries two retired labels" "$calls" "label delete documentation"

  # A gh that refuses the first create stops the run rather than reporting a
  # taxonomy it did not write.
  create=fail
  run "a refused gh label create" 1
  never "a refused gh label create" "$out" "done."

  rm -r "$work"
  echo "gh-labels: self-test OK."
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit 0
fi

command -v gh >/dev/null 2>&1 || die "the GitHub CLI (gh) is not installed"
command -v jq >/dev/null 2>&1 || die "jq is not installed"
gh repo view --json nameWithOwner --jq .nameWithOwner >/dev/null 2>&1 ||
  die "could not resolve the current repository (run inside a gh-authenticated clone)"

# The labels the repository carries today, read once, so no gh call writes
# into a pipe grep has already stopped reading under `set -o pipefail`.
EXISTING="$(gh label list --limit 200 --json name --jq '.[].name')"

# label <name> <hex-color> <description>
label() {
  gh label create "$1" --color "$2" --description "$3" --force >/dev/null
  echo "ok: $1"
}

# retire <name>: delete a label the taxonomy no longer carries, if it exists.
retire() {
  if grep -qx -- "$1" <<<"$EXISTING"; then
    gh label delete "$1" --yes >/dev/null
    echo "retired: $1"
  fi
}

echo "== retired labels (the native issue type and the Priority field carry these now) =="
for l in bug enhancement P0 P1 P2 P3; do retire "$l"; done

echo "== work-kind labels (on a Task only; picks the conventional-commit type) =="
label documentation 0075ca "Docs-only work. Maps to a docs/ branch and a docs: commit."
label chore         c5def1 "Maintenance with no product change. chore/ and chore:."
label refactor      d4c5f9 "Behaviour-preserving code change. refactor/ and refactor:."
label perf          f9d0c4 "Performance work. perf/ and perf:."
label test          0e8a16 "Test-only work. test/ and test:."
label ci            bfd4f2 "CI, automation and build pipeline. ci/ and ci:."

echo "== domain labels (one per openEHR component) =="
for c in BASE RM AM LANG QUERY TERM SM CNF ITS ITS-XML ITS-JSON ITS-BMM ITS-REST; do
  label "spec:$c" 0e8a16 "openEHR $c component"
done

echo "== spec-update triage =="
label spec-update           b60205 "Upstream openEHR specification change awaiting conformance triage"
label spec-impact:behaviour d93f0b "Triaged: requires implementation/regeneration to stay conformant"
label spec-impact:docs-only fbca04 "Triaged: editorial/typo upstream change, no behaviour impact"
label spec-impact:none      c2e0c6 "Triaged: no impact on our surface (adjudicated, with citation)"
label spec-version:current  0e8a16 "Upstream fix within the release line we pin: act now (re-vendor, regenerate, implement)"
label spec-version:next     5319e7 "Lands in a newer upstream release than our pin: collect for the version-adoption track"
label blocked-upstream      999999 "Waiting on upstream publication (resolved in Jira, normative text not yet in the public spec repos)"

echo "== outbound reports =="
label upstream-report    8b0000 "Outbound report of a defect/contradiction/silence in the released openEHR specs"
label upstream-confirmed b45309 "Upstream-report verified first-hand as genuine; awaiting openEHR action"

echo "== product areas =="
label viewer     5319e7 "The FerroEHR Viewer (the console, its own OCI image)"
label fhir       f58220 "FHIR R4 connector (mapping, outbound, terminology client, subject proxy): our own extension"
label regulation 0e8a16 "A regulation, standard or jurisdiction FerroEHR should be measured against"

echo "== workflow labels =="
label on-hold      6e7781 "Deliberately parked by owner decision: not a pickup candidate until the hold record lifts"
label dependencies 0366d6 "Pull requests that update a dependency file"

echo "== pull-request escape hatches (read by CI) =="
label no-changelog        bfdadc "Skip the changelog-guard CI job (change is not user-visible)"
label no-crate-bump       fbca04 "crate-version-guard escape: diff provably does not alter packaged crate bytes"
label no-chart-bump       bfd4f2 "chart-version-guard: packaged chart diff that cannot change consumer output"
label no-ui-visual-change bfdadc "Skip the ui-screenshot-guard: viewer source change with no visual effect"
label no-conformance-run  d4c5f9 "veredictum-pin-guard escape: the pin bump's acceptance run is deliberately deferred"

echo "done."
