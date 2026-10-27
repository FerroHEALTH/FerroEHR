#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# One GitHub API read, retried through GitHub's transient errors (#4173).
#
# GitHub answers bursts of 5xx that last several minutes: pr-guards.yml failed
# three times in one day on a 502 from the jobs listing after a five-minute
# window. A read is therefore retried with a doubling backoff, capped at two
# minutes, for fifteen minutes. Only an answer that cannot pass by waiting
# fails at once: an HTTP 4xx other than 429. A read that gives up says it was
# the API, so nobody reads the failure as a guard's verdict. No openEHR spec
# governs this: our own design.
#
# Source it from a script or a workflow step as:
#
#   # shellcheck source=scripts/lib/gh-retry.sh
#   . scripts/lib/gh-retry.sh
#   jobs=$(gh_retry gh run view "$run" --json jobs) || exit 1
#
# or run it:
#
#   scripts/lib/gh-retry.sh --self-test
#
# GH_RETRY_WINDOW (seconds, default 900) and GH_RETRY_SLEEP (the command that
# waits, default sleep) exist for the self-test.

# gh_retry <command...>: run the command until it succeeds, print its stdout,
# and return 1 once it fails for good.
gh_retry() {
  local delay=5 waited=0 window="${GH_RETRY_WINDOW:-900}" out err errfile
  errfile="$(mktemp)"
  while :; do
    if out="$("$@" 2>"$errfile")"; then
      rm -f "$errfile"
      printf '%s\n' "$out"
      return 0
    fi
    err="$(tail -n 5 "$errfile" | tr '\n' ' ')"
    if grep -qE 'HTTP 4[0-9]{2}' <<<"$err" && ! grep -q 'HTTP 429' <<<"$err"; then
      rm -f "$errfile"
      echo "::error::the GitHub API refused '$*', and waiting cannot change that answer: ${err}" >&2
      return 1
    fi
    if ((waited >= window)); then
      rm -f "$errfile"
      echo "::error::the GitHub API kept failing for ${waited}s on '$*', so this job stopped before it judged anything. The API failed, not a guard: re-run this job. Last answer: ${err}" >&2
      return 1
    fi
    echo "the GitHub API failed on '$*' (${err}); retrying in ${delay}s" >&2
    "${GH_RETRY_SLEEP:-sleep}" "$delay"
    waited=$((waited + delay))
    delay=$((delay * 2 > 120 ? 120 : delay * 2))
  done
}

_gh_retry_self_test() {
  local tmp out status
  tmp="$(mktemp -d)"
  # fake_api N MESSAGE: fails with MESSAGE on stderr until it has been called
  # N times, then prints "ok". The count lives in a file: gh_retry runs it in
  # a command substitution.
  fake_api() {
    local calls
    calls=$(($(cat "$tmp/calls" 2>/dev/null || echo 0) + 1))
    echo "$calls" >"$tmp/calls"
    if ((calls > $1)); then
      echo ok
    else
      echo "$2" >&2
      return 1
    fi
  }
  # check NAME WANT_STATUS WANT_TEXT WANT_CALLS FAILURES MESSAGE
  check() {
    rm -f "$tmp/calls"
    status=0
    out="$(GH_RETRY_SLEEP=true GH_RETRY_WINDOW=60 gh_retry fake_api "$5" "$6" 2>&1)" || status=$?
    if [[ "$status" != "$2" ]] || ! grep -qF -- "$3" <<<"$out" || [[ "$(cat "$tmp/calls")" != "$4" ]]; then
      echo "self-test: $1: exit $status after $(cat "$tmp/calls") calls, wanted exit $2 after $4 calls and '$3' in:" >&2
      echo "$out" >&2
      rm -rf "$tmp"
      return 1
    fi
  }
  check "a 502 burst is ridden out" 0 "ok" 4 3 "gh: HTTP 502: Bad Gateway (https://api.github.com/repos/o/r/actions/runs/1/jobs?per_page=100)" || return 1
  check "the wait between tries is said" 0 "retrying in 5s" 2 1 "gh: HTTP 503: Service Unavailable" || return 1
  check "a rate limit is waited out" 0 "ok" 2 1 "gh: HTTP 429: Too Many Requests" || return 1
  check "a dropped connection is retried" 0 "ok" 2 1 "Post \"https://api.github.com/graphql\": read: connection reset by peer" || return 1
  # Backoff 5, 10, 20, 40: 75 s waited after five tries passes the 60 s window.
  check "a lasting 5xx gives up and blames the API" 1 "The API failed, not a guard" 5 99 "gh: HTTP 502: Bad Gateway" || return 1
  check "a 404 fails at once" 1 "waiting cannot change that answer" 1 99 "gh: Not Found (HTTP 404)" || return 1
  check "a 403 fails at once" 1 "waiting cannot change that answer" 1 99 "gh: Resource not accessible by integration (HTTP 403)" || return 1
  rm -rf "$tmp"
  echo "gh-retry: self-test OK (5xx, 429 and a dropped connection retried; a lasting 5xx blamed on the API; a 4xx refused at once)."
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  set -euo pipefail
  case "${1:-}" in
  --self-test) _gh_retry_self_test ;;
  *)
    echo "usage: $0 --self-test (source this file for gh_retry)" >&2
    exit 2
    ;;
  esac
fi
