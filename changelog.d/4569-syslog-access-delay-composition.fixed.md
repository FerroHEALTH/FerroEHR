- The syslog form (DICOM PS3.15, IHE ITI-20) of an EHR-level audit record
  names its object beside the patient, as a URI participant object with the
  id its FHIR `AuditEvent` names. The record of placing, releasing or
  expiring an EHDS Art. 3(3) access delay names the held composition as
  `access-delay:<vo>`, so an Audit Record Repository fed by syslog can tell
  which composition a hold covers.
