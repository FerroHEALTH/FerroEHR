#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# The default demographic definitions a release attaches (#5428; no
# specification governs this: our own design). Every ADL 2 file under
# deploy/definitions/ rides each release as an asset of its own, beside a
# `<file>.sha256sum` in the form `sha256sum` writes, so a deployment fetches
# the set by version, as it fetches docker-compose.yml.
#
#   definition-assets.sh list
#       Prints the name of every asset the set adds to a release, one a line.
#   definition-assets.sh stage <dir>
#       Copies each file into <dir>, an empty directory, with its checksum
#       file, then checks every checksum. Run it on the release lane's own
#       checkout.
#   definition-assets.sh --self-test
#       Proves a staged set lists what `list` prints and verifies, and that a
#       changed copy fails its checksum.
set -euo pipefail

usage() {
  echo "usage: definition-assets.sh list | stage <dir> | --self-test" >&2
  exit 2
}

cd "$(dirname "$0")/../.."

# The files of the set, sorted, so the list is the same on every machine.
files() {
  local found
  found="$(find deploy/definitions -mindepth 2 -maxdepth 2 -name '*.adls' | LC_ALL=C sort)"
  [[ -n "$found" ]] || { echo "definition-assets: no .adls file under deploy/definitions/" >&2; return 1; }
  printf '%s\n' "$found"
}

list() {
  local file names
  names="$(files | while IFS= read -r file; do
    printf '%s\n%s.sha256sum\n' "${file##*/}" "${file##*/}"
  done)"
  if [[ -n "$(printf '%s\n' "$names" | LC_ALL=C sort | uniq -d)" ]]; then
    echo "definition-assets: two files under deploy/definitions/ share a name, and a release asset name is unique" >&2
    return 1
  fi
  printf '%s\n' "$names"
}

stage() {
  local dir="$1" file name
  [[ -d "$dir" ]] || { echo "definition-assets: $dir is not a directory" >&2; return 1; }
  [[ -z "$(find "$dir" -mindepth 1 -maxdepth 1)" ]] || { echo "definition-assets: $dir is not empty" >&2; return 1; }
  list >/dev/null
  while IFS= read -r file; do
    name="${file##*/}"
    cp "$file" "$dir/$name"
    (cd "$dir" && sha256sum "$name" > "$name.sha256sum")
  done < <(files)
  while IFS= read -r name; do
    [[ "$name" == *.sha256sum ]] || continue
    (cd "$dir" && sha256sum --check --quiet "$name")
  done < <(list)
}

self_test() {
  local work staged expected first
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' RETURN
  mkdir "$work/out"
  stage "$work/out"
  staged="$(find "$work/out" -mindepth 1 -maxdepth 1 -exec basename {} \; | LC_ALL=C sort)"
  expected="$(list | LC_ALL=C sort)"
  [[ "$staged" == "$expected" ]] || { echo "self-test: the staged files are not the listed assets" >&2; return 1; }
  first="$(list | sed -n 1p)"
  printf '\n' >> "$work/out/$first"
  if (cd "$work/out" && sha256sum --check --quiet "$first.sha256sum" >/dev/null 2>&1); then
    echo "self-test: a changed copy passed its checksum" >&2
    return 1
  fi
  mkdir "$work/busy"
  touch "$work/busy/left-over"
  if stage "$work/busy" 2>/dev/null; then
    echo "self-test: a directory that is not empty was staged into" >&2
    return 1
  fi
  echo "definition-assets: self-test passed ($(files | wc -l | tr -d ' ') definitions)"
}

case "${1:-}" in
  list) [[ "$#" -eq 1 ]] || usage; list ;;
  stage) [[ "$#" -eq 2 ]] || usage; stage "$2" ;;
  --self-test) [[ "$#" -eq 1 ]] || usage; self_test ;;
  *) usage ;;
esac
