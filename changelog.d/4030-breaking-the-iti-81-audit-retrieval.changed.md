- **Breaking: the ITI-81 audit retrieval (`GET /fhir/r4/AuditEvent`) requires
  a `date` parameter** (#4030). The query is now read by `ihe-atna`'s
  `retrieve`, the family's audit crate, and the answer written by it:
  RESTful ATNA §3.81.4.1.2.1 has at least one `date` present, so a query
  without one is a `400` `OperationOutcome`. A `date` value takes every FHIR
  prefix (`eq` when none, so `date=2026-07-12` is that whole day, read in
  UTC), `patient`, `agent` and `entity` also answer to their `.identifier`
  forms, `_count=0` returns the total alone, and the Bundle carries `self` and
  `next` links. A parameter the local store cannot apply yet is a `400`
  `not-supported` where it was silently ignored. Upgrade: an API client, a
  patient portal on the subject-scoped audit role included, adds a date
  window to every retrieval (for example `date=ge2026-07-01`); the viewer's
  audit page sends the last 30 days when no window is set. `ihe-atna` and
  `ehds-logging` move to 0.2.8.
