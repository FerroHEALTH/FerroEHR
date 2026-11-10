- The configuration is redacted by its secret types alone. `GET
  /management/env`, `GET /ferroehr/rest/extensions/v1/admin/config` and
  `ferroehr config check` show the tree as its `Secret` and `SecretUrl`
  values serialize, and the second masking by key name and by URL shape is
  gone. `[audit.syslog] url`, `[auth.smart] platform_base_url` and
  `public_base_url`, and `[auth.scim] public_base_url` are secret URLs now,
  as are the OAuth, onward and introspection endpoints of `ferrohealth-smart`
  0.17 and the viewer's `[oidc]` endpoints of `ferrohealth-session` 0.11, so
  each renders with its credentials and query masked. A `platform_base_url`
  given as a bare path renders as `***`.
