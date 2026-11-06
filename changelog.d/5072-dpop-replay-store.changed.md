- FerroEHR records every DPoP proof it accepts in its PostgreSQL database
  until the proof's window ends, so a proof is accepted once across all
  replicas, and a proof replayed to another replica is refused there
  (RFC 9449 §11.1). If the database cannot record a proof, the request is
  refused with `503`. The Helm chart (22.1.0) no longer refuses an issuer with
  `sender_constraint: required` beside `replicaCount` above 1 or autoscaling.
