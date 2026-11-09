- **Breaking: the outbound integrations take the family's endpoint table.**
  `[terminology]`, `[pixm]`, `[audit.fhir_feed]` and `[authz.abac.remote]`
  read where they call and
  how from one `[<x>.endpoint]` table, the same keys for every outbound
  integration of every FerroHEALTH product: `url`, `timeout`,
  `connect_timeout`, `retry` (`max_attempts`, `initial_backoff`,
  `max_backoff`), `tls` (`trust_roots_file`, `client_identity_file`,
  `min_version`) and `credentials` (HTTP Basic `user` with `password` or
  `password_file`, or the client credentials grant `[...credentials.oauth]`).
  `terminology.url`, `timeout` and `[terminology.oauth]` are
  `terminology.endpoint.url`, `timeout` and
  `[terminology.endpoint.credentials.oauth]`; `pixm.url`, `timeout`,
  `[pixm.tls]` and `[pixm.oauth]` are `pixm.endpoint.url`, `timeout`,
  `[pixm.endpoint.tls]` and `[pixm.endpoint.credentials.oauth]`;
  `audit.fhir_feed.url`, `[audit.fhir_feed.retry]` and
  `[audit.fhir_feed.oauth]` are `audit.fhir_feed.endpoint.url`,
  `[audit.fhir_feed.endpoint.retry]` and
  `[audit.fhir_feed.endpoint.credentials.oauth]`; `authz.abac.remote.url`,
  `connect_timeout`, `timeout` and `[authz.abac.remote.oauth]` are
  `authz.abac.remote.endpoint.url`, `connect_timeout`, `timeout` and
  `[authz.abac.remote.endpoint.credentials.oauth]`. The v5.6 keys are refused
  naming their new home. Every client now honours `retry`: the terminology,
  PIXm and remote policy clients try a call that got no answer or a `5xx` up
  to `max_attempts` times (three by default; each tried once before), so set
  `max_attempts = 1` to keep one attempt, and the FHIR feed's delivery rounds
  take `max_attempts` tries each, from `initial_backoff`, before the round
  waits `max_backoff`; the feed still never gives a record up. The environment
  follows the tree (`FERROEHR__TERMINOLOGY__ENDPOINT__URL`,
  `FERROEHR__AUDIT__FHIR_FEED__ENDPOINT__URL`), and so do the Helm chart's
  values and its client secret routing.
