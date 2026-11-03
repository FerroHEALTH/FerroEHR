- The viewer signs in and keeps its session through `ferrohealth-session`
  0.2.0, FerroAUTH's console sign-in (#4873). Its own OpenID Connect client
  and sealed-session code are gone. The sign-in page moves to `/sign-in`, and
  the sign-in routes are `/login` (which begins the OpenID Connect sign-in),
  `/auth/callback`, `POST /logout` (which now also revokes the token and uses
  the provider's end-session endpoint when it has one) and `/session`, the
  session watch. Sessions are kept in the viewer process by default
  (`session.store = "memory"`), and in one sealed cookie with
  `session.store = "sealed"` and a key. A sealed store without a key refuses
  the start, so there is no ephemeral key any more. FerroEHR pins
  `ferrohealth-smart` 0.8.8. **Upgrade note:**
  - The provider must allow the redirect URI `<public_base_url>/auth/callback`
    in place of `/auth/oidc/callback`.
  - `[auth.oidc]` becomes `[oidc]`. `enabled` moves to `auth.oidc_enabled` and
    `public_base_url` to `auth.public_base_url`. `tls.trust_roots_file` becomes
    `oidc.ca_file`, and `resolve = "host=ip:port"` becomes
    `oidc.resolve = ["host=ip:port"]`. `tls.client_identity` (one PEM)
    becomes `oidc.tls.client_certificate` with `oidc.tls.client_key` (or
    `client_key_file`), and `tls.min_version` becomes `oidc.tls.min_version`,
    still `"1.3"` by default.
  - In `[session]`, `idle_minutes` becomes `idle_timeout_s` and
    `absolute_minutes` becomes `absolute_timeout_s`, both in seconds;
    `cookie_secure` becomes `secure_cookie`. `secret` and `secret_file`
    become `key` and `key_file`, which only a sealed store takes, so a scaled
    deployment sets `session.store = "sealed"` beside its key.
  - The variables follow: for example `FERROEHR_VIEWER__AUTH__OIDC__ISSUER` is
    now `FERROEHR_VIEWER__OIDC__ISSUER`. Bookmarks to `/login` reach the
    OpenID Connect sign-in, not the sign-in page.
  - The Helm chart (21.0.0-rc10) renders the new variables from the same
    `viewer.auth.oidc.*` values.
