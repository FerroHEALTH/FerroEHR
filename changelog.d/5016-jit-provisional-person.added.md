- Just-in-time creation of a provisional `PERSON` at a first login, off by
  default and enabled per `persons` issuer in a new
  `[[authz.party_binding.issuer]]` table (`jit`, `guests_jit`,
  `user_name_claim`, `idp_claim`, `admit_idp`, `admit_groups`, `admit_roles`,
  `admit_assurance`). A login of a person or a delegated caller that matches
  no party and meets every declared admission condition commits one
  `553|incomplete|` `PERSON` holding the binding key, the `userName` and the
  name claims of the access token, with its professional `ROLE`, as a
  creation by the person, and is served bound to it; any other mapped
  attribute is written only where its mapping sets `jit = true`, and
  `externalId` never. Concurrent first requests commit one `PERSON`, a
  deprovisioned or inactive login is never recreated, a taken `userName`
  refuses the login, and a SCIM client of the key's issuer completes the
  `PERSON`. Boot refuses `jit` without an admission condition or a mapping
  that can write it. `ferroehr party-binding check` lists the provisional
  persons older than `[authz.party_binding] provisional_report_days`
  (default 30).
