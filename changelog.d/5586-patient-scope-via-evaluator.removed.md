- **Breaking: the token-claim patient scope is gone, with `[auth]
  patient_claim` and `[auth] patient_namespace`.** Both are unknown keys that
  refuse the start, from the file and from the environment, with no
  migration aid. The configuration reference lists 736 keys, down from 738.
  The AQL subject filter and the collection of the EHRs and subjects a query
  touched, which only the scope read, are removed, and so is the query
  refusal of a subject id held under several namespaces. The access record
  names no organisation of its own: audit migration 0032 drops
  `audit_event.organisation` and its place in `audit.audit_record` and
  `audit.access_trail`, a new record is chain digest version 4, and the
  access details no longer write `at0008`.
