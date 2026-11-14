- The performance gate runs the access path at scale: an access policy that
  requires a care relationship, a clinician with a care team and a clearance,
  directives on every seeded EHR, and access-log links signed with an OpenPGP
  key the gate generates (`PERF_SIGNING_KEY`, `ed25519` by default or
  `rsa3072`). It sends the clinical groups as the clinician, adds an export and
  an access-log read, and reports each arm's access-log storage.
  `scripts/perf/gate.sh replicas` measures the reload load of one, two and
  three instances on one PostgreSQL. The measurements, with the evaluator's
  micro-benchmark (`cargo bench -p ferroehr --bench access_decision`) and the
  replica memory (`scaleseed replica`), are rendered into the book from
  `docs/benchmarks/access-path/record.json`.
