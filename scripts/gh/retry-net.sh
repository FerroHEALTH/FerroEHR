#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# scripts/gh/retry-net.sh — the two network idioms a publishing lane may use.
#
# WHY THIS EXISTS (#2775): a publishing lane runs under `set -euo pipefail`, and
# a bare network call in a command substitution therefore ENDS the lane on any
# transient failure. At v4.0.4 one TLS handshake timeout inside the chart lane's
# wait loop failed a release with nothing wrong with the chart. The lesson is not
# "retry more" — it is that the two kinds of network call have two different
# correct shapes:
#
#   1. An IDEMPOTENT READ retries inside curl and fails loud when it truly
#      cannot be served: `retry_read`.
#   2. A POLL — "has the thing appeared yet?" — treats ANY failure as "not
#      observed yet" and lets the deadline decide: `poll_until`. A poll that
#      dies on a transient error reports an absence that was never measured.
#
# A STATE-CHANGING call (a push, a signature) is neither: it retries a bounded
# number of times with a growing pause and fails loud, because "did it happen?"
# is not answerable by asking again. `retry_state` is that shape (the chart
# lane's 3-attempt cosign loop, which exists because chart 6.0.14 was lost to a
# single Fulcio connection reset).
#
# This is a FUNCTION LIBRARY, not a wrapper action: a composite action cannot
# wrap an arbitrary step of its caller. Source it inside the `run:` block:
#
#   source scripts/gh/retry-net.sh
#   body="$(retry_read "https://crates.io/api/v1/crates/openehr-base/versions")"
#
# curl flags, per the official documentation
# (https://everything.curl.dev/usingcurl/downloads/retry.html):
#   --retry N              retry transient failures (timeouts, 408/429/5xx)
#   --retry-connrefused    also retry a refused connection — a service still
#                          starting, which is the common CI case
#   --retry-all-errors     retry EVERYTHING, including a non-transient 4xx; only
#                          for a request that is safe to re-send and where a
#                          wrong answer is worse than a slow one. Not used by
#                          the helpers here.
#   --fail                 a 4xx/5xx body is not output as if it were data (the
#                          defect that let an error body reach jq and end a step)
set -uo pipefail

# retry_read <url> [curl-arg...] — GET an idempotent resource, retried inside
# curl, printing the body on stdout. Returns curl's exit status, so the caller
# decides whether a failure is fatal.
retry_read() {
  local url="$1"
  shift || true
  curl --proto '=https' --tlsv1.2 -sSL --fail \
    --retry 5 --retry-connrefused --retry-delay 2 --max-time 60 \
    "$@" "$url"
}

# poll_until <deadline-epoch> <interval-seconds> <command...> — run the command
# until it succeeds or the deadline passes. Transient failure is "not observed
# yet", never lane death: the command runs guarded in an `if`, so nothing it
# does can trip `set -e` in the caller.
#
# Returns 0 as soon as the command succeeds, 1 when the deadline passes. The
# caller writes the ::error:: message, because only it knows what the absence
# means and what a human should do about it.
poll_until() {
  local deadline="$1" interval="$2"
  shift 2
  while :; do
    if "$@"; then
      return 0
    fi
    if [[ "$(date +%s)" -ge "$deadline" ]]; then
      return 1
    fi
    sleep "$interval"
  done
}

# deadline_in <seconds> — an absolute epoch deadline for poll_until.
deadline_in() {
  echo $(( $(date +%s) + $1 ))
}

# retry_state <attempts> <what> <command...> — a STATE-CHANGING call, retried a
# bounded number of times with a growing pause. Returns 0 on the first success,
# 1 when every attempt failed; the caller writes the ::error:: message.
retry_state() {
  local attempts="$1" what="$2"
  shift 2
  local attempt
  for (( attempt = 1; attempt <= attempts; attempt++ )); do
    if "$@"; then
      return 0
    fi
    if [[ "$attempt" -lt "$attempts" ]]; then
      echo "${what} attempt ${attempt} failed; retrying in $(( attempt * 10 ))s"
      sleep $(( attempt * 10 ))
    fi
  done
  return 1
}
