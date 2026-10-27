- **`[auth] ca_file` is trusted under introspection mode too**: with
  `ferrohealth-smart` 0.8.6 the server reads an introspecting issuer's
  metadata, and calls its introspection and token endpoints, trusting the
  bundle beside the platform roots and the grant's own `ca_file`. The e2e
  compose overlay names the development Keycloak CA through
  `FERROEHR__AUTH__CA_FILE` in place of `SSL_CERT_FILE` and `SSL_CERT_DIR`.
