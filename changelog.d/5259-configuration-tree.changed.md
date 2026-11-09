- **Breaking: the configuration tree is one table per concern.** `ferroehr.toml`
  moves to `[deployment]`, `[api]`, `[database]`, `[auth]` (with `[auth.smart]`
  and `[auth.scim]`), `[authz]` (with `[[authz.origin]]`), `[management]`,
  `[audit]`, `[privacy]` (with `identifier_protection`, `cohort` and
  `access_delay`), `[record]` (with `subject_index`, `versioning`, `signing`,
  `query`, `change_feed` and `multimedia`), `[terminology]` and `[pixm]`.
  FerroEHR's own keys take one duration type (`"500ms"`, `"30s"`, `"10min"`,
  `"12h"`, `"7d"`, UCUM unit codes, so a bare number and `m` are refused), one
  size type (`"256KiB"`, `"64MiB"`, IEC binary units, decimal units refused)
  and one calendar retention period (`"P5Y"`, `"P1830D"`, unset keeps forever).
  An optional feature is on when its table is present: `[terminology]`,
  `[pixm]`, `[auth.scim]`, `[audit.syslog]`, `[audit.fhir_feed]`,
  `[privacy.identifier_protection]` and `[privacy.access_delay]` carry no
  `enabled`; `[authz.abac]`, `[record.multimedia]` and `[auth.smart]` keep a
  switch because some of their keys apply while off. `record.subject_index`
  has no default, so the server refuses to start until `"pixm"` or `"local"`
  is chosen. `[api.cors]` replaces `cors_permissive` with the Fetch Standard
  response headers (`allow_origins`, `allow_methods`, `allow_headers`,
  `expose_headers`, `allow_credentials`, `max_age`). The environment follows
  the tree (`FERROEHR__DB__URL` is `FERROEHR__DATABASE__URL`,
  `FERROEHR__EHR_INDEX__PROVIDER` is `FERROEHR__RECORD__SUBJECT_INDEX`). The
  Helm chart moves to 24.0.0 with `config:` on the same tree. Every key a
  release before v5.7.0 read is refused at load with its replacement named,
  all at once, so `ferroehr config check --config <old file>` prints the whole
  move list; the refusals ship in v5.7.0 only and the next release removes
  them. To upgrade, run `config check` on the old file and move each key it
  names:
  `spec_profile`, `deployment_profile`, `deployment_accepts` and
  `licence.file` are `deployment.spec_profile`, `deployment.profile`,
  `deployment.accepts` and `deployment.licence_file`;
  `[rest]` is `[api]` (`swagger_ui` is `openapi_ui`, `body_limit_bytes` is
  `body_limit`, `cors_permissive = true` is `[api.cors] allow_origins = ["*"]`,
  `[rest.identity]` is `[api.options]` with `solution` and `vendor`; the
  version keys are facts of the build), `admin.enabled` is `api.admin` and
  `terminology.api_enabled` is `api.terminology`;
  `[db]` is `[database]` (`acquire_timeout_secs` is `acquire_timeout`,
  `statement_timeout_ms` is `statement_timeout`, `migrate` is `migrations`),
  `[storage.<domain>]` is `[database.<domain>]` and
  `[storage.migrate.<domain>] url` is `[database.<domain>] migrate_url`;
  `[smart]` is `[auth.smart]` (`enabled` is `discovery`, `episode.enabled` is
  `episode_launch`, `launch_base64_json = true` is `"launch-base64-json"` in
  `auth.capabilities`);
  `[scim]` is `[auth.scim]` (`max_results` is `filter_max_results`,
  `deactivation_window_minutes` is `deactivation_window`,
  `contact_erasure_minutes` is `contact_erasure_delay`);
  `[authz.abac.cedar] reload_secs` is `reload_interval`,
  `[authz.abac.remote] server`, `connect_timeout_ms` and `request_timeout_ms`
  are `url`, `connect_timeout` and `timeout`,
  `authz.party_binding.provisional_report_days` is `provisional_report_after`,
  and an agreement's `discard_window_hours` and `chained_log_retention_years`
  are `discard_window` and `chained_log_retention`;
  `[[origins.registered]]` is `[[authz.origin]]`;
  `management.port` is `management.listen` (an `address:port`) and
  `management.profiling.max_seconds` is `max_duration`;
  `audit.store.retention_days` and `retention_years` are `audit.store.retention`,
  `verify_interval_seconds` and `full_verify_interval_seconds` are
  `verify_interval` and `full_verify_interval`;
  `[audit.syslog]` `host` and `port` are `url = "tls://host:port"`,
  `spool_max_messages` is `spool_max_events`, `spool_max_bytes` is
  `spool_max_size`, `retry_max_ms` is `retry.max_backoff`, and
  `[audit.fhir_feed]` `poll_interval_ms` and `retry_max_ms` are
  `poll_interval` and `retry.max_backoff`;
  `[demographic.identifier_protection]` is `[privacy.identifier_protection]`,
  `[cohort]` is `[privacy.cohort]`, and `[access_delay]` is
  `[privacy.access_delay]` with `hours` as `max_delay`;
  `ehr_index.provider` is `record.subject_index` (`"ihe"` is `"pixm"`) and the
  rest of `[ehr_index]` is `[pixm]` (`pixm_url` is `url`, `invocation` is
  `method`, `request_timeout_ms` is `timeout`);
  `versioning.skip_identical_resend` is `record.versioning.idempotent_resend`;
  `[signing]` is `[record.signing]` (`key_path` is `key_file`,
  `retired_key_paths` is `retired_key_files`), `[query]` is `[record.query]`
  (`timeout_ms` is `timeout`), `[outbox]` is `[record.change_feed]`
  (`retention_days` is `retention`, `prune_interval_secs` is
  `prune_interval`), and `[multimedia]` is `[record.multimedia]` (`endpoint` is
  `url`, `threshold_bytes` is `threshold`, and `allow_http` is retired: an
  `http://` url is plain HTTP);
  `[terminology.server]` is `[terminology]` (`fail_on_error` is
  `on_error = "accept" | "refuse"`, `request_timeout_ms` is `timeout`).
  `[server]`, `[telemetry]`, `[usage_report]` and the family keys of `[auth]`
  keep their names in this change.
