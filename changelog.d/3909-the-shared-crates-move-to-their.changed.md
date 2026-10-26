- **The shared crates move to their newest lockstep releases**: `ferrohealth-smart` 0.7.3,
  `ferrohealth-authz` 0.1.3, `ihe-atna` and `ehds-logging` 0.2.12 and `ferrobridge-term`
  0.1.138. Every secret in the `[auth]` section now renders as `***`, the family
  mask, and `[auth] ca_file` names a PEM bundle of extra certificate authorities
  for the issuer's metadata and key set, for an issuer behind a private PKI.
