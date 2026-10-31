#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Duplication ratchet: the duplicated lines of the hand-written Rust may fall
# and never rise (#4454).
#
# jscpd (docs/VERSIONS.md, CI tools) tokenizes every .rs file under
# app/*/src/, crates/*/src/ and tools/*/src/ and reports each block of at
# least MIN_TOKENS tokens over at least MIN_LINES lines that stands in two
# places. The threshold is our own design, measured and recorded in
# .claude/rules/rust-style.md, "Code is written once". The scan leaves out:
#   - a file that opens with the `// @generated` banner: its source is the
#     emitter, and .claude/rules/codegen.md forbids editing it by hand;
#   - the `use` declarations: two files that import the same names carry no
#     code twice, and an import list shrinks by itself when code moves;
#   - a file from its first `#[cfg(test)]` line in column one to its end,
#     the inline test module, and tests/, which no root reaches: a test
#     spells out its own input so a failure reads in one place.
#
# Two files beside this one hold the ratchet. duplication.count is the total
# of duplicated lines the tree may carry, and duplication-baseline.txt holds
# the key of every block counted in it, which is how a rise names the blocks
# that are new. The key is our own and never jscpd's baseline, whose
# fingerprint of a block moves when other files change: it is both places of
# the block, each as its path and the sha256 of the text of its lines read
# from the file, the two sorted, with no line number in it. A rise fails and
# names the new blocks by both places; a fall fails too, until the count is
# lowered with --update, so the ratchet only tightens. --update never raises
# the count: a rise that has to stay is a hand edit of duplication.count in a
# change that says why.
#
# The pins are read from docs/VERSIONS.md, the one place they live. The
# detector is a jscpd on PATH that answers the pinned version, else the
# pinned image through docker, else through podman, else the pinned release
# binary --install puts under target/tools/, in this checkout or in the main
# checkout of a worktree, so one download serves every worktree. A machine
# with none of them fails with exit 2 and says how to get one; the guard never
# skips. No per-edit hook runs it: a move copies code before it deletes the
# original, so a ratchet per edit would fire in the middle of every
# restructure. CI runs it per push.
#
# Usage:
#   scripts/checks/duplication.sh              # measure the tree
#   scripts/checks/duplication.sh --update     # record a count that held or fell, and the keys
#   scripts/checks/duplication.sh --install    # fetch the pinned release binary into target/tools/
#   scripts/checks/duplication.sh --self-test  # the detector order, a rise, a fall, an unchanged count, a move
#
# Exit 0 = the count holds, 1 = it rose or fell, 2 = usage, a missing pin or
# no detector.
#
# No specification governs this: our own design. The flags are jscpd's own
# (https://github.com/kucherenko/jscpd/blob/v5.4.2/docs/rust.md).
set -Eeuo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

readonly VERSIONS_FILE=docs/VERSIONS.md
readonly MIN_TOKENS=50
readonly MIN_LINES=5
readonly COUNT_FILE=scripts/checks/duplication.count
readonly BASELINE_FILE=scripts/checks/duplication-baseline.txt
# One byte order for every sort and every comparison of a key, on any machine.
export LC_ALL=C

