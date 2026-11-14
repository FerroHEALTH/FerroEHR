- Metrics for the access path, with a runbook in the book for each failure an
  operator can see. The policy copies report each listener's state
  (`policy_listener_up`), its lost connections, its notifications and the
  time since the newest one, the reads a request made because a listener was
  down, and the refresh time of the grants and caller facts. The per-EHR
  access copy and the class copy report their size, hits, misses, refills and
  drops. The access evaluator counts its decisions by outcome and `refused-*`
  code (`access_decisions`) and each emergency override by what it lifted.
  The access log reports its appends, the latency and failures of link
  signing, of each checkpoint and system chain seal and of each retention run,
  the tombstones written, and the depth of the audit drain's queue. The
  scheduled chain verification reports its duration and result, the time of
  the newest run and whether the chains read intact (`audit_chain_intact`).
  No label carries a patient, an EHR id or clinical content. The new book page
  "Access path metrics and runbooks" covers a lost listener, a copy that does
  not refill, the audit drain backed up under `fail_mode = "closed"`, a failed
  chain verification, a signing key mismatch, signing key rotation, a failing
  seal and a retention run that falls behind.
