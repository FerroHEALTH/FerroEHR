#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Comment-style guard (.claude/rules/comments.md — RFC 505 / RFC 1574).
#
# Checks HAND-WRITTEN .rs files (files carrying the `@generated` marker are
# skipped — their comments are fixed in the openehr-codegen emitter):
#
#   1. block comments      `/* … */` is banned; line comments only (RFC 505).
#   2. TODO form           every TODO names its issue: `TODO(#NNNN):`.
#   3. marker vocabulary   only TODO(#N)/NOTE/SAFETY are sanctioned; FIXME,
#                          HACK, XXX, WIP and the (port) forms all fail.
#   4. NOTE budget         a `// NOTE:` block is a citation + one sentence —
#                          at most $NOTE_MAX physical comment lines.
#   5. essay budget        a plain `//` comment run is at most $RUN_MAX lines;
#                          longer prose belongs in doc comments, the PR
#                          description, or the tracker — not in code.
#   6. orphaned lines      a comment whose whole content is punctuation
#                          (`//.`, `///:` …) is sweep residue, not prose.
#   7. quoted markers      a doc line carrying a backtick-quoted marker
#                          (`` `// NOTE:` `` …) reads as a marker to a human
#                          and is invisible to checks 2-4 — describe the
#                          marker in words instead.
#   8. empty sections      a bare `/// # Errors` / `# Panics` heading with no
#                          body satisfies the doc lints and tells a caller
#                          nothing.
#
# NOT machine-checked, deliberately (#2981): module-doc (`//!`) block LENGTH.
# The tree's longest module docs are governing-section maps and matching-rule
# contracts, longer than any essay the #2942 sweep condensed — a cap loose
# enough to keep them catches nothing, and a cap tight enough to catch
# essays would force condensing legitimate reference docs. Essay-vs-reference
# is judgment; review carries it (comments.md §Enforcement register).
#
# Usage:
#   scripts/checks/comment-style.sh [--all]             # whole tree (the CI job)
#   scripts/checks/comment-style.sh --diff <base> [head]  # changed files only
#   scripts/checks/comment-style.sh --files <f.rs>...   # named files (hook)
#
# A named file is read from the repository root, else from the caller's
# directory, so a path relative to either works.
#
# Exit 0 = clean, 1 = violations (listed as file:line: message), 2 = usage.

set -euo pipefail

NOTE_MAX=3
RUN_MAX=8

caller="$PWD"
cd "$(dirname "$0")/../.."

# --diff reads the change's own paths from the merge base (#4129).
# shellcheck source=scripts/lib/changed-paths.sh
. scripts/lib/changed-paths.sh

mode="${1:---all}"
files=()
case "$mode" in
--all)
  # `git ls-files` reads the index, so a file deleted from the worktree but
  # not yet staged would still be listed — skip it rather than letting awk
  # fail on a missing path.
  while IFS= read -r f; do [[ -f "$f" ]] && files+=("$f"); done \
    < <(git ls-files 'crates/*.rs' 'app/*.rs' 'tools/*.rs')
  ;;
--diff)
  base="${2:?usage: --diff <base> [head]}"
  head="${3:-HEAD}"
  while IFS= read -r f; do
    [[ -f "$f" ]] && files+=("$f")
  done < <(change_paths "$base" "$head" 'crates/*.rs' 'app/*.rs' 'tools/*.rs')
  ;;
--files)
  shift
  for f in "$@"; do
    case "$f" in
    *.rs)
      if [[ -f "$f" ]]; then
        files+=("$f")
      elif [[ -f "$caller/$f" ]]; then
        files+=("$caller/$f")
      fi
      ;;
    *) ;;
    esac
  done
  ;;
*)
  echo "usage: $0 [--all | --diff <base> [head] | --files <f.rs>...]" >&2
  exit 2
  ;;
esac

[[ "${#files[@]}" -eq 0 ]] && {
  echo "comment-style: no files to check — OK."
  exit 0
}
if [[ "$mode" != --files ]]; then
  echo "comment-style: checking ${#files[@]} .rs files under app/, crates/ and tools/ ($mode)."
fi

