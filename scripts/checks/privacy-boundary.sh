#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Privacy-boundary guard: a change that moves the pseudonymisation boundary is
# reviewed as such, and no real personal data enters the tree.
#
# Two gates, both over the diff:
#
#   1. A diff touching a privacy-boundary path needs the pull request template's
#      'Privacy boundary' checklist present and fully ticked. The checklist is
#      the record that the data flow, the roles, the grants and the access
#      events were thought about; this gate is what makes it binding.
#   2. The ADDED lines of the diff carry no value that looks like a real Dutch
#      identifier: a nine-digit number passing the BSN eleven-test (elfproef),
#      or a postcode followed by a house number. Tests and fixtures use
#      synthetic data (CONTRIBUTING.md § Personal data).
#
# GDPR Art. 25 makes the boundary a design-time duty
# (https://eur-lex.europa.eu/eli/reg/2016/679/oj); no openEHR spec governs any
# of this - our own design.
#
# THE PATH LIST IS A SET OF PATTERNS MATCHED AGAINST THE DIFF, NEVER AN
# ASSERTION THAT THE PATHS EXIST. The demographic and linkage schemas, the
# linkage service, the identifier scanner and the access-event model are being
# built (issues #3153, #3158, #3154, #3156). The guard has to pass cleanly on a
# tree where those paths are absent and fire the moment the first of them
# appears, so it matches names rather than checking a directory listing.
#
# The eleven-test is arithmetic, not a lookup: a nine-digit number d1..d9 is a
# candidate BSN when 9*d1 + 8*d2 + 7*d3 + 6*d4 + 5*d5 + 4*d6 + 3*d7 + 2*d8 - d9
# is divisible by 11. Roughly one nine-digit number in eleven passes by chance,
# so the run has to be delimited by non-alphanumeric characters - without that,
# every hex digest and base64 blob in the tree produces hits at that rate.
#
# One structural carve-out, measured rather than guessed: a survey over every
# tracked file produced 22 hits and all 22 were SNOMED CT identifiers in AQL
# tests and OPT fixtures. An SCTID ends in a two-digit partition identifier plus
# a check digit, and every concept, description and relationship partition is
# `00`, `01`, `02`, `10`, `11` or `12`
# (https://confluence.ihtsdotools.org/display/DOCRELFMT), so a nine-digit run
# with that shape ON A LINE ALSO MENTIONING snomed, sct or code is read as a
# terminology code. Both halves are required: the shape alone would blind the
# guard to 6% of the BSN space, and the words alone would blind it to any BSN
# beside the word "code".
#
# A deliberately synthetic value that still passes carries `privacy-allow:` and
# a reason on the same line, the shape ShellCheck directives use here. The
# marker exempts that line and nothing else.
#
# Usage:
#   scripts/checks/privacy-boundary.sh                 # diff against origin/main
#   scripts/checks/privacy-boundary.sh --diff <base> [head]
#   CHANGED_FILES="a b c" scripts/checks/privacy-boundary.sh
#   scripts/checks/privacy-boundary.sh --self-test     # prove the detectors
#
# PR_BODY carries the pull request description. Unset means the checklist gate
# cannot be judged (a local run) and is reported as skipped; set-but-empty is a
# pull request with no body and fails.
set -euo pipefail
cd "$(dirname "$0")/../.."

# The privacy boundary, as path patterns (extended regular expressions, matched
# against the repository-relative path).
readonly BOUNDARY_PATTERNS=(
  '(^|/)migrations/(demographic|linkage)/'
  '^app/ferroehr/src/service/demographic/'
  '^app/ferroehr/src/service/linkage/'
  '^app/ferroehr[^/]*/src/.*identifier_scan'
  '^app/ferroehr[^/]*/src/.*outbox'
)

# Paths the data scan skips, each for a reason: vendored material is upstream's
# and never edited here (the openEHR specs, the clinical-model corpora, the
# legal texts under docs/law/, whose publishers' structural keys are nine-digit
# runs that pass the eleven-test by chance), and the machine-written artifacts
# (the conformance records and the benchmark records) carry long digit runs
# with no personal data in them by construction.
readonly SKIP_PATTERN='^(docs/specs/|docs/law/|corpus/|docs/conformance/|docs/benchmarks/)|(^|/)vendor/|(^|/)Cargo\.lock$|\.svg$'

readonly POSTCODE_RE='(^|[^0-9A-Za-z])[1-9][0-9]{3}[ ]?[A-Z]{2}[^0-9A-Za-z]{1,3}[0-9]{1,4}([^0-9A-Za-z]|$)'

# ── detectors ────────────────────────────────────────────────────────────────
# Both read `path:line:text` records on stdin and print the offending records.

