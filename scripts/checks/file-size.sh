#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# File-size ratchet: a hand-written source file under app/*/src/,
# crates/*/src/ or tools/*/src/ carries at most 800 lines, its inline test
# module included (#4454).
#
# The budget is .claude/rules/rust-style.md, "Code is written once", and no
# specification governs the number: our own design. A file over it stands in
# file-size.list beside this guard with the length it had when it was
# listed, and it may shrink and never grow. The tree run refuses:
#   - a file over the budget that the list does not carry;
#   - a listed file longer than its entry;
#   - a listed file shorter than its entry, within the budget or no longer
#     tracked, until --update lowers or removes the entry, so the list only
#     tightens.
# --update lowers and removes entries. It never raises one and never adds a
# file: a file over the budget is split, and a rise that has to stay is a
# hand edit of the list in a change that says why.
#
# The per-edit mode (--files, run by .claude/hooks/rust_fmt_clippy.sh)
# refuses the first two and passes a shrink: a split shortens its file edit
# by edit, and a list lowered halfway would refuse the edit that follows.
# The tree run reads tracked files, so an untracked draft never fails it.
# Lines count as `wc -l` counts them. A file that opens with `// @generated`
# is the emitter's output (.claude/rules/codegen.md) and is skipped.
#
# Usage:
#   scripts/checks/file-size.sh                    # every tracked file in scope
#   scripts/checks/file-size.sh --files <f.rs>...  # named files (the per-edit hook)
#   scripts/checks/file-size.sh --update           # lower or remove entries that shrank
#   scripts/checks/file-size.sh --self-test        # prove the guard catches
#
# Exit 0 = within the budget and the list, 1 = a finding, 2 = usage or a
# malformed list.
#
# No specification governs this: our own design.
set -Eeuo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/../.."

readonly BUDGET=800
readonly LIST_FILE=scripts/checks/file-size.list
readonly SCOPE='^(app|crates|tools)/[^/[:space:]]+/src/[^[:space:]]+\.rs$'

# The path of $1 inside the checkout that holds it, so a file edited in a
# worktree under .claude/worktrees/ is measured by where it stands in its own
# tree. A relative path, and a path in no checkout, is returned as given.
repo_relative() {
  local f=$1 dir top
  case "$f" in
    /*) ;;
    *)
      printf '%s\n' "$f"
      return 0
      ;;
  esac
  if ! dir="$(cd "$(dirname "$f")" 2>/dev/null && pwd -P)" ||
    ! top="$(git -C "$dir" rev-parse --show-toplevel 2>/dev/null)"; then
    printf '%s\n' "$f"
    return 0
  fi
  top="$(cd "$top" && pwd -P)"
  case "$dir/" in
    "$top"/*) ;;
    *)
      printf '%s\n' "$f"
      return 0
      ;;
  esac
  local rel="${dir#"$top"}/${f##*/}"
  printf '%s\n' "${rel#/}"
}

# True when the repository path $1 is a file the budget measures.
in_scope() {
  [[ "$1" =~ $SCOPE ]]
}

# True when the file $1 opens with the generated banner.
generated() {
  [[ "$(head -n 1 "$1" 2>/dev/null)" == '// @generated'* ]]
}

# The length of the file $1 in lines, as wc -l counts them.
lines_of() {
  local n
  n="$(wc -l <"$1")"
  printf '%s\n' "${n//[[:space:]]/}"
}

# Prints the entries of the list $1 as `<path> <lines>`, one per line, and
# returns 2 on a line that is no entry or on a path listed twice.
entries_of() {
  local file="$1" line number=0 seen="" path n
  [[ -f "$file" ]] || return 0
  while IFS= read -r line || [[ -n "$line" ]]; do
    number=$((number + 1))
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    path="" n=""
    if [[ "$line" =~ ^([^[:space:]]+)\ ([1-9][0-9]*)$ ]]; then
      path="${BASH_REMATCH[1]}" n="${BASH_REMATCH[2]}"
    fi
    if [[ -z "$path" ]] || ! in_scope "$path"; then
      echo "file-size: $file:$number: '$line' is no '<{app,crates,tools}/*/src/ path> <lines>' entry" >&2
      return 2
    fi
    if grep -qxF "$path" <<<"$seen"; then
      echo "file-size: $file:$number: $path is listed twice" >&2
      return 2
    fi
    seen+="$path"$'\n'
    printf '%s %s\n' "$path" "$n"
  done <"$file"
}

