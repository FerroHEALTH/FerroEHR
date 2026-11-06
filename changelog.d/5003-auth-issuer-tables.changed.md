- `[auth]` takes the `ferrohealth-smart` 0.9 shape. Every trusted issuer is
  an `[[auth.issuer]]` table with a required `kind` (`persons`, `patients`,
  `machines` or `federation`) and a `profile` (`rfc9068` by default, or
  `entra`, `keycloak`, `okta`, `nl`), and holds every setting of its tokens
  itself. A configuration in the old shape refuses the start, naming what
  replaces it. Upgrade steps: write a flat `issuer = "…"` as an
  `[[auth.issuer]]` table; move `audience`, `algorithms`,
  `clock_skew_seconds`, `jwks_max_age_seconds`,
  `jwks_refresh_interval_seconds`, `jwks_file`, `timeout_ms`, `mode`,
  `role_claims` and `[auth.introspection]` onto each issuer table
  (`[auth.issuer.introspection]`); give each issuer its `kind`, with patients
  and professionals at separate issuers (a realm or tenant each), since one
  issuer is one kind; replace `token_profile = "jwt"` with
  `token_types = ["JWT"]` and the `required_claims` the issuer's tokens
  carry, and drop `token_profile = "rfc9068"`; and write each
  `scopeless_subjects` entry as `{ issuer = "…", sub = "…" }`.
- A `persons` token is admitted only through a client its issuer lists in
  `first_party_clients` or `delegates`. Upgrade step: list the deployment's
  own clients, the viewer's among them, in `first_party_clients`, and a
  third-party client that acts for a person in `delegates`. The other 0.9
  profile keys (`person_key`, `tenant_claim`, `client_claims`, `app_only`,
  `guest`, `impersonation_marker`, `expect`, `user_claims`,
  `person_equality`, `unstable_claims`, `max_actor_chain`,
  `representation`, `scope_claim`, `app_only_scope_claim`, `adapter`,
  `sender_constraint` and the `onward_*` keys) are documented in the
  configuration reference.
- An array of tables, such as `[[auth.issuer]]`, is set from the environment
  by element index (`FERROEHR__AUTH__ISSUER__0__ISSUER`), through
  `ferrohealth-config` 0.2.10.
