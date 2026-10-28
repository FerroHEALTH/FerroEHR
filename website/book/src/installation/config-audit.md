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
permit_header = "x-data-permit"
purpose_codes = []
emergency_purpose_codes = []
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
| `permit_header` | string | `x-data-permit` | The request header a secondary-use caller declares the data permit it acts under in (EHDS Art. 68), recorded on every access record as `permit`. The value is the permit's identifier, 1 to 128 characters of letters, digits and `.` `_` `:` `/` `#` `-`; any other value is recorded as absent. No specification names a header for it, so the header is FerroEHR's own. See [Secondary use](../compliance/secondary-use.md#the-access-record). |
| `purpose_codes` | list of string | `[]` | The purpose codes this deployment accepts. Empty records whatever the caller declares. A non-empty list records a declared code only when it is on the list, so an unagreed string does not sit in the trail reading like an established purpose. |
| `emergency_purpose_codes` | list of string | `[]` | The purpose codes that mark an access as an [emergency access](../audit.md#emergency-access-ehds-art-115) in the vital interest of the patient (EHDS Art. 11(5)). Each must also be on `purpose_codes`, or the server refuses to start. Empty marks nothing. The mark is recorded on the access record and shown to the patient through the access log; it lifts no restriction. |
| `legal_basis` | string | unset | The legal basis this deployment processes under, recorded on every access record. A deployment-level fact: the controller establishes the GDPR Art. 6/9 condition once. Unset records nothing rather than a guess. |

> [!NOTE]
> The local store renders a FHIR R4 `AuditEvent` from each record when it is
> read, and the ATX:FHIR Feed sends one, so both need the `fhir` build feature, on in the published binary
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
| `verify_interval_seconds` | int | `3600` | Seconds between two scheduled verifications of the store's hash chain; `0` turns the schedule off. Each run verifies the records written since the newest link an earlier run found intact. A finding is logged at `ERROR`, counted in `atna_audit_chain_findings` and reported by the `audit_chain` indicator on `GET /health/readiness`. See [Tamper evidence](../audit.md#tamper-evidence). |
| `full_verify_interval_seconds` | int | `604800` | Seconds between two scheduled full passes over the hash chain, which also recompute the records before the checkpoint; `0` leaves the full pass to `SELECT * FROM audit.verify_audit_chain()` on demand. A run that found damage is always followed by a full pass. |

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

The map every access record is classified through by EHDS category (Annex II
3.2(c); see [the EHDS logging elements](../audit.md#the-ehds-logging-elements-mapped)).
The classification is done by `ehds-logging`, the EHDS logging component the
FerroHEALTH products share (FerroAUDIT); this section is the deployment's map.
A template id, or failing that a root archetype id, maps to a list of
categories, or to the word `none` for data of no category. **Empty by
default**: FerroEHR ships no map, and an access the map cannot classify is
recorded `unclassified` with its ids, never refused.

```toml
[audit.categories]
national = ["urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld"]

[audit.categories.templates]
"International Patient Summary" = ["Patient-Summaries"]
"Laboratory Report" = ["Laboratory-Reports"]

[audit.categories.archetypes]
"openEHR-EHR-COMPOSITION.report-result.v1" = ["Laboratory-Reports"]
"openEHR-EHR-COMPOSITION.encounter.v1" = "none"
```

| Key | Type | Default | Description |
|---|---|---|---|
| `national` | list of `<system>\|<code>` | empty | The national categories (EHDS Art. 14(1) third subparagraph) the map may name. |
| `templates` | table of template id → list of category, or `"none"` | empty | Consulted first, by the template id the object was committed against. |
| `archetypes` | table of archetype id → list of category, or `"none"` | empty | Consulted for an object whose template the map does not name, by its root archetype id. A key that is not an archetype id refuses boot. |

A priority category of EHDS Art. 14(1) is named by its code in HL7 Europe's
`EEHRxFDocumentPriorityCategoryCS`: `Patient-Summaries`,
`Electronic-Prescriptions`, `Electronic-Dispensations`, `Medical-Imaging`,
`Laboratory-Reports` and `Discharge-Reports`, or as `<system>|<code>`. A
national category is named by the `<system>|<code>` it is declared with. Keys
compare case-insensitively. An unknown code, an empty list, a word other than
`none`, or two keys that differ only in case refuses boot with a message naming
the entry. A record stores each category as its `<system>|<code>` token.

The map's SHA-256 digest is on the boot line, on `GET /management/info` under
`audit.category_map`, and on every access record. At boot the map is also
mirrored into the retention register, so a retention period can be keyed on a
category through the [admin marks API](../operations-admin-apis.md).

### `[audit.syslog]`: the classic DICOM/syslog feed (ITI-20)

The DICOM PS3.15 audit message, framed as RFC 5424 syslog and sent over RFC
5425 TLS 1.2 or later by `ihe-atna` (FerroAUDIT). TLS is the one transport: an
audit message names a patient (ITI TF-2 §3.20.4.1.2.1.1).

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | Ship DICOM PS3.15 audit records to an external repository over syslog. |
| `host` | string | `localhost` | Repository host. |
| `port` | int | `6514` | Repository port, the RFC 5425 port by default. |
| `tls_ca_file` | path | unset | PEM file with the repository CA to trust beside the platform's roots. |
| `tls_identity_cert_file` | path | unset | Client-certificate PEM for mutual TLS. |
| `tls_identity_key_file` | path | unset | Client-key PEM for mutual TLS. |
| `spool_dir` | path | unset | The directory of the spool every frame is stored in before it is sent. It must give no access to the group or to other users. Unset, the spool is held in memory, and a restart loses the frames not yet delivered. |
| `spool_max_messages` | int | `100000` | The most frames the spool holds; a frame past it is refused and counted in `atna_audit_dropped`. |
| `spool_max_bytes` | int | `268435456` | The most bytes the spool holds. |
| `retry_max_ms` | int | `60000` | The longest wait between two delivery attempts after a failure. |

The drain stores each frame in `ihe-atna`'s spool, and the crate's forwarder
delivers it: one connection held open, a failed send retried with an
exponential backoff and jitter, and the connection closed with a TLS
`close_notify` on shutdown, so nothing is lost to a reset (ITI TF-2
§3.20.4.1.1, RFC 5425 §4.4). The local audit store keeps only the FHIR
document, never the syslog frame, so the spool, not the store, is syslog's
outbox. `delivered_syslog_at` is no longer written.

### `[audit.fhir_feed]`: the RESTful-ATNA feed (ITI-20 ATX:FHIR Feed)

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `false` | `POST` each FHIR `AuditEvent` to an external FHIR Audit Record Repository. |
| `url` | secret URL | `http://localhost:8080/fhir` | The repository's FHIR base; records go to `{url}/AuditEvent`. Credentials embedded in the URL are redacted from every rendering. |
| `poll_interval_ms` | int | `2000` | How long the forwarder waits for a new record before it reads the store again. |
| `retry_max_ms` | int | `60000` | The longest wait between two delivery attempts after a failure. |

`ihe-atna`'s forwarder delivers the feed. With the local store on, it
delivers from the store, oldest record first: a record not yet delivered has
no `delivered_fhir_feed_at`, so a repository outage loses nothing, and the
record is stamped once the repository accepts it. A record the repository
refuses (a `4xx` other than `401`, `408` or `429`) is set aside until the next
start. It is reported on `GET /management/health` as `audit_delivery`, and
the records behind it keep flowing. With the store off, the feed delivers
from an in-memory spool. `batch_size` and `max_retries` are gone and stop
startup by name: the forwarder retries until the repository answers.

#### `[audit.fhir_feed.oauth]`: authenticating with OAuth 2.0 client credentials

The feed authenticates with HTTP Basic through the credentials in `url`, or,
with this table, as an OAuth 2.0 client (the IHE IUA authorized-client model):
FerroEHR obtains a token with the client credentials grant, sends it as a
bearer token, renews it before it expires, and when the repository refuses the
held token with `401` fetches a new one and sends the record once more. Set
one of the two, never both: a `url` with credentials beside this table stops
startup.

| Key | Type | Default | Description |
|---|---|---|---|
| `token_endpoint` | URL | unset | The token endpoint. Set this or `issuer`. |
| `issuer` | URL | unset | The issuer whose metadata (`/.well-known/oauth-authorization-server`) names the token endpoint. |
| `client_id` | string | unset | The client identifier. Required. |
| `client_secret` / `client_secret_file` | secret | unset | The client secret, inline or read from a file. One is required. |
| `client_auth` | enum{client_secret_basic,client_secret_post,private_key_jwt,tls_client_auth,self_signed_tls_client_auth} | `client_secret_basic` | How the client authenticates to the token endpoint. |
| `scope` | string | unset | The `scope` of the token request. |
| `audience` | string | unset | The `audience` parameter, for an authorization server that reads it. |
| `resource` | URL | unset | The `resource` indicator (RFC 8707) naming the repository. |
| `private_key_file` | path | unset | The PEM private key that signs the client assertion under `private_key_jwt` (RFC 7523). |
| `kid` | string | the RFC 7638 thumbprint | The `kid` the client assertion names. |
| `assertion_alg` | string | by key type | The algorithm the client assertion is signed with. |
| `assertion_audience` | enum{token_endpoint,issuer} | `token_endpoint` | The `aud` of the client assertion; `issuer` needs `issuer` set. |
| `client_certificate_file` / `client_key_file` | path | unset | The client certificate chain and its key presented over mutual TLS (RFC 8705); set together. |
| `ca_file` | path | unset | PEM trust roots added to the built-in ones, for a token endpoint behind a private certificate authority. |
| `refresh_margin_ms` | int | `30000` | How long before the token expires a new one is fetched. |
| `timeout_ms` | int | `10000` | How long one call to the authorization server may take. |

With the local store on, the feed delivers from the store and loses nothing
across a repository outage. With the store off it delivers from an in-memory
spool, so a restart loses what the spool holds.
