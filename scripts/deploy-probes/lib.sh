#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Shared probe vocabulary for the deployment-conformance harness.
#
# The instrument's whole point is that a red row is ACTIONABLE, so every
# assertion here takes the LAYER it indicts and the evidence it saw. A probe
# that can only say "it did not work" costs more than it saves — the same
# discipline `.claude/rules/cnf-triage.md` imposes on a red conformance row.
#
# Sourced by the platform families (compose.sh, kubernetes.sh); never run
# directly.

# ── The ledger ────────────────────────────────────────────────────────────────
# Counters and the machine-readable rows, appended as the run proceeds. The
# NOT-EXERCISED ledger is deliberately a first-class output: silence read as
# coverage is how the defects this instrument exists for reached a release.
PROBE_PASS=0
PROBE_FAIL=0
PROBE_SKIP=0
PROBE_ROWS=()
PROBE_UNCOVERED=()

# The probe currently running, and what it indicts when it fails.
PROBE_ID=""
PROBE_TITLE=""
PROBE_LAYER=""
PROBE_STATE=""
PROBE_ISSUE=""
PROBE_FAILED=0

red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }
bold()  { printf '\033[1m%s\033[0m\n' "$*"; }
dim()   { printf '\033[2m%s\033[0m\n' "$*"; }

# probe <id> <state> <layer-when-it-fails> <issue-or--> <title>
#
# `state` is one of off | working | broken — the three states #2178 requires,
# because two of the defects that prompted this instrument lived in states
# nobody exercised.
probe() {
  PROBE_ID="$1"
  PROBE_STATE="$2"
  PROBE_LAYER="$3"
  PROBE_ISSUE="$4"
  PROBE_TITLE="$5"
  PROBE_FAILED=0
  printf '  %-22s %-8s %s\n' "$PROBE_ID" "[$PROBE_STATE]" "$PROBE_TITLE"
}

# Record the outcome of the probe that just ran.
probe_done() {
  local outcome="pass"
  if [[ "$PROBE_FAILED" -eq 1 ]]; then
    outcome="fail"
    PROBE_FAIL=$((PROBE_FAIL + 1))
  else
    PROBE_PASS=$((PROBE_PASS + 1))
  fi
  PROBE_ROWS+=("$(printf '{"id":"%s","state":"%s","outcome":"%s","layer":"%s","issue":"%s","title":"%s"}' \
    "$PROBE_ID" "$PROBE_STATE" "$outcome" \
    "$([[ "$outcome" = fail ]] && printf '%s' "$PROBE_LAYER" || printf '')" \
    "$PROBE_ISSUE" "$(json_string "$PROBE_TITLE")")")
}

# The failure path: name the layer, quote what was actually seen.
#
# `expected`/`actual` are printed verbatim rather than summarized, because the
# summary is what makes a red row unactionable.
probe_fail() {
  local expected="$1" actual="$2" note="${3:-}"
  PROBE_FAILED=1
  red   "    FAIL  layer=$PROBE_LAYER${PROBE_ISSUE:+  regression-of=$PROBE_ISSUE}"
  echo  "      expected: $expected"
  echo  "      actual:   $actual"
  [[ -n "$note" ]] && echo "      note:     $note"
  return 0
}

# assert_eq <expected> <actual> [note]
assert_eq() {
  [[ "$1" = "$2" ]] && return 0
  probe_fail "$1" "$2" "${3:-}"
}

# assert_contains <haystack> <needle> [note]
assert_contains() {
  case "$1" in
    *"$2"*) return 0 ;;
    *) ;;
  esac
  probe_fail "output containing '$2'" "${1:0:300}" "${3:-}"
}

# assert_not_contains <haystack> <needle> [note]
assert_not_contains() {
  case "$1" in
    *"$2"*) probe_fail "output WITHOUT '$2'" "${1:0:300}" "${3:-}" ;;
    *) return 0 ;;
  esac
}

