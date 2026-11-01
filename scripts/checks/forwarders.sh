#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Forwarder ratchet: the functions whose whole body forwards to one other
# call may fall in number and never rise (#4663).
#
# tools/forwarders parses every .rs file under app/*/src/, crates/*/src/ and
# tools/*/src/ with syn and lists each free function and inherent method
# whose body is one call, to a function or to a method of `self`, wrapped at
# most in `.await`, `?`, `Ok(..)`, `Some(..)`, `.into()` or an error-only
# `.map_err`, with arguments a caller could pass itself. It leaves out what
# .claude/rules/rust-style.md, "No function only forwards to another",
# exempts: trait implementations and `main`, a function whose doc comment
# cites the openEHR specification page that defines it, a constructor that
# fixes an argument, the `#[cfg]` arms of one function, a file that opens
# with `// @generated`, anything under `#[cfg(test)]`, and tests/.
#
# Two files beside this one hold the ratchet. forwarders.count is the total
# the tree may carry, and forwarders-baseline.txt holds the key of every
# forwarder counted in it: its path, its owner (inline modules, impl type and
# name) and the call it makes, with no line number, so code moving inside a
# file keeps its key. A baseline line may carry a tab and the reason a
# forwarder stays (a crate-root re-export, say); --update keeps the reason
# while the key stands.
#
# A rise fails and names each new forwarder with its target and place. A fall
# passes with a one-line note, so a pull request that removes forwarders
# touches neither file; the scheduled ratchet-tighten workflow runs --update
# on main and opens the pull request that lowers the count. --update never
# raises the count: a forwarder that has to stay is a hand edit of both files
# in a change that says why. --files checks only the forwarders of the named
# files against the baseline, which is what the per-edit hook runs.
#
# Usage:
#   scripts/checks/forwarders.sh                  # measure the tree
#   scripts/checks/forwarders.sh --update         # record a count that held or fell, and the keys
#   scripts/checks/forwarders.sh --files F...     # the named files carry no forwarder the baseline lacks
#   scripts/checks/forwarders.sh --self-test      # the exceptions, a rise, a fall, --update and --files
#
# Exit 0 = the count held or fell, 1 = it rose or a named file carries a new
# forwarder, 2 = usage or the tool failed.
#
# No specification governs this: our own design.
set -Eeuo pipefail

# shellcheck source=scripts/lib/guard-args.sh
. "$(dirname "$0")/../lib/guard-args.sh"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

readonly GRAMMAR="[--update | --files FILE... | --self-test]"

readonly COUNT_FILE=scripts/checks/forwarders.count
readonly BASELINE_FILE=scripts/checks/forwarders-baseline.txt
# One byte order for every sort and every comparison of a key, on any machine.
export LC_ALL=C

# Lists the forwarders of the tree $1, or of the files after it, one per
# line: the key, a tab, and the place as path:line.
detect() {
  local tree="$1"
  shift
  cargo run --manifest-path "$root/Cargo.toml" --locked --quiet --release -p forwarders -- \
    --root "$tree" "$@"
}

# The keys the baseline file $1 carries, one per line, without its comments
# and reasons.
carried_keys() {
  [[ -f "$1" ]] || return 0
  awk -F'\t' '/^[[:space:]]*(#|$)/ { next } { print $1 }' "$1"
}

# Each forwarder of the listing $1 as it reads, with NEW in front of one whose
# key the baseline file $2 does not carry. A key that stands twice in the
# listing needs two lines in the baseline.
marked() {
  [[ -n "$1" ]] || return 0
  awk -F'\t' '
    FILENAME == ARGV[1] { carried[$0]++; next }
    { if (carried[$1] > 0) { carried[$1]--; print $1 " (" $2 ")" } else { print "NEW " $1 " (" $2 ")" } }
  ' <(carried_keys "$2") - <<<"$1"
}

# The committed count in the file $1, or nothing when it holds none.
committed_count() {
  [[ -f "$1" ]] || return 0
  awk '/^[[:space:]]*(#|$)/ { next } { print; exit }' "$1" | tr -d '[:space:]'
}

write_count() {
  {
    # REUSE-IgnoreStart
    printf '# SPDX-FileCopyrightText: Cadasto B.V.\n'
    printf '# SPDX-License-Identifier: BUSL-1.1\n'
    # REUSE-IgnoreEnd
    printf '# The forwarders scripts/checks/forwarders.sh counts in app/*/src/,\n'
    printf '# crates/*/src/ and tools/*/src/. It may fall and never rise; --update\n'
    printf '# writes a lower one, and a higher one is a hand edit that says why.\n'
    printf '%s\n' "$2"
  } >"$1"
}

