- **The ITI-81 audit retrieval applies every search parameter RESTful ATNA
  §3.81.4.1.2.2 says the repository shall support** (#4040): `address`,
  `agent.identifier`, `patient.identifier`, `entity.identifier`,
  `entity-type`, `entity-role`, `source.identifier`, `type`, `subtype` and
  `outcome`, beside `patient`, `agent`, `entity` and `action`. Several
  comma-separated values match any of them, and a repeated parameter must
  match too. Tokens take the `code`, `|code` and `system|code` forms of FHIR
  R4 Search §3.1.1.4.10. These parameters were a `400` `not-supported`. The
  subject-scoped audit role still names exactly one subject, by `patient` or
  `patient.identifier`.
