#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Measures the viewer's release site bundle (app/ferroehr-viewer) and fails
# when its brotli-compressed WebAssembly is over its byte budget (no openEHR
# spec governs this: our own design).
#
# It reads the bundle `scripts/cargo-leptos.sh build --release` writes (the
# file names carry content hashes, `ferroehr-viewer.<hash>.wasm`), prints the
# WebAssembly and its JavaScript glue raw, gzip-compressed (gzip -9) and
# brotli-compressed (brotli -q 11) as a Markdown table, and appends the table
# to $GITHUB_STEP_SUMMARY when that is set. The budget is read from the budget
# table of .claude/rules/leptos-ui.md, beside its reasoning, so the number
# lives in one place.
#
# Usage: scripts/checks/viewer-bundle.sh [pkg-dir]
#   pkg-dir defaults to target/site/pkg.
#
# Exit 0 = within the budget. Exit 1 = over the budget, a bundle file missing,
# a compressor missing, or the budget unreadable. Exit 2 = a usage error.
set -Eeuo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
rule="$root/.claude/rules/leptos-ui.md"
name="ferroehr-viewer"

case "$#" in
  0) pkg="$root/target/site/pkg" ;;
  1) pkg="$1" ;;
  *)
    echo "usage: scripts/checks/viewer-bundle.sh [pkg-dir]" >&2
    exit 2
    ;;
esac

for tool in gzip brotli; do
  if ! command -v "$tool" > /dev/null; then
    echo "viewer-bundle: $tool is not on PATH; the bundle cannot be measured." >&2
    exit 1
  fi
done

# The newest file of the bundle with extension `$1`, hashed name or plain: a
# local tree keeps the files of earlier builds beside the latest one.
one() {
  local found=() newest=""
  shopt -s nullglob
  found=("$pkg/$name".*."$1" "$pkg/$name.$1")
  shopt -u nullglob
  for file in "${found[@]}"; do
    if [[ -z "$newest" ]] || [[ "$file" -nt "$newest" ]]; then
      newest="$file"
    fi
  done
  if [[ -z "$newest" ]] || [[ ! -s "$newest" ]]; then
    echo "viewer-bundle: no $name .$1 in $pkg; run scripts/cargo-leptos.sh build --release first." >&2
    exit 1
  fi
  printf '%s' "$newest"
}

wasm="$(one wasm)"
js="$(one js)"

# The one row of the budget table whose first cell names the gating measure.
budget="$(awk -F'|' '
  $2 ~ /^[[:space:]]*WebAssembly, brotli-compressed[[:space:]]*$/ {
    value = $3
    sub(/^[[:space:]]*/, "", value)
    sub(/[[:space:]]*bytes.*$/, "", value)
    print value
  }
' "$rule")"
if ! [[ "$budget" =~ ^[0-9]+$ ]]; then
  echo "viewer-bundle: $rule has no single \"| WebAssembly, brotli-compressed | N bytes |\" budget row." >&2
  exit 1
fi

raw() { local file="$1"; wc -c < "$file" | tr -d '[:space:]'; }
gz() { local file="$1"; gzip -9 -c "$file" | wc -c | tr -d '[:space:]'; }
br() { local file="$1"; brotli -q 11 -c "$file" | wc -c | tr -d '[:space:]'; }

wasm_br="$(br "$wasm")"
percent=$((wasm_br * 100 / budget))

table="$(
  echo "### Viewer bundle"
  echo
  echo "| File | Raw | gzip -9 | brotli -q 11 |"
  echo "|---|---:|---:|---:|"
  echo "| \`$(basename "$wasm")\` | $(raw "$wasm") | $(gz "$wasm") | $wasm_br |"
  echo "| \`$(basename "$js")\` | $(raw "$js") | $(gz "$js") | $(br "$js") |"
  echo
  echo "Budget: $budget brotli bytes of WebAssembly; this build uses $wasm_br ($percent%)."
)"
echo "$table"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  echo "$table" >> "$GITHUB_STEP_SUMMARY"
fi

if [[ "$wasm_br" -gt "$budget" ]]; then
  echo "viewer-bundle: the WebAssembly is $wasm_br brotli bytes, over the $budget-byte budget in $rule." >&2
  exit 1
fi