# Writes the keys of the listing $2 to the baseline $1, each with the reason
# the old baseline gave it, if any.
write_baseline() {
  local reasons
  reasons="$(awk -F'\t' '/^[[:space:]]*(#|$)/ { next } NF > 1 { print }' "$1" 2>/dev/null || true)"
  {
    # REUSE-IgnoreStart
    printf '# SPDX-FileCopyrightText: Cadasto B.V.\n'
    printf '# SPDX-License-Identifier: BUSL-1.1\n'
    # REUSE-IgnoreEnd
    printf '# The key of every forwarder scripts/checks/forwarders.sh counts, written\n'
    printf '# by --update: the path, the owner and the call it makes. A tab and a\n'
    printf '# reason after a key record why that forwarder stays.\n'
    if [[ -n "$2" ]]; then
      cut -f1 <<<"$2" | sort | awk -F'\t' '
        FILENAME == ARGV[1] { reason[$1] = $2; next }
        { if ($0 in reason) { print $0 "\t" reason[$0] } else { print } }
      ' <(printf '%s\n' "$reasons") -
    fi
  } >"$1.new"
  mv "$1.new" "$1"
}

# Prints each line of $1 two spaces in, without its NEW mark.
indented() {
  local line
  while IFS= read -r line; do
    printf '  %s\n' "${line#NEW }"
  done <<<"$1"
}

# Measures the tree $1 against the count file $2 and the baseline $3. With
# $4 = update it records a count that held or fell, and the baseline with it.
ratchet() {
  local tree="$1" count_file="$2" baseline="$3" mode="$4"
  local listing total count new status=0
  echo "forwarders: scanning app/*/src, crates/*/src and tools/*/src."
  listing="$(detect "$tree")" || status=$?
  if [[ "$status" -ne 0 ]]; then
    echo "forwarders: the tool stopped with status $status" >&2
    return 2
  fi
  total="$(grep -c '' <<<"$listing" || true)"
  [[ -n "$listing" ]] || total=0
  count="$(committed_count "$count_file")"

  if [[ "$mode" == update ]]; then
    if [[ -n "$count" && "$total" -gt "$count" ]]; then
      echo "forwarders: the tree carries $total forwarders and $count_file allows $count;"
      echo "--update never raises it: call the targets directly, or raise the file by hand and say why."
      return 1
    fi
    write_count "$count_file" "$total"
    write_baseline "$baseline" "$listing"
    echo "forwarders: recorded $total forwarders in $count_file and their keys in $baseline."
    return 0
  fi

  if [[ ! "$count" =~ ^[0-9]+$ ]]; then
    echo "forwarders: $count_file holds no count; run $0 --update and commit it."
    return 1
  fi
  new="$(marked "$listing" "$baseline" | grep '^NEW ' || true)"
  if [[ "$total" -gt "$count" ]]; then
    echo "forwarders: the forwarders rose from $count to $total. The ones the baseline does not carry:"
    if [[ -n "$new" ]]; then
      indented "$new"
    else
      echo "  none; a forwarder the baseline carries now stands twice."
    fi
    echo "Call the target directly (.claude/rules/rust-style.md, \"No function only forwards to another\")."
    return 1
  fi
  if [[ "$total" -lt "$count" ]]; then
    echo "forwarders: OK, the forwarders fell from $count to $total; the scheduled ratchet-tighten workflow records the lower count."
    return 0
  fi
  if [[ -n "$new" ]]; then
    echo "forwarders: OK ($total forwarders, as $count_file records), and $(grep -c '' <<<"$new") of them"
    echo "moved since $baseline was written; $0 --update records their keys."
    return 0
  fi
  echo "forwarders: OK ($total forwarders, as $count_file records, every one in $baseline)."
}

