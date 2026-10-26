- **Breaking: the server reads only the configuration file it is told to
  read** (#4087). The loader is `ferrohealth-config` 0.2.5, the FerroHEALTH
  products' shared one, and it reads the file `--config` or `FERROEHR_CONFIG`
  names. `./ferroehr.toml` and `/etc/ferroehr/ferroehr.toml` are no longer
  searched: a deployment that mounts a file there without naming it starts on
  the environment and the defaults alone. Set
  `FERROEHR_CONFIG=/etc/ferroehr/ferroehr.toml`; the Compose files, the
  conformance stacks and Helm chart 16.0.2 set it. The fallbacks keep their
  order (the libpq variables, `DATABASE_URL`, `DATABASE_URL_UNPOOLED`,
  `RUST_LOG`, `PORT`), and `ferroehr config check` prints each key beside the
  source that set it.
- **Breaking: the viewer reads only the configuration file
  `FERROEHR_VIEWER_CONFIG` names** (#4087), through the same
  `ferrohealth-config` loader as the CDR. `./ferroehr-viewer.toml` and
  `/etc/ferroehr/viewer.toml` are no longer searched. A variable under
  `FERROEHR_VIEWER_` that is neither an override nor the Compose port or image
  parameter refuses the start, and a `*_file` secret loses only its trailing
  line ending, where all surrounding whitespace was trimmed; an empty file is
  refused.
- **Breaking: `[auth] role_claims` defaults to `["roles", "groups",
  "entitlements"]`** (#4087), the RFC 9068 §2.2.3.1 carriers every
  FerroHEALTH product reads. `realm_access.roles` is no longer in the default:
  a Keycloak issuer sets `FERROEHR__AUTH__ROLE_CLAIMS=roles,groups,entitlements,realm_access.roles`,
  as the Keycloak Compose overlay and the viewer's journey stack now do.
- **Breaking: the server no longer refuses a `FERROEHR_` variable outside the
  override form, and a file refusal says less** (#4087). Only a
  `FERROEHR__<SECTION>__<KEY>` override that names no key refuses the start.
  A single-`_` spelling such as `FERROEHR_DB_MAX_CONNECTIONS`, or a
  per-subsystem pointer such as `FERROEHR_SIGNING_CONFIG`, sets nothing and
  is left alone, because the viewer's `FERROEHR_VIEWER_` variables share the
  prefix (`ferrohealth-config` 0.2.5). A refusal of an unknown key in the file
  names the key and the closest valid one, without its line number; a syntax
  error still names its line and column.
- **Breaking: `/health/readiness` and `/management/health` serve the
  FerroHEALTH readiness document of `ferrohealth-health` 0.1.1** (#4087). The
  body is `{"state", "phase", "indicators": [{"name", "state", "detail"?}]}`
  with lowercase states (`up`, `degraded`, `down`), replacing `{"status",
  "components"}`; the public probe leaves out every `detail`. Readiness also
  answers `503` while the process boots and from the moment it receives
  `SIGTERM` or `SIGINT`, before the listener stops. The optional indicators
  (`audit_sender`, `audit_delivery`, `audit_chain`, `events`) read at worst
  `degraded`, so a damaged audit chain now reads `degraded` where it read
  `DOWN`, and an indicator that is up carries no detail. The viewer's health
  tab reads the same document. Probes that test only the status code are
  unaffected.
- **Breaking: logging, the request log and the OpenTelemetry export move to
  the `[telemetry]` section of `ferrohealth-telemetry` 0.2.1** (#4087), the
  FerroHEALTH products' shared one, with no aliases for the old keys. `[log]`
  is gone: `log.format` and `log.filter` become `telemetry.format` and
  `telemetry.filter` (`FERROEHR__TELEMETRY__FORMAT`,
  `FERROEHR__TELEMETRY__FILTER`, `RUST_LOG` still a fallback), and the
  filter now defaults to `info,hyper=warn,tower=warn,h2=warn` and refuses the
  start when it does not parse. The export moves to a `[telemetry.otlp]`
  table: `otlp_endpoint` becomes `otlp.endpoint`, `traces_sample_ratio`
  becomes `otlp.sample_ratio`, and `metrics_push` becomes `otlp.metrics`,
  which defaults to `true`, so a configured endpoint now receives the metrics
  as well as the spans. `telemetry.flame_file` moves to
  `management.flame_file`. `telemetry.service_name` is removed: the
  `service.name` is always `ferroehr`. `telemetry.environment` no longer
  defaults to `dev` and is left off every span and metric unless set. A
  file or variable that still spells an old key refuses the start, naming
  it. Helm chart 17.0.0 renames `config.log` to `config.telemetry`.
- **Breaking: the HTTP metrics are the `ferrohealth-telemetry` request log's**
  (#4087). `http_server_request_duration_seconds` is labelled
  `http_request_method`, `http_route`, `http_response_status_code` and
  `url_scheme` (OpenTelemetry HTTP semantic conventions); the
  `status_class` label is gone, so a query on `status_class="5xx"` becomes
  `http_response_status_code=~"5.."`, as the bundled alert rules and Grafana
  dashboard now read. The `http_server_active_requests`,
  `http_server_request_body_size` and `http_server_response_body_size`
  series are removed, and no series on `/management/prometheus` carries the
  `otel_scope_name` label any more. The viewer's management page loses its in-flight
  requests tile, and the dashboard's in-flight panel shows the request rate.
- **Breaking: responses no longer carry `x-trace-id`, and a client's
  `traceparent` is no longer read** (#4087). Every request starts a trace of
  the server's own, so a caller cannot join or steer the server's traces;
  correlate a request through its `x-request-id`, which the request log
  writes on every line.
