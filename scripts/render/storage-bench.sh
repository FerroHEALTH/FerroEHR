#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Render the storage-benchmark section for the book FROM the committed records
# (docs/benchmarks/storage/<generation>/record.json, #3367).
#
# The stale-numbers gate (scripts/checks/conformance-numbers.sh) forbids a
# hand-typed latency in the site sources, so the performance page includes this
# output instead; website/book/generated/ is render output and gitignored.
#
# COMMITTED records only. The harness writes a record on every run and the
# directory ignores them by default, so the page is a pure function of what the
# repository actually carries — a local exploration run never changes it, and
# `--check` means the same thing here as it does in CI.
#
# With no record committed the page says so. A benchmark page that silently
# renders nothing reads as though the numbers were withheld.
#
# Usage:
#   scripts/render/storage-bench.sh            write the page
#   scripts/render/storage-bench.sh --check    fail when the page is missing or stale
set -euo pipefail
cd "$(dirname "$0")/../.."
command -v jq >/dev/null || { echo "error: jq is required" >&2; exit 1; }

OUT="website/book/generated/storage-bench.md"
MODE="write"
case "${1:-}" in
  --check) MODE="check" ;;
  "") ;;
  *) echo "usage: scripts/render/storage-bench.sh [--check]" >&2; exit 2 ;;
esac

records=()
while IFS= read -r -d '' path; do
  records+=("$path")
done < <(git ls-files -z 'docs/benchmarks/storage/*/record.json')

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

if [[ ${#records[@]} -eq 0 ]]; then
  cat > "$tmp" <<'EOF'
No storage benchmark record is committed in this repository, so this section
carries no numbers. The harness is
`cargo bench -p ferroehr --bench storage`; committing a record is described in
`docs/benchmarks/storage/README.md`.
EOF
else
  : > "$tmp"
  for record in "${records[@]}"; do
    # One section per schema generation: the run's identity, the per-operation
    # latencies, and the relations the run left behind.
    jq -r '
      def t: if . == null then "—"
             elif . >= 1000000000 then ((. / 100000000 | round) / 10 | tostring) + " s"
             elif . >= 1000000 then ((. / 100000 | round) / 10 | tostring) + " ms"
             else ((. / 100 | round) / 10 | tostring) + " µs" end;
      def b: if . == null then "—"
             elif . >= 1048576 then ((. / 104857.6 | round) / 10 | tostring) + " MiB"
             elif . >= 1024 then ((. / 102.4 | round) / 10 | tostring) + " KiB"
             else (tostring) + " B" end;
      def n: tostring | if length > 3 then (.[:-3] | n) + " " + .[-3:] else . end;
      "### Schema generation `\(.schema_generation)`, class `\(.bench_class)`",
      "",
      "| Operation | Iterations | p50 | p95 | p99 | WAL | Buffer hits |",
      "|---|---|---|---|---|---|---|",
      (.groups[] | .operations[] |
        "| `\(.name)` | \(.iterations | floor | n) | \(.p50_ns | t) | \(.p95_ns | t) | \(.p99_ns | t) | \(.database.wal_bytes | b) | \(.database.blks_hit_delta | n) |"),
      "",
      "One representative commit wrote \(.commit_probe.database.wal_bytes | b) of WAL and touched \(.commit_probe.database.blks_hit_delta | n) buffers. The population CONTAINS statement plans as `\(.explain.node_type)` and reads \(.explain.shared_hit_blocks | n) shared blocks.",
      "",
      "| Relation | Live rows | Dead rows | Updates | HOT updates | Size |",
      "|---|---|---|---|---|---|",
      (.relations[] |
        "| `\(.relation)` | \(.n_live_tup | n) | \(.n_dead_tup | n) | \(.n_tup_upd | n) | \(.n_tup_hot_upd | n) | \(.total_bytes | b) |"),
      "",
      "Measured over \(.seed.ehrs | n) EHRs and \(.seed.compositions | n) compositions of template `\(.seed.template_id)` at commit `\(.measured_at_commit[0:12])` on \(.cpu_count) cores against \(.postgres_version | split(" ") | .[0:2] | join(" ")).",
      ""
    ' "$record" >> "$tmp"
  done
fi

if [[ "$MODE" == "check" ]]; then
  if [[ ! -f "$OUT" ]]; then
    echo "storage-bench: $OUT is missing — run scripts/render/storage-bench.sh" >&2
    exit 1
  fi
  if ! diff -u "$OUT" "$tmp"; then
    echo "storage-bench: $OUT is stale — re-run scripts/render/storage-bench.sh" >&2
    exit 1
  fi
  echo "storage-bench: $OUT matches the committed records."
  exit 0
fi

mkdir -p "$(dirname "$OUT")"
cp "$tmp" "$OUT"
echo "storage-bench: wrote $OUT from ${#records[@]} committed record(s)."
