# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# shellcheck shell=bash
# Shared helpers for the vendor scripts that pin each file by URL and sha256.
#
# The sourcing script names every file it takes and the sha256 it expects. A
# download that hashes to anything else refuses the run before the vendored
# tree is touched, so a re-run either reproduces the committed bytes or fails.
#
# No specification governs this file; it is FerroEHR's own design.

pinned_script="$(basename "${BASH_SOURCE[1]:-${BASH_SOURCE[0]}}" .sh)"

die() {
  printf '%s: %s\n' "$pinned_script" "$*" >&2
  exit 1
}

say() { printf '%s: %s\n' "$pinned_script" "$*"; }

pinned_require() {
  local tool
  for tool in "$@"; do
    command -v "$tool" >/dev/null 2>&1 || die "missing required tool: $tool"
  done
}

pinned_sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }

# Downloads URL $1 to $2 and refuses the run unless it hashes to $3.
pinned_fetch() {
  local url="$1" out="$2" want="$3" got
  mkdir -p "$(dirname "$out")"
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    --retry 3 -o "$out" "$url" || die "download of $url failed"
  got="$(pinned_sha256 "$out")"
  [[ "$got" == "$want" ]] || die "$url hashes to $got, the pin records $want"
}

# Fetches every `path sha256` pin after the first two arguments from the base
# URL $1 into the directory $2, keeping the upstream path.
pinned_fetch_all() {
  local base="$1" into="$2" pin path
  shift 2
  for pin in "$@"; do
    path="${pin% *}"
    pinned_fetch "$base/$path" "$into/$path" "${pin##* }"
    say "$path"
  done
}

# One Markdown table row per `path sha256` pin argument.
pinned_rows() {
  local pin
  for pin in "$@"; do
    # shellcheck disable=SC2016 # the backticks are literal Markdown, not expansions
    printf '| `%s` | `%s` |\n' "${pin% *}" "${pin##* }"
  done
}

# Replaces the vendored tree $2 with the scratch tree $1.
pinned_install() {
  local from="$1" dest="$2"
  rm -rf "$dest"
  mkdir -p "$(dirname "$dest")"
  cp -R "$from" "$dest"
}
