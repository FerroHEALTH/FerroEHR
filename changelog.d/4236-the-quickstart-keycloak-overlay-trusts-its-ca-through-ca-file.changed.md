- **The quickstart Keycloak overlay names its development CA through
  `[auth] ca_file`**: `docker-compose.keycloak.yml` sets
  `FERROEHR__AUTH__CA_FILE` in place of `SSL_CERT_FILE` and `SSL_CERT_DIR`,
  which only the Linux platform verifier reads.
