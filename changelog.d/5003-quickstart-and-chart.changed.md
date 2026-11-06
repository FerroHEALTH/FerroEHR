- The compose quickstart, the dev and conformance stacks, the hosted sandbox
  and the Helm chart's Basic overlays bind their Basic users under a
  `persons` issuer that signs nothing: its key set, which every FerroEHR
  image carries at `/opt/sandbox-posture/basic-users-jwks.json`, is the
  public half of a key pair whose private key was discarded.
- The quickstart has one copy. `docker-compose.yml` on `main` pulls the
  FerroEHR images at `:main` and carries `main`'s configuration, which CI
  checks with the server image the change builds; the copy attached to each
  release pulls that release's images, written by the release lane, which
  refuses a copy that still names `:main`. Download the release asset
  (`releases/latest/download/docker-compose.yml`) to run a release.
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