# One JSON string value: the control characters and quoting a record must
# escape to stay parseable.
#
# Every reason in this harness is written as a wrapped shell string, so it
# carries literal newlines — interpolated raw, they made the emitted record
# invalid JSON, which is what a record is FOR (found 2026-09-10 while adding
# the backup family: `jq` refused both the artifact and the committed
# docs/conformance/deployment/compose.json). Newlines and the runs of
# indentation after them collapse to one space; quotes, backslashes and tabs
# are escaped (RFC 8259 §7).
json_string() {
  printf '%s' "$1" | tr -d '\r' | awk '
    { gsub(/\\/, "\\\\"); gsub(/"/, "\\\""); gsub(/\t/, " ");
      sub(/^[[:space:]]+/, ""); printf "%s%s", (NR > 1 ? " " : ""), $0 }
  '
}

# uncovered <what> <why> — the honest half of the report.
uncovered() {
  PROBE_UNCOVERED+=("$(printf '{"what":"%s","why":"%s"}' \
    "$(json_string "$1")" "$(json_string "$2")")")
  PROBE_SKIP=$((PROBE_SKIP + 1))
}

# Poll an HTTP endpoint until it answers, or fail the RUN (not a probe) — a
# stack that never came up is a harness problem, not a finding about the SUT.
wait_http() {
  local url="$1" tries="${2:-90}"
  for _ in $(seq 1 "$tries"); do
    curl -sf -o /dev/null "$url" && return 0
    sleep 2
  done
  red "FATAL: $url never answered after $((tries * 2))s — the stack did not come up"
  return 1
}

# Wait until an HTTP endpoint reports a specific status, for the states where
# the expected answer is a REFUSAL (readiness going 503 when its database dies).
wait_status() {
  local url="$1" want="$2" tries="${3:-30}"
  for _ in $(seq 1 "$tries"); do
    [[ "$(curl -s -o /dev/null -w '%{http_code}' "$url")" = "$want" ]] && return 0
    sleep 2
  done
  return 1
}

http_code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }

# Emit the report: the human summary, the uncovered ledger, and the JSON record.
probe_report() {
  local out="$1" platform="$2"
  echo
  bold "── not exercised by this run ───────────────────────────────"
  if [[ "${#PROBE_UNCOVERED[@]}" -eq 0 ]]; then
    echo "  (nothing declared — which is itself suspicious; see lib.sh 'uncovered')"
  else
    local row
    for row in "${PROBE_UNCOVERED[@]}"; do
      printf '  %s\n' "$(printf '%s' "$row" | sed -E 's/.*"what":"([^"]*)","why":"([^"]*)".*/\1 — \2/')"
    done
  fi

  mkdir -p "$(dirname "$out")"
  {
    printf '{"platform":"%s","passed":%d,"failed":%d,"uncovered":%d,"probes":[' \
      "$platform" "$PROBE_PASS" "$PROBE_FAIL" "$PROBE_SKIP"
    local i
    for i in "${!PROBE_ROWS[@]}"; do
      [[ "$i" -gt 0 ]] && printf ','
      printf '%s' "${PROBE_ROWS[$i]}"
    done
    printf '],"not_exercised":['
    for i in "${!PROBE_UNCOVERED[@]}"; do
      [[ "$i" -gt 0 ]] && printf ','
      printf '%s' "${PROBE_UNCOVERED[$i]}"
    done
    printf ']}\n'
  } > "$out"

  echo
  bold "── result ──────────────────────────────────────────────────"
  echo "  passed $PROBE_PASS   failed $PROBE_FAIL   not exercised $PROBE_SKIP"
  echo "  record: $out"
  [[ "$PROBE_FAIL" -eq 0 ]] || { red "DEPLOYMENT PROBES FAILED"; return 1; }
  green "ALL DEPLOYMENT PROBES PASSED (read the 'not exercised' list above)"
}
