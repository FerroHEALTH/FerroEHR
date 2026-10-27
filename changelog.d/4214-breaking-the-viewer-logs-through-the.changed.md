- **Breaking: the viewer logs through the family's `[telemetry]` section of
  `ferrohealth-telemetry` 0.2.2, as the CDR does** (#4214). The console writes
  to stdout where it wrote to stderr, and `telemetry.format` defaults to
  `auto`, which renders `json` when stdout is not a terminal, so a container's
  log is now JSON lines; set `FERROEHR_VIEWER__TELEMETRY__FORMAT=pretty` for
  the old human lines. `RUST_LOG` is now the lower-priority fallback of
  `telemetry.filter` (`FERROEHR_VIEWER__TELEMETRY__FILTER` wins over it), the
  filter defaults to `info,hyper=warn,tower=warn,h2=warn` where it was `info`,
  and a filter that does not parse refuses the start where it fell back to
  `info`. Each request writes the CDR's request line (method, matched route,
  status, latency, request id), and a panic writes the CDR's one fixed line
  without the panic message. `[telemetry.otlp]` exports the viewer's spans and
  its `http.server.request.duration` metric as `service.name`
  `ferroehr-viewer`. Helm chart 19.2.0 sets the viewer's format and filter
  from `viewer.telemetry`, and the Compose viewer services set
  `FERROEHR_VIEWER__TELEMETRY__FORMAT`.
