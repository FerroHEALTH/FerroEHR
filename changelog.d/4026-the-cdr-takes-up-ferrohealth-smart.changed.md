- **The CDR takes up `ferrohealth-smart` 0.6.0 and `ferrobridge-term`
  0.1.134** (#4026, #4027). Token checking, Basic users, the discovery
  document and every outbound client credentials token behave as before.
  The terminology client now
  refuses, before sending, a code with no system on an STU3 server; the
  CDR's server is R4B, so its binding checks are unchanged. The new
  `[auth] mode = "introspection"` key of the shared section, and an
  `[auth.introspection]` table, are refused at boot: this server checks
  bearer tokens as JWTs only, and a setting it would ignore stops the start.
