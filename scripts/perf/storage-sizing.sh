#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The storage sizing driver: what one composition version, one node row and
# one audit record take on disk, measured against a running FerroEHR. It is
# the measurement behind the book's storage sizing section (operations.md,
# "Storage sizing").
#
# `run` drives the server over ITS-REST in phases, with a fixed count per
# phase: the template upload, EHR creates, composition creates (spread over
# the EHRs), updates with If-Match, deletes and composition reads. After each
# phase it runs ANALYZE and CHECKPOINT and reads every relation's heap
# (pg_relation_size), TOAST and index sizes, the exact row counts of the
# node, version and audit tables, the database size and the WAL position.
# `report` turns those snapshots into bytes per create, update, delete and
# read, per relation, and bytes per node row. No openEHR spec governs this:
# our own tooling.
#
# Start the server on a fresh database, so the snapshots hold only this run.
# The WAL figures include the full-page images every CHECKPOINT forces, so
# they overstate a steady state.
#
# Usage:
#   scripts/perf/storage-sizing.sh run <out-dir>
#       writes <out-dir>/snapshots.json and <out-dir>/report.json and prints
#       a Markdown summary
#   scripts/perf/storage-sizing.sh report <snapshots.json>
#       prints the report JSON of an earlier run
#   scripts/perf/storage-sizing.sh --self-test
#
# Environment for `run` (defaults are the quickstart's):
#   SIZING_BASE_URL   ITS-REST base (http://localhost:8080/ferroehr/rest/openehr/v1)
#   SIZING_USER, SIZING_PASSWORD   HTTP Basic credentials (ferroehr / ferroehr)
#   SIZING_PSQL       the command that runs psql as a superuser on the server's
#                     database, reading SQL on stdin (docker compose exec -T
#                     ferroehr-postgres psql -U postgres -d ferroehr)
#   SIZING_TEMPLATE   corpus/templates/ckm/<name>.opt and .example.json
#                     (vital-signs; international-patient-summary is the large one)
#   SIZING_EHRS, SIZING_PER_EHR, SIZING_UPDATES, SIZING_DELETES, SIZING_READS
#                     the phase counts (300, 10, 750, 300, 6000)
#   SIZING_WORKERS    concurrent requests (8)
# Exit 0 on success, 1 on a failed request or snapshot, 2 on a usage error.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
readonly ROOT_DIR

usage() {
  echo "usage: $0 run <out-dir> | report <snapshots.json> | --self-test" >&2
  exit 2
}

# The snapshot of one phase: one JSON object on one line.
readonly SNAPSHOT_SQL="
SELECT jsonb_build_object(
  'database_bytes', pg_database_size(current_database()),
  'wal_lsn_bytes', pg_current_wal_lsn() - '0/0'::pg_lsn,
  'rows', jsonb_build_object(
    'node', (SELECT count(*) FROM clinical.node_hot),
    'composition_versions', (SELECT count(*) FROM clinical.version_hot WHERE kind = 'COMPOSITION'),
    'audit_records', (SELECT count(*) FROM audit.audit_event)),
  'relations', (
    SELECT jsonb_object_agg(n.nspname || '.' || c.relname, jsonb_build_object(
      'heap', pg_relation_size(c.oid),
      'toast', COALESCE(pg_total_relation_size(NULLIF(c.reltoastrelid, 0)), 0),
      'indexes', pg_indexes_size(c.oid)))
    FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE c.relkind = 'r' AND n.nspname IN ('clinical', 'party', 'audit', 'ext')))::text;
"

