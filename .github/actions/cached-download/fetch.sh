#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# The download half of the cached-download action: verifies a file the
# Actions cache restored, or fetches it from its pinned URL on a miss, and
# checks it against its pinned sha256 either way.
#
#   fetch.sh            # reads TOOL_URL, TOOL_SHA256, TOOL_FILE, CACHE_HIT
#   fetch.sh --self-test
#
# On a cache hit it never runs curl: a restored file that is missing or off
# its pin fails the step, so a warm run makes no request to the release host.
# On a miss curl retries five times with exponential backoff (1, 2, 4, 8 and
# 16 seconds) on every error, a refused connection included. On 2026-11-08
# GitHub answered 504 for every release asset for hours. No specification
# governs this: our own design, shared by the FerroHEALTH repositories.
set -euo pipefail

self="${BASH_SOURCE[0]}"

die() {
  printf 'cached-download: %s\n' "$1" >&2
  exit 1
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Exit 0 when the file exists and matches the pin.
matches_pin() {
  [[ -f "$1" ]] && [[ "$(sha256_of "$1")" == "$2" ]]
}

fetch() {
  local url="${TOOL_URL:?TOOL_URL names the pinned download}"
  local sum="${TOOL_SHA256:?TOOL_SHA256 pins the download}"
  local file="${TOOL_FILE:?TOOL_FILE names where the download lives}"
  local hit="${CACHE_HIT:-false}"
  [[ "$sum" =~ ^[0-9a-f]{64}$ ]] || die "TOOL_SHA256 is not a lowercase sha256: '$sum'"
  [[ "$url" == https://* ]] || die "TOOL_URL is not https: '$url'"

  if [[ "$hit" == true ]]; then
    matches_pin "$file" "$sum" ||
      die "the cache restored $file off its pin $sum; delete the cache entry, a download would hide it"
    printf 'cached-download: %s restored from the Actions cache, no download\n' "$file"
  elif matches_pin "$file" "$sum"; then
    printf 'cached-download: %s already on disk and on its pin, no download\n' "$file"
  else
    mkdir -p "$(dirname "$file")"
    rm -f "$file.part"
    curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fsSL \
      --connect-timeout 20 --max-time 600 \
      --retry 5 --retry-all-errors --retry-connrefused \
      -o "$file.part" "$url" ||
      die "could not download $url after 5 retries"
    if ! matches_pin "$file.part" "$sum"; then
      rm -f "$file.part"
      die "$url does not match its pin $sum"
    fi
    mv "$file.part" "$file"
    printf 'cached-download: fetched %s and checked it against its pin\n' "$url"
  fi
  if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    printf 'path=%s\n' "$file" >>"$GITHUB_OUTPUT"
  fi
}

# Drives fetch against a stub curl that logs each call, so it shows a hit
# makes no request and every refusal holds, without the network.
self_test() {
  local work failed=0 n=0 good bad
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' RETURN
  mkdir -p "$work/bin"
  printf 'the pinned bytes\n' >"$work/payload"
  good="$(sha256_of "$work/payload")"
  bad="$(printf '%064d' 0)"
  cat >"$work/bin/curl" <<EOF
#!/usr/bin/env bash
echo call >>"$work/calls"
out=""
while [[ \$# -gt 0 ]]; do
  if [[ "\$1" == -o ]]; then out="\$2"; shift; fi
  shift
done
[[ -f "$work/offline" ]] && exit 22
cp "$work/payload" "\$out"
EOF
  chmod 0755 "$work/bin/curl"

  # case NAME EXPECT(0|1) EXPECT_CALLS HIT SHA256 [prime]
  case_() {
    local name="$1" want="$2" calls="$3" hit="$4" sum="$5" prime="${6:-}" got
    n=$((n + 1))
    rm -rf "$work/cache" "$work/calls"
    : >"$work/calls"
    mkdir -p "$work/cache"
    [[ "$prime" == prime ]] && cp "$work/payload" "$work/cache/tool"
    [[ "$prime" == corrupt ]] && printf 'other bytes\n' >"$work/cache/tool"
    if PATH="$work/bin:$PATH" TOOL_URL=https://example.invalid/tool TOOL_SHA256="$sum" \
      TOOL_FILE="$work/cache/tool" CACHE_HIT="$hit" GITHUB_OUTPUT="$work/out" \
      bash "$self" >/dev/null 2>&1; then got=0; else got=1; fi
    if [[ "$got" != "$want" ]]; then
      printf 'FAIL %s: exit %s, wanted %s\n' "$name" "$got" "$want" >&2
      failed=1
    elif [[ "$(grep -c call "$work/calls")" != "$calls" ]]; then
      printf 'FAIL %s: %s curl calls, wanted %s\n' "$name" "$(grep -c call "$work/calls")" "$calls" >&2
      failed=1
    fi
  }

  touch "$work/offline"
  case_ "a cache hit on its pin makes no request" 0 0 true "$good" prime
  case_ "a cache hit with no file fails with no request" 1 0 true "$good"
  case_ "a cache hit off its pin fails with no request" 1 0 true "$good" corrupt
  case_ "a file on disk on its pin makes no request" 0 0 false "$good" prime
  case_ "a miss while the host refuses fails" 1 1 false "$good"
  rm "$work/offline"
  case_ "a miss fetches once and checks the pin" 0 1 false "$good"
  case_ "a miss off its pin fails" 1 1 false "$bad"
  [[ ! -e "$work/cache/tool" && ! -e "$work/cache/tool.part" ]] || {
    echo "FAIL a download off its pin was left on disk" >&2
    failed=1
  }
  case_ "a sha256 that is not one is refused" 1 0 false "not-a-sum"

  if [[ "$failed" -ne 0 ]]; then
    echo "cached-download self-test: FAILED" >&2
    return 1
  fi
  echo "cached-download self-test: OK ($n cases)"
}

case "${1:-}" in
  --self-test) self_test ;;
  "") fetch ;;
  *)
    echo "usage: $0 [--self-test]" >&2
    exit 2
    ;;
esac
