- **Breaking: `[audit.categories]`, `[privacy.access_delay]` and `[authz.abac.cedar]` are
  removed from `ferroehr.toml`.** Each is now an admin register, and a configuration that
  still sets one refuses boot as an unknown key. Write the category map, the access delay
  and the Cedar policy set through `{ext}/admin/registers/{register}` once the server
  runs; until then the shipped category map applies, the delay is off, and the Cedar set
  is empty, so an ABAC deployment on the embedded engine permits nothing until its
  policies are written. The Helm chart drops `config.audit.categories`, and the server
  no longer reads Cedar policy files from a directory.
