- A caller confined to the EHR of its SMART launch context, a patient or any
  caller whose grants are `patient/` grants alone, is refused `403` with
  `refused-not-subject` on every route that names another EHR, recorded in
  the access log as step 0 (subject). The DIRECTORY, item tag,
  `GET /message/export/{ehr_id}`, `POST /message/import/{ehr_id}`, manifest
  export, query and import routes answered a bare `403` from the SMART layer
  with no refusal code and no access-log step. The access evaluator now holds
  the launch context alone: the SMART layer checks a grant's resource family
  and permission and no longer refuses on EHR identity, and an operation on
  EHR data that names no EHR (an EHR create, a lookup by subject, a query
  with no single-EHR scope, an import naming no EHR) is refused the same way.
