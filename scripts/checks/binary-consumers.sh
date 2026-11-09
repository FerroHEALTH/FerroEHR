#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Every lane that starts a consumer of the server binary also builds it.
#
# `ui-e2e-binary` builds the server binary once per change, and the jobs that
# need it (chart-boot, perf, deploy-probe, the viewer lanes, quickstart-config,
# conformance) are skipped with it when it is skipped. A consumer whose own
# condition names a lane the builder's condition does not name then reports
# `skipping` on exactly the change it exists for: a deploy-harness change set
# the `deploy` lane alone and ran no deploy probe (#5271). This guard reads
# ci.yml and fails when a lane in a consumer's `if:` is missing from the
# builder's `if:`.
#
# Usage: scripts/checks/binary-consumers.sh [--self-test] [<workflow>]
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
guard_known_flags "[--self-test] [<workflow>]" "--self-test" "$@"

readonly BUILDER=ui-e2e-binary

# lanes <condition>: the change lanes a condition reads, one per line, without
# queue_verified and cache_refresh, which gate every lane alike.
lanes() {
  grep -oE 'needs\.changes\.outputs\.[a-z_]+' <<<"$1" | sed 's/^needs\.changes\.outputs\.//' \
    | grep -vxE 'queue_verified|cache_refresh' | sort -u || true
}

check() {
  local workflow="$1" built consumers job missing failures=0
  built=$(lanes "$(yq -r ".jobs.\"$BUILDER\".if // \"\"" "$workflow")")
  consumers=$(yq -o=json '.jobs' "$workflow" | jq -r --arg b "$BUILDER" \
    'to_entries[] | select((.value.needs // []) | (if type == "string" then [.] else . end) | index($b)) | .key')
  [[ -n "$consumers" ]] || { echo "error: no job of $workflow needs $BUILDER" >&2; return 1; }
  while IFS= read -r job; do
    missing=$(comm -23 <(lanes "$(yq -r ".jobs.\"$job\".if // \"\"" "$workflow")") <(echo "$built"))
    if [[ -n "$missing" ]]; then
      echo "error: $workflow: '$job' runs on lane(s) $(paste -sd, - <<<"$missing") that '$BUILDER' does not build for, so it is skipped there" >&2
      failures=1
    fi
  done <<<"$consumers"
  [[ "$failures" -eq 0 ]] || return 1
  echo "binary-consumers: every lane of the $(wc -l <<<"$consumers" | tr -d ' ') jobs that need $BUILDER builds it."
}

self_test() {
  local dir
  dir=$(mktemp -d)
  trap 'rm -rf "$dir"' RETURN
  cat >"$dir/ok.yml" <<'YAML'
jobs:
  ui-e2e-binary:
    if: >-
      needs.changes.outputs.viewer == 'true' ||
      ((needs.changes.outputs.code == 'true' || needs.changes.outputs.deploy == 'true') && needs.changes.outputs.queue_verified != 'true')
  deploy-probe:
    needs: [changes, ui-e2e-binary]
    if: (needs.changes.outputs.code == 'true' || needs.changes.outputs.deploy == 'true') && needs.changes.outputs.queue_verified != 'true'
  ui-e2e:
    needs: [changes, ui-e2e-binary]
    if: needs.changes.outputs.viewer == 'true'
YAML
  # The bad tree: the builder no longer names deploy, the consumer still does.
  cp "$dir/ok.yml" "$dir/bad.yml"
  yq -i '.jobs."ui-e2e-binary".if = "needs.changes.outputs.viewer == '"'true'"' || needs.changes.outputs.code == '"'true'"'"' "$dir/bad.yml"
  check "$dir/ok.yml" >/dev/null || { echo "self-test: a builder naming every consumer lane was refused" >&2; return 1; }
  if check "$dir/bad.yml" 2>/dev/null; then
    echo "self-test: a deploy-probe whose deploy lane the builder lacks was accepted" >&2
    return 1
  fi
  echo "binary-consumers: self-test OK (a missing deploy lane is refused, a complete builder passes)."
}

if [[ "${1-}" == --self-test ]]; then
  self_test
else
  check "${1:-.github/workflows/ci.yml}"
fi
