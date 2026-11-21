#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The capacity test's instruments beside the load (#5777), sourced by
# scripts/perf/capacity.sh and never run directly:
#   - the server's own latency per group, read from its request log over the
#     measured window, beside the generator's;
#   - the host and its other tenants: the load averages of the host and of
#     the kernel the containers run on, its clock source, and the CPU every
#     other container spent over the step;
#   - the peak memory of each instance and of PostgreSQL;
#   - the store's bytes per composition version and per access-log entry.
# No openEHR spec governs this: our own design.

# kernel_sh <script>: <script> run by the kernel the containers run on: this
# host on Linux, the Podman machine on macOS. Fails when neither is reachable.
kernel_sh() {
  if [[ "$(uname -s)" == Linux ]]; then
    bash -c "$1"
  elif command -v podman >/dev/null; then
    podman machine ssh -- "$1"
  else
    return 1
  fi
}

# The shell function every kernel script that reads a container's cgroup
# defines first: `cg <id>` prints the cgroup directory of that container.
# shellcheck disable=SC2016 # the script runs in the containers' kernel, which expands it
readonly KERNEL_CG='cg() { find /sys/fs/cgroup -maxdepth 4 -type d \( -name "libpod-$1.scope" -o -name "docker-$1.scope" -o -name "$1" \) 2>/dev/null | sed -n 1p; }'
# `mem <role> <n> <id> <pid>`: one line of a container's peak resident set
# and its cgroup's peak, current and anonymous bytes, `null` where unread.
# shellcheck disable=SC2016 # the script runs in the containers' kernel, which expands it
readonly KERNEL_MEM="$KERNEL_CG"'; mem() { d=$(cg "$3"); h=$(awk '\''$1 == "VmHWM:" { print $2 * 1024 }'\'' "/proc/$4/status" 2>/dev/null); p=$(cat "$d/memory.peak" 2>/dev/null); c=$(cat "$d/memory.current" 2>/dev/null); a=$(awk '\''$1 == "anon" { print $2 }'\'' "$d/memory.stat" 2>/dev/null); echo "$1 $2 ${h:-null} ${p:-null} ${c:-null} ${a:-null}"; }'

# load_averages: the 1, 5 and 15 minute load averages of this host.
load_averages() {
  uptime | sed -E 's/.*load averages?: *//; s/,/ /g' | awk '{ print $1, $2, $3 }'
}

