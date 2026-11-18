#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The CPU instructions of each codec micro-path (#6007): the canonical JSON
# and XML parse and render of a COMPOSITION, over the lab composition and the
# IPS composition, each counted by callgrind in a release build and held to
# its ceiling in perf/instructions.ceilings.
#
# A count is deterministic for one binary on one platform, so the lane runs
# on Linux only, where valgrind runs. Only the path itself is counted:
# `--toggle-collect` collects inside the example's `measured` function and
# nowhere else. No openEHR spec governs the cost of the codec: our own design.
#
# Usage:
#   scripts/perf/instructions.sh            measure, report, fail on a ceiling
#   scripts/perf/instructions.sh --no-build measure the example already built
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

ceilings="perf/instructions.ceilings"
example="target/release/examples/codec_instructions"
declare -A fixtures=(
  [lab]="corpus/templates/ckm/generic-lab-test-result.example.json"
  [ips]="crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json"
)

command -v valgrind >/dev/null || {
  echo "instructions: valgrind is not installed" >&2
  exit 2
}
if [[ "${1:-}" != "--no-build" ]]; then
  cargo build --locked --release -p openehr-its --example codec_instructions
fi

out="$(mktemp -d)"
trap 'rm -f -v "$out"/*.out >/dev/null; rmdir "$out"' EXIT

over=0
printf '%-11s %-4s %15s %15s\n' path fix instructions ceiling
while read -r path fixture ceiling; do
  [[ -z "$path" || "$path" == \#* ]] && continue
  file="${fixtures[$fixture]:-}"
  [[ -n "$file" ]] || {
    echo "instructions: unknown fixture \`$fixture\`" >&2
    exit 2
  }
  trace="$out/$path-$fixture.out"
  valgrind --tool=callgrind --toggle-collect='*measured*' \
    --callgrind-out-file="$trace" "$example" "$path" "$file" 2>/dev/null
  count="$(awk '/^summary:/ { print $2 }' "$trace")"
  [[ -n "$count" ]] || {
    echo "instructions: no count for $path $fixture" >&2
    exit 2
  }
  verdict=""
  if [[ "$ceiling" != "report" && "$count" -gt "$ceiling" ]]; then
    verdict="  OVER"
    over=1
  fi
  printf '%-11s %-4s %15s %15s%s\n' "$path" "$fixture" "$count" "$ceiling" "$verdict"
done <"$ceilings"

if [[ "$over" -ne 0 ]]; then
  echo "instructions: a codec path exceeds its ceiling in $ceilings" >&2
  exit 1
fi
