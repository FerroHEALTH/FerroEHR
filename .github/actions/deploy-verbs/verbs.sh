#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# Reads the verbs the sandbox's deploy script offers from its answer to the
# `verbs` verb, and refuses a verb the calling workflow needs that the box's
# script lacks (#4372). deploy.sh does not update itself, so the copy on the
# box can be an older release's than the workflow on main that calls it. A
# deploy.sh from before the `verbs` verb answers it as an unknown verb with
# the list of the verbs it runs, which is read the same way. No openEHR spec
# governs this: our own design.
#
#   verbs.sh check <verb> < answer    exit 0 when the answer offers <verb>
#   verbs.sh --self-test               the answers of both script generations
set -euo pipefail

# The verbs an answer on stdin offers, one line, space separated.
offered() {
  local answer line
  answer="$(cat)"
  line="$(grep -m 1 -E '^verbs ' <<<"$answer" || true)"
  if [[ -n "$line" ]]; then
    printf '%s\n' "${line#verbs }"
    return 0
  fi
  line="$(grep -m 1 -F 'this key runs' <<<"$answer" || true)"
  if [[ -n "$line" ]]; then
    grep -oE "'[a-z]+'" <<<"${line#*this key runs}" | tr -d "'" | paste -sd' ' -
    return 0
  fi
  return 1
}

check() {
  local verb="$1" list
  if ! list="$(offered)"; then
    echo "::error::the sandbox's deploy.sh answered the verbs verb with neither a verb list nor an unknown-verb refusal; read its output on the box." >&2
    return 1
  fi
  if [[ " $list " != *" $verb "* ]]; then
    echo "::error::the sandbox's deploy.sh offers '${list}', and this workflow calls '${verb}'. deploy.sh does not update itself: install the deploy.sh of the release the box serves (deploy/hosted/sandbox-ferropix-rollout.sh --step 3, run from that release's tag)." >&2
    return 1
  fi
  echo "the sandbox's deploy.sh offers ${verb} (${list})"
}

self_test() {
  local failures=0 answer verb want got
  while IFS='|' read -r answer verb want; do
    got=refuse
    printf '%b' "$answer" | check "$verb" > /dev/null 2>&1 && got=run
    if [[ "$got" != "$want" ]]; then
      echo "self-test: '$answer' for $verb: got $got, want $want" >&2
      failures=1
    fi
  done <<'CASES'
verbs deploy restart wipe identities verbs\n|identities|run
verbs deploy restart wipe identities verbs\n|reseed|refuse
deploy: unknown verb 'verbs' (this key runs 'deploy', 'restart', 'wipe' or 'identities')\n|wipe|run
deploy: unknown verb 'verbs' (this key runs 'deploy', 'restart' or 'wipe')\n|identities|refuse
Permission denied (publickey).\n|deploy|refuse
CASES
  [[ "$failures" -eq 0 ]] || exit 1
  echo "deploy-verbs: self-test OK (both script generations are read, a missing verb and a non-answer are refused)."
}

case "${1:-}" in
  check)
    [[ $# -eq 2 ]] || { echo "usage: verbs.sh check <verb> < answer | --self-test" >&2; exit 2; }
    check "$2"
    ;;
  --self-test) self_test ;;
  *) echo "usage: verbs.sh check <verb> < answer | --self-test" >&2; exit 2 ;;
esac
