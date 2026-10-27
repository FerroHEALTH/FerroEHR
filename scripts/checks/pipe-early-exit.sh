#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# No reader that stops early sits on a pipe (#4208).
#
# `producer | grep -q pattern` under `set -o pipefail` reads a match as a
# miss once the producer's output tops the pipe buffer (64 KiB on Linux):
# grep exits at the first match, the producer dies of SIGPIPE, and the
# pipeline returns 141, which an `if` takes as "no match". The CI path
# classifiers had that shape, so a large pull request could skip the lanes it
# needed. `head`, `grep -m` and `grep -l` stop reading the same way.
#
# The safe forms read the whole input first: a here-string
# (`grep -q pattern <<<"$list"`), a variable, or a reader that never stops
# early (`sed -n 1p` in place of `head -1`).
#
# The check refuses, in every workflow and composite action and in every shell
# program this repository wrote (the scope of shellcheck-lane.sh), a line
# where a pipe feeds `head`, `read` or a `grep` carrying -q, -m, -l, --quiet,
# --silent or --max-count. A pipeline split across lines counts as one line:
# a line ending in `\` or in a lone `|` joins the next, so `producer |` over
# `head -1` is refused like `producer | head -1`. A YAML block scalar
# (`run: |`) is not a pipe and joins nothing. Comment lines are skipped, and
# so is this script, whose self-test cases are the refused shapes. A line
# that runs the shape on purpose, to prove it fails, ends in
# `# pipe-early-exit: deliberate`.
# `sed q` and `awk exit` stop early too and are not matched: review carries
# them.
#
# Usage:
#   scripts/checks/pipe-early-exit.sh              every file in scope
#   scripts/checks/pipe-early-exit.sh <file>...    just those files
#   scripts/checks/pipe-early-exit.sh --self-test  prove the pattern
#
# Exit 0 = clean, 1 = findings (file:line: text), 2 = usage.
set -euo pipefail

# A pipe (never `||`), then `head`, `read` or a grep with an early-exit flag.
readonly PIPE='(^|[^|])[|][[:space:]]*'
readonly GREP_EARLY='e?f?grep([[:space:]]+[^|;&]*)?[[:space:]](-[A-Za-z]*[qml][A-Za-z0-9]*|--quiet|--silent|--max-count)([[:space:]=]|$)'
readonly HEAD='head([[:space:]]|$)'
readonly READ='([A-Za-z_][A-Za-z0-9_]*=[^[:space:]]*[[:space:]]+)*read([[:space:]]|$)'
readonly PATTERN="${PIPE}(${GREP_EARLY}|${HEAD}|${READ})"
# A line whose first non-blank character is `#` is a comment in shell and YAML.
readonly COMMENT='^[[:space:]]*#'
readonly DELIBERATE='# pipe-early-exit: deliberate$'
readonly SELF='scripts/checks/pipe-early-exit.sh'

readonly VENDORED='^(docs/specs/|corpus/|website/book/theme/|(crates|tools)/[^/]+/vendor/)'

