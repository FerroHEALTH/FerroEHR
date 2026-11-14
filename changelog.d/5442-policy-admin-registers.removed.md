- **Breaking: `[audit.categories]` and `[privacy.access_delay]` are removed from
  `ferroehr.toml`.** Each is now an admin register, and a configuration that
  still sets one refuses boot as an unknown key. Write the category map and the
  access delay through `{ext}/admin/registers/{register}` once the server
  runs; until then the shipped category map applies and the delay is off. The
  Helm chart 26.0.0 drops `config.audit.categories` (a major bump: a values
  file that sets it is refused).