scan_bsn() {
  # LC_ALL=C: the records are arbitrary file bytes, and a multibyte-aware awk
  # aborts mid-stream on invalid UTF-8, which would silently truncate the scan.
  LC_ALL=C awk '
    function elfproef(n,   i, d, sum) {
      sum = 0
      for (i = 1; i <= 9; i++) {
        d = substr(n, i, 1) + 0
        sum += d * (i < 9 ? 10 - i : -1)
      }
      return (sum % 11 == 0)
    }
    function sctid(n) {
      return substr(n, 7, 2) ~ /^[01][0-2]$/
    }
    index($0, "privacy-allow:") > 0 { next }
    {
      terminology = (tolower($0) ~ /snomed|sct|code/)
      rest = $0
      while (match(rest, /[0-9]+/)) {
        run = substr(rest, RSTART, RLENGTH)
        before = (RSTART > 1) ? substr(rest, RSTART - 1, 1) : ""
        after = substr(rest, RSTART + RLENGTH, 1)
        delimited = (before !~ /[0-9A-Za-z_]/) && (after !~ /[0-9A-Za-z_]/)
        if (terminology && sctid(run)) {
          rest = substr(rest, RSTART + RLENGTH)
          continue
        }
        if (delimited && length(run) == 9 && run !~ /^0+$/ && elfproef(run)) {
          print
          next
        }
        rest = substr(rest, RSTART + RLENGTH)
      }
    }
  '
}

scan_postcode() {
  LC_ALL=C grep -avF 'privacy-allow:' | LC_ALL=C grep -aE "$POSTCODE_RE" || true
}

# ── self-test ────────────────────────────────────────────────────────────────
# The detectors are the whole value of this guard, so they are exercised on
# every run rather than trusted. Each case names what it proves.

self_test_failures=0

expect_detector() { # <detector> <expected hit|miss> <label> <text>
  local detector="$1" expected="$2" label="$3" text="$4" hits
  hits=$(printf 'fixture.txt:1:%s\n' "$text" | "$detector" | grep -c . || true)
  local actual="miss"
  [[ "$hits" -gt 0 ]] && actual="hit"
  if [[ "$actual" != "$expected" ]]; then
    printf 'self-test FAIL: %s (%s expected %s, got %s)\n' \
      "$label" "$detector" "$expected" "$actual" >&2
    self_test_failures=$((self_test_failures + 1))
  fi
}

expect_checklist() { # <expected state> <label> <body>
  local expected="$1" label="$2" body="$3" actual
  actual=$(printf '%s\n' "$body" | checklist_state)
  if [[ "$actual" != "$expected" ]]; then
    printf 'self-test FAIL: %s (expected %s, got %s)\n' "$label" "$expected" "$actual" >&2
    self_test_failures=$((self_test_failures + 1))
  fi
}

expect_path() { # <expected hit|miss> <path>
  local expected="$1" path="$2" hits actual
  hits=$(printf '%s\n' "$path" | boundary_hits | grep -c . || true)
  actual="miss"
  [[ "$hits" -gt 0 ]] && actual="hit"
  if [[ "$actual" != "$expected" ]]; then
    printf 'self-test FAIL: path %s (expected %s, got %s)\n' "$path" "$expected" "$actual" >&2
    self_test_failures=$((self_test_failures + 1))
  fi
}

