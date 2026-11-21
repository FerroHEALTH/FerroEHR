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
# source_id = "cdr.hospital.example"   # unset: [api] system_id
value_if_missing = "UNKNOWN"
suppress_login_events = true
resolve_subject = true
purpose_header = "x-purpose-of-use"
permit_header = "x-data-permit"
purpose_codes = []
purpose_system = "http://terminology.hl7.org/CodeSystem/v3-ActReason"
emergency_purpose_codes = []
```

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | Master audit switch. |
| `enterprise_site_id` | string | unset | The `AuditEnterpriseSiteID` field. |
| `source_id` | string | unset ⇒ `[api] system_id` | The audit source id (`AuditSourceID`, the BALP `AuditEvent.source`), also used for the destination participant. |
| `role_code_system` | string | unset ⇒ `[api] system_id` | The `codeSystemName` of the `RoleIDCode` a caller's roles are written as in the DICOM audit message: the deployment's own role vocabulary, which no published code system names. |
| `value_if_missing` | string | `UNKNOWN` | Fill value for empty mandatory fields. |
| `suppress_login_events` | bool | `true` | Skip successful-login records. Rejected accesses (`401`/`403`) are always recorded. |
| `resolve_subject` | bool | `true` | Enrich the patient participant with an indexed lookup of the EHR's subject, remembered per EHR for an hour, so a request seldom asks. The IHE BALP patient patterns and the patient-centric audit search need the subject. |
| `server_host` | string | unset ⇒ the `value_if_missing` fill | This node's advertised network address, reported as the destination `NetworkAccessPointID`. |
| `purpose_header` | string | `x-purpose-of-use` | The request header a caller declares its purpose of use in, recorded on every access record. NEN 7513 asks on whose authority a record was read and EHDS Art. 9 asks why; neither is derivable from the request, so the caller declares it. IHE carries the equivalent in a SAML attribute rather than a header, so the header is FerroEHR's own. An issuer with an `[auth.issuer.<name>.attributes]` table states the purposes in its token, and a header naming another purpose is refused ([where the purpose comes from](../audit.md#where-the-purpose-of-use-comes-from)). |
| `permit_header` | string | `x-data-permit` | The request header a secondary-use caller declares the data permit it acts under in (EHDS Art. 68), recorded on every access record as `permit`. The value is the permit's identifier, 1 to 128 characters of letters, digits and `.` `_` `:` `/` `#` `-`; any other value is recorded as absent. No specification names a header for it, so the header is FerroEHR's own. See [Secondary use](../compliance/secondary-use.md#the-access-record). |
| `purpose_codes` | list of string | `[]` | The purpose codes this deployment accepts. Empty records whatever the caller declares. A non-empty list records a declared code only when it is on the list, so an unagreed string does not sit in the trail reading like an established purpose. |
| `purpose_system` | string | `http://terminology.hl7.org/CodeSystem/v3-ActReason` | The canonical URL of the FHIR `CodeSystem` the `purpose_codes` belong to, HL7's `ActReason` by default. `GET {extensions}/access/purposes` answers the header, this system and each accepted code with the display text the [terminology server](config-integrations.md#terminology)'s `CodeSystem/$lookup` gives it, so a client such as the viewer offers the codes by name. Any authenticated caller reads that list, one with no role included: it names no personal data. No specification names the system of a purpose header, so the key is FerroEHR's own. |
| `emergency_purpose_codes` | list of string | `[]` | The purpose codes that mark an access as an [emergency access](../audit.md#emergency-access-ehds-art-115) in the vital interest of the patient (EHDS Art. 11(5)). Each must also be on `purpose_codes`, or the server refuses to start. Empty marks nothing. The mark is recorded on the access record and shown to the patient through the access log; it lifts no restriction. |
| `legal_basis` | string | unset | The legal basis this deployment processes under, recorded on every access record. A deployment-level fact: the controller establishes the GDPR Art. 6/9 condition once. Unset records nothing rather than a guess. |

Every record of a request is written to the local store before the response
leaves, and a request whose records cannot be written is refused with `503`
and `Retry-After`. There is no fail mode and no queue: `fail_mode` and
`queue_capacity` are unknown keys, refused at load. See
[how a record is written](../audit.md#how-a-record-is-written).

> [!NOTE]
> The local store writes each access-log entry with its FHIR R4 `AuditEvent`
> projection, so auditing needs the `fhir` build feature, on in the published
> binary and container images. A binary built with `--no-default-features`
> refuses to start with `[audit] enabled = true`.

> [!NOTE]
> There is no `[atna]` section. Configuration is strict, so a file or
> environment variable still setting an `[atna]` key fails at boot with an
> unknown-key error; move the setting under `[audit]`.

### `[audit.store]`: the local Audit Record Repository

| Key | Type | Default | Description |
|---|---|---|---|
| `verify_interval` | duration | `"1h"` | The time between two scheduled verifications of the store's hash chain; `"0s"` turns the schedule off. Each run verifies the records written since the newest link an earlier run found intact. A finding is logged at `ERROR`, counted in `atna_audit_chain_findings` and reported by the `audit_chain` indicator on `GET /health/readiness`. See [Tamper evidence](../audit.md#tamper-evidence). |
| `full_verify_interval` | duration | `"7d"` | The time between two scheduled full passes over the hash chain, which also recompute the records before the checkpoint; `"0s"` leaves the full pass to `SELECT * FROM audit.verify_audit_chain()` on demand. A run that found damage is always followed by a full pass. |
| `max_verify_lag` | duration | `"4h"` | How long a record may wait past its chain's verification cursor before the `audit_chain` indicator reads `DEGRADED`; `"0s"` reports the lag only as the `audit_chain_verification_lag` and `audit_chain_unverified_age_seconds` metrics. Set it above `verify_interval`. |

The local store is always on while auditing is: every record is written to
it before its access is reported, and syslog and the FHIR feed deliver from
it, so a down repository loses nothing. `enabled` is an unknown key, refused
at load.

How long the store and the access log keep a record is the controller's
access policy (`ACCESS_POLICY.log_retention`), not a configuration key: the
period, whether EHDS binds the controller, and the floors and ceilings its law
sets, each with its provision. `retention` and `sgb_v_309_controller` are
unknown keys and refused at load. See
[Audit trail](../audit.md#retention-and-who-chooses-it).

### The audit category register

The map every access record is classified through by EHDS category (Annex II
3.2(c); see [the EHDS logging elements](../audit.md#the-ehds-logging-elements-mapped))
is an [admin register](../operational-policy.md#the-admin-registers), not a
configuration key: you read and replace it through
`{ext}/admin/registers/audit-categories`, every version is kept with its
committer, and a change applies on the next request without a restart. The
classification is done by `ehds-logging`, the EHDS logging component the
FerroHEALTH products share (FerroAUDIT); the register is the deployment's map.
A template id, or failing that a root archetype id, maps to a list of
categories, or to the word `none` for data of no category. An access the map
in force cannot classify is recorded `unclassified` with its ids, never
refused. `[audit.categories]` in `ferroehr.toml` is an unknown key and refuses
boot.

**The shipped map.** While the register declares no template and no
archetype, the map FerroEHR ships applies
([`app/ferroehr/assets/audit-categories.default.toml`](https://github.com/FerroHEALTH/FerroEHR/blob/main/app/ferroehr/assets/audit-categories.default.toml)).
It is also the register's default, version 0, before anyone writes it. It
names the published CKM templates and COMPOSITION archetypes whose purpose
places their data in one priority category:

| Key | Category |
|---|---|
| template `International Patient Summary`, archetype `openEHR-EHR-COMPOSITION.health_summary.v1` | `Patient-Summaries` |
| template `ePrescription (FHIR)`, archetype `openEHR-EHR-COMPOSITION.prescription.v0` | `Electronic-Prescriptions` |
| template `Generic lab test result example simple` | `Laboratory-Reports` |
| template `CCTA report` | `Medical-Imaging` |
| archetype `openEHR-EHR-COMPOSITION.transfer_summary.v1` | `Discharge-Reports` |

It leaves out the generic `report.v1` and `report-result.v1` containers, which
carry laboratory, imaging and other reports alike, so an access through them
is recorded `unclassified` instead of guessed. CKM publishes no composition for
an electronic dispensation. A map that declares any template or archetype
replaces the shipped map whole, so copy its entries into your own map to keep
them. A national category declared alone keeps the shipped map.
`shipped_map = false` with nothing declared leaves no map: every access is then
recorded `unclassified`. With `deployment.profile = "production"` and auditing
on, the register refuses that write as the `no_category_map` gap unless
`deployment.accepts` names it; this floor stays in `ferroehr.toml`.

```json
{
  "national": ["urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld"],
  "templates": {
    "International Patient Summary": ["Patient-Summaries"],
    "Laboratory Report": ["Laboratory-Reports"]
  },
  "archetypes": {
    "openEHR-EHR-COMPOSITION.report-result.v1": ["Laboratory-Reports"],
    "openEHR-EHR-COMPOSITION.encounter.v1": "none"
  }
}
```

| Member | Type | Default | Description |
|---|---|---|---|
| `shipped_map` | bool | `true` | Apply the map FerroEHR ships while no template and no archetype is declared. |
| `national` | list of `<system>\|<code>` | empty | The national categories (EHDS Art. 14(1) third subparagraph) the map may name. |
| `templates` | object of template id → list of category, or `"none"` | empty | Consulted first, by the template id the object was committed against. |
| `archetypes` | object of archetype id → list of category, or `"none"` | empty | Consulted for an object whose template the map does not name, by its root archetype id. A key that is not an archetype id refuses the write. |

A priority category of EHDS Art. 14(1) is named by its code in HL7 Europe's
`EEHRxFDocumentPriorityCategoryCS`: `Patient-Summaries`,
`Electronic-Prescriptions`, `Electronic-Dispensations`, `Medical-Imaging`,
`Laboratory-Reports` and `Discharge-Reports`, or as `<system>|<code>`. A
national category is named by the `<system>|<code>` it is declared with. Keys
compare case-insensitively. An unknown code, an empty list, a word other than
`none`, or two keys that differ only in case refuses the write with a message
naming the entry, and so does a map that no longer names a category the
[access-delay register](config-auth.md#the-access-delay-register) holds. A
record stores each category as its `<system>|<code>` token.

The digest of the map that classified a record is on the record. A write of
the register also mirrors the map into the retention register in the same
transaction, so a retention period can be keyed on a category through the
[admin marks API](../operations-admin-apis.md).

### The origins a deployment recognises

Every version records where its data came from at commit: each `FEEDER_AUDIT`
originating-system id its body carries, and the access log reports them (EHDS
Annex II 3.2(e)). Annex II 3.4 also asks for retention periods and access
rights that take the origin into account. The ids a sending system writes are
free text, so the origins your deployment recognises are policy in the CDR,
not configuration: each is the `origin` element of a `data_origin` grant in
force, which a
[retention period](../compliance/retention.md#keying-a-period-on-an-origin)
and an [access directive's data scope](../security.md#access-directives) may then
name ([Operational policy](../operational-policy.md#what-the-grants-decide)).
A body that carries no `FEEDER_AUDIT`, committed by the holder of such a
grant, is stamped with its origin; a direct commit by any other caller keeps
this server's `[api] system_id`.

### `[audit.syslog]`: the classic DICOM/syslog feed (ITI-20)

The DICOM PS3.15 audit message, framed as RFC 5424 syslog and sent over RFC
5425 TLS 1.2 or later by `ihe-atna` (FerroAUDIT). TLS is the one transport: an
audit message names a patient (ITI TF-2 §3.20.4.1.2.1.1).

| Key | Type | Default | Description |
|---|---|---|---|
| `url` | string | `tls://localhost:6514` | The repository, `tls://host:port`; RFC 5425 §4.1 names port 6514. The table's presence ships records; absent, nothing is sent. |
| `tls.trust_roots_file` | path | unset | PEM bundle of trust roots for the repository, beside the platform's. |
| `tls.client_identity_file` | path | unset | The client certificate chain and its private key in one PEM, for mutual TLS. |
| `tls.client_identity` | secret | unset | The same PEM inline, in place of `client_identity_file`. |
| `tls.min_version` | string | `"1.3"` | The oldest TLS version the client speaks; set `"1.2"` for a repository that offers no TLS 1.3. |
| `retry.max_backoff` | duration | `"1min"` | The longest wait between two delivery attempts after a failure. |

`[audit.syslog.tls]` is the outbound TLS table every FerroHEALTH product
takes, as `[pixm.endpoint.tls]` is. Concatenate the certificate chain and the private
key into the one PEM `client_identity_file` names.

`ihe-atna`'s forwarder delivers from the local store, oldest record first:
the chainer enters each stored record in `audit.syslog_outbox`, and the
forwarder renders its DICOM message from the stored record when it reads
it. It holds one connection open, retries a failed send with an exponential
backoff and jitter, and closes the connection with a TLS `close_notify` on
shutdown, so nothing is lost to a reset (ITI TF-2 §3.20.4.1.1, RFC 5425
§4.4). A record leaves `audit.syslog_outbox` once the repository takes it;
the forwarder's depth, the records the outbox still holds, is reported on
`GET /management/health` as `audit_delivery`. `spool_dir`, `spool_max_events` and `spool_max_size` are
unknown keys, refused at load.

### `[audit.fhir_feed]`: the RESTful-ATNA feed (ITI-20 ATX:FHIR Feed)

| Key | Type | Default | Description |
|---|---|---|---|
| `endpoint` | [endpoint table](config-integrations.md#the-endpoint-table) | required | The repository: `url`, its FHIR base (records go to `{url}/AuditEvent`; credentials embedded in it are masked in every rendering), `timeout` for one `POST`, the `retry` of a delivery (`retry.max_backoff` defaults to `"1min"` here), the TLS and the credential. The table's presence turns the feed on. |
| `poll_interval` | duration | `"2s"` | How long the forwarder waits for a new record before it reads the store again. |

`ihe-atna`'s forwarder delivers the feed from the local store, oldest record
first: a record not yet delivered keeps its row in `audit.fhir_feed_outbox`,
so a repository outage loses nothing, and the row goes once the repository
accepts the record. The records the outbox still holds are the forwarder's
depth on `GET /management/health` (`audit_delivery`). A record the repository
refuses (a `4xx` other than `401`, `408` or `429`) is set aside until the next
start. It is reported on `GET /management/health` as `audit_delivery`, and
the records behind it keep flowing. The forwarder retries until the
repository answers.

#### Authenticating the feed

The feed authenticates with the credential of `[audit.fhir_feed.endpoint]`:
HTTP Basic, or the OAuth 2.0 client credentials grant
`[audit.fhir_feed.endpoint.credentials.oauth]` (the IHE IUA authorized-client
model), with the [grant's keys](config-integrations.md#the-endpoint-table).
Under the grant FerroEHR sends the token as a bearer token, renews it before
it expires, and when the repository refuses the held token with `401` fetches
a new one and sends the record once more. Credentials in `url` beside a
credential stop startup.

The feed delivers from the store and loses nothing across a repository
outage or a restart.

### `[audit.access_log]`: publication and trusted systems

| Key | Type | Default | Description |
|---|---|---|---|
| `publish_directory` | path | unset | A directory outside the database, ideally on write-once storage, where the server publishes the newest access-log checkpoint every 30 seconds as its own file, created once and never rewritten. Unset, the `access_log_checkpoints` health indicator reads `DEGRADED`. |
| `trusted_key_files` | list of paths | `[]` | Armored OpenPGP public certificates of the systems whose exported logs this deployment imports. An import is refused when neither these nor the keyring's own key signed its checkpoint. |

Signing the access log needs no key of this section. Whenever
`[record.signing]` holds an OpenPGP key (`mode = "pgp"` with a `key_file`),
every append of the access log ends, in its own transaction, with one signed
checkpoint of its last link; without one nothing is sealed. Use an Ed25519
key: every audit write waits while an append holds the head of the chain, and
RSA signing is far slower. The key is the one `[record.signing]` configures
for version signatures, with its retired certificates: one key per
deployment, kept in the secret store its `key_file` and `key_passphrase_file`
name.

Set `publish_directory` in production. Someone who can write the database
can remove the newest entries together with the checkpoints that seal them,
and only a checkpoint published where that person cannot write shows it.
Mount the directory from write-once storage, or from storage another system
collects from, and give every instance the same one. See the
[audit page](../audit.md#checkpoints-and-the-transfer-of-a-log).
