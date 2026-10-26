- **The EHR index asks an IHE patient identity source over PIXm (ITI-83) by
  default** (#3380). `[ehr_index] provider = "ihe"` names a PIX Manager by its
  FHIR base (`pixm_url`) and the clinical domain (`assigning_authority`, one of
  `privacy.subject_namespaces`), with the client credentials grant of
  `[ehr_index.oauth]` (renewed once on a `401`) and the TLS material of
  `[ehr_index.tls]` (`trust_roots_file`, `client_identity_file`,
  `min_version`). The client is `ihe-iti` 0.0.62's PIXm Consumer and sends only
  what ITI-83 specifies, by `POST` unless `invocation = "get"`, so any
  conformant PIX Manager serves. `has_ehr_for_subject` and
  `create_ehr_for_subject` ask the source for the subject's pseudonym, and the
  subjects of an EHR are the pseudonym its `EHR_STATUS` carries; the source is
  never asked to resolve a pseudonym back. The server reads the source's
  `CapabilityStatement` at boot, and a source that fails a request is an
  error: nothing falls back to the in-instance map. PDQm is not used (GDPR
  Art. 5(1)(c)).
