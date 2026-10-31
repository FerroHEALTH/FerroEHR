#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Stray-literal guard: a string a reader sees belongs in a catalog under
# locales/, never in the code (no specification governs this: our own
# design).
#
# It reads the hand-written .rs files under each ROOT, up to the file's
# `#[cfg(test)]` module, and reports every string literal that reads as
# words a reader sees:
#
#   1. the value of an attribute a reader sees or hears (`aria-label`,
#      `title`, `placeholder`, `alt`, `data-label`, `aria-description`,
#      `aria-roledescription`, `aria-valuetext`, `label`) holding a letter;
#   2. a text node of a `view!`: a literal between two tags (`>"Close"<`)
#      or alone on its line, holding a letter;
#   3. any other literal holding words: an upper-case letter, or a space
#      next to a letter, outside its `{}` placeables, and not a class list.
#
# A single lower-case word in code (`=> "fatal"`) reads as a key and is
# passed over, so a word drawn that way is review's to catch. Lines that
# speak to a developer are passed over: `expect(`, log macros,
# `write!(f`, `f.write_str(`, `debug_struct(`, `.field(`, a lint's `reason`, assertions and
# attributes. A literal the check reads wrong is allowed by a line of the
# allow file, and a file the check does not cover is skipped there:
#
#   allow <perl regex matched against the literal's text>   # the reason
#   skip <path or directory prefix, from the repository root> # the reason
#
# The baseline file holds `<count> <path>` lines for a tree whose strings
# are still moving to the catalogs: a file may hold up to its count and no
# more, and a file not named may hold none. A file below its count passes
# with a one-line note, so a pull request that moves strings to the catalogs
# touches no baseline and conflicts with no other one; the scheduled
# ratchet-tighten workflow runs --update on main and opens the one pull
# request that lowers the counts (#4554). With --update the script lowers
# each count to what it finds and removes a file that holds none. It never
# raises a count and never adds a file: a rise that has to stay is a hand
# edit of the baseline in a change that says why.
#
# Usage:
#   scripts/checks/stray-literals.sh [--update] ROOT...
#   scripts/checks/stray-literals.sh --self-test   # prove the guard catches
#
# The allow file is scripts/checks/stray-literals.allow and the baseline
# scripts/checks/stray-literals.baseline, both optional.
#
# Exit 0 = clean, 1 = a stray literal (listed as file:line: literal), 2 =
# a usage error.
set -Eeuo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

# Runs a copy of this guard in a miniature project and asserts its verdicts.
self_test() {
  local self work project answer status
  self="$root/scripts/checks/stray-literals.sh"
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: the path is fixed and the trap runs after this function returns
  trap "rm -rf '$work'" EXIT
  project="$work/project"
  answer="$work/answer"
  mkdir -p "$project/scripts/checks" "$project/src"
  cp "$self" "$project/scripts/checks/stray-literals.sh"

  fail() {
    echo "stray-literals: SELF-TEST FAILED: $1" >&2
    exit 1
  }
  # Writes a Rust file at src/$1 holding $2 literals a reader sees, beside a
  # key the guard passes over.
  literals() {
    awk -v n="$2" 'BEGIN {
      print "pub fn level() -> String {"
      print "    \"fatal\".to_owned()"
      print "}"
      for (i = 1; i <= n; i++) printf "pub fn text%d() -> String {\n    let s = \"Words number %d\";\n    s.to_owned()\n}\n", i, i
    }' >"$project/src/$1"
  }
  run() {
    status=0
    bash "$project/scripts/checks/stray-literals.sh" "$@" >"$answer" 2>&1 || status=$?
    printf '%s\n' "$status"
  }
  baseline_of() {
    awk -v f="src/$1" '$2 == f { print $1 }' "$project/scripts/checks/stray-literals.baseline"
  }
  write_baseline() {
    {
      printf '# The miniature baseline.\n'
      if [[ -n "$1" ]]; then printf '%s\n' "$1"; fi
    } >"$project/scripts/checks/stray-literals.baseline"
  }

  printf 'allow ^Cadasto B\\.V\\.$   # the maker legal name\n' >"$project/scripts/checks/stray-literals.allow"
  literals held.rs 2
  literals clean.rs 0
  printf 'pub fn maker() -> String {\n    let s = "Cadasto B.V.";\n    s.to_owned()\n}\n' >"$project/src/maker.rs"
  write_baseline "2 src/held.rs"

  [[ "$(run src)" == 0 ]] || fail "a tree within its baseline was refused: $(cat "$answer")"
  grep -q 'note:' "$answer" && fail "a tree at its baseline was noted as a fall: $(cat "$answer")"

  # A new literal in a file the baseline does not name fails and is named,
  # and --update never adds the file.
  literals clean.rs 1
  [[ "$(run src)" == 1 ]] || fail "a new literal passed"
  grep -qF 'src/clean.rs:5: "Words number 1"' "$answer" || fail "a new literal was not named: $(cat "$answer")"
  [[ "$(run --update src)" == 1 ]] || fail "--update passed a new literal"
  [[ -z "$(baseline_of clean.rs)" ]] || fail "--update added a file to the baseline"
  literals clean.rs 0

  # A rise in a listed file fails, and --update never raises its count.
  literals held.rs 3
  [[ "$(run src)" == 1 ]] || fail "a rise past the baseline passed"
  grep -qF 'src/held.rs holds 3 stray literal(s), 2 allowed' "$answer" || fail "a rise was not named: $(cat "$answer")"
  [[ "$(run --update src)" == 1 ]] || fail "--update passed a rise"
  [[ "$(baseline_of held.rs)" == 2 ]] || fail "--update raised the count to $(baseline_of held.rs)"

  # A fall passes with the note and leaves the baseline, and --update lowers
  # the count, then removes the file that holds none.
  literals held.rs 1
  [[ "$(run src)" == 0 ]] || fail "a fall was refused: $(cat "$answer")"
  grep -qF 'note: 1 file(s) hold fewer than' "$answer" || fail "a fall was not noted: $(cat "$answer")"
  [[ "$(baseline_of held.rs)" == 2 ]] || fail "a check changed the baseline"
  [[ "$(run --update src)" == 0 ]] || fail "--update refused a fall: $(cat "$answer")"
  [[ "$(baseline_of held.rs)" == 1 ]] || fail "--update did not lower the count to 1"
  literals held.rs 0
  [[ "$(run --update src)" == 0 ]] || fail "--update refused a file that holds none: $(cat "$answer")"
  [[ -z "$(baseline_of held.rs)" ]] || fail "--update kept a file that holds none"
  [[ "$(run src)" == 0 ]] || fail "the emptied baseline was refused: $(cat "$answer")"

  # The allow line exempts its literal, and without it the literal fails.
  : >"$project/scripts/checks/stray-literals.allow"
  [[ "$(run src)" == 1 ]] || fail "the maker literal passed with no allow line"
  grep -qF 'src/maker.rs:2: "Cadasto B.V."' "$answer" || fail "the literal was not named: $(cat "$answer")"

  echo "stray-literals: self-test OK (a new literal, a rise, a fall with its note, --update lowering and never raising, and an allow line)."
}

