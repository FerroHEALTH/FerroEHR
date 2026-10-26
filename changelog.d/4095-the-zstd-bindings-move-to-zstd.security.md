- The zstd bindings move to zstd 0.14.3, zstd-safe 8.2.0 and zstd-sys 2.1.2,
  past RUSTSEC-2026-0391, RUSTSEC-2026-0398, RUSTSEC-2026-0403 and
  RUSTSEC-2026-0410. Response compression and the outbound clients' response
  decompression reach these crates, and the affected functions were not shown
  unreachable (GHSA-cwhv-87pq-3c3c).
