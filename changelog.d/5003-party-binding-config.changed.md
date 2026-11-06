- Every person is bound to a demographic party, with no switch. A `persons`
  issuer needs its `[[auth.party]]` table, two issuers of one node may not
  share an audience, and `[authz.party_binding] provider` (the healthcare
  provider the deployment acts for, EHDS Art. 13(4) and Annex II 3.2(a)) is
  required once `[auth]` admits a credential; each refuses the start naming
  what to set. A Basic user is a person: it maps to the binding key of its
  `PERSON` in `[[authz.party_binding.basic_user]]` (`user`, `key_issuer`,
  `key`), and a Basic user without a mapping refuses the start, naming the
  user. Upgrade steps: add an `[[auth.party]]` table to each `persons`
  issuer, set `provider`, and map each Basic user under the
  `identifier_issuer` of an `[[auth.party]]` table, which needs a `persons`
  issuer; a deployment of Basic users alone declares one whose `jwks_file`
  is a key set that signs nothing, as the quickstart and the sandbox do.
- A token grants `[authz.rbac] admin_role`, `emergency_access_role`,
  `import_role` and `identifier_reader_role` only when its issuer is listed
  in the new `[authz.rbac] role_issuers`, each a `persons` issuer; any other
  kind refuses the start. Upgrade step: list the issuers whose role claims
  grant these roles; a Basic user keeps the roles its own table grants.
