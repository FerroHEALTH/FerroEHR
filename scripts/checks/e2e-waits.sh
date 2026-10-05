#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# E2E journey wait discipline (.claude/rules/leptos-ui.md §10).
#
# A browser journey drives a remote process over HTTP, and two shapes make a
# journey lie about what it observed:
#
#   1. A RAW `WebElement` (or a `SelectElement` built from one). Its commands
#      carry thirtyfour's own 120 s per-request default, which outlasts every
#      poll loop in the suite — so a driver that stops answering reads as a
#      button that was inert, and the journey skips the save and goes green on
#      the prior state. `tests/it/common/mod.rs` hands out `common::Element`
#      instead: it bounds every command and reads a failure to answer as a
#      failure.
#   2. A `sleep`. A timed wait is either too short on a loaded machine (a
#      flake) or wasted time on a quiet one, and it asserts nothing. Journeys
#      wait on conditions — the harness's `wait_*` and `poll_*` helpers.
#
# Scope is the journey files, `app/ferroehr-viewer/tests/it/e2e_*.rs`, and
# NOT `tests/it/common/mod.rs`: the harness is where the raw handle is wrapped
# and where the poll loops that do the sleeping live, so banning the vocabulary
# there would ban its own implementation. The harness's one deliberate timed
# wait is `common::dwell`, which exists because a journey asserting that a
# window passes WITHOUT something happening has no observable to poll for.
#
# Usage: scripts/checks/e2e-waits.sh [--self-test]
#   no args     → every journey file
#   --self-test → prove the detector in both directions
set -euo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"
cd "$(dirname "$0")/../.."

readonly JOURNEY_GLOB='app/ferroehr-viewer/tests/it/e2e_*.rs'
readonly RULE='.claude/rules/leptos-ui.md §10'

# The three banned spellings, as one extended regex. `SelectElement` is matched
# bare as well as fully qualified: an import would make the short form legal
# Rust, and the short form is what a call site reads as.
readonly BANNED='WebElement|SelectElement|tokio::time::sleep|std::thread::sleep|thread::sleep'

# Report every banned mention under `dir`, one `file:line: text` per finding.
# `-H` is explicit so a finding names its file even when the glob expands to a
# single journey: grep prints the name only when it was handed several.
findings() {
  local dir="$1" glob
  glob="$dir/${JOURNEY_GLOB##*/}"
  # shellcheck disable=SC2086 # the glob must expand into the file list
  grep -HnE "$BANNED" $glob 2>/dev/null || true
}

# Proves the detector rather than trusting it: a scratch tree with one clean
# journey and one journey per banned spelling, so a guard whose pattern
# silently matched nothing would fail here.
self_test() {
  local tmp
  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand $tmp now, while it is still in scope
  trap "rm -rf '$tmp'" RETURN
  mkdir -p "$tmp"
  cat >"$tmp/e2e_clean.rs" <<'RS'
async fn journey(h: &Harness) {
    h.wait_css("#save").await.click().await;
    poll_until(async || is_present(h, "#done").await).await;
}
RS
  printf 'async fn f(e: &WebElement) {}\n' >"$tmp/e2e_raw_handle.rs"
  printf 'async fn f() { SelectElement::new(&e).await; }\n' >"$tmp/e2e_select.rs"
  printf 'async fn f() { tokio::time::sleep(d).await; }\n' >"$tmp/e2e_tokio_sleep.rs"
  printf 'async fn f() { std::thread::sleep(d); }\n' >"$tmp/e2e_thread_sleep.rs"

  local found failures=0
  found="$(findings "$tmp")"
  local expected
  for expected in e2e_raw_handle.rs e2e_select.rs e2e_tokio_sleep.rs e2e_thread_sleep.rs; do
    grep -q "$expected" <<<"$found" || {
      echo "self-test: $expected was not caught" >&2
      failures=1
    }
  done
  if grep -q 'e2e_clean.rs' <<<"$found"; then
    echo "self-test: the clean journey was wrongly caught" >&2
    failures=1
  fi
  [[ "$failures" -eq 0 ]] || return 1
  echo "e2e-waits: self-test OK (all four banned spellings caught, the clean journey allowed)."
}

report() {
  local found="$1"
  if [[ -z "$found" ]]; then
    echo "e2e-waits: OK (no raw WebDriver handle and no sleep in the journeys)."
    return 0
  fi
  echo "error: a journey file drives the browser outside the bounded harness" >&2
  echo >&2
  printf '%s\n' "$found" >&2
  echo >&2
  echo "A journey holds a \`common::Element\`, never a raw \`WebElement\` or a" >&2
  echo "\`SelectElement\` built from one: those carry thirtyfour's 120 s request" >&2
  echo "default, so a stalled driver reads as an inert control. And a journey" >&2
  echo "waits on a CONDITION — \`common::wait_*\` / \`common::poll_*\` — never on" >&2
  echo "a timer." >&2
  echo >&2
  echo "The rule is $RULE." >&2
  return 1
}

case "${1:-}" in
--self-test)
  self_test
  ;;
"")
  report "$(findings 'app/ferroehr-viewer/tests/it')"
  ;;
*)
  guard_usage "[--self-test]"
  ;;
esac
