- Every mounted route now carries its own audit classification, and the
  audit layer has no fallback class. 67 extension and FHIR operations, the
  access-log erasure among them, were recorded as an `Execute` on
  `ApplicationActivity` in the system domain whatever they did; each now
  names its action and resource class (the `party_relationship_*` operations
  are recorded in the party domain, the archetype and artefact operations as
  templates, `contribution_list` and the access-delay release as reads and
  updates of EHR data). An operation without a classification is refused with
  `500` before its handler runs, and a test fails the build on any mounted
  route without one. An access-log erasure is now one record on syslog, one
  in the store and one on the FHIR Feed, and that record names the request id
  and the client address.
