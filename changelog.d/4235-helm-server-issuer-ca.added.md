- **The Helm chart mounts a private issuer CA for the server**:
  `auth.caExistingSecret` (with `auth.caExistingSecretKey`, `ca.crt` by
  default) mounts the Secret's PEM bundle and points the server's
  `[auth] ca_file` at it, as `viewer.auth.oidc.caExistingSecret` does for the
  viewer. Chart 19.3.0.
