- The family crates move together to one `ferrohealth-smart`: 0.13.0, with
  `ferrohealth-authz` 0.7.0, `ferrohealth-session` 0.7.0, `ihe-atna` and
  `ehds-logging` 0.8.0, `ihe-iti` 0.0.83, `ferrohealth-server` 0.6.4 and
  `ferrobridge-term` 0.1.147. The 0.13 line refuses a JSON document it
  cannot read without quoting it (GHSA-mhv7-3975-6xq3), so a `jwks_file` or
  `[smart] issuer_metadata_file` that does not parse is refused at boot
  with the crate's message, which names the failure class and position. The
  OAuth client credentials tables (`[authz.abac.remote.oauth]`,
  `[ehr_index.oauth]`, `[terminology.oauth]` and `[audit.fhir_feed.oauth]`)
  read as before.