# Prints `<path> <lines>` for every tracked file in scope of the checkout $1.
measured_of() {
  local tree="$1" path
  while IFS= read -r path; do
    in_scope "$path" || continue
    [[ -f "$tree/$path" ]] || continue
    generated "$tree/$path" && continue
    printf '%s %s\n' "$path" "$(lines_of "$tree/$path")"
  done < <(git -C "$tree" ls-files -- 'app/*.rs' 'crates/*.rs' 'tools/*.rs')
}

# Joins the entries $1 with the measure $2 and prints one verdict per path,
# sorted: FIT, UNLISTED, GREW, HELD, SHRANK, UNDER or GONE, then the path,
# the length measured and the length listed.
verdicts() {
  {
    awk 'NF == 2 { print "L", $0 }' <<<"$1"
    awk 'NF == 2 { print "M", $0 }' <<<"$2"
  } | awk -v budget="$BUDGET" '
    $1 == "L" { listed[$2] = $3; next }
    $1 == "M" { measured[$2] = $3; next }
    END {
      for (path in measured) {
        n = measured[path]
        if (!(path in listed)) {
          print ((n > budget) ? "UNLISTED" : "FIT"), path, n, 0
        } else if (n <= budget) {
          print "UNDER", path, n, listed[path]
        } else if (n > listed[path]) {
          print "GREW", path, n, listed[path]
        } else if (n < listed[path]) {
          print "SHRANK", path, n, listed[path]
        } else {
          print "HELD", path, n, listed[path]
        }
      }
      for (path in listed) {
        if (!(path in measured)) print "GONE", path, 0, listed[path]
      }
    }
  ' | LC_ALL=C sort -k2,2
}

write_list() {
  {
    # REUSE-IgnoreStart
    printf '# SPDX-FileCopyrightText: Cadasto B.V.\n'
    printf '# SPDX-License-Identifier: BUSL-1.1\n'
    # REUSE-IgnoreEnd
    printf '# The hand-written files under app/*/src/, crates/*/src/ and\n'
    printf '# tools/*/src/ over the budget of %s lines that\n' "$BUDGET"
    printf '# scripts/checks/file-size.sh admits, each with the length it may not\n'
    printf '# pass. An entry may fall and never rise; --update lowers and removes\n'
    printf '# entries.\n'
    if [[ -n "$2" ]]; then
      printf '%s\n' "$2"
    fi
  } >"$1"
}

