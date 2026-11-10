- Every outbound integration (the terminology server, the PIXm identity
  source, the ITI-20 FHIR Feed, the remote policy decision point and the
  viewer's CDR client) is built by `ferrohealth-smart`'s `OutboundClient`
  from its endpoint table, and FerroEHR's own client builder is gone. A PIXm
  exchange's `timeout` now bounds the exchange with its tries and the waits
  between them, and the viewer refuses a `[cdr] timeout` of zero at start.
- The family crates move to their newest releases: `ferrohealth-smart`
  0.16.3, `ferrohealth-session` 0.10.0, `ferrohealth-authz` 0.9.0,
  `ferrohealth-config` 0.4.2, `ferrohealth-server` 0.8.1, `ferrobridge-term`
  0.3.0, `ihe-atna` and `ehds-logging` 0.10.1, and `ihe-iti` 0.2.0.
