- **The scheduled audit chain check verifies the records written since the
  last intact link** (#4100). `audit.verify_audit_chain_incremental()`
  recomputes only the records after the checkpoint it keeps in
  `audit.audit_chain_checkpoint`, checks the chain head, and moves the
  checkpoint when it finds nothing: on 428,000 records a full pass took
  about 35 s, a run over the last 1,000 records 76 ms and one with nothing
  new 2 ms. A full pass runs every `[audit.store]
  full_verify_interval_seconds` (new, default one week, `0` for on demand
  only) and after any run that found damage, and `SELECT * FROM
  audit.verify_audit_chain()` stays the full pass on demand. With the
  cheaper run, `verify_interval_seconds` defaults to one hour, where it was
  one day. The chain digest is unchanged.
