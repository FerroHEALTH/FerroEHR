- The Helm chart (23.0.0) connects FerroBRIDGE to the CDR as a machine.
  FerroBRIDGE uses an OAuth 2.0 client credentials client of a `machines`
  issuer that the CDR trusts, set in `bridge.cdr.oauth` (`issuer`, `clientId`,
  and optionally `scope`, `audience` and `resource`). The client secret comes
  from the `client_secret` key of `bridge.cdr.existingSecret`. The chart no
  longer accepts a Basic user for this connection: from ferrohealth-smart 0.9
  a Basic credential belongs to a person and holds no `system/` scope. The
  chart refuses to render when `bridge.cdr.oauth.issuer` is not listed in
  `config.auth.issuer` with `kind: machines`. To upgrade, add that issuer,
  create the client, and replace the `user` and `password` keys of the Secret
  with `client_secret`.