self_test() {
  # The eleven-test itself. The number below sums to 66 and passes; the same
  # number with its last digit changed sums to 65 and fails, which is the
  # mutation that proves the arithmetic runs rather than a length check.
  expect_detector scan_bsn hit 'a nine-digit number passing the eleven-test' \
    'bsn 111222333 in a fixture' # privacy-allow: self-test fixture, not a real number
  expect_detector scan_bsn miss 'the same number with one digit changed' \
    'bsn 111222334 in a fixture' # privacy-allow: self-test fixture, not a real number
  expect_detector scan_bsn miss 'a nine-digit number failing the eleven-test' \
    'sequence 123456789' # privacy-allow: self-test fixture, not a real number
  expect_detector scan_bsn miss 'a passing run inside a longer digit run' \
    'ordinal 1112223334' # privacy-allow: self-test fixture, not a real number
  expect_detector scan_bsn miss 'a passing run inside a hex digest' \
    'sha a111222333b' # privacy-allow: self-test fixture, not a real number
  expect_detector scan_bsn miss 'all zeros, which the eleven-test accepts and no register issues' \
    'padding 000000000'
  expect_detector scan_bsn miss 'a line carrying the marker' \
    'bsn 111222333 # privacy-allow: documented synthetic value'

  # The SNOMED carve-out, and both halves of what makes it narrow. 288526004
  # passes the eleven-test and is a real concept id from the OPT fixtures.
  expect_detector scan_bsn miss 'a SNOMED concept id in a terminology context' \
    '<code_string>288526004</code_string>' # privacy-allow: self-test fixture, a terminology code
  expect_detector scan_bsn hit 'the same digits with no terminology word on the line' \
    'value 288526004' # privacy-allow: self-test fixture, not a real number
  expect_detector scan_bsn hit 'a passing number that is not SCTID-shaped, beside the word code' \
    'code 111222333' # privacy-allow: self-test fixture, not a real number

  # The postcode plus house number. The pair is what identifies a household; a
  # postcode alone does not, and neither half fires on its own.
  expect_detector scan_postcode hit 'a postcode followed by a house number' \
    'address 1012 AB 5' # privacy-allow: self-test fixture, not a real address
  expect_detector scan_postcode hit 'the same pair written without the space' \
    'address 1012AB 5' # privacy-allow: self-test fixture, not a real address
  expect_detector scan_postcode miss 'a postcode with no house number' \
    'area 1012 AB' # privacy-allow: self-test fixture, not a real address
  expect_detector scan_postcode miss 'a postcode-shaped value with a leading zero' \
    'code 0123 AB 4' # privacy-allow: self-test fixture, not a real address
  expect_detector scan_postcode miss 'lowercase letters, which no postcode uses' \
    'code 1012 ab 4' # privacy-allow: self-test fixture, not a real address
  expect_detector scan_postcode miss 'four digits and two letters inside a word' \
    'const X1012ABC9 = 1' # privacy-allow: self-test fixture, not an address

  # The checklist gate.
  expect_checklist ticked 'a fully ticked checklist' \
    '## Privacy boundary
- [x] Data flow described
- [x] Roles named'
  expect_checklist unticked 'one box left empty' \
    '## Privacy boundary
- [x] Data flow described
- [ ] Roles named'
  expect_checklist absent 'the section removed' \
    '## Checks
- [x] Something else'
  expect_checklist absent 'the heading kept and its boxes deleted' \
    '## Privacy boundary

## Checks
- [x] Something else'

  # The path patterns, including the paths that do not exist yet.
  expect_path hit 'app/ferroehr/migrations/demographic/0001_baseline.sql'
  expect_path hit 'app/ferroehr/migrations/linkage/0001_baseline.sql'
  expect_path hit 'app/ferroehr/src/service/linkage/mod.rs'
  expect_path hit 'app/ferroehr/src/service/demographic/party.rs'
  expect_path hit 'app/ferroehr/src/scan/identifier_scanner.rs'
  expect_path hit 'app/ferroehr-ext/src/events/outbox_payload.rs'
  expect_path miss 'app/ferroehr/src/service/ehr/composition.rs'
  expect_path miss 'app/ferroehr/migrations/ehr/0002_event_outbox.sql'
  expect_path miss 'website/book/src/contributing.md'

  if [[ "$self_test_failures" -gt 0 ]]; then
    printf '\nprivacy-boundary: %d self-test case(s) failed.\n' "$self_test_failures" >&2
    return 1
  fi
  echo "privacy-boundary: self-test passed (detectors fire on exactly the expected cases)."
}

# ── the checklist ────────────────────────────────────────────────────────────
# Reads the pull request body on stdin, prints one of absent|unticked|ticked.
# Keyed on the section rather than on fixed line text: a wording change to one
# item must not silently disarm the gate.
checklist_state() {
  tr -d '\r' | awk '
    /^[[:space:]]*#+[[:space:]]/ {
      in_section = (tolower($0) ~ /privacy boundary/) ? 1 : 0
      next
    }
    in_section && /^[[:space:]]*[-*] \[[xX]\]/ { ticked++ }
    in_section && /^[[:space:]]*[-*] \[[[:space:]]\]/ { unticked++ }
    END {
      if (ticked + unticked == 0) { print "absent" }
      else if (unticked > 0) { print "unticked" }
      else { print "ticked" }
    }
  '
}

# ── the diff ─────────────────────────────────────────────────────────────────
boundary_hits() { # reads paths on stdin
  local pattern
  pattern=$(printf '%s|' "${BOUNDARY_PATTERNS[@]}")
  grep -E "${pattern%|}" || true
}

changed_files() {
  if [[ -n "$base" ]]; then
    git diff --name-only "$base...$tip" 2>/dev/null || git diff --name-only "$base" "$tip"
  elif [[ -n "${CHANGED_FILES:-}" ]]; then
    printf '%s\n' "$CHANGED_FILES" | tr ' ' '\n'
  else
    {
      git diff --name-only origin/main...HEAD 2>/dev/null || true
      git diff --name-only 2>/dev/null || true
    }
  fi | sed '/^$/d' | sort -u
}