# Print stdin as logical lines, `line:text`, each numbered by its first
# physical line. A line ending in `\`, or in a lone `|` that no `:` or `-`
# opens as a YAML block scalar, joins the next; blank and comment lines after
# such a `|` are dropped, as the shell drops them. A comment never joins.
logical() {
  awk '
    function flush() { if (buf != "") print start ":" buf; buf = "" }
    {
      line = $0
      if (buf == "") {
        start = NR
        if (line ~ /^[[:space:]]*#/) { buf = line; flush(); next }
        buf = line
      } else if (piped && line ~ /^[[:space:]]*(#|$)/) {
        next
      } else {
        buf = buf " " line
      }
      piped = 0
      if (buf ~ /\\[[:space:]]*$/) {
        sub(/\\[[:space:]]*$/, "", buf)
      } else if (buf ~ /(^|[^|])[|][[:space:]]*$/ && buf !~ /(:|^[[:space:]]*-)[[:space:]]*[|][[:space:]]*$/) {
        piped = 1
      } else {
        flush()
      }
    }
    END { flush() }
  '
}

# Print `line:text` for every finding in stdin.
findings_in() {
  local out line
  out=$(logical | grep -E -- "$PATTERN" || true)
  [[ -n "$out" ]] || return 0
  while IFS= read -r line; do
    [[ "${line#*:}" =~ $COMMENT || "$line" =~ $DELIBERATE ]] && continue
    printf '%s\n' "$line"
  done <<<"$out"
}

# Print `file:line: text` for every finding in the named files.
scan() {
  local f line
  for f in "$@"; do
    [[ -f "$f" ]] || continue
    while IFS= read -r line; do
      [[ -n "$line" ]] && printf '%s:%s\n' "$f" "$line"
    done < <(findings_in <"$f")
  done
}

# The workflows, the composite actions and every tracked shell program.
collect() {
  local f all
  all=$(git ls-files)
  grep -E '^\.github/(workflows|actions)/.*\.ya?ml$' <<<"$all" || true
  grep -E '\.sh$' <<<"$all" | grep -Ev "$VENDORED" | grep -Fxv "$SELF" || true
  while IFS= read -r f; do
    [[ -f "$f" ]] || continue
    if grep -Eq '^#!.*[/ ](bash|sh|dash|ksh|zsh)([[:space:]]|$)' <<<"$(sed -n 1p "$f")"; then
      printf '%s\n' "$f"
    fi
  done < <(grep -Ev "$VENDORED" <<<"$all" | grep -Ev '\.[^/]+$' || true)
}

# Each case is `want|text`, where `<NL>` in the text stands for a line break.
self_test() {
  local failures=0 refused=0 allowed=0 want text got
  while IFS='|' read -r want text; do
    text="${text//<NL>/$'\n'}"
    if [[ -n "$(findings_in <<<"$text")" ]]; then
      got=refused
    else
      got=allowed
    fi
    if [[ "$want" == refused ]]; then
      refused=$((refused + 1))
    else
      allowed=$((allowed + 1))
    fi
    if [[ "$got" != "$want" ]]; then
      echo "self-test: '$text': got $got, want $want" >&2
      failures=1
    fi
  done <<'CASES'
refused|if echo "$changed" | grep -qE '^app/'; then
refused|printf '%s\n' "$assets" | grep -Fxq "$asset"
refused|  | grep -qiE 'static binary' \
refused|list | grep --quiet x
refused|list | grep -m 1 x
refused|git show "$base:Cargo.toml" | grep -m1 '^version = '
refused|list | grep --max-count=1 x
refused|list | grep -l x
refused|sed -nE 's/^v=(.*)/\1/p' Cargo.toml | head -1
refused|out | head
refused|x |head -n 1
refused|list | read -r first
refused|list | IFS= read -r first
refused|list |<NL>  head -1
refused|list |  <NL><NL>  # the first line<NL>  head -1
refused|list | \<NL>  grep -q x
refused|list \<NL>  | grep -qx x
refused|git log \<NL>  --oneline \<NL>  | head -n 3
refused|if printf '%s\n' "$a" |<NL>  grep -Fxq "$b"; then
allowed|if grep -qE '^app/' <<<"$changed"; then
allowed|grep -q x file || echo missing
allowed|true || head -1 file
allowed|sed -nE 's/^v=(.*)/\1/p' Cargo.toml | sed -n 1p
allowed|list | grep -vE '^scripts/'
allowed|list | grep -c x
allowed|head -n 1 "$f"
allowed|# never `printf | grep -q` here
allowed|  # list | head -1
allowed|run: |
allowed|(set -o pipefail; printf '%s\n' "$big" | grep -qx x) # pipe-early-exit: deliberate
allowed|list | while IFS= read -r line; do
allowed|list |<NL>  sed -n 1p
allowed|list | \<NL>  grep -v x
allowed|list \<NL>  --flag<NL>head -1 file
allowed|run: |<NL>  head -1 file
allowed|- |<NL>  grep -q x file
allowed|key: |  <NL>  read -r x <file
allowed|# list |<NL>head -1 file
allowed|true ||<NL>  head -1 file
allowed|(set -o pipefail; printf '%s\n' "$big" |<NL>  grep -qx x) # pipe-early-exit: deliberate
CASES
  [[ "$failures" -eq 0 ]] || return 1
  echo "pipe-early-exit: self-test OK ($refused early readers on a pipe refused, $allowed safe forms allowed)."
}

case "${1-}" in
--self-test)
  [[ "$#" -eq 1 ]] || { echo "usage: $0 [<file>... | --self-test]" >&2; exit 2; }
  self_test
  exit
  ;;
-*)
  echo "error: $0: unknown flag $1" >&2
  echo "usage: $0 [<file>... | --self-test]" >&2
  exit 2
  ;;
*) ;;
esac

cd "$(dirname "$0")/../.."
if [[ "$#" -gt 0 ]]; then
  files=("$@")
else
  files=()
  while IFS= read -r f; do files+=("$f"); done < <(collect | sort -u)
fi

findings=$(scan "${files[@]}")
if [[ -n "$findings" ]]; then
  printf '%s\n' "$findings" >&2
  echo >&2
  echo "pipe-early-exit: a pipe feeds a reader that stops early (head, grep -q/-m/-l)." >&2
  echo "Under pipefail a large input makes the producer die of SIGPIPE and the" >&2
  echo "pipeline return 141. Read the whole input first: grep -q pattern <<<\"\$list\"," >&2
  echo "or sed -n 1p in place of head -1." >&2
  exit 1
fi
echo "pipe-early-exit: OK (${#files[@]} files, no early reader on a pipe)."