# Measures the checkout $1 against the list $2. With $3 = update it first
# lowers every entry that shrank and removes every entry within the budget
# or no longer tracked, then measures again.
ratchet() {
  local tree="$1" list="$2" mode="$3" entries measured table kept status=0
  entries="$(entries_of "$list")" || return 2
  measured="$(measured_of "$tree")"
  table="$(verdicts "$entries" "$measured")"

  if [[ "$mode" == update ]]; then
    kept="$(awk '$1 == "GREW" || $1 == "HELD" { print $2, $4 } $1 == "SHRANK" { print $2, $3 }' <<<"$table")"
    write_list "$list" "$kept"
    echo "file-size: $list now carries $(grep -c . <<<"$kept" || true) entries."
    entries="$(entries_of "$list")" || return 2
    table="$(verdicts "$entries" "$measured")"
  fi

  local verdict path n entry files=0 listed=0 over=0
  while read -r verdict path n entry; do
    [[ -n "${verdict:-}" ]] || continue
    case "$verdict" in
      FIT) files=$((files + 1)) ;;
      HELD)
        files=$((files + 1))
        listed=$((listed + 1))
        over=$((over + n - BUDGET))
        ;;
      UNLISTED)
        files=$((files + 1))
        echo "file-size: $path: $n lines, over the budget of $BUDGET, and $list does not carry it; split it by concern (.claude/rules/rust-style.md, Code is written once)" >&2
        status=1
        ;;
      GREW)
        files=$((files + 1))
        echo "file-size: $path: $n lines, grown past the $entry lines $list records; a listed file may shrink and never grow, so move code out of it" >&2
        status=1
        ;;
      SHRANK)
        files=$((files + 1))
        echo "file-size: $path: $n lines, and $list records $entry; run $0 --update and commit the lower entry" >&2
        status=1
        ;;
      UNDER)
        files=$((files + 1))
        echo "file-size: $path: $n lines, within the budget of $BUDGET, and $list still lists it; run $0 --update and commit the list without it" >&2
        status=1
        ;;
      GONE)
        echo "file-size: $path: $list lists it with $entry lines and the tree no longer tracks it; run $0 --update and commit the list without it" >&2
        status=1
        ;;
    esac
  done <<<"$table"
  if [[ "$status" -eq 0 ]]; then
    echo "file-size: OK ($files hand-written files, $listed listed over the budget of $BUDGET lines, $over lines over it)."
  fi
  return "$status"
}

# Measures the named files against the list $1 and passes a shrink.
per_edit() {
  local list="$1" entries file rel n entry status=0 checked=0
  shift
  entries="$(entries_of "$list")" || return 2
  for file in "$@"; do
    [[ "$file" == *.rs && -f "$file" ]] || continue
    rel="$(repo_relative "$file")"
    in_scope "$rel" || continue
    generated "$file" && continue
    checked=$((checked + 1))
    n="$(lines_of "$file")"
    entry="$(awk -v path="$rel" '$1 == path { print $2 }' <<<"$entries")"
    if [[ -z "$entry" && "$n" -gt "$BUDGET" ]]; then
      echo "file-size: $rel: $n lines, over the budget of $BUDGET, and $list does not carry it; split it by concern (.claude/rules/rust-style.md, Code is written once)" >&2
      status=1
    elif [[ -n "$entry" && "$n" -gt "$entry" ]]; then
      echo "file-size: $rel: $n lines, grown past the $entry lines $list records; a listed file may shrink and never grow, so move code out of it" >&2
      status=1
    fi
  done
  if [[ "$status" -eq 0 ]]; then
    echo "file-size: OK ($checked files in scope)."
  fi
  return "$status"
}