if [[ "${1:-}" = "--self-test" ]]; then
  self_test
  exit 0
fi

update=0
if [[ "${1:-}" = "--update" ]]; then
  update=1
  shift
fi
if [[ "$#" -eq 0 ]]; then
  echo "usage: scripts/checks/stray-literals.sh [--update] ROOT..." >&2
  exit 2
fi
for dir in "$@"; do
  if [[ ! -d "$dir" ]]; then
    echo "stray-literals: $dir is no directory" >&2
    exit 2
  fi
done

allow="scripts/checks/stray-literals.allow"
baseline="scripts/checks/stray-literals.baseline"

files=()
while IFS= read -r file; do
  files+=("$file")
done < <(find "$@" -type f -name '*.rs' | LC_ALL=C sort)
if [[ "${#files[@]}" -eq 0 ]]; then
  echo "stray-literals: no .rs file under $*"
  exit 0
fi

found="$(
  awk -v allowfile="$allow" '
    function wordy(t,    bare, n, i, toks, plain) {
      bare = t
      gsub(/\{[^{}]*\}/, "", bare)
      if (bare !~ /[A-Za-z]/) return 0
      n = split(bare, toks, /[ \t]+/)
      plain = 1
      for (i = 1; i <= n; i++) if (toks[i] != "" && toks[i] !~ /^fui-[a-z0-9-]+$/) plain = 0
      if (plain) return 0
      if (bare ~ /[A-Z]/) return 1
      if (bare ~ /[A-Za-z][ \t]/ || bare ~ /[ \t][A-Za-z]/) return 1
      return 0
    }
    function classes(t,    n, i, toks) {
      n = split(t, toks, /[ \t]+/)
      for (i = 1; i <= n; i++) if (toks[i] != "" && toks[i] !~ /^fui-[a-z0-9-]+$/) return 0
      return 1
    }
    BEGIN {
      nallow = 0
      nskip = 0
      while ((getline line < allowfile) > 0) {
        sub(/[ \t]+#.*$/, "", line)
        if (line ~ /^[ \t]*(#|$)/) continue
        if (line ~ /^allow[ \t]+/) {
          sub(/^allow[ \t]+/, "", line)
          sub(/[ \t]+$/, "", line)
          allow[++nallow] = line
        } else if (line ~ /^skip[ \t]+/) {
          sub(/^skip[ \t]+/, "", line)
          sub(/[ \t]+$/, "", line)
          skip[++nskip] = line
        } else {
          print "stray-literals: " allowfile ": unreadable line: " line > "/dev/stderr"
          exit 2
        }
      }
      visible = "(^|[^A-Za-z-])(aria-label|title|placeholder|alt|data-label|aria-description|aria-roledescription|aria-valuetext|label)[ \t]*=[ \t]*$"
    }
    FNR == 1 {
      off = 0
      for (i = 1; i <= nskip; i++) if (index(FILENAME, skip[i]) == 1) off = 1
      if ($0 ~ /@generated/) off = 1
    }
    off { next }
    /^#\[cfg\(test\)\]/ { off = 1; next }
    /^[ \t]*\/\// { next }
    /^[ \t]*#!?\[/ { next }
    /reason = "|expect\(|logging::|info!\(|warn!\(|error!\(|debug!\(|trace!\(|log!\(|write!\(f|write_str\(|debug_struct\(|\.field\(|assert|panic!\(|unreachable!\(|include_str!\(/ { next }
    {
      code = $0
      while ((s = index(code, "r#\"")) > 0) {
        rest = substr(code, s + 3)
        e = index(rest, "\"#")
        if (e == 0) { code = substr(code, 1, s - 1); break }
        code = substr(code, 1, s - 1) substr(rest, e + 2)
      }
      n = length(code)
      i = 1
      while (i <= n) {
        c = substr(code, i, 1)
        if (c == "'"'"'" && substr(code, i + 2, 1) == "'"'"'") { i += 3; continue }
        if (c != "\"") { i++; continue }
        start = i
        lit = ""
        i++
        while (i <= n) {
          c = substr(code, i, 1)
          if (c == "\\") { lit = lit substr(code, i, 2); i += 2; continue }
          if (c == "\"") break
          lit = lit c
          i++
        }
        pre = substr(code, 1, start - 1)
        after = substr(code, i + 1)
        i++
        hit = 0
        if (pre ~ visible) {
          hit = lit ~ /[A-Za-z]/
        } else if (pre ~ /[A-Za-z:-]=$/) {
          hit = 0
        } else if (pre ~ />[ \t]*$/ && after ~ /^[ \t]*</) {
          hit = lit ~ /[A-Za-z]/
        } else if (pre ~ /^[ \t]*$/ && after ~ /^[ \t]*$/) {
          hit = lit ~ /[A-Za-z]/ && !classes(lit)
        } else {
          hit = wordy(lit)
        }
        if (!hit) continue
        allowed = 0
        for (a = 1; a <= nallow; a++) if (lit ~ allow[a]) allowed = 1
        if (!allowed) print FILENAME ":" FNR ": \"" lit "\""
      }
    }
  ' ${files[@]+"${files[@]}"}
)"

counts="$(printf '%s\n' "$found" | sed -n 's/^\([^:]*\):[0-9]*:.*/\1/p' | LC_ALL=C sort | uniq -c | awk '{print $1, $2}')"

if [[ "$update" -eq 1 ]]; then
  lowered=""
  if [[ -f "$baseline" ]]; then
    lowered="$(printf '%s\n' "$counts" | awk '
      NR == FNR { if (NF == 2) have[$2] = $1; next }
      /^[[:space:]]*(#|$)/ { next }
      NF == 2 && have[$2] + 0 > 0 { print ((have[$2] + 0 < $1 + 0) ? have[$2] : $1), $2 }
    ' - "$baseline")"
  fi
  {
    printf '# %s: %s\n' "SPDX-FileCopyrightText" "Cadasto B.V."
    printf '# %s: %s\n' "SPDX-License-Identifier" "BUSL-1.1"
    echo "# The stray literals each file may still hold while its strings move to"
    echo "# the catalogs; lowered by scripts/checks/stray-literals.sh --update."
    if [[ -n "$lowered" ]]; then printf '%s\n' "$lowered"; fi
  } > "$baseline"
  echo "stray-literals: wrote $baseline"
fi

status=0
while IFS= read -r entry; do
  [[ -n "$entry" ]] || continue
  count="${entry%% *}"
  file="${entry#* }"
  allowed=0
  if [[ -f "$baseline" ]]; then
    allowed="$(awk -v f="$file" '$2 == f { print $1 }' "$baseline")"
    allowed="${allowed:-0}"
  fi
  if [[ "$count" -gt "$allowed" ]]; then
    status=1
    echo "stray-literals: $file holds $count stray literal(s), $allowed allowed:" >&2
    printf '%s\n' "$found" | awk -v f="$file:" 'index($0, f) == 1' >&2
  fi
done <<< "$counts"

fell=0
if [[ -f "$baseline" ]]; then
  fell="$(printf '%s\n' "$counts" | awk '
    NR == FNR { if (NF == 2) have[$2] = $1; next }
    /^[[:space:]]*(#|$)/ { next }
    NF == 2 && $1 > have[$2] + 0 { fell++ }
    END { print fell + 0 }
  ' - "$baseline")"
fi

if [[ "$status" -ne 0 ]]; then
  echo "stray-literals: move each string into the catalog under locales/ and look it up, or allow a literal that is no reader's text in $allow with the reason." >&2
  exit 1
fi
echo "stray-literals: no stray literal under $*"
if [[ "$fell" -gt 0 ]]; then
  echo "stray-literals: note: $fell file(s) hold fewer than $baseline allows; the scheduled ratchet-tighten workflow lowers the counts."
fi
