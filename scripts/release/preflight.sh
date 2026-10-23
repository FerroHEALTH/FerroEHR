#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Replays the release `plan` job locally before the tag is pushed (no
# specification governs this: our own design). Run it on the release branch,
# or on main after the release PR merged, with the tag about to be pushed:
#
#   scripts/release/preflight.sh <tag> [--notes <file>]
#
# It runs `scripts/release/plan.sh checks`, the list the plan job runs, over
# the working tree, so a check added to the plan job is a check the preflight
# runs. The previous release is resolved as the plan job resolves it: from
# `git ls-remote` of origin by version order, so a tag pushed whose release
# never published still counts. The release notes are written to <file>
# (default: a temporary file, whose path is printed) for reading before the
# tag. Needs gh logged in (the milestone) and helm (the chart version).
# Exit 0 when the plan job would pass, 1 when a check refused, 2 on a usage
# error.
set -euo pipefail

usage() {
  echo "usage: preflight.sh <tag> [--notes <file>]" >&2
  exit 2
}

[[ "$#" -eq 1 || ("$#" -eq 3 && "$2" == "--notes" && -n "$3") ]] || usage
tag="$1"
notes="${3:-}"
if [[ -z "$notes" ]]; then
  tmp="${TMPDIR:-/tmp}"
  notes="$(mktemp "${tmp%/}/release-notes-${tag}.XXXXXX")"
fi

root="$(cd "$(dirname "$0")/../.." && pwd)"
if [[ -n "$(git -C "$root" status --porcelain --untracked-files=no)" ]]; then
  echo "note: the working tree has uncommitted changes; the plan job checks the tagged commit only" >&2
fi
echo "preflight $tag at $(git -C "$root" rev-parse --short HEAD) ($(git -C "$root" rev-parse --abbrev-ref HEAD))"

set +e
bash "$root/scripts/release/plan.sh" checks "$tag" --notes "$notes"
rc=$?
set -e
echo "release notes: $notes"
exit "$rc"
