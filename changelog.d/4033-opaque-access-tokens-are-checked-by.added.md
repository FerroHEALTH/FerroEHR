- **Opaque access tokens are checked by token introspection under `[auth]
  mode = "introspection"`** (#4033). `ferrohealth-smart`'s introspection
  check (RFC 7662) posts each bearer token to the issuer's introspection
  endpoint, named by its metadata or by `[auth.introspection] endpoint`, and
  authenticates as the client of `[auth.introspection.client]` or with the
  token of `[auth.introspection.bearer]`. An active response admits the
  caller with its scopes and roles and is reused until its `exp`, an inactive
  token is a `401`, and an endpoint that cannot answer is a `503`. The mode
  and the table were refused at boot. Helm chart 15.1.0 names the mode.
