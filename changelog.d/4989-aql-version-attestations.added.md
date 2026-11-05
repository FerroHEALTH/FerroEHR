- AQL reads the attestations of a version: `v/attestations` and any
  `ATTESTATION` attribute below it (`is_pending`, `reason`, `proof`, `items`,
  `committer`, `time_committed`) under `FROM EHR e CONTAINS VERSION v CONTAINS
  COMPOSITION c`, with predicates on `attestations` and `EXISTS`, and
  `v/commit_audit/is_pending` and `v/commit_audit/reason` on a commit audit
  that is an `ATTESTATION`. `SELECT v/attestations` returns the list, and a
  comparison below `attestations` holds when any attestation matches, so
  `WHERE EXISTS v/attestations[is_pending=true] AND NOT EXISTS
  v/attestations[is_pending=false]` lists the versions still awaiting
  signature.
