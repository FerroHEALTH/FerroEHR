- The FHIR `AuditEvent` of an EHDS access record names the object it reads,
  creates, updates or deletes by its openEHR object id as
  `entity.what.identifier` with the system `urn:openehr:object-id`, where it
  wrote a `what.reference` that no FHIR server resolves and HAPI FHIR refused
  (FHIR R4 references.html §2.3.0.3). ITI-81 finds the record by
  `entity.identifier`. FerroEHR takes FerroAUDIT 0.3.2 (`ihe-atna` and
  `ehds-logging`).
