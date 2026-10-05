#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The licence-free shaped terminology seed has ONE canonical home — the
# directory the compose stack and the conformance lane mount,
# docker/terminology/seed/ — and one copy: the Helm chart carries it under
# files/terminology/ so `terminology.enabled=true` renders a ConfigMap without
# a repository checkout (a chart's `.Files` may only read files packaged with
# the chart, so a symlink or a path outside the chart directory cannot serve
# here). This guard holds the two byte-identical, in both directions: a file
# added, removed or edited on either side fails until the other follows.
#
# Usage: scripts/checks/terminology-seed-drift.sh   (no arguments)
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_no_args "$@"
cd "$(dirname "$0")/../.."

CANONICAL=docker/terminology/seed
CHART_COPY=deploy/helm/ferroehr/files/terminology

for dir in "$CANONICAL" "$CHART_COPY"; do
  [[ -d "$dir" ]] || { echo "terminology-seed-drift: missing $dir" >&2; exit 1; }
done

canonical_files=$(find "$CANONICAL" -maxdepth 1 -name '*.json' -exec basename {} \; | sort)
chart_files=$(find "$CHART_COPY" -maxdepth 1 -name '*.json' -exec basename {} \; | sort)

# A directory that has gone empty would otherwise pass every comparison below
# while the chart renders a ConfigMap with no code system in it.
[[ -n "$canonical_files" ]] || { echo "terminology-seed-drift: no *.json under $CANONICAL" >&2; exit 1; }

status=0
if [[ "$canonical_files" != "$chart_files" ]]; then
  echo "terminology-seed-drift: the two seed directories hold different files" >&2
  diff <(printf '%s\n' "$canonical_files") <(printf '%s\n' "$chart_files") >&2 || true
  status=1
fi

while IFS= read -r name; do
  [[ -f "${CHART_COPY}/${name}" ]] || continue
  if ! diff -u "${CANONICAL}/${name}" "${CHART_COPY}/${name}" >/dev/null; then
    echo "terminology-seed-drift: ${CHART_COPY}/${name} differs from ${CANONICAL}/${name}" >&2
    diff -u "${CANONICAL}/${name}" "${CHART_COPY}/${name}" | head -20 >&2
    status=1
  fi
done <<<"$canonical_files"

if [[ "$status" -ne 0 ]]; then
  echo "copy the canonical seed over the chart's: cp ${CANONICAL}/*.json ${CHART_COPY}/" >&2
  exit 1
fi
echo "terminology-seed-drift: OK."
