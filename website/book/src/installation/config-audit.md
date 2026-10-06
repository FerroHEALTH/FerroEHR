# Audit

The IHE ATNA audit trail: `[audit]` and its three sinks. Precedence, the
environment-name grammar, and file discovery are on the
[Configuration reference](configuration.md) index.

<!-- toc -->

## `[audit]`

The IHE ATNA audit trail (see the [Audit trail chapter](../audit.md) for what a
record contains and how to search it). **On by default** with only the local
store active: every deployment gets a queryable audit trail with nothing leaving
the node, and forwarding to an external Audit Record Repository is opt-in per
sink.

```toml
[audit]
enabled = true
source_id = "ferroehr"
value_if_missing = "UNKNOWN"
suppress_login_events = true
fail_mode = "open"
resolve_subject = true
queue_capacity = 8192
purpose_header = "x-purpose-of-use"
purpose_codes = []
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Master audit switch. |
| `enterprise_site_id` | string | unset | The `AuditEnterpriseSiteID` field. |
| `source_id` | string | `ferroehr` | The audit source id, also used for the destination participant. |
| `value_if_missing` | string | `UNKNOWN` | Fill value for empty mandatory fields. |
| `suppress_login_events` | bool | `true` | Skip successful-login records. Rejected accesses (`401`/`403`) are always recorded. |
| `fail_mode` | enum{open,closed} | `open` | What an undeliverable audit record does. `open` logs, meters and lets the request succeed; `closed` rejects auditable operations with `503` (including when the local store has stopped accepting writes) so no PHI access goes un-audited. |
| `resolve_subject` | bool | `true` | Enrich the patient participant with a background lookup of the EHR's subject. The lookup runs on the background drain, never on the request path; the IHE BALP patient patterns and the patient-centric audit search need the subject. |
| `queue_capacity` | int | `8192` | Bounded audit queue capacity. Sized for write-path bursts: the drain persists in multi-row batches, so the queue only needs to ride out sink latency spikes. |
| `server_host` | string | unset ⇒ the `value_if_missing` fill | This node's advertised network address, reported as the destination `NetworkAccessPointID`. |
| `purpose_header` | string | `x-purpose-of-use` | The request header a caller declares its purpose of use in, recorded on every access record. NEN 7513 asks on whose authority a record was read and EHDS Art. 9 asks why; neither is derivable from the request, so the caller declares it. IHE carries the equivalent in a SAML attribute rather than a header, so the header is FerroEHR's own. |
| `purpose_codes` | list of string | `[]` | The purpose codes this deployment accepts. Empty records whatever the caller declares. A non-empty list records a declared code only when it is on the list, so an unagreed string does not sit in the trail reading like an established purpose. |
| `legal_basis` | string | unset | The legal basis this deployment processes under, recorded on every access record. A deployment-level fact: the controller establishes the GDPR Art. 6/9 condition once. Unset records nothing rather than a guess. |

> [!NOTE]
> The local store and the ATX:FHIR Feed both carry a FHIR R4 `AuditEvent`
> document, so both need the `fhir` build feature, on in the published binary
> and container images. A binary built with `--no-default-features` refuses at
> startup if `audit.store.enabled` or `audit.fhir_feed.enabled` is set; the
> DICOM/syslog feed needs no FHIR and stays available.

> [!NOTE]
> There is no `[atna]` section. Configuration is strict, so a file or
> environment variable still setting an `[atna]` key fails at boot with an
> unknown-key error; move the setting under `[audit]`.

### `[audit.store]`: the local Audit Record Repository

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Persist every record in the `audit` schema, served through the ITI-81 `GET /fhir/r4/AuditEvent` search. |
| `retention_days` | int | `0` | Days to keep records; `0` keeps them forever. Applied hourly by the retention reaper. A non-zero value below the retention floor of a jurisdiction the active `[privacy.identifier_scan]` rules name is a boot error naming both numbers; see [Audit trail](../audit.md#retention-and-who-chooses-it). |
| `retention_years` | int | unset | Calendar years to keep records, in place of `retention_days`. Compared exactly against the floors and ceilings, which are written in years, and reaped with calendar arithmetic. Setting it together with a non-zero `retention_days`, or to `0`, is a boot error. |
| `sgb_v_309_controller` | bool | `false` | Set only if the deploying organisation is, or acts for, one of the controllers SGB V § 307 names for a German telematics-infrastructure application. It caps the horizon at the three-year period of SGB V § 309 Abs. 1, after which Abs. 3 requires deletion without delay, so a declared controller cannot keep records forever. |
| `verify_interval_seconds` | int | `86400` | Seconds between two scheduled verifications of the store's hash chain; `0` turns the schedule off. A finding is logged at `ERROR`, counted in `atna_audit_chain_findings` and reported by the `audit_chain` indicator on `GET /health/readiness`. See [Tamper evidence](../audit.md#tamper-evidence). |

The local store is the durability anchor of the whole subsystem: with it on, the
FHIR feed drains from it, so a down repository loses nothing.

The floors and ceilings are written in calendar years. In every EU Member State
the floor is three years (EHDS Art. 9(2)), and a longer national floor wins:
five years in the Netherlands. Switzerland keeps its one-year floor. A
`retention_years` value is compared against them exactly. A `retention_days`
value is held to the most days the years can span for a floor (`1096` for three
years, `1830` for the Dutch five, `366` for one) and to the fewest for a ceiling
(`1095` for the German three). A declared § 309 controller in Germany therefore
has no `retention_days` value that passes both the EHDS floor and the § 309
ceiling, and sets `retention_years = 3`:

```toml
[audit.store]
retention_years = 3
sgb_v_309_controller = true
```

### `[audit.categories]`

The map every access record is classified through by EHDS priority category
(Annex II 3.2(c); see [the EHDS logging elements](../audit.md#the-ehds-logging-elements-mapped)).
A template id, or failing that a root archetype id, maps to a list of
categories. **Empty by default**: FerroEHR ships no map, and an access the map
cannot classify is recorded `unclassified` with its ids, never refused.

```toml
[audit.categories.templates]
"International Patient Summary" = ["patient-summary"]
"Laboratory Report" = ["test-results"]

