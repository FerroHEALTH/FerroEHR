- The FerroHEALTH family crates move to their newest releases:
  `ferrohealth-smart` 0.18.0 to 0.19.1, `ferrohealth-session` 0.13.0 to
  0.14.0, `ferrohealth-authz` 0.11.0 to 0.12.0, `ferrobridge-term` 0.5.0 to
  0.6.0, `ihe-atna` and `ehds-logging` 0.12.0 to 0.13.0, and `ihe-iti` 0.4.0
  to 0.5.0. A `private_key_jwt` client assertion FerroEHR signs for a token
  request or an onward exchange is now valid for 60 seconds, down from 300,
  so an authorization server that caps the lifetime below five minutes
  accepts it. No configuration key changes.