# host_snapshot <out>: the host's load and cores, and, where the containers'
# kernel is reachable, its load, cores and clock source and every
# container's CPU microseconds, one fact per line.
host_snapshot() {
  {
    echo "host_load $(load_averages)"
    echo "host_cores $(getconf _NPROCESSORS_ONLN)"
    # A native topology on macOS has no containers' kernel to read.
    # shellcheck disable=SC2016 # the script runs in the containers' kernel, which expands it
    kernel_sh 'echo "kernel_load $(cut -d" " -f1-3 /proc/loadavg)"
      echo "kernel_cores $(nproc)"
      echo "clocksource $(cat /sys/devices/system/clocksource/clocksource0/current_clocksource)"
      find /sys/fs/cgroup -maxdepth 3 -type d \( -name "libpod-*.scope" -o -name "docker-*.scope" \) \
        ! -name "*conmon*" 2>/dev/null | while read -r d; do
        id=${d##*/}; id=${id#libpod-}; id=${id#docker-}; id=${id%.scope}
        awk -v id="$id" '\''$1 == "usage_usec" { print "cpu", id, $2 }'\'' "$d/cpu.stat"
      done' 2>/dev/null || true
  } >"$1"
}

# host_json <start> <end> <seconds>: the host record of a step from the
# snapshots taken before and after it: the loads at both ends, the clock
# source, and each container outside this harness with the cores it used.
host_json() {
  # The first line holds the file's place when docker answers nothing.
  { echo "- - -"; docker ps -a --no-trunc --format '{{.ID}} {{.Names}} {{.Image}}' 2>/dev/null || true; } >"$1.names"
  awk -v secs="$3" -v ours="$CAP_SERVER" '
    function arr(a, n, i, s) { s = "["; for (i = 2; i <= n; i++) s = s (i > 2 ? "," : "") a[i]; return s "]" }
    FNR == 1 { file++ }
    file == 1 { name[$1] = $2; image[$1] = $3; next }
    $1 == "cpu" { if (file == 2) before[$2] = $3; else after[$2] = $3; next }
    { key = (file == 2 ? "start_" : "end_") $1; m = split($0, v, " "); val[key] = (m > 2 ? arr(v, m) : (v[2] ~ /^[0-9.]+$/ ? v[2] : "\"" v[2] "\"")) }
    END {
      printf "{"
      sep = ""
      for (k in val) { printf "%s\"%s\":%s", sep, k, val[k]; sep = "," }
      printf "%s\"other_containers\":[", sep
      sep = ""; total = 0
      for (id in after) {
        if (!(id in name) || index(name[id], ours) == 1) continue
        used = (after[id] - (id in before ? before[id] : 0)) / 1e6
        if (used < 0) used = 0
        total += used
        printf "%s{\"name\":\"%s\",\"image\":\"%s\",\"cpu_s\":%.3f,\"cores\":%.3f}", sep, name[id], image[id], used, (secs > 0 ? used / secs : 0)
        sep = ","
      }
      printf "],\"other_cores\":%.3f}\n", (secs > 0 ? total / secs : 0)
    }' "$1.names" "$1" "$2" | jq -S .
}

# memory_json <server-spec>... -- <pg-spec>: the peak memory of each instance
# and of PostgreSQL: the peak resident set of the server process (VmHWM)
# and, for a container, its cgroup's peak, current and anonymous bytes.
memory_json() {
  local spec role=server n=0 script="$KERNEL_MEM" lines="" pid id
  for spec in "$@"; do
    if [[ "$spec" == -- ]]; then role=postgres; n=0; continue; fi
    n=$((n + 1))
    case "$spec" in
    container:*)
      id="${spec#container:}"
      pid=$(docker inspect -f '{{.State.Pid}}' "$id" 2>/dev/null || echo 0)
      script+="; mem $role $n $id $pid"
      ;;
    pid:*)
      pid="${spec#pid:}"
      if [[ -r "/proc/$pid/status" ]]; then
        lines+="$role $n $(awk '$1 == "VmHWM:" { print $2 * 1024 }' "/proc/$pid/status") null null null"$'\n'
      else
        # macOS has no peak resident set of another process: the current one.
        lines+="$role $n null null $(($(ps -o rss= -p "$pid" | tr -d ' ') * 1024)) null"$'\n'
      fi
      ;;
    children:*) lines+="$role $n null null null null"$'\n' ;;
    esac
  done
  [[ "$script" == "$KERNEL_MEM" ]] || lines+="$(kernel_sh "$script" 2>/dev/null || true)"
  awk 'NF == 6 { for (i = 3; i <= 6; i++) if ($i == "" || $i !~ /^[0-9]+$/) $i = "null"
      printf "{\"role\":\"%s\",\"instance\":%s,\"rss_peak_bytes\":%s,\"cgroup_peak_bytes\":%s,\"cgroup_current_bytes\":%s,\"anon_bytes\":%s}\n", $1, $2, $3, $4, $5, $6 }' \
    <<<"$lines" | jq -s .
}

# storage_json: the store's size, each domain schema's bytes (tables with
# their indexes and TOAST), the composition versions and the access-log
# entries it holds, and the bytes the access log's tables hold.
storage_json() {
  local entries="0" versions="0"
  [[ "$(cap_sql "SELECT to_regclass('audit.access_log_record') IS NOT NULL")" == t ]] \
    && entries="(SELECT count(*) FROM audit.access_log_record)"
  [[ "$(cap_sql "SELECT to_regclass('audit.access_log_intake') IS NOT NULL")" == t ]] \
    && entries+=" + (SELECT count(*) FROM audit.access_log_intake)"
  [[ "$(cap_sql "SELECT to_regclass('clinical.version_hot') IS NOT NULL")" == t ]] \
    && versions="(SELECT count(*) FROM clinical.version_hot WHERE kind = 'COMPOSITION')"
  cap_sql "SELECT json_build_object(
      'database_bytes', pg_database_size(current_database()),
      'schema_bytes', (SELECT coalesce(json_object_agg(nspname, bytes), '{}') FROM (
        SELECT n.nspname, sum(pg_total_relation_size(c.oid))::bigint AS bytes FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE c.relkind IN ('r', 'm') AND n.nspname IN ('clinical', 'party', 'audit') GROUP BY 1) s),
      'access_log_bytes', (SELECT coalesce(sum(pg_total_relation_size(c.oid)), 0)::bigint FROM pg_class c
        JOIN pg_namespace n ON n.oid = c.relnamespace
        WHERE c.relkind = 'r' AND n.nspname = 'audit' AND c.relname LIKE 'access_log%'),
      'composition_versions', $versions,
      'access_log_entries', $entries)"
}

