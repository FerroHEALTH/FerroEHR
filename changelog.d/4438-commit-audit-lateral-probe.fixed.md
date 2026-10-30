- The revision history and every point read of a version now reach the
  version's commit audit through a lateral subquery that probes
  `pk_commit_audit` by key once per version row, so stale statistics on
  the partitioned `version` table can no longer turn the read into a walk
  of the whole commit audit index (1 318 buffers against 21 for a
  four-version object beside 1 300 others).
