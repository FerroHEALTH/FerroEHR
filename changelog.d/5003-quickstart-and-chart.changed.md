- The compose quickstart, the dev and conformance stacks, the hosted sandbox
  and the Helm chart's Basic overlays bind their Basic users under a
  `persons` issuer that signs nothing: its key set, which every FerroEHR
  image carries at `/opt/sandbox-posture/basic-users-jwks.json`, is the
  public half of a key pair whose private key was discarded. The quickstart
  pins the FerroEHR 5.7.0 image, whose configuration it carries.
- The Helm chart moves to 22.0.0. `config.auth.issuer` is a list of issuer
  tables, each with its `kind`, `profile` and `audience`, and `values.yaml`
  exposes `config.auth.party`, `config.authz.party_binding`,
  `config.authz.rbac.role_issuers`, `config.authz.system_client` and
  `config.authz.federation.agreement`. The chart refuses an issuer with
  `sender_constraint: required` beside `replicaCount` above 1 or
  autoscaling. Upgrade steps: rewrite `config.auth` as the `[auth]` upgrade
  above describes, set `config.authz.party_binding.provider`, and list the
  issuers whose roles grant the administrator role in
  `config.authz.rbac.role_issuers`.
