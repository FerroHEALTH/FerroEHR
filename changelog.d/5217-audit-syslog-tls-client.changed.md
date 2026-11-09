- `[audit.syslog]` reads its TLS material from `[audit.syslog.tls]`, the
  outbound TLS table of every FerroHEALTH product (`trust_roots_file`,
  `client_identity_file` or `client_identity`, `min_version`), and hands
  `ihe-atna` the rustls configuration `ferrohealth-server` builds from it.
  The keys `tls_ca_file`, `tls_identity_cert_file` and
  `tls_identity_key_file` are retired and refused at boot by name: name the
  roots file as `trust_roots_file`, and concatenate the client certificate
  chain and its private key into the one PEM `client_identity_file` names.
  The client speaks TLS 1.3 unless `min_version = "1.2"` lowers it.
