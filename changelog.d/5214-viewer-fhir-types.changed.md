- The viewer's audit screen reads the ITI-81 searchset and each `AuditEvent`
  as FHIR R4 resources, and a refusal's `OperationOutcome` too. A searchset
  that is no FHIR R4 `Bundle` of `AuditEvent`s now shows as a failed read,
  never as a page of empty rows.
