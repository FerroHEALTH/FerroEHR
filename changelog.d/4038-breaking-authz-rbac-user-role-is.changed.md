- **Breaking: `[authz.rbac] user_role` is retired** (#4038 follow-up). It
  named a role nothing required once a Basic user's roles became its
  `[[auth.basic.users]] roles` list. The key now stops startup by name and
  points at that list. Upgrade: remove `user_role` (on Helm chart 16.0.0,
  `config.authz.rbac.user_role`), and give each Basic user the roles it needs,
  for example `roles = ["USER"]`.
- **Breaking: the caller's roles come from `ferrohealth-smart`, so the two
  role keys move to `[auth]`, and the policy decision is
  `ferrohealth-authz` 0.1.0** (#4038). `[authz.rbac] role_claims` is now
  `[auth] role_claims` (same default, `realm_access.roles` included), and
  `[authz.rbac.basic_roles]` is now the `roles` list of each
  `[[auth.basic.users]]` entry; both old places stop startup by name. Roles
  stay upper-cased, and a Basic user whose entry lists no role holds none,
  where a user absent from `basic_roles` used to hold `user_role`.
  `[authz.assurance]` and `[authz.professional]` keep their keys and are read
  by the shared sections; the RFC 9470 step-up challenge on patient data no
  longer carries an `error_description`. The Cedar engine and the remote
  decision point are the family's: the Cedar schema and the remote wire body
  are unchanged, a remote resource kind with no policy is a `500` instead of
  a `403` (boot validation already refuses that configuration), and
  `authz_cedar_decisions` and `authz_remote_pdp_calls` count one per decision
  where they counted one per attribute combination. Upgrade: move
  `role_claims` from `[authz.rbac]` to `[auth]`
  (`FERROEHR__AUTH__ROLE_CLAIMS` in the environment), and each user's entry
  of `[authz.rbac.basic_roles]` to `roles` under its `[[auth.basic.users]]`
  table (in the Helm chart, under `config.auth.basic.users`), and give a user
  that had no entry `roles = ["USER"]`. Helm chart 15.0.0 takes the same keys
  (`config.auth.role_claims`, `roles` under `config.auth.basic.users`, and
  `token_endpoint` in each grant). `ferrobridge-term` moves to 0.1.136,
  whose `SmartToken` replaces the terminology client's own token adapter
  (#4031).
