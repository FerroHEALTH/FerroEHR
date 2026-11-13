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

# Downloads URL $1 to $2 and refuses the run unless it hashes to $3. curl
# retries a transient answer itself, as `retry_read` in scripts/gh/retry-net.sh
# does, but not a name that does not resolve, so a failed attempt is tried
# again PINNED_ATTEMPTS times in all (five) after a pause that doubles from
# PINNED_PAUSE seconds (five): a passing outage costs about a minute, and a
# lasting one still fails the run (#5398).
pinned_fetch() {
  local url="$1" out="$2" want="$3" got attempt=1 pause="${PINNED_PAUSE:-5}"
  mkdir -p "$(dirname "$out")"
  until curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    --retry 5 --retry-connrefused --retry-delay 2 --max-time 120 -o "$out" "$url"; do
    if ((attempt >= ${PINNED_ATTEMPTS:-5})); then
      rm -f "$out"
      die "download of $url failed after $attempt attempts"
    fi
    say "download of $url failed (attempt $attempt); trying again in ${pause}s"
    sleep "$pause"
    attempt=$((attempt + 1))
    pause=$((pause * 2))
  done
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

# Downloads URL $1 into the git-ignored cache as .vendor-cache/$2/$3 and
# refuses the run unless it hashes to $4. A file the cache already holds is
# used without a download when it hashes to $4; one that hashes to anything
# else refuses the run, so a damaged or tampered cache is never read. A
# cache-only document is one whose terms do not permit redistribution: it is
# read from the cache and never enters the vendored tree.
pinned_cache() {
  local url="$1" corpus="$2" file="$3" want="$4" root out got
  root="${PINNED_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)}"
  out="$root/.vendor-cache/$corpus/$file"
  if [[ -f "$out" ]]; then
    got="$(pinned_sha256 "$out")"
    [[ "$got" == "$want" ]] ||
      die ".vendor-cache/$corpus/$file hashes to $got, the pin records $want; delete it to fetch it again"
    say "$file (cache only, held at its pin, not downloaded)"
    return 0
  fi
  pinned_fetch "$url" "$out" "$want"
  say "$file (cache only)"
}

# Copies the file in directory $1 that hashes to $3 to $2, refusing the run
# when $1 holds no such file. For a document the owner retrieved by hand from
# a source that answers an automated client with a login.
pinned_supplied() {
  local from="$1" out="$2" want="$3" candidate
  while IFS= read -r -d '' candidate; do
    if [[ "$(pinned_sha256 "$candidate")" == "$want" ]]; then
      mkdir -p "$(dirname "$out")"
      cp "$candidate" "$out"
      return 0
    fi
  done < <(find "$from" -maxdepth 1 -type f -print0)
  die "$from holds no file that hashes to $want"
}

# Refuses the run unless directory $1 holds a file that hashes to $2. For a
# document the owner read whose terms permit no copy: its digest is checked
# and recorded, and it is copied nowhere.
pinned_withheld() {
  local from="$1" want="$2" candidate
  while IFS= read -r -d '' candidate; do
    [[ "$(pinned_sha256 "$candidate")" == "$want" ]] && return 0
  done < <(find "$from" -maxdepth 1 -type f -print0)
  die "$from holds no file that hashes to $want"
}

# One Markdown table row per `path sha256` pin argument after the first, with
# the status $1 (`cache only`, `not redistributed`) as a third cell. Those
# rows name a document the tree must not hold.
pinned_status_rows() {
  local status="$1" pin
  shift
  for pin in "$@"; do
    # shellcheck disable=SC2016 # the backticks are literal Markdown, not expansions
    printf '| `%s` | `%s` | %s |\n' "${pin% *}" "${pin##* }" "$status"
  done
}

# Proves the cache and the download without the network: a held file at its
# pin is used with no curl call, a held file off its pin refuses the run, and
# a download that keeps failing refuses it after its attempts. Run as
# `bash scripts/vendor/lib/pinned.sh --self-test`.
pinned_self_test() {
  local work want failures=0
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' RETURN
  mkdir -p "$work/bin" "$work/.vendor-cache/corpus"
  printf '#!/bin/sh\necho curl >> "%s/calls"; exit 6\n' "$work" > "$work/bin/curl"
  chmod +x "$work/bin/curl"
  printf 'held\n' > "$work/.vendor-cache/corpus/held.tgz"
  want="$(pinned_sha256 "$work/.vendor-cache/corpus/held.tgz")"
  # outcome NAME WANT_EXIT -- the call
  outcome() {
    local name="$1" wanted="$2" got=0
    shift 2
    (PATH="$work/bin:$PATH" PINNED_ROOT="$work" PINNED_PAUSE=0 PINNED_ATTEMPTS=2 "$@") \
      > /dev/null 2>&1 || got=$?
    if [[ "$got" != "$wanted" ]]; then
      echo "pinned self-test: $name gave $got, wanted $wanted" >&2
      failures=$((failures + 1))
    fi
  }
  outcome "a held file at its pin" 0 pinned_cache https://example.invalid/held.tgz corpus held.tgz "$want"
  if [[ -s "$work/calls" ]]; then
    echo "pinned self-test: a held file at its pin called curl" >&2
    failures=$((failures + 1))
  fi
  outcome "a held file off its pin" 1 pinned_cache https://example.invalid/held.tgz corpus held.tgz "${want/a/b}0"
  outcome "a download that keeps failing" 1 pinned_cache https://example.invalid/new.tgz corpus new.tgz "$want"
  if [[ "$(wc -l < "$work/calls")" -ne 2 ]]; then
    echo "pinned self-test: the failing download was not tried twice" >&2
    failures=$((failures + 1))
  fi
  if ((failures > 0)); then
    echo "pinned self-test: $failures case(s) failed" >&2
    return 1
  fi
  echo "pinned self-test: OK (a held file used with no download, one off its pin refused, a lasting failure refused after its attempts)"
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  case "${1:-}" in
    --self-test) pinned_self_test ;;
    *) echo "usage: $0 --self-test" >&2; exit 2 ;;
  esac
fi
