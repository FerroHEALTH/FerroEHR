- The server runs the FerroHEALTH HTTP stack of `ferrohealth-server` 0.3.4,
  and `[server]` is its section, the same keys in every FerroHEALTH product:
  `listen`, `base_path` (a path every route is served under, the health routes
  included; empty by default), `request_timeout_ms` (a request still running
  is answered `408`), `header_read_timeout_ms`, `body_limit_bytes`,
  `drain_delay_ms` and `shutdown_timeout_ms` (on `SIGTERM` readiness turns
  `503` at once, the listener closes after the delay, and the requests in
  flight get the bound to finish), `max_concurrent_requests` with
  `retry_after_s`, `trusted_proxies` with `forwarded_header`, and `compression`
  with `compression_min_bytes`. `[server.caller_rate]` (`requests_per_second`,
  `burst`) limits each authenticated caller. `[server.tls]`
  (`certificate_file`, `key_file`, `client_auth`, `client_ca_file`,
  `min_version`, `reload_interval_s`, `healthcheck_identity_file`) reads the
  files again on `SIGHUP` too, and the management listener on its own port
  drains with the main one under the same bound.
- `[rest]` holds the openEHR REST surface: `base_path`, `swagger_ui`,
  `cors_permissive`, `system_id`, `[rest.identity]` and `body_limit_bytes`,
  the clinical body tier below `server.body_limit_bytes`, which the
  template-upload and `/message` routes keep.
- `ferroehr serve` names the default command beside `config check` and
  `healthcheck`.