self_test() {
  local self work tree list status
  self="$PWD/scripts/checks/file-size.sh"
  work="$(mktemp -d)"
  # shellcheck disable=SC2064 # expand now: the path is fixed and the trap runs after this function returns
  trap "rm -rf '$work'" EXIT
  tree="$work/tree"
  list="$work/file-size.list"
  mkdir -p "$tree/app/demo/src/views" "$tree/app/demo/tests/it" "$tree/crates/demo/src" "$tree/tools/demo/src" \
    "$tree/fuzz/fuzz_targets"
  git -c init.defaultBranch=main init -q "$tree"

  fail() {
    echo "file-size: SELF-TEST FAILED: $1" >&2
    exit 1
  }
  # Writes a Rust file of $2 lines at $1 that rustfmt leaves as it is.
  lines() {
    awk -v n="$2" 'BEGIN { for (i = 1; i <= n; i++) printf "pub fn f%d() {}\n", i }' >"$1"
  }
  run() {
    status=0
    ratchet "$tree" "$list" "$1" >"$work/answer" 2>&1 || status=$?
    printf '%s\n' "$status"
  }
  entry() {
    awk -v path="$1" '$1 == path { print $2 }' "$list"
  }

  lines "$tree/app/demo/src/big.rs" 900
  lines "$tree/app/demo/src/views/wide.rs" 850
  lines "$tree/crates/demo/src/deep.rs" 820
  lines "$tree/app/demo/src/small.rs" "$BUDGET"
  # Over the budget where the budget does not look.
  lines "$tree/app/demo/tests/it/long.rs" 1200
  lines "$tree/fuzz/fuzz_targets/long.rs" 1200
  lines "$work/body.rs" 1200
  { printf '// @generated by a miniature. DO NOT EDIT.\n'; cat "$work/body.rs"; } >"$tree/crates/demo/src/generated.rs"
  # An untracked draft over the budget.
  lines "$tree/tools/demo/src/draft.rs" 1200
  git -C "$tree" add -- app crates fuzz
  write_list "$list" "app/demo/src/big.rs 900
app/demo/src/views/wide.rs 850
crates/demo/src/deep.rs 820"

  [[ "$(run check)" == 0 ]] || fail "a tree within the budget and the list was refused: $(cat "$work/answer")"
  grep -q '3 listed over the budget of 800 lines, 170 lines over it' "$work/answer" ||
    fail "the tree run did not count the three listed files: $(cat "$work/answer")"

  # A file over the budget that the list does not carry, in each of the three roots.
  local place
  for place in app/demo/src/grown.rs crates/demo/src/grown.rs tools/demo/src/grown.rs; do
    lines "$tree/$place" 801
    git -C "$tree" add -- "$place"
    [[ "$(run check)" == 1 ]] || fail "an unlisted file of 801 lines at $place passed"
    grep -q "$place: 801 lines, over the budget" "$work/answer" ||
      fail "an unlisted file over the budget was not named: $(cat "$work/answer")"
    [[ "$(run update)" == 1 ]] || fail "--update passed an unlisted file over the budget"
    [[ -z "$(entry "$place")" ]] || fail "--update added a file to the list"
    git -C "$tree" rm -qf --cached -- "$place"
    rm "$tree/$place"
  done

  # A listed file that grew, which --update never raises.
  lines "$tree/app/demo/src/big.rs" 901
  [[ "$(run check)" == 1 ]] || fail "a listed file that grew passed"
  grep -q 'app/demo/src/big.rs: 901 lines, grown past the 900' "$work/answer" ||
    fail "a listed file that grew was not named: $(cat "$work/answer")"
  [[ "$(run update)" == 1 ]] || fail "--update passed a listed file that grew"
  [[ "$(entry app/demo/src/big.rs)" == 900 ]] || fail "--update raised an entry to $(entry app/demo/src/big.rs)"

  # A listed file that shrank fails until --update lowers its entry.
  lines "$tree/app/demo/src/big.rs" 870
  [[ "$(run check)" == 1 ]] || fail "a listed file that shrank passed with its old entry"
  grep -q 'app/demo/src/big.rs: 870 lines, and .* records 900; run .* --update' "$work/answer" ||
    fail "a shrink did not ask for --update: $(cat "$work/answer")"
  [[ "$(run update)" == 0 ]] || fail "--update refused a shrink: $(cat "$work/answer")"
  [[ "$(entry app/demo/src/big.rs)" == 870 ]] || fail "--update did not lower the entry"

  # A listed file within the budget, and a listed file gone, fail until
  # --update removes them.
  lines "$tree/app/demo/src/big.rs" "$BUDGET"
  git -C "$tree" rm -qf -- app/demo/src/views/wide.rs
  [[ "$(run check)" == 1 ]] || fail "a listed file within the budget and a gone one passed"
  grep -q 'app/demo/src/big.rs: 800 lines, within the budget' "$work/answer" ||
    fail "a listed file within the budget was not named: $(cat "$work/answer")"
  grep -q 'app/demo/src/views/wide.rs: .* no longer tracks it' "$work/answer" ||
    fail "a listed file that is gone was not named: $(cat "$work/answer")"
  [[ "$(run update)" == 0 ]] || fail "--update refused to remove two entries: $(cat "$work/answer")"
  [[ "$(entries_of "$list")" == "crates/demo/src/deep.rs 820" ]] ||
    fail "--update left the wrong entries: $(entries_of "$list")"
  git -C "$tree" rm -qf -- crates/demo/src/deep.rs
  [[ "$(run update)" == 0 ]] || fail "--update refused to remove the last entry: $(cat "$work/answer")"
  [[ -z "$(entries_of "$list")" ]] || fail "--update left an entry: $(entries_of "$list")"
  [[ "$(run check)" == 0 ]] || fail "the emptied list was refused: $(cat "$work/answer")"

  # A malformed list is a usage failure, never a pass.
  printf 'app/demo/src/big.rs nine hundred\n' >>"$list"
  [[ "$(run check)" == 2 ]] || fail "a malformed entry did not stop the run"
  write_list "$list" "app/demo/src/big.rs 900
app/demo/src/big.rs 901"
  [[ "$(run check)" == 2 ]] || fail "a path listed twice did not stop the run"
  write_list "$list" "fuzz/fuzz_targets/long.rs 1200"
  [[ "$(run check)" == 2 ]] || fail "an entry outside the three roots did not stop the run"

  # The per-edit mode refuses growth and passes a shrink.
  write_list "$list" "app/demo/src/big.rs 900"
  lines "$tree/app/demo/src/big.rs" 901
  per_edit "$list" "$tree/app/demo/src/big.rs" >/dev/null 2>&1 && fail "--files passed a listed file that grew"
  lines "$tree/app/demo/src/big.rs" 850
  per_edit "$list" "$tree/app/demo/src/big.rs" >/dev/null 2>&1 || fail "--files refused a listed file that shrank"
  per_edit "$list" "$tree/tools/demo/src/draft.rs" >/dev/null 2>&1 && fail "--files passed an unlisted file over the budget"
  per_edit "$list" "$tree/app/demo/tests/it/long.rs" "$tree/fuzz/fuzz_targets/long.rs" \
    "$tree/crates/demo/src/generated.rs" >/dev/null 2>&1 || fail "--files refused a file outside its scope"

  # The hook gives the verdict of the project's guard, measured against the
  # project's list.
  local project="$work/project"
  mkdir -p "$project/scripts/checks" "$project/app/demo/src"
  git -c init.defaultBranch=main init -q "$project"
  cp "$self" "$project/scripts/checks/file-size.sh"
  write_list "$project/$LIST_FILE" "app/demo/src/listed.rs 900"
  lines "$project/app/demo/src/listed.rs" 900
  lines "$project/app/demo/src/over.rs" 801
  lines "$project/app/demo/src/fits.rs" 10
  local hook="$PWD/.claude/hooks/rust_fmt_clippy.sh" pair name want got
  for pair in listed:0 over:2 fits:0; do
    name="${pair%%:*}"
    want="${pair#*:}"
    # From the miniature, where no rust-toolchain.toml sends rustfmt to
    # install a toolchain.
    got=0
    (cd "$work" && printf '{"tool_input":{"file_path":"%s"}}' "$project/app/demo/src/$name.rs" |
      CLAUDE_PROJECT_DIR="$project" "$hook" >/dev/null 2>&1) || got=$?
    [[ "$got" == "$want" ]] || fail "the hook answered $got for the $name file, not $want"
  done

  echo "file-size: self-test OK (an unlisted file in each root, a growth, a shrink, an emptied list, a malformed list, the per-edit mode and the hook)."
}

case "${1:-}" in
  "" | --all) ratchet "$PWD" "$LIST_FILE" check ;;
  --update) ratchet "$PWD" "$LIST_FILE" update ;;
  --files)
    shift
    per_edit "$LIST_FILE" "$@"
    ;;
  --self-test) self_test ;;
  *)
    echo "usage: $0 [--all | --files <f.rs>... | --update | --self-test]" >&2
    exit 2
    ;;
esac
