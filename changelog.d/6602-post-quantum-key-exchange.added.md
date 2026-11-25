- The REST and viewer listeners, every client FerroEHR opens and the
  PostgreSQL connection offer the hybrid key exchange X25519MLKEM768
  (RFC 10024) first, through `ferrohealth-server` 0.12.4, and agree on
  X25519, P-256 or P-384 with a peer that lacks it. `docs/post-quantum.md`
  and `conformance/post-quantum.tsv` list every cryptographic use and
  whether it resists a quantum computer, and the README badge counts the
  safe ones.
