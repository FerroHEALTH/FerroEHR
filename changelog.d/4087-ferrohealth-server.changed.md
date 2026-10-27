- Breaking: the REST-surface keys move from `[server]` to `[rest]`:
  `server.base_path`, `server.swagger_ui`, `server.cors_permissive`,
  `server.system_id` and `[server.identity]` are `rest.base_path`,
  `rest.swagger_ui`, `rest.cors_permissive`, `rest.system_id` and
  `[rest.identity]` (`FERROEHR__REST__…`), with no alias; the old names are
  refused at startup. Upgrade: rename the keys, and move a `[server.identity]`
  table to `[rest.identity]`.
- Breaking: `server.bind` is `server.listen`, an IP address and a port; a host
  name such as `localhost:8080` refuses the start. The binary listens on
  `127.0.0.1:8080` by default, where it listened on every interface; the
  container image sets `PORT=8080`, so it still listens on `0.0.0.0:8080`, and
  the chart and the Compose files set `listen`. Upgrade: write
  `listen = "0.0.0.0:8080"` (or set `PORT`) where the server must be reached off
  the host.
- Breaking: the request-body limit is `server.body_limit_bytes`, 1 MiB by
  default, the ceiling of every route and the limit of the template-upload and
  `/message` routes, and `rest.body_limit_bytes` (unset by default) narrows the
  other routes below it; they were 64 MiB and 16 MiB. The chart and the Compose
  files set 64 MiB and 16 MiB. Upgrade: a deployment that uploads operational
  templates sets `server.body_limit_bytes = 67108864`, since the IPS template
  alone is larger than 1 MiB.
- Breaking: `server.max_in_flight` is `server.max_concurrent_requests`, unset
  (no bound) by default where it was 256, and the bound now covers every
  FerroEHR route (the status document, SMART discovery and the management
  surface included); only the health routes are never refused. Upgrade: set
  `max_concurrent_requests = 256` to keep the old bound.
- Breaking: `[server.rate_limit]` is `[server.caller_rate]`
  (`requests_per_second`, `burst`) keyed on the authenticated subject, and
  limits nobody until the table is set, where the principal tier was on by
  default; the per-address tier is gone. The chart and the Compose quickstart
  set 1024 per second with a burst of 2048. A `429` carries `Retry-After` and
  no `x-ratelimit-*` headers.
- Breaking: `[server.tls]` takes the family keys: the table's presence turns
  TLS on (`enabled` is gone), `cert_file` is `certificate_file`,
  `reload_interval_secs` is `reload_interval_s` (unset by default, where the
  files were read every 60 seconds; `SIGHUP` always reads them), and
  `client_auth = "off"` is `"none"`. The chart's `config.server.tls` follows,
  and an empty table means TLS off.
- Breaking: `[server.connection]` is `[server] header_read_timeout_ms` (30000
  by default, where the bound was 10 seconds, and `0` is refused);
  `max_concurrent_streams` and the HTTP/2 keep-alive keys are gone.
- Breaking: `ferroehr healthcheck` asks `GET /health/readiness` on the
  `server.listen` port (over TLS with `[server.tls]` set) and exits 1 while the
  server drains, where it asked `{rest root}/status`; the reason on stderr reads
  `healthcheck: unhealthy: …`. `GET /health` answers the readiness status with
  `{"state": …}` and `GET /health/liveness` answers `{"state":"up"}`, where both
  answered a plain-text `OK`.
- The request id is the server's own, minted for every request and carried by
  the request log and the access record; a client's `X-Request-Id` is returned
  to it unchanged and no longer recorded. A request past
  `server.request_timeout_ms` or over the body limit, a refused caller and a
  panicked handler answer the openEHR `{ error, message }` body, and the panic
  message reaches neither the body nor the log. Responses are compressed at the
  default gzip and Brotli quality, from 1 KiB up.
- The boot banner is the FerroHEALTH banner: the wordmark, the build line
  `ferroehr <version> (<commit>)`, and the facts (manufacturer, profile, pins,
  support period, deployment) as one list, with the sandbox notice below.
- Chart 19.0.0: `config.server` carries `listen`, `body_limit_bytes`,
  `max_concurrent_requests` and `caller_rate`, and `config.rest` the REST
  surface; the rendered `ferroehr.toml` changes accordingly.
