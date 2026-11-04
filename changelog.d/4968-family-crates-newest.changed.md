- The server, the viewer and the EHR index take the newest FerroHEALTH
  family crates: `ferrohealth-config` 0.2.8, `ferrohealth-server` 0.4.3,
  `ferrohealth-session` 0.2.2, `ferrohealth-telemetry` 0.2.4, `ihe-iti`
  0.0.74 and `ferrobridge-term` 0.1.141. What changes for you:
  `[telemetry.otlp]` now refuses `trust_roots_file` or
  `client_identity_file` beside an endpoint that is not `https://`, and a
  file that is no PEM of its kind, at boot and naming the key, where it
  exported in plain text with the files ignored. Upgrade: point the endpoint
  at `https://` when the collector speaks TLS, or drop the two keys when it
  does not. Each request's exported span is a server span named
  `{method} {route}` whose status is an error on a 5xx answer, where it was
  named `request`. `/management/prometheus` gains
  `http_server_active_requests`, the requests in flight by method and scheme.
  A panic line names the server's id of the request it happened in. A
  configuration refusal no longer echoes a secret written unquoted in the
  file, and a refusal inside an array of tables names its key with the
  element's index.