# The pin of the row whose first cell is `$1` in the CI tools table of
# docs/VERSIONS.md: the first backticked span of its second cell.
pin_of() {
  awk -F'|' -v key="\`$1\`" '
    NF >= 4 {
      name = $2
      gsub(/^[ \t]+|[ \t]+$/, "", name)
      if (name == key && match($3, /`[^`]+`/)) { print substr($3, RSTART + 1, RLENGTH - 2); exit }
    }
  ' "$VERSIONS_FILE"
}

JSCPD_VERSION="$(pin_of jscpd)"
JSCPD_IMAGE="$(pin_of 'jscpd image')"
if [[ ! "$JSCPD_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ||
  ! "$JSCPD_IMAGE" =~ ^[a-z0-9./-]+:$JSCPD_VERSION@sha256:[0-9a-f]{64}$ ]]; then
  echo "duplication: $VERSIONS_FILE pins no jscpd version and image of that version by digest" >&2
  echo "(read '$JSCPD_VERSION' and '$JSCPD_IMAGE')" >&2
  exit 2
fi
readonly JSCPD_VERSION JSCPD_IMAGE

# jscpd splits --ignore-pattern on commas, so no pattern carries one. The
# three are the generated file, the use declaration and the test module.
readonly IGNORED='(?s)\A// @generated.*\z,(?m)^[ \t]*(pub(\([^)]*\))? )?use [^;]*;,(?ms)^#\[cfg\(test\)\]$.*\z'

# The places --install puts the pinned jscpd: this checkout's target/tools/,
# then the main checkout's when this is a worktree. Only builtins and the
# candidates themselves run in detector_kind, so the self-test can hand it a
# PATH and a list of its own.
LOCAL_JSCPD=("$root/target/tools/jscpd-$JSCPD_VERSION/jscpd")
if common="$(git -C "$root" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" &&
  [[ "$(dirname "$common")" != "$root" ]]; then
  LOCAL_JSCPD+=("$(dirname "$common")/target/tools/jscpd-$JSCPD_VERSION/jscpd")
fi

# The first of LOCAL_JSCPD that runs and answers the pinned version.
local_jscpd() {
  local candidate answer
  # The `+` form keeps an empty list legal under set -u in bash 3.2 of macOS.
  for candidate in ${LOCAL_JSCPD[@]+"${LOCAL_JSCPD[@]}"}; do
    if [[ -x "$candidate" ]] && answer="$("$candidate" --version 2>/dev/null)" &&
      [[ "$answer" == "jscpd $JSCPD_VERSION" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

# How this machine runs the detector, in this order: "path", "docker",
# "podman", "local", or "" for none of them.
detector_kind() {
  local answer
  if command -v jscpd >/dev/null 2>&1 && answer="$(jscpd --version 2>/dev/null)" &&
    [[ "$answer" == "jscpd $JSCPD_VERSION" ]]; then
    printf 'path\n'
  elif command -v docker >/dev/null 2>&1 && docker version >/dev/null 2>&1; then
    printf 'docker\n'
  elif command -v podman >/dev/null 2>&1 && podman version >/dev/null 2>&1; then
    printf 'podman\n'
  elif local_jscpd >/dev/null; then
    printf 'local\n'
  else
    printf '\n'
  fi
}

# Runs the detector over the tree $1 and leaves its report in $2/report.json.
# Prints the prefix the report's absolute paths carry.
detect() {
  local tree="$1" out="$2" kind
  local -a roots=() flags=()
  local candidate
  for candidate in "$tree"/app/*/src "$tree"/crates/*/src "$tree"/tools/*/src; do
    [[ -d "$candidate" ]] && roots+=("${candidate#"$tree"/}")
  done
  if [[ "${#roots[@]}" -eq 0 ]]; then
    echo "duplication: no app/*/src, crates/*/src or tools/*/src under $tree" >&2
    return 2
  fi
  kind="$(detector_kind)"
  local seen="$out" binary=jscpd
  case "$kind" in
    path) ;;
    docker | podman) seen=/out ;;
    local) binary="$(local_jscpd)" ;;
    *)
      echo "duplication: no detector: put jscpd $JSCPD_VERSION on PATH, start docker or podman for" >&2
      echo "$JSCPD_IMAGE, or install the pinned release with $0 --install" >&2
      return 2
      ;;
  esac
  flags=(--output "$seen" --format rust --min-tokens "$MIN_TOKENS" --min-lines "$MIN_LINES"
    --ignore-pattern "$IGNORED" --reporters json --absolute --no-tips --fail-on-empty)
  local status=0 copy root
  case "$kind" in
    docker | podman)
      # The engine reads a fresh copy of the roots: the file share of a podman
      # machine on macOS serves a replaced file's old content for seconds.
      copy="$(mktemp -d "$stage_root/tree.XXXXXX")"
      for root in "${roots[@]}"; do
        mkdir -p "$copy/$(dirname "$root")"
        cp -R "$tree/$root" "$copy/$root"
      done
      "$kind" run --rm --network none --user "$(id -u):$(id -g)" --env HOME=/tmp \
        -v "$copy:/src:ro" -v "$out:/out" "$JSCPD_IMAGE" \
        "${flags[@]}" "${roots[@]}" >"$out/detector.log" 2>&1 || status=$?
      rm -rf "$copy"
      ;;
    *)
      (cd "$tree" && "$binary" "${flags[@]}" "${roots[@]}") >"$out/detector.log" 2>&1 || status=$?
      ;;
  esac
  if [[ "$status" -ne 0 || ! -f "$out/jscpd-report.json" ]]; then
    echo "duplication: jscpd ($kind) stopped with status $status:" >&2
    cat "$out/detector.log" >&2
    return 2
  fi
  mv "$out/jscpd-report.json" "$out/report.json"
  case "$kind" in
    docker | podman) printf '/src/\n' ;;
    *) printf '%s/\n' "$(cd "$tree" && pwd -P)" ;;
  esac
}

# The sha256 of standard input in hex, from whichever tool this machine has.
sha256_hex() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum | awk '{ print $1 }'
  else
    shasum -a 256 | awk '{ print $1 }'
  fi
}

# One line per block of the report $1, its paths without the prefix $2 and
# its lines read from the tree $3, sorted by place: how the block reads, a
# tab, and its key.
keyed_blocks() {
  local report="$1" prefix="$2" tree="$3" rows
  local first first_start first_end second second_start second_end lines one other place
  rows="$(jq -r --arg prefix "$prefix" '
    .duplicates[]
    | [(.firstFile | .name | ltrimstr($prefix)), .firstFile.start, .firstFile.end,
       (.secondFile | .name | ltrimstr($prefix)), .secondFile.start, .secondFile.end, .lines]
    | @tsv
  ' "$report")" || {
    echo "duplication: the jscpd report $report could not be read" >&2
    return 2
  }
  [[ -n "$rows" ]] || return 0
  while IFS=$'\t' read -r first first_start first_end second second_start second_end lines; do
    for place in "$first:$first_start:$first_end" "$second:$second_start:$second_end"; do
      if [[ ! "$place" =~ ^[^:/][^:]*:[1-9][0-9]*:[1-9][0-9]*$ || ! -f "$tree/${place%%:*}" ]]; then
        echo "duplication: the jscpd report names a block at '$place', which is no file of $tree" >&2
        return 2
      fi
    done
    one="$first@$(sed -n "${first_start},${first_end}p" "$tree/$first" | sha256_hex)"
    other="$second@$(sed -n "${second_start},${second_end}p" "$tree/$second" | sha256_hex)"
    if [[ "$one" > "$other" ]]; then
      printf '%s:%s-%s = %s:%s-%s (%s lines)\t%s %s\n' \
        "$first" "$first_start" "$first_end" "$second" "$second_start" "$second_end" "$lines" "$other" "$one"
    else
      printf '%s:%s-%s = %s:%s-%s (%s lines)\t%s %s\n' \
        "$first" "$first_start" "$first_end" "$second" "$second_start" "$second_end" "$lines" "$one" "$other"
    fi
  done <<<"$rows" | sort -t: -k1,1 -k2,2n
}

# The keys the baseline file $1 carries, one per line, without its comments.
carried_keys() {
  [[ -f "$1" ]] || return 0
  awk '/^[[:space:]]*(#|$)/ { next } { print }' "$1"
}

# Each block of the keyed listing $1 as it reads, with NEW in front of one
# whose key the baseline file $2 does not carry. A key that stands twice in
# the listing needs two lines in the baseline.
blocks() {
  [[ -n "$1" ]] || return 0
  awk -F'\t' '
    FILENAME == ARGV[1] { carried[$0]++; next }
    { if (carried[$2] > 0) { carried[$2]--; print $1 } else { print "NEW " $1 } }
  ' <(carried_keys "$2") - <<<"$1"
}

write_baseline() {
  {
    # REUSE-IgnoreStart
    printf '# SPDX-FileCopyrightText: Cadasto B.V.\n'
    printf '# SPDX-License-Identifier: BUSL-1.1\n'
    # REUSE-IgnoreEnd
    printf '# The key of every block scripts/checks/duplication.sh counts, written\n'
    printf '# by --update: both places of the block, each as its path and the\n'
    printf '# sha256 of the text of its lines, the two sorted.\n'
    if [[ -n "$2" ]]; then
      cut -f2 <<<"$2" | sort
    fi
  } >"$1"
}

# Prints each line of $1 two spaces in, without its NEW mark.
indented() {
  local line
  while IFS= read -r line; do
    printf '  %s\n' "${line#NEW }"
  done <<<"$1"
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
    printf '# The duplicated lines scripts/checks/duplication.sh measures in\n'
    printf '# app/*/src/, crates/*/src/ and tools/*/src/. It may fall and never\n'
    printf '# rise; --update writes a lower one, and a higher one is a hand edit\n'
    printf '# that says why.\n'
    printf '%s\n' "$2"
  } >"$1"
}

# Measures the tree $1 against the count file $2 and the baseline $3. With
# $4 = update it records a count that held or fell, and the baseline with it.
ratchet() {
  local tree="$1" count_file="$2" baseline="$3" mode="$4"
  local out prefix total count keyed listing new status=0
  out="$(mktemp -d "$stage_root/scan.XXXXXX")"
  prefix="$(detect "$tree" "$out")" || status=$?
  if [[ "$status" -ne 0 ]]; then
    rm -rf "$out"
    return "$status"
  fi
  total="$(jq -r '.statistics.total.duplicatedLines' "$out/report.json")"
  if [[ ! "$total" =~ ^[0-9]+$ ]]; then
    echo "duplication: the jscpd report carries no total of duplicated lines ('$total')" >&2
    rm -rf "$out"
    return 2
  fi
  keyed="$(keyed_blocks "$out/report.json" "$prefix" "$tree")" || status=$?
  rm -rf "$out"
  if [[ "$status" -ne 0 ]]; then
    return "$status"
  fi
  listing="$(blocks "$keyed" "$baseline")"
  count="$(committed_count "$count_file")"

  if [[ "$mode" == update ]]; then
    if [[ -n "$count" && "$total" -gt "$count" ]]; then
      echo "duplication: the tree carries $total duplicated lines and $count_file allows $count;"
      echo "--update never raises it: move the new blocks to one owner, or raise the file by hand and say why."
      return 1
    fi
    write_count "$count_file" "$total"
    write_baseline "$baseline" "$keyed"
    echo "duplication: recorded $total duplicated lines in $count_file and the baseline in $baseline."
    return 0
  fi

  if [[ -z "$listing" ]]; then
    echo "duplication: no block of $MIN_TOKENS tokens over $MIN_LINES lines stands in two places."
  else
    printf 'duplication: blocks of %s tokens over %s lines or more in two places: %s\n' \
      "$MIN_TOKENS" "$MIN_LINES" "$(grep -c '' <<<"$listing")"
    indented "$listing"
  fi
  new="$(grep '^NEW ' <<<"$listing" || true)"

  if [[ ! "$count" =~ ^[0-9]+$ ]]; then
    echo "duplication: $count_file holds no count; run $0 --update and commit it."
    return 1
  fi
  if [[ "$total" -gt "$count" ]]; then
    echo "duplication: the duplicated lines rose from $count to $total. The blocks the baseline does not carry:"
    if [[ -n "$new" ]]; then
      indented "$new"
    else
      echo "  none; a block the baseline carries grew or repeats in a third place."
    fi
    echo "Move the code to one owner in the module that owns the concern (.claude/rules/rust-style.md)."
    return 1
  fi
  if [[ "$total" -lt "$count" ]]; then
    echo "duplication: the duplicated lines fell from $count to $total. Run $0 --update and commit"
    echo "$count_file and $baseline, so the ratchet holds the lower count."
    return 1
  fi
  if [[ -n "$new" ]]; then
    echo "duplication: OK ($total duplicated lines, as $count_file records), and $(grep -c '' <<<"$new") blocks"
    echo "of it moved since $baseline was written; $0 --update records their keys."
    return 0
  fi
  echo "duplication: OK ($total duplicated lines, as $count_file records, every block in $baseline)."
}

# Fetches the pinned release binary of this host into target/tools/ and checks
# it against the sha256 docs/VERSIONS.md pins for its archive.
install_local() {
  local platform asset sum dest stage said
  case "$(uname -s)/$(uname -m)" in
    Darwin/arm64) platform=darwin-arm64 asset=jscpd-darwin-arm64.tar.gz ;;
    Darwin/x86_64) platform=darwin-x64 asset=jscpd-darwin-x64.tar.gz ;;
    Linux/x86_64) platform=linux-x64 asset=jscpd-linux-x64-musl.tar.gz ;;
    Linux/aarch64 | Linux/arm64) platform=linux-arm64 asset=jscpd-linux-arm64-musl.tar.gz ;;
    *)
      echo "duplication: jscpd $JSCPD_VERSION publishes no release binary for $(uname -s)/$(uname -m)" >&2
      return 2
      ;;
  esac
  sum="$(pin_of "jscpd $platform")"
  if [[ ! "$sum" =~ ^[0-9a-f]{64}$ ]]; then
    echo "duplication: $VERSIONS_FILE pins no sha256 for the jscpd $platform archive" >&2
    return 2
  fi
  dest="$root/target/tools/jscpd-$JSCPD_VERSION"
  stage="$(mktemp -d "$stage_root/install.XXXXXX")"
  curl -fsSL --proto '=https' -o "$stage/$asset" \
    "https://github.com/kucherenko/jscpd/releases/download/v$JSCPD_VERSION/$asset" || {
    echo "duplication: could not download $asset" >&2
    return 2
  }
  if [[ "$(sha256_hex <"$stage/$asset")" != "$sum" ]]; then
    echo "duplication: $asset does not match the sha256 $VERSIONS_FILE pins" >&2
    return 2
  fi
  tar -xzf "$stage/$asset" -C "$stage" jscpd
  chmod 0755 "$stage/jscpd"
  said="$("$stage/jscpd" --version 2>&1)" || {
    echo "duplication: the jscpd of $asset does not run on this host: $said" >&2
    return 2
  }
  if [[ "$said" != "jscpd $JSCPD_VERSION" ]]; then
    echo "duplication: the jscpd of $asset answers '$said', not 'jscpd $JSCPD_VERSION'" >&2
    return 2
  fi
  mkdir -p "$dest"
  mv "$stage/jscpd" "$dest/jscpd"
  echo "duplication: installed jscpd $JSCPD_VERSION at $dest/jscpd"
}

# Every scratch directory of a run lives under one directory the exit trap
# removes. It sits under the ignored target/ of this checkout, which a podman
# machine on macOS shares with its containers.
mkdir -p "$root/target"
stage_root="$(mktemp -d "$root/target/duplication.XXXXXX")"
readonly stage_root
trap 'rm -rf "$stage_root"' EXIT

if [[ "${1:-}" == "--self-test" ]]; then
  work="$stage_root/self-test"
  tree="$work/tree"
  mkdir -p "$tree/app/demo/src" "$tree/app/demo/tests" "$tree/crates/demo/src" "$tree/tools/demo/src"

  # A block of 60-odd tokens over seven lines, above both thresholds.
  block() {
    printf 'pub fn %s(entries: &[Entry]) -> Result<Count, Overflow> {\n' "$1"
    printf '    let mut total = Count::ZERO;\n'
    printf '    for entry in entries {\n'
    printf '        let weight = entry.weight().negated_when(entry.kind() == Kind::Deleted);\n'
    printf '        total = total.checked_add(weight).ok_or(Overflow::Total { at: entry.number() })?;\n'
    printf '    }\n'
    printf '    Ok(total)\n'
    printf '}\n\n'
  }
  other_block() {
    printf 'pub fn %s(rows: &[Row], ehr: EhrId) -> Vec<Summary> {\n' "$1"
    printf '    let mut seen = BTreeMap::new();\n'
    printf '    for row in rows.iter().filter(|row| row.ehr() == ehr) {\n'
    printf '        let entry = seen.entry(row.archetype()).or_insert_with(Summary::empty);\n'
    printf '        entry.add(row.committed(), row.deleted(), row.description().trim());\n'
    printf '    }\n'
    printf '    seen.into_values().collect()\n'
    printf '}\n\n'
  }
  # The one pair the scan counts, across two crates.
  block first >"$tree/app/demo/src/one.rs"
  block second >"$tree/tools/demo/src/two.rs"
  # The same block where the scan must not look: a generated file, an import
  # list, the test module that ends a file, and tests/.
  { printf '// @generated by a miniature. DO NOT EDIT.\n'; block third; } >"$tree/crates/demo/src/generated.rs"
  { printf '#[cfg(test)]\nmod tests {\n'; other_block fourth; printf '}\n'; } >>"$tree/app/demo/src/one.rs"
  { printf '#[cfg(test)]\nmod tests {\n'; other_block fifth; printf '}\n'; } >>"$tree/tools/demo/src/two.rs"
  for name in imports_a imports_b; do
    # Long enough to count as a block of its own were it not left out.
    printf 'use crate::store::{\n    Count, EhrId, Entry, Overflow, Row, Kind, Summary,\n    Composition, Contribution, VersionId, Folder, FolderId, Template,\n    Directory, DirectoryId, Archetype, ArchetypeId, Audit, Party, Lineage,\n};\nuse std::collections::{BTreeMap, BTreeSet};\n\npub fn %s() {}\n' \
      "$name" >"$tree/crates/demo/src/$name.rs"
  done
  block sixth >"$tree/app/demo/tests/it.rs"
  count_file="$work/duplication.count"
  baseline="$work/baseline.txt"

  fail() {
    echo "duplication: SELF-TEST FAILED: $1" >&2
    exit 1
  }

  # The pins come from docs/VERSIONS.md.
  [[ "$JSCPD_IMAGE" == *":$JSCPD_VERSION@sha256:"* ]] ||
    fail "the image pin '$JSCPD_IMAGE' is not of the pinned version $JSCPD_VERSION"

  # The detector order: a pinned jscpd on PATH, then a docker that answers,
  # then a podman that answers, then the first local install that answers the
  # pinned version. Each stub is a program of its own directory, and a run
  # sees only the PATH and the local list it is handed.
  stubs="$work/stubs"
  stub() {
    mkdir -p "$(dirname "$1")"
    printf '#!%s\n%s\n' "$BASH" "$2" >"$1"
    chmod +x "$1"
  }
  stub "$stubs/pinned/jscpd" "echo 'jscpd $JSCPD_VERSION'"
  stub "$stubs/other/jscpd" "echo 'jscpd 0.0.1'"
  stub "$stubs/docker-up/docker" "exit 0"
  stub "$stubs/docker-down/docker" "exit 1"
  stub "$stubs/podman-up/podman" "exit 0"
  stub "$stubs/podman-down/podman" "exit 1"
  stub "$stubs/local-good/jscpd" "echo 'jscpd $JSCPD_VERSION'"
  stub "$stubs/local-other/jscpd" "echo 'jscpd 0.0.1'"
  mkdir -p "$stubs/empty"
  # Prints the detector for the PATH $1 and the local candidates after it.
  kind_for() {
    (
      PATH="$1"
      shift
      LOCAL_JSCPD=("$@")
      detector_kind
    )
  }
  good="$stubs/local-good/jscpd"
  other="$stubs/local-other/jscpd"
  [[ "$(kind_for "$stubs/pinned:$stubs/docker-up:$stubs/podman-up" "$good")" == path ]] ||
    fail "a pinned jscpd on PATH did not come first"
  [[ "$(kind_for "$stubs/other:$stubs/docker-up:$stubs/podman-up" "$good")" == docker ]] ||
    fail "a jscpd on PATH of another version was taken over a docker that answers"
  [[ "$(kind_for "$stubs/other:$stubs/docker-down:$stubs/podman-up" "$good")" == podman ]] ||
    fail "a docker that does not answer was taken over a podman that does"
  [[ "$(kind_for "$stubs/other:$stubs/docker-down:$stubs/podman-down" "$good")" == local ]] ||
    fail "an engine that does not answer was taken over the local install"
  [[ "$(kind_for "$stubs/empty" "$stubs/missing/jscpd" "$other" "$good")" == local ]] ||
    fail "the local install of the main checkout was not found after two that do not answer"
  [[ "$(
    LOCAL_JSCPD=("$other" "$good")
    local_jscpd
  )" == "$good" ]] ||
    fail "the local install that answers was not the one picked"
  [[ -z "$(kind_for "$stubs/other:$stubs/docker-down:$stubs/podman-down" "$other")" ]] ||
    fail "a local install of another version was taken"
  [[ -z "$(kind_for "$stubs/empty")" ]] || fail "a machine with no detector found one"
  status=0
  (
    # shellcheck disable=SC2123 # the subshell runs detect with no detector on PATH on purpose
    PATH="$stubs/empty"
    LOCAL_JSCPD=()
    detect "$tree" "$work"
  ) >/dev/null 2>"$work/answer" || status=$?
  [[ "$status" == 2 ]] || fail "a machine with no detector answered $status, not 2"
  grep -Fq -- '--install' "$work/answer" ||
    fail "a machine with no detector was not told how to install one: $(cat "$work/answer")"
  run() {
    local status=0
    ratchet "$tree" "$count_file" "$baseline" "$1" >"$work/answer" 2>&1 || status=$?
    printf '%s\n' "$status"
  }

  [[ "$(run check)" == 1 ]] || fail "a tree with no committed count passed: $(cat "$work/answer")"
  [[ "$(run update)" == 0 ]] || fail "--update refused to record the first count: $(cat "$work/answer")"
  first="$(committed_count "$count_file")"
  [[ "$first" =~ ^[1-9][0-9]*$ ]] || fail "--update recorded '$first' for a tree with one duplicated block"

  # Unchanged: the one pair, and nothing from the places the scan skips.
  [[ "$(run check)" == 0 ]] || fail "an unchanged count was refused: $(cat "$work/answer")"
  grep -q 'in two places: 1$' "$work/answer" ||
    fail "the scan did not count exactly the one pair: $(cat "$work/answer")"
  grep -Fq 'app/demo/src/one.rs:1-8 = tools/demo/src/two.rs:1-8' "$work/answer" ||
    grep -Fq 'tools/demo/src/two.rs:1-8 = app/demo/src/one.rs:1-8' "$work/answer" ||
    fail "the pair was not named by both places: $(cat "$work/answer")"
  grep -q 'every block in' "$work/answer" ||
    fail "the one pair was not found in the baseline --update wrote: $(cat "$work/answer")"

  # A move around the block: code above it and a file beside it change, the
  # block itself does not, so its key and the baseline stay as they were.
  cp "$baseline" "$work/baseline.before"
  {
    printf 'pub const LIMIT: u32 = 3;\n\n'
    cat "$tree/tools/demo/src/two.rs"
  } >"$work/two.rs"
  mv "$work/two.rs" "$tree/tools/demo/src/two.rs"
  printf 'pub fn unrelated(left: u32, right: u32) -> u32 {\n    left.max(right)\n}\n' >"$tree/app/demo/src/unrelated.rs"
  [[ "$(run check)" == 0 ]] || fail "a change beside the block was refused: $(cat "$work/answer")"
  grep -Fq 'tools/demo/src/two.rs:3-10' "$work/answer" ||
    fail "the block did not move down with the code above it: $(cat "$work/answer")"
  grep -q 'every block in' "$work/answer" ||
    fail "a block that did not change was reported as new: $(cat "$work/answer")"
  [[ "$(run update)" == 0 ]] || fail "--update refused an unchanged count: $(cat "$work/answer")"
  cmp -s "$baseline" "$work/baseline.before" ||
    fail "a change beside the block changed the baseline: $(diff "$work/baseline.before" "$baseline")"

  # A rise: a new pair across two crates fails and is named by both places.
  other_block seventh >"$tree/app/demo/src/three.rs"
  other_block eighth >"$tree/crates/demo/src/four.rs"
  [[ "$(run check)" == 1 ]] || fail "a rise passed"
  grep -q 'rose from' "$work/answer" || fail "a rise was not reported as one: $(cat "$work/answer")"
  new_block="$(grep -A3 'does not carry' "$work/answer" | grep 'three.rs' || true)"
  [[ "$new_block" == *four.rs* ]] ||
    fail "a rise did not name the new block by both places: $(cat "$work/answer")"
  [[ "$(run update)" == 1 ]] || fail "--update raised the count"
  [[ "$(committed_count "$count_file")" == "$first" ]] || fail "--update wrote a higher count"

  # A fall: fails until --update lowers the count, which then holds.
  rm "$tree/app/demo/src/three.rs" "$tree/crates/demo/src/four.rs"
  block second_moved >"$tree/app/demo/src/one.rs"
  printf 'pub fn empty() {}\n' >"$tree/tools/demo/src/two.rs"
  [[ "$(run check)" == 1 ]] || fail "a fall passed without a lower count"
  grep -q 'fell from' "$work/answer" || fail "a fall was not reported as one: $(cat "$work/answer")"
  [[ "$(run update)" == 0 ]] || fail "--update refused a fall: $(cat "$work/answer")"
  [[ "$(committed_count "$count_file")" == 0 ]] || fail "--update did not record the lower count"
  [[ "$(run check)" == 0 ]] || fail "the lowered count was refused: $(cat "$work/answer")"

  echo "duplication: self-test OK (the pins, the detector order, a rise, a fall, an unchanged count and a move around a block)."
  exit 0
fi

case "${1:-}" in
  "") ratchet "$root" "$COUNT_FILE" "$BASELINE_FILE" check ;;
  --update) ratchet "$root" "$COUNT_FILE" "$BASELINE_FILE" update ;;
  --install) install_local ;;
  *)
    echo "usage: $0 [--update | --install | --self-test]" >&2
    exit 2
    ;;
esac
