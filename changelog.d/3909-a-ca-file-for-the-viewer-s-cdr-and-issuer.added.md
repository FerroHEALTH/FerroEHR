- **The viewer trusts a private CA for the CDR and the OIDC issuer on every
  platform**: `[cdr.tls]` and `[auth.oidc.tls]` take `trust_roots_file`, a PEM
  bundle trusted beside the platform's roots, with `client_identity`
  (`_file`) for mutual TLS and `min_version`, read through
  `ferrohealth_server::tls_client`. A file that cannot be read or holds no
  certificate refuses the start. The Helm chart names the CDR's CA through
  `cdr.tls.trust_roots_file` in place of `SSL_CERT_DIR`, and
  `viewer.auth.oidc.caExistingSecret` mounts an issuer CA; the UI journeys
  pass the development Keycloak CA the same way.