# storage_step_json <before> <after>: the store at the end of a step, the
# bytes per entry it holds, and what the step added.
storage_step_json() {
  jq -n --slurpfile b "$1" --slurpfile a "$2" '
    def per($bytes; $n): if $n > 0 then ($bytes / $n | round) else null end;
    $b[0] as $b | $a[0] as $a
    | $a + {
        bytes_per_composition_version: per($a.schema_bytes.clinical // 0; $a.composition_versions),
        bytes_per_access_log_entry: per($a.access_log_bytes; $a.access_log_entries),
        step: {
          database_bytes: ($a.database_bytes - $b.database_bytes),
          clinical_bytes: (($a.schema_bytes.clinical // 0) - ($b.schema_bytes.clinical // 0)),
          audit_bytes: (($a.schema_bytes.audit // 0) - ($b.schema_bytes.audit // 0)),
          composition_versions: ($a.composition_versions - $b.composition_versions),
          access_log_entries: ($a.access_log_entries - $b.access_log_entries),
          bytes_per_composition_version: per(($a.schema_bytes.clinical // 0) - ($b.schema_bytes.clinical // 0);
            $a.composition_versions - $b.composition_versions),
          bytes_per_access_log_entry: per($a.access_log_bytes - $b.access_log_bytes;
            $a.access_log_entries - $b.access_log_entries)}}'
}

# request_log <since> <server-spec>...: every instance's request log lines
# written since the epoch second <since>.
request_log() {
  local since="$1" spec i=1
  shift
  for spec in "$@"; do
    case "$spec" in
    container:*) docker logs --since "$since" "${spec#container:}" 2>&1 || true ;;
    pid:*) cat "$CAP_OUT_ABS/server-$i.log" 2>/dev/null || true ;;
    esac
    i=$((i + 1))
  done
}

# server_latency <what> <since> <warmup> <duration> <server-spec>...: the
# server's own p50, p95 and p99 per group over the step's measured window,
# from the request log's `latency_ms`. The window starts <warmup> seconds
# after the step's first request. The three AQL groups share one route, so
# in the mix their requests are one group, `aql`.
server_latency() {
  local what="$1" since="$2" warmup="$3" duration="$4"
  shift 4
  request_log "$since" "$@" | jq -R -s --arg what "$what" --argjson since "$since" \
    --argjson warmup "$warmup" --argjson duration "$duration" '
    def ts: .timestamp as $t
      | ($t | sub("\\.[0-9]+"; "") | fromdateiso8601) + (($t | capture("\\.(?<f>[0-9]+)") | "0." + .f | tonumber) // 0);
    def group: (.method + " " + .route) as $k
      | if ($k | test("^GET .*/health/liveness$")) then "health"
        elif ($k | test("^GET .*/ehr/\\{[^}/]+\\}$")) then "ehr_read"
        elif ($k | test("^GET .*/ehr/\\{[^}/]+\\}/composition/\\{[^}/]+\\}$")) then "composition_read"
        elif ($k | test("^POST .*/ehr/\\{[^}/]+\\}/composition$")) then "composition_commit"
        elif ($k | test("^POST .*/query/aql$")) then (if ($what | startswith("aql_")) then $what else "aql" end)
        elif ($k | test("^GET .*/AuditEvent$")) then "access_log_read"
        elif ($k | test("^GET .*/demographic/person/\\{[^}/]+\\}$")) then "party_read"
        elif ($k | test("^POST .*/demographic/party_relationship$")) then "care_commit"
        elif ($k | test("^GET .*/export/\\{[^}/]+\\}$")) then "export"
        else null end;
    def q($s; $p): $s[([($p * ($s | length) | ceil) - 1, 0] | max)];
    [split("\n")[] | fromjson? | select(type == "object" and .target == "ferrohealth_telemetry::request"
        and (.latency_ms | type) == "number" and (.route | type) == "string")
      | {t: ts, g: group, ms: .latency_ms}
      | select(.t >= $since and .g != null and ($what == "mix" or .g == $what))] as $lines
    | if ($lines | length) == 0 then [] else
        ([$lines[].t] | min + $warmup) as $from
        | [$lines[] | select(.t >= $from and .t < $from + $duration)]
        | group_by(.g)
        | map((map(.ms) | sort) as $s
            | {group: .[0].g, count: ($s | length), p50: q($s; 0.5), p95: q($s; 0.95), p99: q($s; 0.99)})
      end'
}

# metrics_json <prometheus-text>...: per instance, the memory the server
# reports of itself: its resident set (process_resident_memory_bytes) and
# each in-memory copy's entries and bytes (access_copy_entries,
# access_copy_size_bytes, by copy).
metrics_json() {
  local file n=0
  for file in "$@"; do
    n=$((n + 1))
    awk -v n="$n" '
      /^process_resident_memory_bytes[ {]/ { rss = $NF }
      /^access_copy_(entries|size_bytes)\{/ {
        kind = ($1 ~ /^access_copy_entries/ ? "entries" : "bytes")
        match($0, /copy="[^"]*"/); copy = substr($0, RSTART + 6, RLENGTH - 7)
        v[kind, copy] = $NF; copies[copy] = 1 }
      END {
        printf "{\"instance\":%d,\"rss_bytes\":%s,\"copies\":{", n, (rss == "" ? "null" : rss); sep = ""
        for (c in copies) {
          printf "%s\"%s\":{\"entries\":%s,\"bytes\":%s}", sep, c, ((("entries", c) in v) ? v["entries", c] : "null"), ((("bytes", c) in v) ? v["bytes", c] : "null"); sep = "," }
        print "}}" }' "$file"
  done | jq -s 'map(.copies |= with_entries(.) ) | map(. + {copy_bytes: ([.copies[].bytes | select(. != null)] | add // 0),
    copy_entries: ([.copies[].entries | select(. != null)] | add // 0)})'
}

# soak_sample <out> <stop> <interval>: every <interval> seconds until <stop>
# exists, one line per instance: the epoch second, the instance, its
# resident set and its copies' bytes and entries, as the server reports them.
soak_sample() {
  local out="$1" stop="$2" interval="$3" inst i tmp="$1.scrape"
  while [[ ! -e "$stop" ]]; do
    i=1
    for inst in $(state_get instance_urls); do
      scrape "$inst" "$tmp"
      metrics_json "$tmp" | jq -r --argjson t "$(date +%s)" --argjson i "$i" \
        '.[0] | "\($t)\t\($i)\t\(.rss_bytes // "")\t\(.copy_bytes)\t\(.copy_entries)"' >>"$out" 2>/dev/null || true
      i=$((i + 1))
    done
    sleep "$interval"
  done
  rm -f "$tmp"
}

# soak_json <samples> <from> <max-mib-per-hour>: per instance, the resident
# set over the soak and its least-squares slope over the samples from the
# epoch second <from> on (after the warm-up and the settling); an instance
# fails when that slope exceeds <max-mib-per-hour> and its last sample is
# above its first in the window, so a resident set that keeps rising shows.
soak_json() {
  awk -F'\t' -v from="$2" -v max="$3" '
    $3 == "" { next }
    { i = $2; inst[i] = 1; n_all[i]++; if (!(i in first_all)) first_all[i] = $3; last_all[i] = $3
      if ($3 > peak[i]) peak[i] = $3; cb[i] = $4; ce[i] = $5
      if ($1 < from) next
      if (!(i in t0)) { t0[i] = $1; w_first[i] = $3 }
      x = ($1 - t0[i]) / 3600; y = $3 / 1048576
      n[i]++; sx[i] += x; sy[i] += y; sxx[i] += x * x; sxy[i] += x * y; w_last[i] = $3; span[i] = x }
    END {
      printf "["; sep = ""
      for (i in inst) {
        d = n[i] * sxx[i] - sx[i] * sx[i]
        slope = (n[i] >= 3 && d > 0) ? (n[i] * sxy[i] - sx[i] * sy[i]) / d : "null"
        rising = (slope != "null" && slope > max && w_last[i] > w_first[i])
        printf "%s{\"instance\":%d,\"samples\":%d,\"fit_samples\":%d,\"fit_hours\":%.3f,\"rss_first_bytes\":%s,\"rss_last_bytes\":%s,\"rss_peak_bytes\":%s,\"slope_mib_per_hour\":%s,\"copy_bytes_last\":%s,\"copy_entries_last\":%s,\"passed\":%s}",
          sep, i, n_all[i], n[i] + 0, span[i] + 0, first_all[i], last_all[i], peak[i], slope, cb[i] + 0, ce[i] + 0,
          (slope == "null" ? "null" : (rising ? "false" : "true"))
        sep = "," }
      print "]" }' "$1" | jq --argjson max "$3" '{max_mib_per_hour: $max, instances: (sort_by(.instance)),
        passed: (if any(.[]; .passed == null) then null else all(.[]; .passed) end)}'
}
