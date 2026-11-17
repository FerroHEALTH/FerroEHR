#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The capacity test's verdicts and reports (#5777), sourced by
# scripts/perf/capacity.sh and never run directly: a step's runs folded into
# one verdict with each p99's median and spread, the summary's tables, and
# the comparison of two arms that calls a difference only outside both
# spreads. No openEHR spec governs this: our own design.

# The jq helpers every report shares: the median of a list (the mean of the
# two middle values of an even one) and a number rounded to two decimals.
readonly REPORT_JQ='
  def median: map(select(. != null)) | sort
    | if length == 0 then null
      elif length % 2 == 1 then .[(length - 1) / 2]
      else (.[length / 2 - 1] + .[length / 2]) / 2 end;
  def r2: if . == null then "-" else (. * 100 | round / 100 | tostring) end;
  def mb: if . == null then "-" else (. / 1048576 | round | tostring) end;
'

# fold_runs <arm> <run-step.json>...: one verdict over a step's runs. Each
# latency is the median over the runs, a p99 carries every run's value and
# their spread, and the step passes when every run kept its schedule and
# answered as expected and each group's median p99, client and server, lies
# within its SLO.
fold_runs() {
  local arm="$1"
  shift
  jq -s --arg arm "$arm" "$REPORT_JQ"'
    . as $runs
    | def over(f): [$runs[] | f] | median;
    ($runs[0].groups | map(.group as $g
        | [$runs[] | .groups[] | select(.group == $g)] as $gs
        | {group: $g, slo: .slo, answered: ([$gs[].answered] | add),
           p50: ([$gs[].p50] | median), p95: ([$gs[].p95] | median), p99: ([$gs[].p99] | median),
           p99_runs: [$gs[].p99], p99_spread: [([$gs[].p99] | min), ([$gs[].p99] | max)],
           server_group: .server_group,
           server_coverage: ([$gs[].server_coverage | select(. != null)] | if length > 0 then min else null end),
           server_p50: ([$gs[].server_p50] | median), server_p95: ([$gs[].server_p95] | median),
           server_p99: ([$gs[].server_p99] | median), server_p99_runs: [$gs[].server_p99],
           server_p99_spread: ([$gs[].server_p99 | select(. != null)] | if length > 0 then [min, max] else null end)})) as $groups
    | $runs[-1] + {
        arm: $arm, runs: ($runs | length), run_verdicts: [$runs[].passed],
        throughput: over(.throughput), answered: ([$runs[].answered] | add),
        failed: ([$runs[].failed] | add), saturated: any($runs[]; .saturated),
        lag_p99_ms: ([$runs[].lag_p99_ms] | max),
        groups: $groups,
        server_cores: over(.server_cores), pg_cores: over(.pg_cores),
        server_cpu_ms_per_request: over(.server_cpu_ms_per_request),
        pg_cpu_ms_per_request: over(.pg_cpu_ms_per_request),
        transactions_per_request: over(.transactions_per_request),
        host_load_1m: ([$runs[].host_load_1m] | max),
        kernel_load_1m_max: ([$runs[].host | (.end_kernel_load // [null])[0]] | map(select(. != null)) | max),
        other_cores_max: ([$runs[].host.other_cores] | max),
        memory_peak: {
          server_rss_bytes: ([$runs[].memory[] | select(.role == "server") | .rss_peak_bytes] | map(select(. != null)) | max),
          server_cgroup_bytes: ([$runs[].memory[] | select(.role == "server") | .cgroup_peak_bytes] | map(select(. != null)) | max),
          postgres_cgroup_bytes: ([$runs[].memory[] | select(.role == "postgres") | .cgroup_peak_bytes] | map(select(. != null)) | max)}}
    | .passed = (all($runs[]; .failed == 0 and (.saturated | not)
          and .lag_p99_ms != null and .lag_p99_ms <= .max_lag_ms and .throughput >= 0.98 * .rate)
        and all(.groups[]; (.p99 // .p95) != null and (.p99 // .p95) <= .slo
          and (.server_p99 == null or .server_p99 <= .slo)))' "$@"
}

# summary: the tables of every step run so far, the latest step of each arm,
# group and rate, with the forwarder floor and the noise each step ran in.
summary() {
  [[ -f "$CAP_OUT_ABS/steps.jsonl" ]] || cap_die "no step has run"
  jq -rs "$REPORT_JQ"'
    (group_by([.arm, .what, .rate]) | map(.[-1]) | sort_by(.arm, .what, .rate)) as $steps
    | ($steps | map(select(.what == "health"))) as $health
    | "### Steps",
      "",
      "Latencies are milliseconds: the generator'"'"'s (client) and the server'"'"'s own request log (server), each the median over the runs, with the spread of the p99 over the runs. The coverage is the share of answered requests the request log holds in the window (the lowest run); below 1 the server'"'"'s figures are a sample.",
      "",
      "| arm | what | instances | rate | runs | answered rps | verdict | failed | worst group | client p50 / p95 / p99 | p99 spread | server p50 / p95 / p99 | server log coverage | server cores | server CPU ms/req | PostgreSQL cores | PG CPU ms/req | xact/req |",
      "|---|---|---:|---:|---:|---:|---|---:|---|---|---|---|---:|---:|---:|---:|---:|---:|",
      ($steps[] | (.groups | max_by(.p99 // 1e9)) as $w
        | "| \(.arm // "") | \(.what) | \(.instances) | \(.rate) | \(.runs // 1) | \(.throughput | . * 10 | floor / 10) | \(if .passed then "pass" else "FAIL" end) | \(.failed) | \($w.group) | \($w.p50 | r2) / \($w.p95 | r2) / \($w.p99 | r2) | \(($w.p99_spread // [$w.p99, $w.p99]) | map(r2) | join(" to ")) | \($w.server_p50 | r2) / \($w.server_p95 | r2) / \($w.server_p99 | r2) | \($w.server_coverage | r2) | \(.server_cores | r2) | \(.server_cpu_ms_per_request | r2) | \(.pg_cores | r2) | \(.pg_cpu_ms_per_request | r2) | \(.transactions_per_request | r2) |"),
      "",
      "### The forwarder floor",
      "",
      (if ($health | length) == 0 then
        "No `health` step ran, so this run names no forwarder floor: `capacity.sh sweep health 500 1000` measures it."
      else
        ($health[] | (.groups[0]) as $g
          | "- \(.arm // "" | if . == "" then "" else . + ": " end)at \(.rate) rps the client p50 of `/health/liveness` is \($g.p50 | r2) ms and the server'"'"'s \($g.server_p50 | r2) ms: about \(if $g.server_p50 == null then $g.p50 else $g.p50 - $g.server_p50 end | r2) ms of every client latency in this run is the generator'"'"'s own scheduling lag (p99 \(.lag_p99_ms | r2) ms) and the path to the server (generator: \(.generator // "host")).")
      end),
      "",
      "### The host",
      "",
      "A step whose host load exceeded its cores, or whose other containers used a core or more, ran on a shared host: read its latency as an upper bound.",
      "",
      "| arm | what | rate | host load (1 min, worst run) / cores | containers'"'"' kernel load / cores | clock source | other containers, cores (worst run) | busiest other containers |",
      "|---|---|---:|---|---|---|---:|---|",
      ($steps[] | "| \(.arm // "") | \(.what) | \(.rate) | \(.host_load_1m | r2) / \(.host_cores) | \(.kernel_load_1m_max | r2) / \(.host.end_kernel_cores // "-") | \(.host.end_clocksource // "-") | \(.other_cores_max | r2) | \(.host.other_containers // [] | sort_by(-.cores) | map(select(.cores >= 0.05)) | .[:3] | map("\(.name) \(.cores | r2)") | join(", ")) |"),
      "",
      "### Memory and storage",
      "",
      "Memory is in MiB: the peak since each process started of the server'"'"'s resident set (VmHWM) and cgroup and of PostgreSQL'"'"'s cgroup (page cache included), and the resident set and in-memory copies the server reports of itself at the end of the step. Storage is the store at the end of the step, and what the step itself added.",
      "",
      "| arm | what | rate | server peak RSS | server RSS at the end (its own report) | in-memory copies: entries, KiB | server cgroup peak | PostgreSQL cgroup peak | database MiB | bytes per composition version | bytes per access-log entry | step: versions, bytes each | step: entries, bytes each |",
      "|---|---|---:|---:|---:|---|---:|---:|---:|---:|---:|---|---|",
      ($steps[] | "| \(.arm // "") | \(.what) | \(.rate) | \(.memory_peak.server_rss_bytes | mb) | \([.server_metrics[]?.rss_bytes] | max | mb) | \([.server_metrics[]?.copy_entries] | add // "-"), \([.server_metrics[]?.copy_bytes] | add | if . == null then "-" else (. / 1024 | round | tostring) end) | \(.memory_peak.server_cgroup_bytes | mb) | \(.memory_peak.postgres_cgroup_bytes | mb) | \(.storage.database_bytes | mb) | \(.storage.bytes_per_composition_version // "-") | \(.storage.bytes_per_access_log_entry // "-") | \(.storage.step.composition_versions // "-"), \(.storage.step.bytes_per_composition_version // "-") | \(.storage.step.access_log_entries // "-"), \(.storage.step.bytes_per_access_log_entry // "-") |"),
      "",
      "PostgreSQL runs with `track_io_timing = on` and `pg_stat_statements.track_planning = on`: on a clock source that traps to the hypervisor (`arch_sys_counter` in a VM) each of their clock reads costs PostgreSQL CPU, so a figure from such a host overstates PostgreSQL'"'"'s CPU per request."
  ' "$CAP_OUT_ABS/steps.jsonl"
  soak_summary
  [[ -f "$CAP_OUT_ABS/sweeps.jsonl" ]] || return 0
  echo
  echo "### Sweep outcomes"
  echo
  jq -rs '"| arm | what | rate | outcome |", "|---|---|---:|---|",
    (.[] | "| \(.arm) | \(.what) | \(.rate) | \(.outcome) |")' "$CAP_OUT_ABS/sweeps.jsonl"
}

# compare <arm> <arm>: every step both arms ran, group by group, with each
# arm's median p99 and spread. A difference is called only when the two
# spreads do not overlap and both arms ran at least three times.
compare() {
  [[ -f "$CAP_OUT_ABS/steps.jsonl" ]] || cap_die "no step has run"
  jq -rs --arg a "$1" --arg b "$2" "$REPORT_JQ"'
    def last($arm): map(select(.arm == $arm)) | group_by([.what, .rate]) | map(.[-1]);
    def spread: (.p99_spread // [.p99, .p99]);
    def call($x; $y): if ($x.runs // 1) < 3 or ($y.runs // 1) < 3 then "fewer than 3 runs: no call"
      elif ($y.g | spread)[0] > ($x.g | spread)[1] then "\($b) higher"
      elif ($y.g | spread)[1] < ($x.g | spread)[0] then "\($b) lower"
      else "within the spread" end;
    (last($a)) as $as | (last($b)) as $bs
    | "| what | rate | group | \($a) p99 (spread) | \($b) p99 (spread) | \($a) server p99 | \($b) server p99 | call |",
      "|---|---:|---|---|---|---:|---:|---|",
      ($as[] as $x | ($bs[] | select(.what == $x.what and .rate == $x.rate)) as $y
        | $x.groups[] as $gx | ($y.groups[] | select(.group == $gx.group)) as $gy
        | "| \($x.what) | \($x.rate) | \($gx.group) | \($gx.p99 | r2) (\($gx | spread | map(r2) | join(" to "))) | \($gy.p99 | r2) (\($gy | spread | map(r2) | join(" to "))) | \($gx.server_p99 | r2) | \($gy.server_p99 | r2) | \(call({runs: $x.runs, g: $gx}; {runs: $y.runs, g: $gy})) |")
  ' "$CAP_OUT_ABS/steps.jsonl"
}

# soak_summary: every soak run so far, each instance's resident set over the
# soak with its slope after the settling, against the stated tolerance.
soak_summary() {
  [[ -f "$CAP_OUT_ABS/soaks.jsonl" ]] || return 0
  echo
  echo "### Soak"
  echo
  echo "Each instance's resident set as the server reports it, sampled through the soak; the slope is fitted to the samples after the warm-up and the settling. A soak fails when an instance's slope exceeds the tolerance and its last sample is above its first in the fit."
  echo
  jq -rs "$REPORT_JQ"'
    "| arm | what | rate | hours | instance | RSS first / last / peak MiB | slope MiB per hour (tolerance) | copies at the end: entries, KiB | verdict |",
    "|---|---|---:|---:|---:|---|---|---|---|",
    (.[] | . as $s | .memory.instances[]
      | "| \($s.arm) | \($s.what) | \($s.rate) | \($s.seconds / 3600 | r2) | \(.instance) | \(.rss_first_bytes | mb) / \(.rss_last_bytes | mb) / \(.rss_peak_bytes | mb) | \(.slope_mib_per_hour | r2) (\($s.memory.max_mib_per_hour)) | \(.copy_entries_last), \(.copy_bytes_last / 1024 | round) | \(if .passed == null then "too few samples" elif .passed then "pass" else "FAIL: rising" end) |")' \
    "$CAP_OUT_ABS/soaks.jsonl"
}
