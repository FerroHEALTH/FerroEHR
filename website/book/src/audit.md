# Audit trail (IHE ATNA)

FerroEHR keeps a full security audit trail of API access (_who_ did _what_
to _which_ resource, with _what outcome_, from _where_, and _when_) following
the IHE **ATNA** (Audit Trail and Node Authentication) profile, the standard
openEHR itself points at (the platform Service Model names the System Log
component "IHE ATNA-compliant system log"). It is **on by default**: every
deployment records a queryable audit trail with zero external dependencies.

> [!WARNING]
> Two postures are legitimate to run and wrong to run silently, so the server
> says them at `warn` on boot (structured field `posture = "audit"`), in the
> `audit_sender` readiness indicator and on `/management/info`: **auditing
> off** (`[audit] enabled = false`) writes no access log and leaves the EHDS
> logging component off, and the shipped default **`fail_mode = "open"`**
> drops a record the audit queue cannot take while the request succeeds, so
> an access under load can go unrecorded. A deployment that must never serve
> an unrecorded access sets `fail_mode = "closed"`, which answers `503` and
> `Retry-After` instead. A deployment declaring
> [`deployment_profile = "production"`](installation/configuration.md#deployment_profile)
> refuses to start with auditing off.

<!-- toc -->

The trail is orthogonal to openEHR's own `CONTRIBUTION`/`AUDIT_DETAILS`
change-control audit (which the server always writes in the same transaction
as every version change): openEHR audit records what a version says about its
own authorship; ATNA records security surveillance of *access*, including
reads and rejected attempts.

## The record, in both official formats

Every audited operation produces one record, rendered in the two formats the
IHE standards define:

- **FHIR R4 `AuditEvent`** following the IHE **BALP** (Basic Audit Log
  Patterns) content profiles: the modern RESTful-ATNA form and the canonical
  stored form. Patient-centric operations carry the resolved EHR subject as
  the patient entity (`PatientRead`/`PatientCreate`/… profile claims); query
  executions carry the search expression; Bearer-authenticated requests
  record the token's `jti` (and never the token itself) per
  `OAUTHaccessTokenUse.Minimal`.
- **DICOM Audit Message** (DICOM PS3.15 §A.5 XML): the classic ATNA form,
  shipped over syslog per IHE ITI-20 when the syslog sink is enabled.
  Dedicated DICOM event ids are used throughout: Patient Record (110110),
  Query (110112), Export/Import (110106/110107, the EHR-Extract directions),
  User Authentication (110114, with the Login `EventTypeCode` 110122).

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
| `domain` | The pseudonymisation domain read: `ehr`, `demographic`, `linkage`, or `system` for an operation that touches no domain. |
| `organisation` | The organisation the caller acted for, read from the access token's `[authz.abac] organization_claim` — the same claim the ABAC layer decides on, so no second setting can name a different one. The recording does not depend on that layer being switched on. A Basic-authenticated caller and an unconfigured claim both leave it empty. |
| `purpose` | The purpose of use the caller declared in the `x-purpose-of-use` header, when the deployment accepts it. |
| `emergency_access` | `true` when the declared purpose is one of `[audit] emergency_purpose_codes`: an [emergency access](#emergency-access-ehds-art-115) (EHDS Art. 11(5)). |
| `legal_basis` | The basis the deployment processes under, from `[audit] legal_basis`. |
| `result_count` | How many records the operation served. |
| `request_id` | The `x-request-id` correlation id the response carried. |

A query is the case a single record cannot describe. One AQL statement is one
operation but potentially many disclosures, so it produces its own execute
record **plus one access record per EHR it served**, each carrying that EHR's
served-row count. The served set is read off the rows the caller actually
received rather than the rows the statement matched: a legal record of access
must not name an EHR whose content was never disclosed. Two query shapes carry
no per-EHR breakdown, because the shape makes it underivable —
`SELECT DISTINCT`, where an extra column changes which rows are distinct, and
an aggregate projection, where a per-row EHR is not valid SQL. Those record the
statement and its row count.

### NEN 7513 content, field by field

NEN 7513 §5 lists what a logged event has to contain. This is where each item
lives in the record:

| NEN 7513 asks for | FerroEHR records it as |
|---|---|
| The identity of the person who accessed the record | `principal` — the authenticated Basic username or OAuth `sub`; an unattributable denial is recorded as unattributed, never under a fabricated identity. With `[auth.oidc.professional]` configured, `professional_id` names the natural person when the token names a client application |
| The identity of the patient whose record was accessed | `patient_id` — the resolved EHR subject, which under the pseudonymisation boundary is an opaque pseudonym |
| Which record, or which part of it | `resource_class` + `resource_id`: the version uid, party uid or EHR id the operation touched |
| The date and time | `recorded_at`, the event time; `stored_at` is when the row was persisted |
| The kind of action | `action` (DICOM `C`/`R`/`U`/`D`/`E`) plus `operation`, the ITS-REST operation id |
| Whether it succeeded | `outcome`, the DICOM outcome indicator (0 success, 4 minor, 8 serious, 12 major) |
| Under which role or authority | `roles` — the roles the caller held at access time (the RFC 9068 §2.2.3.1 claim carriers for a bearer, the user definition for Basic), rendered as FHIR `agent.role` on the requestor and as a `RoleIDCode` per role on the DICOM source participant |
| On whose authority, and why | `purpose` and `legal_basis` |
| Which system, and from where | `AuditSourceID` and the source participant's network address (`client_ip`) |

Two of these are the deployment's to supply. `purpose` is only as meaningful as
the vocabulary you agree with your callers: set `[audit] purpose_codes` to that
vocabulary and a code outside it is recorded as absent rather than as free text
that reads at review time like an established purpose. `legal_basis` is a
deployment-level fact and is recorded on every access once you set it.

Mapping the recorded fields onto your own retention schedule and review process
stays yours: the software records the events and serves them back, and no
supplier document discharges the obligation to review them.

## The EHDS logging elements, mapped

Annex II 3.2 of [Regulation (EU) 2025/327](https://eur-lex.europa.eu/eli/reg/2025/327/oj)
lists five things the European logging software component must record "on every
access event or group of events". This is where each one lands, or does not.
The elements were read from the published text on 2026-09-10; the regulation is
the authority and the wording below is a paraphrase.

| Annex II 3.2 | Access-event field | DICOM PS3.15 | FHIR `AuditEvent` | State |
|---|---|---|---|---|
| (a) the healthcare provider or other individuals having accessed | `organisation` | — (no such attribute) | a second `agent` with `who` referencing `Organization/…` | Recorded |
| (b) the specific natural person or persons having accessed | `principal` (+ `token_id`, `client_ip`), and with `[auth.oidc.professional]` configured `professional_id` + `acting_mode` | `ActiveParticipant/@UserID` (the professional; a client application acting for one is a second, non-requesting participant; the token subject is `@AlternativeUserID` when it differs) | `agent.who` (the same split: the professional as requestor, the client application as a second agent; the token subject as `agent.altId` when it differs) | Recorded |
| (c) the categories of data accessed | `categories` + `category_basis` + `category_evidence` + `category_map_digest`, classified through the `[audit.categories]` map (beside `resource_class`, `domain` and `result_count`) | none (no such element) | an `entity` named "categories of the data accessed", with one `detail` per category, the basis, each piece of evidence and the map digest | Recorded |
| (d) the time and date of access | `recorded_at` | `EventIdentification/@EventDateTime` | `recorded` | Recorded |
| (e) the origin or origins of data | `origins` + `origin_count` (the `FEEDER_AUDIT` originating systems of the served version bodies, or the server that created them, derived at commit onto `version.origins`) | — (no such element) | one `entity` per origin, named "origin of the served data" | Recorded |

Read (a) beside (b): they are both about the accessing side, and the pair
distinguishes the **organisation** on whose behalf the access happened from the
**natural person** who made it. The principal answers (b); the `organisation`
column answers (a), resolved from the same access-token claim the ABAC layer
reads (`[authz.abac] organization_claim`) rather than from a second setting
that could name a different claim. Sharing the setting is not sharing the
switch: the organisation is recorded whether or not the ABAC gate is enabled,
because the trail describes a caller rather than deciding anything about them.
The FHIR rendering carries it as a second `agent` referencing an
`Organization`, with no participation code, because `AuditEvent.agent.type` is
optional and inventing a code would be worse than omitting one. The DICOM
rendering carries nothing: PS3.15 §A.5 defines no organisation attribute on
`ActiveParticipant`, and `AuditEnterpriseSiteID` is the reporting node's own
site rather than the caller's organisation. Where no claim is configured or the
caller authenticated with Basic, the field stays empty rather than being
guessed.

Element (b) asks for a **natural person**, and a bearer token can name a client
application instead. With [`[auth.oidc.professional]`](security.md#assurance-level-and-the-natural-person-on-patient-data)
configured, every record says which it was and who stood behind it:

| Field | Meaning | DICOM PS3.15 | FHIR `AuditEvent` |
|---|---|---|---|
| `acting_mode` | `person` when the token names a natural person, `client` when it names a client application | the participant split below | the agent split below |
| `professional_id` | the natural person: the person's own claim, or the professional a client token acts for | `ActiveParticipant/@UserID` of the requesting participant | `agent.who` of the requesting agent |
| `principal` | the token subject, as before: the client application in `client` mode | a second `ActiveParticipant` with `UserIsRequestor="false"` and the network address; `@AlternativeUserID` in `person` mode when it differs from the professional | a second `agent` with `requestor: false` and the network address; `agent.altId` in `person` mode when it differs |
| `assurance_level` | the eIDAS level (`low`, `substantial`, `high`) the token's assurance claim maps to under `[auth.oidc.assurance]` | none (no such attribute) | none (no such element) |
| `assurance_value` | the raw claim value the level was read from, usually `acr` | none | none |

The assurance level is the one value neither export carries.
[PS3.15 §A.5](https://dicom.nema.org/medical/dicom/current/output/chtml/part15/sect_A.5.html)
gives `ActiveParticipant` no assurance attribute. FHIR R4 has no element for it
either: `agent.policy` holds the
["policy or plan that authorized the activity"](https://hl7.org/fhir/R4/auditevent-definitions.html#AuditEvent.agent.policy),
which a level of assurance is not, and BALP already uses it for the token
`jti`. The level and its raw value therefore live in the local store's columns only;
ITI-81 serves the FHIR document, so it does not return them either. The syslog feed carries the DICOM message, so it has the same
gap. Unlike the FHIR document, these columns are outside the tamper-evidence
digest; the append-only trigger still refuses any change to them.

Without `[auth.oidc.professional]` the mode and the professional stay empty and
(b) is answered by `principal`, which names a client application when the token
names no person. Without `[auth.oidc.assurance] levels` the level stays empty;
the raw value is still recorded when the token carries the claim. A Basic
credential records none of the four.

Element (c) asks for the **priority category** of the data, in the sense of
Art. 14(1): patient summaries, electronic prescriptions, electronic
dispensations, medical imaging, medical test results and discharge reports,
plus any category a Member State adds in national law. AQL selects archetypes
and templates, never categories, and no openEHR specification maps the one
onto the other, so the deployment declares the mapping in
[`[audit.categories]`](installation/config-audit.md#auditcategories): a
template id, or failing that the root archetype id, maps to a set of categories
(`patient-summary`, `eprescription`, `edispensation`, `imaging`,
`test-results`, `discharge-report`, `national:<code>`, or `none` for content
that holds no priority-category data). FerroEHR ships no map.

Each record is classified when the access happens, from what it served or
wrote, and never at commit, because the map is your reading of your templates
and can change:

- a composition read or write is classified by the version's template id, then
  by its root archetype id;
- an AQL execution is classified by the template and root archetype ids of the
  versions its result page came from; where the answer carries no version (a
  `DISTINCT` or aggregate projection, or zero rows) by the query's positive
  archetype and template constraints, so an operand under `NOT CONTAINS` or a
  predicate under `NOT` never counts. Each per-EHR record of a query carries
  the categories that EHR disclosed;
- a logical delete writes no content, so it is classified by the version it
  deleted; a `VERSIONED_COMPOSITION` read by the composition it holds;
- a `CONTRIBUTION` read, with or without `Prefer: resolve_refs`, is classified
  by the versions the contribution committed, a deleted version again by the
  version it deleted;
- an EHR Extract export is classified by every version it carried, and an
  Extract import by the newest content of every versioned object it landed;
- `EHR`, `EHR_STATUS`, `EHR_ACCESS`, the directory, item tags and revision
  histories are recorded as `none`, by resource kind.

An access the map cannot classify is recorded `unclassified`, with the
template and archetype ids as evidence, and is never refused.
`category_basis` says what the classification rests on (`template`,
`archetype`, `query` or `resource-kind`), and `category_map_digest` names the
map that produced it, so a later map never rereads an old record. The map's
digest is also on the boot line and on `GET /management/info` under
`audit.category_map`. The classification stays in the access log: it is never
written to the request log, a span or a metric label. The DICOM rendering has
no element for it (PS3.15 §A.5), so it travels in the local store and the FHIR
export only.

Records in the demographic domain carry no classification, because no
priority category lives there.

Element (e) asks where the DATA came from, which is not where the request came
from. openEHR models that provenance as `FEEDER_AUDIT` on the content itself,
so every committed version records the distinct set of
`FEEDER_AUDIT.originating_system_audit.system_id` values found in its body, or
this server's own system id when it carries none. A composition, status or
folder read then records the origins of the version it served, and an AQL
execution the union over the version rows its result page came from. The record
carries the true distinct count beside a set capped at 32, so a page drawing on
more origins than that says how many rather than looking complete. A version
committed before the column existed is assessed when it is read.

One further element, adjacent to the five: **the declared purpose of use
reaches one export of the two**, and only one of them could carry it. It is
stored on the record, served by ITI-81, and rendered as
[`agent.purposeOfUse`](https://hl7.org/fhir/R4/auditevent.html) on the
requesting person's agent in the FHIR export. The coding carries the code
alone, with no system: the vocabulary is the one you agree with your callers
in `[audit] purpose_codes`, which has no published code system URI, and
attributing the code to a system it does not come from would misdescribe it.
The
[DICOM Audit Message schema](https://dicom.nema.org/medical/dicom/current/output/chtml/part15/sect_A.5.html)
of PS3.15 §A.5 defines no purpose element at all — `EventIdentification`,
`ActiveParticipant`, `AuditSourceIdentification` and
`ParticipantObjectIdentification`, and none of them carries one — so on that
side it is a limit of the format, not something this code withholds.

> [!NOTE]
> Every row of this table is asserted by a test
> (`app/ferroehr/tests/it/audit_ehds_mapping.rs`), **including the gaps**: the
> tests that record an absence fail the day the field arrives, so the table and
> the code cannot drift apart in either direction.

The common specifications the regulation's Article 36 provides for have not
been adopted. When they are, this mapping is re-verified against them rather
than assumed to still hold; see the
[EHDS readiness page](compliance/ehds-readiness.md).

### Emergency access (EHDS Art. 11(5))

A natural person may restrict health professionals' access to parts of their
data (EHDS Art. 8). Art. 11(5) lets a professional reach restricted data where
the person's vital interests require it, and requires such cases to be "logged
in a clear and understandable format" and "easily accessible for the data
subject". Art. 9(1) counts these accesses among the ones the person is told
about.

You name the purpose codes your callers declare for an emergency in
`[audit] emergency_purpose_codes`. Each one must also be on
`[audit] purpose_codes`; the server refuses to start otherwise. An access
whose declared purpose is one of them carries the emergency mark:

- **The stored record:** the `emergency_access` column is `true`. It is
  `false` for every other access and for records written before the column
  existed.
- **The FHIR `AuditEvent`:** a
  [`purposeOfEvent`](https://hl7.org/fhir/R4/auditevent-definitions.html#AuditEvent.purposeOfEvent)
  with the HL7 v3 `ActReason` code `ETREAT` ("Emergency Treatment", system
  `http://terminology.hl7.org/CodeSystem/v3-ActReason`) and the mark in plain
  words as its text: "Emergency access: declared as necessary to protect the
  vital interests of the patient (Regulation (EU) 2025/327 Art. 11(5))". The
  declared code stays on `agent.purposeOfUse`. A patient portal keys on the
  `ETREAT` coding and shows the text.
- **The DICOM syslog feed:** no mark. The PS3.15 schema has no purpose
  element.

The person reads their own log, the marked accesses included, through the
[subject-scoped ITI-81 retrieval](#retrieving-audit-records-iti-81).

```toml
[audit]
purpose_codes = ["TREAT", "ETREAT"]
emergency_purpose_codes = ["ETREAT"]
```

The mark records the declaration and lifts no restriction. A composition or
EHR restricted under GDPR Art. 18 stays refused with an emergency purpose,
because Art. 18(2) admits processing only with consent, for legal claims, for
the rights of another person or for an important public interest, and the
person's own vital interests are not on that list. The refused attempt is
still logged with the mark. FerroEHR does not hold an EHDS Art. 8 restriction
today, so it neither hides restricted data from a professional nor grants the
override: that is the Member State's access service or your own system, and
the restriction itself is planned in
[#3682](https://github.com/FerroHEALTH/FerroEHR/issues/3682). An emergency the
caller does not declare is not marked.

### Retention, and who chooses it

`[audit.store] retention_days` decides how long the repository keeps a record:
`0`, the default, keeps them forever, and any other value reaps hourly
everything older. **The default is deliberate.** An access log is the evidence
a data subject's right of access is served from, and a repository that silently
forgets is worse than one that grows — so the software keeps everything until
an operator says otherwise, rather than choosing a horizon on their behalf.

Choosing that horizon is the deployment's, and it is a legal question:
EHDS Art. 9(2) makes the access information "available for at least three
years from each date of access to the data", and national law can ask for
longer. NEN 7513 is the Dutch reference for this log and the Wabvpz for how
long it must survive. The setting is per node, and one node serves one
organisation, so two organisations needing different horizons run two
instances.

**A floor is enforced where a jurisdiction sets one.** The jurisdictions a
deployment answers to are the ones its `[privacy.identifier_scan]` rules name,
and where one publishes a minimum retention for the access log the server
refuses to boot with a shorter horizon, naming the configured value and the
floor. `0` always passes. The floors are written in calendar years, as the law
writes them: a horizon set as `[audit.store] retention_years` is compared
against the years exactly, and one set as `retention_days` against the most
days those years can span (the day count in brackets below). The floors
registered:

| Jurisdiction | Floor | Source |
|---|---|---|
| Every EU Member State (AT, BE, BG, CY, CZ, DE, DK, EE, ES, FI, FR, GR, HR, HU, IE, IT, LT, LU, LV, MT, NL, PL, PT, RO, SE, SI, SK) | three years from each date of access (`1096` days, the most three calendar years can span) | [Regulation (EU) 2025/327](https://eur-lex.europa.eu/eli/reg/2025/327/oj) (EHDS) Art. 9(2); a longer national floor wins. Iceland, Liechtenstein and Norway are not listed: EHDS reaches the EEA only through an EEA Joint Committee decision |
| NL | five years from the moment the entry is written (`1830` days, the most five calendar years can span) | [Besluit vaststelling bewaartermijn logging](https://wetten.overheid.nl/BWBR0042391) (Stcrt. 2019, 38007), under Art. 5 of the [Besluit elektronische gegevensverwerking door zorgaanbieders](https://wetten.overheid.nl/BWBR0040238), which binds the period to NEN 7513 |
| CH | one year, and kept apart from the processing system (`366` days, the most one calendar year can span) | [Datenschutzverordnung (DSV) Art. 4 Abs. 5](https://www.fedlex.admin.ch/eli/cc/2022/568/de), as amended on 1 December 2025, for the logs of a large-scale automated processing of sensitive personal data; the separate storage is the deployment's, through the [forwarding sinks](#getting-the-log-out) |

For the Netherlands the five national years win over the three EHDS years;
Switzerland is outside the Union and keeps its one year. Of the other shipped
jurisdictions, DE, FI and SE carry the EHDS floor, while GB and NO have no
floor registered: an unknown requirement is never guessed at, and a deployment
there sets its own. Art. 9 applies from 26 March 2029 for patient summaries,
prescriptions and dispensations and from 26 March 2031 for the other priority
categories (Art. 105). The floor is enforced from this release on, so a log
kept today already meets it when Art. 9 applies.
The effective retention is on the boot line beside the audit-enabled facts and
on `GET /management/info` under `audit`.

### Getting the log out

Two routes, and they answer different questions.

- **ITI-81 retrieval** — the [audit search](#retrieving-audit-records-iti-81)
  serves the records back as FHIR `AuditEvent` resources, filtered by patient,
  agent, date and outcome. This is the operator's route and the one a patient
  portal would build on: everything Annex II 3.2 records is reachable through
  it, including the purpose and the organisation, which the syslog feed's
  DICOM form cannot carry.
- **The BALP feed** — records fan out to syslog and to a FHIR feed as they are
  written, for a SIEM or a central Audit Record Repository.

There is no patient-facing route in the product: a portal that shows a person
who accessed their record builds on ITI-81 and authenticates the person itself.
What the product gives that portal is a grant of its own size:
`authz.rbac.subject_audit_role` reads the log for one subject at a time, with
the `patient` parameter required, so the portal never holds an admin credential
over every patient's log ([GDPR](https://eur-lex.europa.eu/eli/reg/2016/679/oj)
Art. 15 with Recital 63, under which the controller "should be able to provide
remote access to a secure system" where possible; EHDS Art. 9; for Dutch
deployments Wabvpz Art. 15e). The portal's build is the deployment's, and the
[shared-responsibility page](compliance/shared-responsibility.md) says so.

## Sinks

Records fan out to independently configured sinks (`[audit]` in
`ferroehr.toml`; see the
[configuration reference](installation/config-audit.md#audit)):

| Sink | Default | What it does |
|---|---|---|
| `[audit.store]` | **on** | The local **Audit Record Repository**: records persist in the dedicated `audit` PostgreSQL schema (append-only and tamper-evident, strictly outside the EHR content), served back via the ITI-81 search below. `retention_days` prunes old records hourly (`0` = keep forever). |
| `[audit.syslog]` | off | The classic ATNA feed: DICOM PS3.15 XML over syslog (RFC 5424; UDP or TLS transport) to an external ARR, per IHE ITI-20. |
| `[audit.fhir_feed]` | off | The RESTful-ATNA feed (ITI-20 **ATX: FHIR Feed**): each FHIR `AuditEvent` is `POST`ed to an external FHIR ARR. With the local store on, delivery is **outbox-driven**: an ARR outage loses nothing, pending records ship on recovery. |

Emission never blocks the request path: a record is handed to a bounded queue
(`queue_capacity`) and written by a background drain.

The local store is the durability anchor. Under `fail_mode = "closed"`, an
operation whose audit record cannot be recorded answers
`503 Service Unavailable`: the deployment demanded an audit trail it cannot
currently deliver, so no un-audited PHI access happens, and the same holds for
the domain-level records the linkage, subject and identifier resolutions and
the EHR-Extract transfers write. `open`, the default, drops-and-meters instead,
and every loss path is metered (the `atna_audit_*` counters in
[Operations](operations.md)); the server announces that default at boot and on
the readiness indicator as a stated caution, because a metered loss is still a
loss. Login and rejection records never gate a response in either mode.

## Tamper evidence

An audit trail that anything with the application's database password can
quietly rewrite is a log, not an accountability record. The local store is
therefore tamper-**evident**: every record is linked into a SHA-256 hash chain
maintained inside PostgreSQL, so each record commits to its predecessor and to
its own content. The chain is built by the database itself, not by the server,
which means it covers every writer: the per-event insert, the batched drain,
and any statement typed by hand.

Four controls sit on top of it, and they are separate on purpose:

- **The table refuses the ordinary rewrite paths outright.** The only permitted
  change to a stored record is the per-sink forwarding stamp; an `UPDATE` of any
  other column, a `DELETE`, and a `TRUNCATE` are refused by the database.
  Retention pruning goes through the one sanctioned deletion path, which records
  *which* records it removed and what the surviving chain must link back to, so
  reaping does not look like tampering, and tampering does not look like
  reaping.
- **The privileges are narrow.** The runtime role may record an event, stamp it
  forwarded, and read the trail back. It holds nothing that can rewrite a
  record, remove one, or alter the chain's own bookkeeping. That posture is only
  fully in force with `db.migrate = "verify"`; see
  [Operations](operations.md#database-roles-and-least-privilege), because a
  self-migrating server owns the schema and can therefore turn the enforcement
  off.
- **Verification is a query you can run yourself.** It recomputes every digest,
  re-walks every link, and checks both ends of the chain:

  ```sql
  SELECT * FROM audit.verify_audit_chain();
  ```

  An empty result means the trail is intact. Any row names one record (its
  chain position, its id, when it was recorded) and what is wrong with it:
  content modified after it was written, records deleted with no retention
  record for the removal, or records removed from the end of the chain where no
  successor would have noticed.
- **The server runs that query itself, on a schedule.** With the local store on,
  the check runs one minute after boot and then every
  `[audit.store] verify_interval_seconds` (default `86400`, one day; `0` turns
  it off). A run that finds damage logs at `ERROR`, one summary line with the
  finding count and one line per finding (the first 20), and adds the count to
  the `atna_audit_chain_findings` counter with `kind = "damage"`. A run that
  cannot verify at all counts as `kind = "unverifiable"`, because a trail that
  cannot be checked is never a pass. The latest result is the `audit_chain`
  indicator on `GET /health/readiness`: `UP` while intact, `DOWN` with
  the first finding when damaged, `DEGRADED` when unverifiable. It never flips
  readiness, because the trail keeps recording while the damage is
  investigated. Alert on the counter or on that component, and use the query
  above to see every finding. Each replica runs its own check.

> [!IMPORTANT]
> This is **detection, not prevention**, and the boundary is worth stating
> plainly. The chain is unkeyed, so a party with unrestricted write access to
> the `audit` schema (the database owner, or a superuser) can delete a record
> and recompute every hash after it. What closes that case is keeping the trail
> somewhere that party does not control. Enable
> `[audit.syslog]` or `[audit.fhir_feed]` so records leave the box as they are
> written, and give the server an app-role-only DSN so it is not that party.

## Retrieving audit records (ITI-81)

The RESTful-ATNA **ITI-81 Retrieve ATNA Audit Event** transaction is served
at the FHIR façade:

```text
GET /ferroehr/rest/openehr/v1/fhir/r4/AuditEvent
```

It returns a FHIR `searchset` Bundle of the stored `AuditEvent` documents,
newest first, with the full match `total`. Supported search parameters:

| Parameter | Meaning |
|---|---|
| `date` | event-time bound, `ge`/`le`-prefixed RFC 3339 instants, repeatable |
| `patient` | the recorded patient (EHR subject) id |
| `agent` | the authenticated principal |
| `entity` | the touched resource id |
| `outcome` | the outcome indicator: `0`, `4`, `8` or `12` |
| `action` | the action code: `C`, `R`, `U`, `D` or `E` |
| `_count` / `_offset` | paging; page size defaults to 50, capped at 1000 |

Other FHIR search parameters are ignored (lenient search); a malformed value on
a supported one is a `400` carrying a FHIR `OperationOutcome`. Under RBAC the
unscoped retrieval is **admin-only**; a caller holding the configured
`authz.rbac.subject_audit_role` reads it **scoped to one subject**, `patient`
required, and is refused with a `403` that names the parameter when it omits
it. Reading a subject's log is itself recorded as an access naming that
subject and the record count served. An
[emergency access](#emergency-access-ehds-art-115) in the subject's log
carries its mark as `purposeOfEvent`. The surface answers `404` when the local
store is disabled.

```bash
# Who accessed patient-42's data this month?
curl -u admin:pw \
  "https://cdr.example.org/ferroehr/rest/openehr/v1/fhir/r4/AuditEvent?patient=patient-42&date=ge2026-07-01T00:00:00Z"
```

This is also the openEHR "record demerging" instrument: when data lands in
the wrong EHR, the patient-filtered audit search shows exactly who read it.

> [!NOTE]
> Audit records are themselves sensitive (they name patients, subjects and
> actions) so this endpoint is a PHI-disclosure surface and is authorized like
> any other.

## Node authentication (ITI-19, mutual TLS)

ATNA's second half is node authentication. `[server.tls]` terminates TLS
natively and can demand a verified client certificate:

```toml
[server.tls]
enabled = true
cert_file = "/etc/ferroehr/server.pem"
key_file = "/etc/ferroehr/server.key"
min_version = "1.3"               # 1.3 (default) | 1.2
client_auth = "required"          # off | optional | required
client_ca_file = "/etc/ferroehr/client-ca.pem"
```

The protocol floor is **TLS 1.3 only** by default, following the OWASP
Transport Layer Security cheat sheet. Setting `min_version = "1.2"` enables
1.2 *alongside* 1.3, never instead of it, for a client that genuinely cannot
do 1.3, such as an older integration engine or a pinned Java runtime. TLS 1.1
and 1.0 are not selectable at all.

With `client_auth = "required"`, only clients presenting a certificate
chaining to your explicit trust anchor complete the handshake: the IHE
mutually-authenticated-node posture. Deployments terminating TLS at an
ingress keep `[server.tls]` off and enforce mTLS there instead.

> [!WARNING]
> The separate-port management listener (`management.port`) stays **plain HTTP**
> even with `[server.tls]` enabled, and it binds all interfaces. Treat it as an
> internal surface and keep it off any publicly routed port. Under
> `deployment_profile = "production"` setting it opens the `plaintext_listener`
> gap, so the server refuses to start unless the gap is accepted by name.

Complete the posture with time synchronisation (IHE Consistent Time): run
NTP/chrony on every node so audit timestamps align across systems.

## Configuration summary

Auditing defaults to on with the local store only; see the
[configuration reference](installation/config-audit.md#audit) for every
`[audit]` key and its `FERROEHR__AUDIT__*` environment form. The syslog sink's
own keys are `host` / `port` / `transport` / `tls_ca_file` /
`tls_identity_cert_file` / `tls_identity_key_file` under `[audit.syslog]`; the
switches shared by every sink stay directly under `[audit]`.

> [!NOTE]
> There are no alternative spellings for any of these: an unrecognized
> `FERROEHR_*` variable or an unknown TOML key is a **boot error** naming the
> spelling it should have had, never a silently ignored setting. Validate a
> deployment's configuration up front with `ferroehr config check`.