# Checks the forwarders of the files after $1 and $2 against the baseline $2,
# in the tree $1.
files_check() {
  local tree="$1" baseline="$2"
  shift 2
  local listing new status=0
  local file here
  local -a inside=()
  here="$(cd "$tree" && pwd -P)"
  # A file of another checkout, a worktree say, is that checkout's to measure.
  for file in "$@"; do
    [[ -f "$file" ]] || continue
    [[ "$(cd "$(dirname "$file")" && pwd -P)/" == "$here"/* ]] && inside+=("$file")
  done
  [[ "${#inside[@]}" -gt 0 ]] || return 0
  set -- "${inside[@]}"
  listing="$(detect "$tree" "$@")" || status=$?
  if [[ "$status" -ne 0 ]]; then
    echo "forwarders: the tool stopped with status $status" >&2
    return 2
  fi
  new="$(marked "$listing" "$baseline" | grep '^NEW ' || true)"
  [[ -z "$new" ]] && return 0
  echo "forwarders: a function whose whole body forwards to one other call:"
  indented "$new"
  echo "Call the target directly (.claude/rules/rust-style.md, \"No function only forwards to another\")."
  return 1
}

self_test() {
  local work tree count_file baseline answer
  mkdir -p "$root/target"
  work="$(mktemp -d "$root/target/forwarders.XXXXXX")"
  # shellcheck disable=SC2064 # the path is fixed now, on purpose
  trap "rm -rf '$work'" EXIT
  tree="$work/tree"
  count_file="$work/forwarders.count"
  baseline="$work/baseline.txt"
  mkdir -p "$tree/app/demo/src" "$tree/app/demo/tests" "$tree/crates/demo/src" "$tree/tools/demo/src"

  fail() {
    echo "forwarders: SELF-TEST FAILED: $1" >&2
    exit 1
  }
  run() {
    local status=0
    ratchet "$tree" "$count_file" "$baseline" "$1" >"$work/answer" 2>&1 || status=$?
    printf '%s\n' "$status"
  }

  # The one forwarder the scan counts, beside every exception the rule names.
  cat >"$tree/app/demo/src/lib.rs" <<'EOF'
pub fn load(id: u32) -> u32 {
    store::load(id)
}
impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
impl Pool {
    pub fn new() -> Self {
        Self::with_capacity(16)
    }
    pub fn bounded(size: usize) -> Self {
        Self::with_limit(size, MAX)
    }
    /// `second_validity` (`org.openehr.am.aom2.c_time.adoc` §Functions).
    pub fn second_validity(&self) -> u32 {
        self.slot(2)
    }
    fn len(&self) -> usize {
        self.items.len()
    }
}
#[cfg(feature = "store")]
fn ends(rows: &[u32]) -> Vec<u32> {
    rows.iter().map(|row| row + 1).collect()
}
#[cfg(not(feature = "store"))]
fn ends(rows: &[u32]) -> Vec<u32> {
    empty(rows)
}
#[cfg(test)]
mod helpers;
#[cfg(test)]
mod tests {
    fn inner(x: u32) -> u32 {
        other(x)
    }
}
EOF
  printf 'pub fn helper(x: u32) -> u32 {\n    other(x)\n}\n' >"$tree/app/demo/src/helpers.rs"
  printf '// @generated by a miniature. DO NOT EDIT.\npub fn made(x: u32) -> u32 {\n    other(x)\n}\n' \
    >"$tree/crates/demo/src/lib.rs"
  printf 'fn it(x: u32) -> u32 {\n    other(x)\n}\n' >"$tree/app/demo/tests/it.rs"
  printf 'fn main() {\n    demo::run()\n}\n' >"$tree/tools/demo/src/main.rs"

  [[ "$(run check)" == 1 ]] || fail "a tree with no committed count passed: $(cat "$work/answer")"
  [[ "$(run update)" == 0 ]] || fail "--update refused the first count: $(cat "$work/answer")"
  [[ "$(committed_count "$count_file")" == 1 ]] ||
    fail "the exceptions were counted: $(committed_count "$count_file") forwarders, not 1: $(cat "$baseline")"
  grep -Fxq 'app/demo/src/lib.rs: load -> store::load(id)' "$baseline" ||
    fail "the one forwarder was not keyed by path, owner and target: $(cat "$baseline")"
  [[ "$(run check)" == 0 ]] || fail "an unchanged count was refused: $(cat "$work/answer")"
  grep -q 'every one in' "$work/answer" || fail "the one forwarder was not found in the baseline: $(cat "$work/answer")"

  # A move inside the file keeps the key.
  {
    printf 'pub const LIMIT: u32 = 3;\n\n'
    cat "$tree/app/demo/src/lib.rs"
  } >"$work/lib.rs"
  mv "$work/lib.rs" "$tree/app/demo/src/lib.rs"
  [[ "$(run check)" == 0 ]] || fail "a line moved above the forwarder was refused: $(cat "$work/answer")"
  grep -q 'every one in' "$work/answer" || fail "a forwarder that moved down got a new key: $(cat "$work/answer")"

  # A rise fails and names the new forwarder with its target and place.
  printf 'impl Store {\n    pub async fn get(&self, id: u32) -> Result<u32, E> {\n        Ok(self.read(id).await?)\n    }\n}\n' \
    >"$tree/app/demo/src/store.rs"
  [[ "$(run check)" == 1 ]] || fail "a rise passed"
  grep -Fq 'app/demo/src/store.rs: Store::get -> self.read(id) (app/demo/src/store.rs:2)' "$work/answer" ||
    fail "a rise did not name the new forwarder, its target and its place: $(cat "$work/answer")"
  [[ "$(run update)" == 1 ]] || fail "--update raised the count"
  [[ "$(committed_count "$count_file")" == 1 ]] || fail "--update wrote a higher count"

  # --files: a named file with a new forwarder fails, one the baseline carries passes.
  answer=0
  files_check "$tree" "$baseline" "$tree/app/demo/src/store.rs" >"$work/answer" 2>&1 || answer=$?
  [[ "$answer" == 1 ]] || fail "--files passed a file with a new forwarder: $(cat "$work/answer")"
  grep -Fq 'Store::get -> self.read(id)' "$work/answer" || fail "--files did not name it: $(cat "$work/answer")"
  answer=0
  files_check "$tree" "$baseline" "$tree/app/demo/src/lib.rs" >"$work/answer" 2>&1 || answer=$?
  [[ "$answer" == 0 ]] || fail "--files refused a file the baseline carries: $(cat "$work/answer")"
  cp "$tree/app/demo/src/store.rs" "$work/elsewhere.rs"
  answer=0
  files_check "$tree" "$baseline" "$work/elsewhere.rs" >"$work/answer" 2>&1 || answer=$?
  [[ "$answer" == 0 ]] || fail "--files measured a file outside the tree: $(cat "$work/answer")"

  # A reason on a baseline line survives --update while its key stands.
  rm "$tree/app/demo/src/store.rs"
  awk -F'\t' '/^app\// { print $0 "\tthe crate root names it" ; next } { print }' "$baseline" >"$work/reasoned"
  mv "$work/reasoned" "$baseline"
  [[ "$(run update)" == 0 ]] || fail "--update refused an unchanged count: $(cat "$work/answer")"
  grep -Fxq "$(printf 'app/demo/src/lib.rs: load -> store::load(id)\tthe crate root names it')" "$baseline" ||
    fail "--update dropped the reason of a key that stands: $(cat "$baseline")"
  [[ "$(run check)" == 0 ]] || fail "a baseline with a reason was refused: $(cat "$work/answer")"

  # A fall passes with a note, and --update lowers the count, which then holds.
  rm "$tree/app/demo/src/helpers.rs"
  printf 'pub fn load(id: u32) -> u32 {\n    store::load(id) + 1\n}\n' >"$tree/app/demo/src/lib.rs"
  [[ "$(run check)" == 0 ]] || fail "a fall was refused: $(cat "$work/answer")"
  grep -q 'fell from 1 to 0' "$work/answer" || fail "a fall was not reported as one: $(cat "$work/answer")"
  [[ "$(committed_count "$count_file")" == 1 ]] || fail "a check wrote the count file"
  [[ "$(run update)" == 0 ]] || fail "--update refused a fall: $(cat "$work/answer")"
  [[ "$(committed_count "$count_file")" == 0 ]] || fail "--update did not record the lower count"
  [[ "$(run check)" == 0 ]] || fail "the lowered count was refused: $(cat "$work/answer")"

  echo "forwarders: self-test OK (the exceptions, a move, a rise, --files, a kept reason, a fall)."
}

case "${1:-}" in
  "") ratchet "$root" "$COUNT_FILE" "$BASELINE_FILE" check ;;
  --update)
    guard_no_args "${@:2}"
    ratchet "$root" "$COUNT_FILE" "$BASELINE_FILE" update
    ;;
  --files)
    shift
    [[ "$#" -gt 0 ]] || guard_usage "$GRAMMAR"
    guard_known_flags "$GRAMMAR" "" "$@"
    files_check "$root" "$BASELINE_FILE" "$@"
    ;;
  --self-test)
    guard_no_args "${@:2}"
    self_test
    ;;
  *) guard_usage "$GRAMMAR" ;;
esac
