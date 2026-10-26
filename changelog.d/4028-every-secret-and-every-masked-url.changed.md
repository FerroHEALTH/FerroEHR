- **Every secret and every masked URL renders through `ferrohealth-secret`,
  the family's one mask** (en-route finding of #4028). EHR's own
  `config::secret` module gives way to `ferrohealth-secret`'s `Secret`,
  `SecretUrl` and `RedactedUrl`. A secret still renders as `***` and a URL's
  userinfo as `***@`, and a URL's query is now masked as `?***` too, in
  `ferroehr config check`, `GET /admin/config`, `/management/env` and
  `ferroehr report`. `ferrohealth-smart` moves to 0.7.1 and
  `ferrohealth-authz` to 0.1.1, `ihe-atna` and `ehds-logging` to 0.2.11, and
  `ferrobridge-term` to 0.1.137.
- **Breaking: the outbound client credentials tables take the
  `ferrohealth-smart` client table, so `token_url` is now `token_endpoint`**
  (#4028). `[terminology.server.oauth]`, `[audit.fhir_feed.oauth]` and
  `[authz.abac.remote.oauth]` are the family's shared grant: the old key
  `token_url` stops startup by name, and each table gains `private_key_jwt`
  (RFC 7523), mutual TLS (`tls_client_auth` and
  `self_signed_tls_client_auth`, RFC 8705), `ca_file`, `refresh_margin_ms` and
  `timeout_ms`. `issuer`, `client_id`, the client secret, `scope`, `audience`
  and `resource` keep their names. Upgrade: rename `token_url` to
  `token_endpoint` in each table (in the Helm chart, under
  `config.terminology.server.oauth` and the other two), or
  `..._OAUTH__TOKEN_URL` to `..._OAUTH__TOKEN_ENDPOINT` in the environment.
  `ferrohealth-smart` moves to 0.7.0 and `ihe-atna` and `ehds-logging` to
  0.2.10, the FHIR Feed sends its bearer token through `ihe-atna`, and every
  BALP AuditEvent's `source.observer` carries an `identifier` beside its
  `display`.
