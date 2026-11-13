- **Breaking: `[authz.import] trusted_systems`, `[[authz.origin]]`, `[record.change_feed]
  readers` and `[[auth.scim.client]]` are removed from `ferroehr.toml`.** Each is now a
  grant in the CDR, read on every request, and a configuration that still sets one refuses
  boot as an unknown key. An import's `creating_system` element vouches for the systems
  whose wrapped committers it may carry, a `data_origin` grant names an origin, a
  `read_change_feed` grant names a reader of the change feed, and a `scim_provision` grant
  names a SCIM client's `binding_issuer` and `token_scope`; `[authz.grants] allowed` must
  admit each operation. The Helm chart 30.0.0 drops the outbox-readers line of its notes
  and documents the grants (a major bump: a values file that sets one of these keys is
  refused).