# Prints `path:line:text` for every added line the data scan covers. With a base
# the added lines come from the diff; with only a file list there is no diff to
# read, so the file's whole content is treated as added (documented in the usage
# banner above).
added_records() { # <file>...
  local f
  for f in "$@"; do
    [[ -f "$f" ]] || continue
    printf '%s\n' "$f" | grep -qE "$SKIP_PATTERN" && continue
    if [[ -n "$base" ]]; then
      git diff --unified=0 "$base...$tip" -- "$f" 2>/dev/null |
        LC_ALL=C awk -v path="$f" '
          /^\+\+\+/ { next }
          /^@@/ {
            split($3, a, ",")
            ln = a[1]
            sub(/^\+/, "", ln)
            ln = ln + 0
            next
          }
          /^\+/ { print path ":" ln ":" substr($0, 2); ln++ }
        '
    else
      # -H prefixes the path, -I drops binary files, and the byte locale keeps a
      # non-UTF-8 file from aborting the scan halfway through.
      LC_ALL=C grep -HnI '' "$f" || true
    fi
  done
}

# ── run ──────────────────────────────────────────────────────────────────────
base=""
tip="HEAD"
files=()

case "${1:-}" in
--self-test)
  self_test
  exit 0
  ;;
--diff)
  base="${2:?usage: --diff <base> [head]}"
  tip="${3:-HEAD}"
  ;;
"") ;;
*)
  echo "usage: $0 [--self-test | --diff <base> [head]]" >&2
  exit 2
  ;;
esac

# The detectors run on every invocation, before anything else can report OK.
self_test >/dev/null

while IFS= read -r f; do files+=("$f"); done < <(changed_files)

if [[ "${#files[@]}" -eq 0 ]]; then
  echo "privacy-boundary: no changed files to check."
  exit 0
fi

status=0

touched=$(printf '%s\n' "${files[@]}" | boundary_hits)
if [[ -n "$touched" ]]; then
  if [[ -z "${PR_BODY+set}" ]]; then
    echo "privacy-boundary: privacy-boundary paths touched; PR_BODY is unset, so the"
    echo "                  checklist gate is skipped (it runs in CI on the pull request)."
  else
    state=$(printf '%s\n' "${PR_BODY}" | checklist_state)
    if [[ "$state" != "ticked" ]]; then
      echo "error: this change touches the privacy boundary:" >&2
      printf '%s\n' "$touched" | sed 's/^/  - /' >&2
      echo >&2
      if [[ "$state" = "unticked" ]]; then
        echo "The 'Privacy boundary' checklist in the pull request body has an" >&2
        echo "unticked box. Every item is a claim about this change; tick each one" >&2
        echo "only once it holds." >&2
      else
        echo "The pull request body carries no 'Privacy boundary' checklist (the" >&2
        echo "pull request template's section was removed). Restore it and tick" >&2
        echo "each item." >&2
      fi
      echo >&2
      echo "The items are: the data flow described, the roles named, no grant" >&2
      echo "spanning the clinical and demographic domains, synthetic data only," >&2
      echo "and an access event emitted where a read was added." >&2
      echo "The rules are CONTRIBUTING.md § Personal data." >&2
      status=1
    fi
  fi
fi

records=$(added_records "${files[@]}")

if [[ -n "$records" ]]; then
  if bsn_hits=$(printf '%s\n' "$records" | scan_bsn) && [[ -n "$bsn_hits" ]]; then
    echo >&2
    echo "error: added lines carry a nine-digit value passing the BSN eleven-test:" >&2
    printf '%s\n' "$bsn_hits" | sed 's/^/  /' >&2
    echo >&2
    echo "Tests and fixtures use synthetic data. If the value is synthetic and" >&2
    echo "must keep this shape, put 'privacy-allow: <reason>' on the same line." >&2
    status=1
  fi
  if pc_hits=$(printf '%s\n' "$records" | scan_postcode) && [[ -n "$pc_hits" ]]; then
    echo >&2
    echo "error: added lines carry a Dutch postcode with a house number:" >&2
    printf '%s\n' "$pc_hits" | sed 's/^/  /' >&2
    echo >&2
    echo "The pair identifies a household. Use a synthetic address, or put" >&2
    echo "'privacy-allow: <reason>' on the same line." >&2
    status=1
  fi
fi

if [[ "$status" -eq 0 ]]; then
  echo "privacy-boundary: ${#files[@]} changed file(s) checked - OK."
fi
exit "$status"
