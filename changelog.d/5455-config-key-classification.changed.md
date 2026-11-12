- `ferroehr.default.toml` opens with a "What you must set" block: the ten keys a deployment that
  holds real data sets (`[deployment] profile`, `[[auth.issuer]] issuer`, `kind` and `audience`,
  `[authz.party_binding.provider] issuer` and `id`, `[api] system_id`, `[database] url`,
  `[privacy] subject_namespaces` and `[record] subject_index`). Every other key is marked
  optional with its built-in default, and each key's note says why it is in the file:
  infrastructure, trust, or a tuning key. The configuration types carry the classification
  (`x-ferroehr-class`), and the reference generator, run by the `config-reference` guard, fails
  on a key with no class, an optional key with no default, or a class entry that names no key.
  The file lists 921 keys, the same as 5.7.0, which marked none as required; this change adds
  and removes no key. The shipped configurations (the quickstart, the hosted sandbox, the
  development, perf and conformance files, the chart's `values.yaml`, `values-production.yaml`
  and CI overlays) no longer restate a default: `deployment.spec_profile`, `auth.enabled`,
  `[[auth.issuer]] profile = "rfc9068"`, the default `[server]` listener and body limit, the
  chart's `api`, `database`, `authz.rbac`, `authz.abac`, `authz.party_binding.namespace`,
  `management.endpoints`, `record.signing`, `record.multimedia` and `audit.store` defaults and
  the like. The server reads the same values as before.