# One awk reads every file (#4591), each file's state flushed at its first line.
# The emitter writes its banner as the FIRST line of every generated file, so
# the skip anchors there. Matching the marker anywhere let a hand-written file
# exempt itself by merely mentioning it in prose (#2266), and a `head -n 3`
# window was closed only by the SPDX headers that happened to fill it (#2323).
fail=0
# shellcheck disable=SC2016 # an awk program, which xargs runs
out="$(printf '%s\0' "${files[@]}" | xargs -0 awk -v NOTE_MAX="$NOTE_MAX" -v RUN_MAX="$RUN_MAX" '
    function flush_note() {
      if (note_len > NOTE_MAX)
        printf "%s:%d: NOTE block is %d lines (max %d) — a NOTE is a citation + one sentence; move the essay to the PR/issue\n", file, note_start, note_len, NOTE_MAX
      note_len = 0
    }
    function flush_run() {
      if (run_len > RUN_MAX)
        printf "%s:%d: comment run is %d lines (max %d) — long prose belongs in doc comments or on the PR/issue, not in code\n", file, run_start, run_len, RUN_MAX
      run_len = 0
    }
    function flush_doc_note() {
      if (doc_note_len > RUN_MAX)
        printf "%s:%d: NOTE paragraph in a doc comment is %d lines (max %d) — an adjudication essay lives on the PR/issue, not in rustdoc\n", file, doc_note_start, doc_note_len, RUN_MAX
      doc_note_len = 0
    }
    function flush_sec() {
      if (sec_start)
        printf "%s:%d: `# %s` doc section has no body — it satisfies the doc lint and tells a caller nothing; write the contract or drop the heading\n", file, sec_start, sec_name
      sec_start = 0
    }
    FNR == 1 {
      flush_note(); flush_run(); flush_doc_note(); flush_sec()
      file = FILENAME
      skip = (index($0, "// @generated") == 1)
    }
    skip { next }
    {
      line = $0
      sub(/^[[:space:]]+/, "", line)
      is_doc  = (line ~ /^\/\/[\/!]/)
      is_line = (!is_doc && line ~ /^\/\//)

      # 1. block comments on code lines: a `/*` at line start or after
      # whitespace, before any string literal on the line. The position
      # guard skips glob/media-type/URL text inside multi-line string
      # literals (`*/*`, `dir/*.xml`, `://***@`), which an earlier-opened
      # string puts on a quote-less line.
      if (!is_doc && !is_line) {
        bc = index($0, "/*"); q = index($0, "\"")
        if (bc > 0 && (q == 0 || bc < q) \
            && (bc == 1 || substr($0, bc - 1, 1) ~ /[[:space:]]/))
          printf "%s:%d: block comment — use line comments (`//`) only (RFC 505)\n", file, FNR
      }

      # 2 + 3. marker forms. TODO is judged only as the LEADING marker of a
      # comment — prose or a verbatim spec quotation mentioning the word is
      # not a marker.
      if (is_doc || is_line) {
        if (line ~ /^\/\/+[!\/]?[[:space:]]*TODO/ && line !~ /TODO\(#[0-9]+\):/)
          printf "%s:%d: TODO without an issue reference — the only sanctioned form is `TODO(#NNNN):`\n", file, FNR
        if (line ~ /PORT NOTE|PORT STATUS|TODO\(port\)|PERF\(port\)|NOTE\(port\)|FIXME|HACK:|(^|[^A-Za-z0-9_])XXX([^A-Za-z0-9_]|$)|\/\/[[:space:]]*WIP[: ]/)
          printf "%s:%d: unsanctioned comment marker — the only forms are TODO(#NNNN): / NOTE: / SAFETY:\n", file, FNR
      }

      # 6. orphaned punctuation-only comment lines — residue an earlier
      # rewriting sweep left behind (`//.`, `///:`, `//!,` …).
      if ((is_doc || is_line) && line ~ /^\/\/[\/!]?[[:space:]]*[.;:,)]+[[:space:]]*$/)
        printf "%s:%d: comment line carries punctuation only — sweep residue; delete it\n", file, FNR

      # 7. a backtick-quoted marker USED AS a marker — leading the doc
      # line, leading a bullet, or opening a parenthetical — reads as a
      # marker to a human and is invisible to checks 2-4 (#2981). A
      # mid-sentence DESCRIPTION ("the emitter writes a `// NOTE:` …")
      # stays legal: precision over recall keeps the guard trusted.
      if (is_doc && (line ~ /^\/\/[\/!][[:space:]]*([-*][[:space:]]+)?`\/\/[[:space:]]?(NOTE|TODO|SAFETY)/ \
          || line ~ /\(`\/\/[[:space:]]?(NOTE|TODO|SAFETY)/))
        printf "%s:%d: doc line uses a backtick-quoted comment marker as a marker — invisible to the marker checks; write a real NOTE/TODO or plain prose\n", file, FNR

      # 4 + 5. NOTE / plain-run budgets
      if (is_line) {
        if (line ~ /^\/\/[[:space:]]*NOTE/) {
          flush_note(); flush_run()
          note_start = FNR; note_len = 1
        } else if (note_len > 0) {
          note_len++
        } else {
          if (run_len == 0) run_start = FNR
          run_len++
        }
        next
      }
      # The NOTE budget inside doc comments (`/// NOTE:` / `//! NOTE:`) —
      # a doc-relocated essay is the same essay (#2441). The paragraph ends
      # at a blank doc line, per rustdoc paragraph semantics.
      if (is_doc) {
        if (line ~ /^\/\/[\/!][[:space:]]*NOTE/) {
          flush_doc_note()
          doc_note_start = FNR; doc_note_len = 1
        } else if (doc_note_len > 0) {
          # The paragraph ends at a blank doc line (rustdoc paragraph
          # semantics) or at the next list item (a NOTE inside a list does
          # not swallow its sibling items).
          if (line ~ /^\/\/[\/!][[:space:]]*$/ \
              || line ~ /^\/\/[\/!][[:space:]]+([-*][[:space:]]|[0-9]+\.[[:space:]])/ \
              || line ~ /^\/\/[\/!][[:space:]]*#/)
            flush_doc_note()
          else doc_note_len++
        }
        # 8. a lint-required section heading must carry a body: a bare
        # `# Errors` / `# Panics` satisfies missing_errors_doc /
        # missing_panics_doc while documenting nothing. Pending state
        # clears on the first non-blank, non-heading doc line and trips on
        # another heading or the end of the doc block.
        if (line ~ /^\/\/[\/!][[:space:]]*#[[:space:]]*(Errors|Panics)[[:space:]]*$/) {
          flush_sec()
          sec_start = FNR
          sec_name = (line ~ /Errors/) ? "Errors" : "Panics"
        } else if (sec_start) {
          if (line ~ /^\/\/[\/!][[:space:]]*#/) flush_sec()
          else if (line !~ /^\/\/[\/!][[:space:]]*$/) sec_start = 0
        }
        flush_note(); flush_run()
        next
      }
      flush_note(); flush_run(); flush_doc_note(); flush_sec()
    }
    END { flush_note(); flush_run(); flush_doc_note(); flush_sec() }
  ')"
if [[ -n "$out" ]]; then
  printf '%s\n' "$out"
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  echo "comment-style: violations found (rules: .claude/rules/comments.md)." >&2
  exit 1
fi
echo "comment-style: OK (${#files[@]} files)."
