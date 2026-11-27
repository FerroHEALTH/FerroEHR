# Audit trail (IHE ATNA)

FerroEHR keeps a full security audit trail of API access (_who_ did _what_
to _which_ resource, with _what outcome_, from _where_, and _when_) following
the IHE **ATNA** (Audit Trail and Node Authentication) profile, the standard
the openEHR reference text points at (its platform service description names a
system log "IHE ATNA-compliant"). It is **on by default**: every
deployment records a queryable audit trail with zero external dependencies.

> [!WARNING]
> **Auditing off** (`[audit] enabled = false`) writes no access log and leaves
> the EHDS logging component off. It is legitimate to run and wrong to run
> silently, so the server says it at `warn` on boot (structured field
> `posture = "audit"`), in the `audit_sender` readiness indicator and on
> `/management/info`. With auditing on, FerroEHR refuses an access it cannot
> record, at any rate: see [how a record is written](#how-a-record-is-written).
> A deployment declaring
> [`deployment.profile = "production"`](installation/configuration.md#deploymentprofile)
> refuses to start with auditing off.

<!-- toc -->

The trail is orthogonal to openEHR's own `CONTRIBUTION`/`AUDIT_DETAILS`
change-control audit (which the server always writes in the same transaction
as every version change): openEHR audit records what a version says about its
own authorship; ATNA records security surveillance of *access*, including
reads and rejected attempts.

This page covers the record and its sinks. The openEHR entry is on [The access
log](access-log.md), the EHDS elements on [The EHDS logging
elements](ehds-logging.md), and the seals, ITI-81 retrieval and node
authentication on [Tamper evidence, seals and retrieval](audit-integrity.md).

## The record, in both official formats

Every audited operation produces one record, rendered in the two formats the
IHE standards define. Each operation the server mounts names the action and
resource class its record carries, and an operation without one is refused
with `500` rather than recorded under a guessed class. An act that writes its
own record, such as an access-log erasure, is that one record on every sink:
syslog, the store and the FHIR Feed. Both are written by the shared audit crates of the
FerroHEALTH products (FerroAUDIT): `ehds-logging` writes the EHDS access
record, and `ihe-atna` writes the BALP `AuditEvent` and the DICOM audit
message and sends them. FerroEHR decides which accesses are logged, what each
record says and where it is stored.

- **FHIR R4 `AuditEvent`** following the IHE **BALP** 1.1.4 (Basic Audit Log
  Patterns) content profiles: the modern RESTful-ATNA form and the canonical
  stored form. An access to clinical content is the EHDS access record:
  `ehds-logging` writes it on the `RESTful` pattern of its action, with the
  patient behind the EHR as the patient entity (`PatientRead`,
  `PatientCreate` and the other `Patient*` claims), the categories, the
  retention, the security labels and the origins. Every other record (an
  authentication, a template or party operation, a refused access) is a BALP
  exchange FerroEHR fills directly. The person is the `agent:user` slice,
  named by the token's issuer and subject, with the token's `jti` as its
  `policy`, the caller's roles as its `role` and the client application as an
  agent of its own (BALP 1.1.4 §3:5.7.5.4); a Basic user is named by the
  account alone, with no issuer and no client. The ITS-REST operation id is a
  second `subtype` (`urn:openehr:its-rest:operation`). The object an access
  record reads, creates, updates or deletes is named by its openEHR object id
  as `entity.what.identifier` in the system `urn:openehr:object-id`, never as
  a `what.reference`, since it is no FHIR resource (FHIR R4 references.html
  §2.3.0.3). The outcome is stated
  in `outcomeDesc`, and `source.type` names an application server. Query
  executions carry the search expression.
- **DICOM Audit Message** (DICOM PS3.15 §A.5 XML): the classic ATNA form,
  shipped as RFC 5424 syslog over RFC 5425 TLS per IHE ITI-20 when the syslog
  sink is enabled, with the optional `EventOutcomeDescription`,
  `AuditSourceTypeCode` (`4`, an application server) and each participant
  object's `ParticipantObjectDataLifeCycle`. A record about a patient's data
  names the patient (`Patient Number`) and, when it has one, its object as a
  System Object with the ID type URI (PS3.15 Tables A.5.1.2.2-1 and
  A.5.1.2.4-1), the same id the FHIR form names.
  Dedicated DICOM event ids are used throughout: Patient Record (110110),
  Query (110112), Export/Import (110106/110107, the EHR-Extract directions),
  User Authentication (110114, with the Login `EventTypeCode` 110122).

Both formats give the client and the server the same data-flow role: on a
read the server is Source (DCM 110153) and the client Destination (110152),
because the data moves from the server to the client (DICOM PS3.15 §A.5.2.7,
the BALP `Read` profiles); on a create, an update and a query the client is
Source (PS3.15 §A.5.3.20 for a query, the BALP `Create`, `Update` and `Query`
profiles).

Every server operation is audited (an unrecognised extension operation fails
*closed* to a generic audited class, so nothing is silently unaudited) and
access refusals are always recorded: a `401`, a `403`, and the `400` a
malformed `Authorization` header earns. A refusal is attributed to the caller
**only when one actually authenticated**; an unattributable denial is recorded
as unattributed rather than under a fabricated subject, so no audit record
ever names an identity that did not authenticate.

> [!NOTE]
> **Successful-login records are suppressed by default**
> (`[audit] suppress_login_events = true`). Set it to `false` to record them.
> Even then, a login record marks a genuine authentication *event*, not every
> authenticated request: it is emitted when credentials were actually
> verified: a Basic verified-credential-cache miss. A cache hit continues an
> established session, and a Bearer request authenticated out of band at your
> identity provider, so neither mints a per-request login record. Rejections
> are recorded regardless of this switch.

## Reads are logged too, per record

A write leaves its own trail in openEHR's contribution and audit chain. A read
leaves nothing behind unless the server records it, and reads are what the
access-logging rules are about: [NEN 7513](https://www.nen.nl) requires per-access
logging of who consulted which record and on whose authority, the
[Wabvpz](https://wetten.overheid.nl/BWBR0023864) gives the patient the right to
know who consulted their record, and
[EHDS Art. 9](https://eur-lex.europa.eu/eli/reg/2025/327/oj) requires logging of
access to electronic health data for primary use. No openEHR specification
governs the read side, so this is FerroEHR's own extension.

Every retrieval in the EHR, Query and Demographic APIs produces a record, and a
test walks the generated route tables to prove it: a newly generated `GET` that
emits nothing fails the build.

Each record additionally carries:

| Field | What it holds |
|---|---|
| `domain` | The pseudonymisation domain read: `ehr`, `demographic`, `linkage` for a crossing that resolves a party or a subject to its EHRs, or `system` for an operation that touches no domain. |
| `purpose` | The purpose of use the caller declared in the `x-purpose-of-use` header, when the deployment accepts it. |
| `token_attributes` | The purposes of use and the professional roles the access token states, each with its code system, for an issuer with an `[auth.issuer.<name>.attributes]` table ([where the purpose comes from](ehds-logging.md#where-the-purpose-of-use-comes-from)). |
| `justification` | `{"kind": "justification" \| "emergency", "reason": …}` when the access rested on a justification of the access policy, with the reason the clinician stated; `NULL` otherwise ([justified and emergency access](ehds-logging.md#justified-and-emergency-access)). |
| `legal_basis` | The legal basis the deployment processes under, from `[audit.regulatory] legal_basis`: the GDPR Art. 6(1) point and, for data concerning health, the Art. 9(2) condition, stored as `{"lawfulness": "public-task", "condition": "health-or-social-care"}`. |
| `result_count` | How many records the operation served. |
| `request_id` | The `x-request-id` correlation id the response carried. |

A query is the case a single record cannot describe. One AQL statement is one
operation but potentially many disclosures, so it produces its own execute
record **plus one access record per EHR it served**, each carrying that EHR's
served-row count. The served set is read off the rows the caller actually
received rather than the rows the statement matched: a legal record of access
must not name an EHR whose content was never disclosed. Each row names the EHR
of every `EHR` source and every versioned object it was drawn from, so a
statement that reads only the EHR is attributed too. Every EHR a query
touches is accessed (EHDS Art. 9(1)): an EHR the query names by its `ehr_id`
scope gets an access record whatever its rows hold, a `COUNT`, an empty page
and a page past the end included, and an aggregate projection names, per row,
every EHR whose data its aggregate read, so a population `COUNT` writes one
access record per contributing EHR. A query the access evaluator refuses is
recorded against the EHR it asked for. One shape carries no per-EHR
breakdown, because the shape makes it underivable: `SELECT DISTINCT`, where an
extra column changes which rows are distinct. It records the statement and its
row count.

### NEN 7513 content, field by field

NEN 7513 §5 lists what a logged event has to contain. This is where each item
lives in the record:

| NEN 7513 asks for | FerroEHR records it as |
|---|---|
| The identity of the person who accessed the record | `principal`: the authenticated Basic username or OAuth `sub`; an unattributable denial is recorded as unattributed, never under a fabricated identity. When a client acts for a person, `professional_id` names that natural person |
| The identity of the patient whose record was accessed | `patient_id`: the resolved EHR subject, which under the pseudonymisation boundary is an opaque pseudonym |
| Which record, or which part of it | `resource_class` + `resource_id`: the version uid, party uid or EHR id the operation touched |
| The date and time | `recorded_at`, the event time; `stored_at` is when the row was persisted |
| The kind of action | `action` (DICOM `C`/`R`/`U`/`D`/`E`) plus `operation`, the released ITS-REST operation id. It equals the served OpenAPI `operationId` except for the demographic CONTRIBUTION pair, served as `demographic_contribution_create` and `demographic_contribution_get` because the released `contribution_create` and `contribution_get` also name the EHR operations and one document needs unique ids |
| Whether it succeeded | `outcome`, the DICOM outcome indicator (0 success, 4 minor, 8 serious, 12 major) |
| Under which role or authority | `roles`: the roles the caller held at access time (the RFC 9068 §2.2.3.1 claim carriers for a bearer, the user definition for Basic), rendered as FHIR `agent.role` on the requestor and as a `RoleIDCode` per role on the DICOM requesting participant |
| On whose authority, and why | `purpose` and `legal_basis`, and `permit`, the identifier of the data permit a secondary-use caller declares it acts under ([Secondary use](compliance/secondary-use.md#the-access-record)) |
| Which system, and from where | `AuditSourceID` and the source participant's network address (`client_ip`): the connection's peer, or the client a peer in `[server] trusted_proxies` names in its `forwarded_header` ([Server](installation/config-server.md)) |
| Who the person is in the demographic store, and for whom they act | `binding` and the fields beside it ([the binding outcome](ehds-logging.md#the-binding-outcome)): `party` (the bound `PERSON`), `party_groups`, `provider`, `representation` for a representative, and `acting_mode` (`person`, `delegated`, `external` or `system`); for a verified external accessor `external_fields` and `agreement` stand in for the party, and an `organisation` record names the organisation. The FHIR `AuditEvent`, the DICOM message and the syslog feed carry each in the `ehds-accessor` and `ehds-relayed` entities ([the binding fields in the renderings](ehds-logging.md#the-binding-fields-in-the-renderings)) |

Two of these are the deployment's to supply. `purpose` is only as meaningful as
the vocabulary you agree with your callers: set `[audit] purpose_codes` to that
vocabulary and a code outside it is recorded as absent rather than as free text
that reads at review time like an established purpose. The legal basis is a
deployment-level fact and is recorded on every access once you declare it in
`[audit.regulatory]`, beside the controllers and processors the FHIR
`AuditEvent` names (Xt-EHR D5.2 §7.2.7):

```toml
[audit.regulatory]
legal_basis = { lawfulness = "public-task", condition = "health-or-social-care" }
controllers = ["urn:oid:2.16.528.1.1007.3.3:zh-noordwest"]
processors = ["urn:oid:2.16.528.1.1007.3.3:hosting-provider"]
```

`lawfulness` names the GDPR Art. 6(1) point (`consent`, `contract`,
`legal-obligation`, `vital-interests`, `public-task` or
`legitimate-interests`) and `condition` the Art. 9(2) point
(`explicit-consent`, `employment`, `vital-interests`, `not-for-profit-body`,
`made-public`, `legal-claims`, `substantial-public-interest`,
`health-or-social-care`, `public-health` or `archiving-or-research`). The
FHIR `AuditEvent` writes them as `gdpr-art-6-1-<point>` and
`gdpr-art-9-2-<point>`.

Mapping the recorded fields onto your own retention schedule and review process
stays yours: the software records the events and serves them back, and no
supplier document discharges the obligation to review them.

## Sinks

Records fan out to independently configured sinks (`[audit]` in
`ferroehr.toml`; see the
[configuration reference](installation/config-audit.md#audit)):

| Sink | Default | What it does |
|---|---|---|
| `[audit.store]` | **always on** | The local **Audit Record Repository**: records persist in the dedicated `audit` PostgreSQL schema (append-only and tamper-evident, strictly outside the EHR content), served back via the ITI-81 search below. Every record is written here before its access is reported. |
| `[audit.syslog]` | off | The classic ATNA feed: DICOM PS3.15 XML as RFC 5424 syslog over RFC 5425 TLS to an external ARR, per IHE ITI-20. |
| `[audit.fhir_feed]` | off | The RESTful-ATNA feed (ITI-20 **ATX: FHIR Feed**): each FHIR `AuditEvent` is `POST`ed to an external FHIR ARR. Delivery runs from the store by a cursor: an ARR outage loses nothing, pending records ship on recovery. The feed is owed every record written from the moment it is configured; with the feed off, no record is owed. |

syslog delivers from the store the same way, by its own cursor, so both
feeds are projections of the stored records and neither holds a record in
memory.

### How a record is delivered

No loss and no skip: a sink delivers each log in id order from its cursor,
and the cursor moves only after the repository accepted the record, so
after a crash it repeats at most the record it had in flight (a FHIR
`AuditEvent` keeps its id; a syslog receiver may see one message twice).

- **The cursor.** Each feed keeps one cursor per log (the system records and
  the access log): every record at or below it was accepted by the
  repository or parked. The feed reads the next record past it, up to the
  frontier the chainer moves about once a second and below every record a
  write still holds, renders it from the store, sends it, and moves the
  cursor once the repository accepted it.
- **A new feed** starts at the last record written when the server that
  first runs with it starts, and is owed every record from then on.
- **A parked record.** A record the feed cannot render, or the repository
  refuses (a `4xx` other than `401`, `408` or `429`), is parked: counted in
  `atna_audit_parked_total`, logged at `ERROR` with its log and id, and the
  `audit_delivery` indicator reads `DEGRADED` while any record is parked.
  The cursor moves past it only because it is parked; it is never skipped.
  `GET {ext}/admin/audit-delivery` lists the parked records and every
  cursor; `POST {ext}/admin/audit-delivery/retry` offers a parked record
  again before the next one, and `POST {ext}/admin/audit-delivery/dismiss`
  removes it for good. Each act is recorded naming its operator.
- **Retention waits a day.** A record a configured feed has not delivered
  is removed, or replaced by its tombstone, only one day after its end; it
  is then parked for that feed (`reaped`, `tombstoned`), so an outage of a
  working day loses nothing and a longer one is still shown.
- **A feed taken out of the configuration** keeps its cursors and parked
  records: the server names it at `WARN` at start with how many records it
  has not received, the view shows it as not configured, and only
  `POST {ext}/admin/audit-delivery/forget` removes it.

### How a record is written

Every record of a request, the operation's own, one per EHR or party it
served, a refusal's and a login's, is written into the local store in one
transaction **before the response leaves**. A request whose records cannot be
written is answered `503 Service Unavailable` with `Retry-After: 1`, so no
response reports an access the log does not hold; under load that is
back-pressure: requests slow down, and the excess is refused rather than
recorded late or not at all. The same holds for the records the service
writes itself: subject resolutions, EHR-Extract transfers, the access-log
erasure and the access-policy write. There is no queue in memory and no fail
mode (`fail_mode` and `queue_capacity` are unknown keys).

Each access-log entry is written once, into `audit.access_log_entry`, and
each other record once, into `audit.audit_event` with an intake row naming
it; no request waits on a chain. A request whose records are all access-log
entries hands them to a small set of writers. Each writer writes everything
waiting in one statement that commits on its own, with the entries' search
tokens, and answers each request once that commit is durable, so under load
one commit serves many requests. An entry is in every read of the log from
that commit on. The commit writes the entry's row and its key only: about
once a second the audit task copies the columns the searches filter and
order on into `audit.access_log_index` in one batch, and every read takes
the entries the index does not cover yet from their rows by key, so a
search finds an entry from its commit on as before.

A system record is written the same way, once, into `audit.audit_event`,
by one statement per batch, and is in every read from its commit on.

A round on every replica's audit runtime, the **chainer**, about once a
second moves the frontier of both logs, owes the records below it to the
configured feeds, and about once a minute seals them in windows
([Seals, checkpoints and the transfer of a log](audit-integrity.md#seals-checkpoints-and-the-transfer-of-a-log));
a graceful stop seals what both logs hold. The `audit_sender` readiness
indicator reads `DEGRADED` when the oldest record of either log has waited
longer than three minutes for its seal. A record is durable from its
commit: a restart, a crash or a lost replica leaves nothing behind.

A write's records live in the audit cluster, which production keeps apart
from the clinical and party ones, so they cannot join the write's own
transaction. They are written first, **held** under a reservation id (an
`audit.access_log_hold` row per entry, an `audit.audit_event_hold` row per
system record), which every read, the feeds and the sealer skip; the id goes into the
write's own domain in the write's transaction, and the write commits. Once
the response is built, the reservation goes to the chainer, whose next round
releases the held records, so a write costs two commits. A replica that
stops before that round leaves the records held: two minutes later the
reconciler of any running replica releases those records, exactly as they
were written and once, and discards the held records of a write that never
committed. The reservation holds the id alone, no identity of the caller.
While a reservation waits past those two minutes the `audit_sender`
indicator reads `DEGRADED` with their count and age, and the metrics
`audit_reservations_held`, `audit_reservation_age_seconds` and
`audit_reservations_reconciled_total` show them.

### While a composition is held from the person

With [the access-delay register](access-control.md#delayed-access-by-the-person-ehds-art-33)
on, a new composition can be held from the person under
[EHDS](https://eur-lex.europa.eu/eli/reg/2025/327/oj) Art. 3(3) until a
professional releases it or its period passes. Only one route honours the
hold: the subject-scoped [ITI-81 read](audit-integrity.md#retrieving-audit-records-iti-81), the
one an app serves the person's own trail from, leaves out every
record that names a held composition. Every other sink and stream is a feed to
the controller's own systems and carries those records and events while the
hold is in force:

| Sink or stream | What it carries during a hold | Why it is not filtered |
|---|---|---|
| `[audit.store]`, read without the `patient` scope | every record, the hold's own included | The unscoped ITI-81 read is the controller's, under the admin role |
| `[audit.syslog]` | every record, sent as it is written: the commit, each read and the hold's own record name the composition, the last as `access-delay:<vo>` beside the patient | An ITI-20 feed goes to an Audit Record Repository, which "receives event audit reports and stores them" ([IHE ITI TF-1 §9.1.1.3](https://profiles.ihe.net/ITI/TF/Volume1/ch-9.html)) |
| `[audit.fhir_feed]` | the same records as FHIR `AuditEvent` resources, `POST`ed as they are written, the hold's own record naming the same `access-delay:<vo>` | The same ITI-20 repository, over ATX: FHIR Feed; the reader of a BALP `AuditEvent` is an Audit Consumer ([BALP §1:52.1](https://profiles.ihe.net/ITI/BALP/volume-1.html)) |
| [Change events](beyond-core/outbox-feed.md) (`[record.change_feed]`) | the commit of the held composition: its `vo_id`, kind, change type and template | The feed serves the deployment's own readers; it withholds what binds them, a restriction or objection (GDPR Art. 18 and 21, EHDS Art. 8), and the Art. 3(3) hold binds only the person's access |

EHDS Art. 9(2) gives the person the access information "through electronic
health data access services", which are yours to run. A repository, a feed
reader or anything built on them never reaches the person from FerroEHR, so none of
them filters the hold: a filter there would also hide the record from the
controller who answers for it. What you must not do, while a hold is in force:

- Serve the person's trail from the external Audit Record Repository the
  syslog or FHIR feed writes to, for example through that repository's own
  ITI-81 query. Serve it from FerroEHR's subject-scoped read instead.
- Send the person a notification, under Art. 9(1) or otherwise, from a record
  on either feed or from a change event that names a held composition. Ask
  the hold first: a request in the person's context reads the composition as
  absent (**404**) until the hold ends.

The [shared-responsibility page](compliance/shared-responsibility.md#the-delay-of-the-persons-access-ehds-art-33)
puts the same duty on the deployment's side.

## Configuration summary

Auditing defaults to on with the local store only; see the
[configuration reference](installation/config-audit.md#audit) for every
`[audit]` key and its `FERROEHR__AUDIT__*` environment form. The syslog sink's
own keys are `host` and `port` under `[audit.syslog]`, with its TLS material
under `[audit.syslog.tls]`; the
switches shared by every sink stay directly under `[audit]`.

> [!NOTE]
> There are no alternative spellings for any of these: an unrecognized
> `FERROEHR_*` variable or an unknown TOML key is a **boot error** naming the
> spelling it should have had, never a silently ignored setting. Validate a
> deployment's configuration up front with `ferroehr config check`.
