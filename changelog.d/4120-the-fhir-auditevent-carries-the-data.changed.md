- **The FHIR `AuditEvent` carries the data permit** (#4120). The stored
  record, the ITI-81 retrieval and the ATX: FHIR Feed write the permit a
  caller declared in `[audit] permit_header` at `agent.policy` of the user
  agent, after the token's `jti` (FHIR R4 `AuditEvent.agent.policy`), on an
  access record and on every other record. The DICOM audit message over
  syslog has no element for a policy (DICOM PS3.15 A.5.1.1), so it carries
  none.
- **The per-EHR access record of an Extract export or import carries the
  declared purpose and data permit** (#4120), taken from the request as the
  other service-layer access records take them, so the subject-scoped ITI-81
  retrieval answers an export of `GET /message/export/{ehr_id}` with its
  permit.