# report SNAPSHOTS: the per-unit report of a snapshots file, on stdout.
report() {
  local snapshots="$1"
  jq '
    def rel($a; $b; $k): (($b.relations[$k] // {heap: 0, toast: 0, indexes: 0}) as $y
      | ($a.relations[$k] // {heap: 0, toast: 0, indexes: 0}) as $x
      | {heap: ($y.heap - $x.heap), toast: ($y.toast - $x.toast), indexes: ($y.indexes - $x.indexes)});
    def per($n): if $n > 0 then (. / $n | round) else null end;
    def unit($a; $b; $n):
      ([$b.relations | keys[] | . as $k | {key: $k, value: rel($a; $b; $k)}
        | select(.value.heap + .value.toast + .value.indexes > 0)] | from_entries) as $grew
      | {count: $n,
         clinical_bytes: ([$grew | to_entries[] | select(.key | startswith("clinical."))
                           | .value.heap + .value.toast + .value.indexes] | add // 0 | per($n)),
         audit_bytes: ([$grew | to_entries[] | select(.key | startswith("audit."))
                        | .value.heap + .value.toast + .value.indexes] | add // 0 | per($n)),
         database_bytes: (($b.database_bytes - $a.database_bytes) | per($n)),
         wal_bytes: (($b.wal_lsn_bytes - $a.wal_lsn_bytes) | per($n)),
         relations: ($grew | map_values(map_values(per($n)))) };
    .workload as $w
    | (.snapshots | map({key: .phase, value: .}) | from_entries) as $s
    | {workload: $w,
       per_unit: {
         create: unit($s.ehrs; $s.creates; $w.creates),
         update: unit($s.creates; $s.updates; $w.updates),
         delete: unit($s.updates; $s.deletes; $w.deletes),
         read: unit($s.deletes; $s.reads; $w.reads)},
       node: (rel($s.ehrs; $s.creates; "clinical.node_hot") as $g
         | ($s.creates.rows.node - $s.ehrs.rows.node) as $rows
         | {rows_per_version: (if $w.creates > 0 then ($rows / $w.creates * 10 | round / 10) else null end),
            heap_bytes_per_row: ($g.heap | per($rows)),
            index_bytes_per_row: ($g.indexes | per($rows))})}
  ' "$snapshots"
}

# The phase driver. Each request function prints what the next phase needs.
http() {
  curl -sS --fail-with-body -u "$SIZING_USER:$SIZING_PASSWORD" "$@"
}

header_of() {
  local name="$1"
  tr -d '\r' | sed -n "s/^$name: *//Ip" | tail -n 1
}

create_ehr() {
  http -o /dev/null -D - -X POST -H 'Prefer: return=minimal' "$SIZING_BASE_URL/ehr" \
    | header_of location | sed 's#.*/##'
}

create_composition() {
  local ehr="$1"
  printf '%s %s\n' "$ehr" "$(http -o /dev/null -D - -X POST -H 'Content-Type: application/json' \
    -H 'Prefer: return=minimal' --data-binary "@$SIZING_COMPOSITION" \
    "$SIZING_BASE_URL/ehr/$ehr/composition" | header_of etag | sed 's#^W/##; s#"##g')"
}

update_composition() {
  local ehr="$1" uid="$2"
  printf '%s %s\n' "$ehr" "$(http -o /dev/null -D - -X PUT -H 'Content-Type: application/json' \
    -H 'Prefer: return=minimal' -H "If-Match: \"$uid\"" --data-binary "@$SIZING_COMPOSITION" \
    "$SIZING_BASE_URL/ehr/$ehr/composition/${uid%%::*}" | header_of etag | sed 's#^W/##; s#"##g')"
}

delete_composition() {
  local ehr="$1" uid="$2"
  http -o /dev/null -X DELETE "$SIZING_BASE_URL/ehr/$ehr/composition/$uid"
}

read_composition() {
  local ehr="$1" uid="$2"
  http -o /dev/null -H 'Accept: application/json' "$SIZING_BASE_URL/ehr/$ehr/composition/${uid%%::*}"
}

# parallel FN: FN with the words of each line of stdin, SIZING_WORKERS at a
# time, each in its own run of this script (the internal `request` command).
parallel() {
  local fn="$1"
  xargs -P "$SIZING_WORKERS" -L 1 "${BASH_SOURCE[0]}" request "$fn"
}

run() {
  local out="$1"
  : "${SIZING_BASE_URL:=http://localhost:8080/ferroehr/rest/openehr/v1}"
  : "${SIZING_USER:=ferroehr}" "${SIZING_PASSWORD:=ferroehr}"
  : "${SIZING_PSQL:=docker compose exec -T ferroehr-postgres psql -U postgres -d ferroehr}"
  : "${SIZING_TEMPLATE:=vital-signs}" "${SIZING_WORKERS:=8}"
  : "${SIZING_EHRS:=300}" "${SIZING_PER_EHR:=10}" "${SIZING_UPDATES:=750}"
  : "${SIZING_DELETES:=300}" "${SIZING_READS:=6000}"
  local opt="$ROOT_DIR/corpus/templates/ckm/$SIZING_TEMPLATE.opt"
  SIZING_COMPOSITION="$ROOT_DIR/corpus/templates/ckm/$SIZING_TEMPLATE.example.json"
  [[ -f "$opt" && -f "$SIZING_COMPOSITION" ]] || {
    echo "error: no template '$SIZING_TEMPLATE' under corpus/templates/ckm" >&2
    exit 2
  }
  local creates=$((SIZING_EHRS * SIZING_PER_EHR))
  if ((SIZING_UPDATES > creates || SIZING_DELETES > creates || SIZING_EHRS < 1 || SIZING_PER_EHR < 1)); then
    echo "error: updates and deletes take at most the $creates creates, and EHRs and compositions per EHR at least 1" >&2
    exit 2
  fi
  export SIZING_BASE_URL SIZING_USER SIZING_PASSWORD SIZING_COMPOSITION
  mkdir -p "$out"
  local snaps="$out/snapshots.ndjson" work="$out/work"
  mkdir -p "$work"
  : >"$snaps"
  local -a psql
  read -ra psql <<<"$SIZING_PSQL"
  snapshot() {
    local phase="$1" s
    s="$("${psql[@]}" -At -v ON_ERROR_STOP=1 -c 'ANALYZE' -c 'CHECKPOINT' -c "$SNAPSHOT_SQL" | tail -n 1)"
    jq -c --arg phase "$phase" '{phase: $phase} + .' <<<"$s" >>"$snaps"
    echo "  $phase: database $(jq -r .database_bytes <<<"$s") bytes"
  }
  snapshot start
  local status
  status="$(curl -sS -o /dev/null -w '%{http_code}' -u "$SIZING_USER:$SIZING_PASSWORD" -X POST \
    -H 'Content-Type: application/xml' --data-binary "@$opt" "$SIZING_BASE_URL/definition/template/adl1.4")"
  [[ "$status" == 201 || "$status" == 204 || "$status" == 409 ]] || {
    echo "error: the template upload answered $status" >&2
    exit 1
  }
  snapshot template
  seq "$SIZING_EHRS" | parallel create_ehr >"$work/ehrs"
  snapshot ehrs
  for _ in $(seq "$SIZING_PER_EHR"); do cat "$work/ehrs"; done | parallel create_composition >"$work/created"
  snapshot creates
  head -n "$SIZING_UPDATES" "$work/created" | parallel update_composition >"$work/updated"
  snapshot updates
  # The deletes take the first rows of the update list, then the creates
  # that were not updated, so each deletes the latest version of its object.
  { cat "$work/updated"; tail -n +"$((SIZING_UPDATES + 1))" "$work/created"; } >"$work/latest"
  head -n "$SIZING_DELETES" "$work/latest" | parallel delete_composition
  snapshot deletes
  tail -n +"$((SIZING_DELETES + 1))" "$work/latest" >"$work/live"
  [[ -s "$work/live" ]] || { echo "error: the deletes left no composition to read" >&2; exit 2; }
  local lines
  lines="$(wc -l <"$work/live")"
  for i in $(seq 0 $((SIZING_READS - 1))); do sed -n "$((i % lines + 1))p" "$work/live"; done \
    | parallel read_composition
  snapshot reads
  jq -s --arg template "$SIZING_TEMPLATE" --argjson ehrs "$SIZING_EHRS" --argjson creates "$creates" \
    --argjson updates "$SIZING_UPDATES" --argjson deletes "$SIZING_DELETES" --argjson reads "$SIZING_READS" \
    --argjson workers "$SIZING_WORKERS" \
    '{workload: {template: $template, ehrs: $ehrs, creates: $creates, updates: $updates,
                 deletes: $deletes, reads: $reads, workers: $workers},
      snapshots: .}' "$snaps" >"$out/snapshots.json"
  rm -rf "$work" "$snaps"
  report "$out/snapshots.json" >"$out/report.json"
  jq -r '
    "| per | clinical bytes | audit bytes | WAL bytes |", "|---|---|---|---|",
    (.per_unit | to_entries[] | "| \(.key) (\(.value.count)) | \(.value.clinical_bytes) | \(.value.audit_bytes) | \(.value.wal_bytes) |"),
    "", "Node rows per version \(.node.rows_per_version), \(.node.heap_bytes_per_row) B heap and \(.node.index_bytes_per_row) B of index a row."
  ' "$out/report.json"
}

self_test() {
  local dir
  dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN
  # Two phases' snapshots shaped as `run` writes them: 10 creates that add
  # 430 node rows, then 20 reads that add only audit records.
  cat >"$dir/snapshots.json" <<'EOF'
{"workload": {"template": "vital-signs", "ehrs": 2, "creates": 10, "updates": 0, "deletes": 0, "reads": 20, "workers": 8},
 "snapshots": [
  {"phase": "ehrs", "database_bytes": 1000000, "wal_lsn_bytes": 5000,
   "rows": {"node": 4, "composition_versions": 0, "audit_records": 2},
   "relations": {"clinical.node_hot": {"heap": 8192, "toast": 0, "indexes": 16384},
                 "clinical.version_hot": {"heap": 8192, "toast": 0, "indexes": 8192},
                 "audit.audit_event": {"heap": 8192, "toast": 0, "indexes": 8192}}},
  {"phase": "creates", "database_bytes": 1200000, "wal_lsn_bytes": 405000,
   "rows": {"node": 434, "composition_versions": 10, "audit_records": 12},
   "relations": {"clinical.node_hot": {"heap": 110592, "toast": 0, "indexes": 53248},
                 "clinical.version_hot": {"heap": 32768, "toast": 0, "indexes": 12288},
                 "audit.audit_event": {"heap": 16384, "toast": 0, "indexes": 12288}}},
  {"phase": "updates", "database_bytes": 1200000, "wal_lsn_bytes": 405000,
   "rows": {"node": 434, "composition_versions": 10, "audit_records": 12},
   "relations": {"clinical.node_hot": {"heap": 110592, "toast": 0, "indexes": 53248},
                 "clinical.version_hot": {"heap": 32768, "toast": 0, "indexes": 12288},
                 "audit.audit_event": {"heap": 16384, "toast": 0, "indexes": 12288}}},
  {"phase": "deletes", "database_bytes": 1200000, "wal_lsn_bytes": 405000,
   "rows": {"node": 434, "composition_versions": 10, "audit_records": 12},
   "relations": {"clinical.node_hot": {"heap": 110592, "toast": 0, "indexes": 53248},
                 "clinical.version_hot": {"heap": 32768, "toast": 0, "indexes": 12288},
                 "audit.audit_event": {"heap": 16384, "toast": 0, "indexes": 12288}}},
  {"phase": "reads", "database_bytes": 1220480, "wal_lsn_bytes": 505000,
   "rows": {"node": 434, "composition_versions": 10, "audit_records": 32},
   "relations": {"clinical.node_hot": {"heap": 110592, "toast": 0, "indexes": 53248},
                 "clinical.version_hot": {"heap": 32768, "toast": 0, "indexes": 12288},
                 "audit.audit_event": {"heap": 32768, "toast": 0, "indexes": 16384}}}]}
EOF
  local got want
  got="$(report "$dir/snapshots.json" | jq -c '{
    create: (.per_unit.create | {count, clinical_bytes, audit_bytes, database_bytes, wal_bytes,
             node: .relations["clinical.node_hot"]}),
    update: .per_unit.update.count, read: (.per_unit.read | {clinical_bytes, audit_bytes, wal_bytes}),
    node}')"
  want='{"create":{"count":10,"clinical_bytes":16794,"audit_bytes":1229,"database_bytes":20000,"wal_bytes":40000,"node":{"heap":10240,"toast":0,"indexes":3686}},"update":0,"read":{"clinical_bytes":0,"audit_bytes":1024,"wal_bytes":5000},"node":{"rows_per_version":43,"heap_bytes_per_row":238,"index_bytes_per_row":86}}'
  if [[ "$got" != "$want" ]]; then
    echo "self-test FAILED: the report of the fixture is" >&2
    echo "  $got" >&2
    echo "wanted" >&2
    echo "  $want" >&2
    return 1
  fi
  # The book's sizing section names this command, so it reproduces its figures.
  grep -q 'scripts/perf/storage-sizing.sh run' "$ROOT_DIR/website/book/src/operations.md" || {
    echo "self-test FAILED: website/book/src/operations.md does not name scripts/perf/storage-sizing.sh run" >&2
    return 1
  }
  echo "storage-sizing.sh self-test: the report shape and the book's pointer pass"
}

case "${1:-}" in
--self-test) self_test ;;
report) [[ "$#" -eq 2 ]] || usage; report "$2" ;;
run) [[ "$#" -eq 2 ]] || usage; run "$2" ;;
request)
  case "${2:-}" in
  create_ehr | create_composition | update_composition | delete_composition | read_composition)
    shift
    "$@"
    ;;
  *) usage ;;
  esac
  ;;
*) usage ;;
esac