[audit.categories.archetypes]
"openEHR-EHR-COMPOSITION.report-result.v1" = ["test-results"]
"openEHR-EHR-COMPOSITION.encounter.v1" = ["none"]
```

| Key | Type | Default | Description |
|---|---|---|---|
| `templates` | table of template id → list of category | empty | Consulted first, by the template id the object was committed against. |
| `archetypes` | table of archetype id → list of category | empty | Consulted for an object whose template the map does not name, by its root archetype id. A key that is not an archetype id refuses boot. |

The categories are `patient-summary`, `eprescription`, `edispensation`,
`imaging`, `test-results` and `discharge-report` (EHDS Art. 14(1)), a national
additional category as `national:<code>`, and `none` for content that holds no
priority-category data. Keys compare case-insensitively. An unknown spelling,
an empty list, `none` combined with a category, or two keys that differ only in
case refuses boot with a message naming the entry.

The map's SHA-256 digest is on the boot line, on `GET /management/info` under
`audit.category_map`, and on every access record. At boot the map is also
mirrored into the retention register, so a retention period can be keyed on a
category through the [admin marks API](../operations-admin-apis.md).

### `[audit.syslog]`: the classic DICOM/syslog feed (ITI-20)

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Ship DICOM PS3.15 audit records to an external repository over syslog. |
| `host` | string | `localhost` | Repository host. |
| `port` | int | `514` | Repository port (514 for UDP, 6514 for TLS, conventionally). |
| `transport` | enum{udp,tls} | `udp` | Syslog transport: RFC 5426 UDP or RFC 5425 TLS. Use `tls` for PHI-adjacent audit. |
| `tls_ca_file` | path | unset | PEM file with the repository CA to trust for the TLS transport. |
| `tls_identity_cert_file` | path | unset | Client-certificate PEM for mutual TLS. |
| `tls_identity_key_file` | path | unset | Client-key PEM for mutual TLS. |

### `[audit.fhir_feed]`: the RESTful-ATNA feed (ITI-20 ATX:FHIR Feed)

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | `POST` each FHIR `AuditEvent` to an external FHIR Audit Record Repository. |
| `url` | secret URL | `http://localhost:8080/fhir` | The repository's FHIR base; records go to `{url}/AuditEvent`. Credentials embedded in the URL are redacted from every rendering. |
| `batch_size` | int | `64` | Outbox rows shipped per poll. |
| `poll_interval_ms` | int | `2000` | Outbox poll interval when idle. |
| `max_retries` | int | `3` | Per-record `POST` retries before the record is left pending (local store on) or dropped and metered (store off). |

With the local store on, the feed drains the store's outbox and is therefore
loss-free across a repository outage. With the store off it ships in-drain, and
a record that exhausts its retries is dropped and counted.
