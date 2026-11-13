- `ferroehr.default.toml` lists 855 keys, down from 920 in 5.7.0, and marks 10
  of them as must-set, where 5.7.0 marked none. 72 keys are gone: the
  operational policy moved into the CDR (#5432, #5436 to #5441, #5521), three
  tables became admin registers (#5442), the representation mapping became a
  code the relationship carries (#5446) and the three public URLs became one
  (#5465). 7 keys are new, each infrastructure or trust: `[server]
  public_base_url` (#5465), `[auth] described_issuer` (#5425), the three keys
  of `[authz.party_binding.patient]` (#5462), the grant ceiling `[authz.grants]
  allowed` (#5435) and `[authz.rbac] first_admin`, the one Basic user that
  holds `admin_role` from the file and writes the first policy parties
  (#5438). Both counts are the ones the file's "What you must set" block
  states.
