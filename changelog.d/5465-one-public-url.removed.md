- **Breaking: `[auth.smart] public_base_url`, `[auth.scim] public_base_url` and
  `[auth.dpop] external_url` are removed from `ferroehr.toml`, and
  `[server] public_base_url` takes their place.** A deployment names the URL
  clients reach it at once. The SMART discovery document's `baseUrl` is that
  URL with `server.base_path` and `api.base_path`, every SCIM `meta.location`
  hangs off it with `server.base_path` and `/scim/v2`, and a DPoP proof's `htu`
  must name it with `server.base_path` and the request path (RFC 9449 §4.3).
  The SMART `baseUrl` and the SCIM locations now carry `server.base_path`,
  which they left out before. The key is required with `auth.smart.discovery`,
  `[auth.scim]` or `[auth.dpop]`, and a configuration that still sets a removed
  key refuses boot as an unknown key: move its value to `[server]
  public_base_url`. `ferroehr.default.toml` lists two keys fewer. The Helm
  chart 28.0.0 takes `config.server.public_base_url` in place of
  `config.auth.smart.public_base_url` and `config.auth.scim.public_base_url`
  (a major bump: a values file that sets either is refused at boot).
