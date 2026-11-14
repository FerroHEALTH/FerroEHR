# Audit trail (IHE ATNA)

FerroEHR keeps a full security audit trail of API access (_who_ did _what_
to _which_ resource, with _what outcome_, from _where_, and _when_) following
the IHE **ATNA** (Audit Trail and Node Authentication) profile, the standard
the openEHR reference text points at (its platform service description names a
system log "IHE ATNA-compliant"). It is **on by default**: every
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
> [`deployment.profile = "production"`](installation/configuration.md#deploymentprofile)
> refuses to start with auditing off.

<!-- toc -->

The trail is orthogonal to openEHR's own `CONTRIBUTION`/`AUDIT_DETAILS`
change-control audit (which the server always writes in the same transaction
as every version change): openEHR audit records what a version says about its
own authorship; ATNA records security surveillance of *access*, including
reads and rejected attempts.

## The record, in both official formats

Every audited operation produces one record, rendered in the two formats the
IHE standards define. Both are written by the shared audit crates of the
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

## The openEHR access-log entry

Beside each record, every access to EHR data also writes an openEHR
access-log entry: an `ACCESS_LOG_ENTRY` of the draft
`org.openehr.rm.access_log` 0.1.0 model, FerroEHR's proposal to openEHR,
which no openEHR release defines yet. Reads, writes, AQL queries, exports,
refusals and reads of the access log itself each write one. An access that
reaches several EHRs writes one entry per EHR, and the entries of one request
share its request id, so a population query writes one entry for each EHR
that contributed a row.

The entry names who accessed (the bound party, the groups, the provider, the
binding outcome and the credential by its issuer, subject, client and token
id), what (the EHR, the version or object, its categories and origins), why
(the declared purposes), under which authorisation (the agreement, the
representation, a data permit, the emergency mark), the outcome (`granted`,
`refused` or `failed`) and the request. It carries no data value of the EHR:
a query is named by its stored name or the SHA-256 of its text, never the
text, and a read of the access log through ITI-81 is named by the digest of
its search.

The entry is written in the same database transaction as the record it
accompanies, so the store holds both or neither, and an access whose record
the store cannot take is refused under `fail_mode = "closed"` exactly as
before. Entries are kept in `audit.access_log_record`, chained with the
records under one hash chain (see [Tamper evidence](#tamper-evidence)), and
each states the date it is kept until and why. The store requires
`[audit.store]`; with the store off no entry is written.

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
| `organisation` | The organisation the caller acted for, read from the access token's `[authz.abac] organization_claim` — the same claim the ABAC layer decides on, so no second setting can name a different one. The recording does not depend on that layer being switched on. A Basic-authenticated caller and an unconfigured claim both leave it empty. |
| `purpose` | The purpose of use the caller declared in the `x-purpose-of-use` header, when the deployment accepts it. |
| `token_attributes` | The purposes of use and the professional roles the access token states, each with its code system, for an issuer with an `[[auth.attributes]]` table ([where the purpose comes from](#where-the-purpose-of-use-comes-from)). |
| `emergency_access` | `true` when the declared purpose, or a purpose the token states, is one of `[audit] emergency_purpose_codes`: an [emergency access](#emergency-access-ehds-art-115) (EHDS Art. 11(5)). |
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
| The identity of the person who accessed the record | `principal` — the authenticated Basic username or OAuth `sub`; an unattributable denial is recorded as unattributed, never under a fabricated identity. With `[authz.professional]` configured, `professional_id` names the natural person when the token names a client application |
| The identity of the patient whose record was accessed | `patient_id` — the resolved EHR subject, which under the pseudonymisation boundary is an opaque pseudonym |
| Which record, or which part of it | `resource_class` + `resource_id`: the version uid, party uid or EHR id the operation touched |
| The date and time | `recorded_at`, the event time; `stored_at` is when the row was persisted |
| The kind of action | `action` (DICOM `C`/`R`/`U`/`D`/`E`) plus `operation`, the released ITS-REST operation id. It equals the served OpenAPI `operationId` except for the demographic CONTRIBUTION pair, served as `demographic_contribution_create` and `demographic_contribution_get` because the released `contribution_create` and `contribution_get` also name the EHR operations and one document needs unique ids |
| Whether it succeeded | `outcome`, the DICOM outcome indicator (0 success, 4 minor, 8 serious, 12 major) |
| Under which role or authority | `roles` — the roles the caller held at access time (the RFC 9068 §2.2.3.1 claim carriers for a bearer, the user definition for Basic), rendered as FHIR `agent.role` on the requestor and as a `RoleIDCode` per role on the DICOM requesting participant |
| On whose authority, and why | `purpose` and `legal_basis`, and `permit`, the identifier of the data permit a secondary-use caller declares it acts under ([Secondary use](compliance/secondary-use.md#the-access-record)) |
| Which system, and from where | `AuditSourceID` and the source participant's network address (`client_ip`): the connection's peer, or the client a peer in `[server] trusted_proxies` names in its `forwarded_header` ([Server](installation/config-server.md)) |
| Who the person is in the demographic store, and for whom they act | `binding` and the fields beside it ([the binding outcome](#the-binding-outcome)): `party` (the bound `PERSON`), `party_groups`, `provider`, `representation` for a representative, and `acting_mode` (`person`, `delegated`, `external` or `system`); for a verified external accessor `external_fields` and `agreement` stand in for the party, and an `organisation` record names the organisation. The FHIR `AuditEvent`, the DICOM message and the syslog feed carry each in the `ehds-accessor` and `ehds-relayed` entities ([the binding fields in the renderings](#the-binding-fields-in-the-renderings)) |

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
| (a) the healthcare provider or other individuals having accessed | `organisation` | — (no such attribute) | an `agent` whose `who` is a logical reference to an `Organization` | Recorded |
| (b) the specific natural person or persons having accessed | `principal` (+ `token_id`, `client_ip`), and `professional_id` + `acting_mode` for a caller its issuer classifies as a person or a delegate | `ActiveParticipant/@UserID` (the professional; a client application acting for one is a second, non-requesting participant; the token subject is `@AlternativeUserID` when it differs) | the `agent:user` slice, `who` the token's issuer and subject, the professional's identifier as its `ihe-otherId`, and the client application as an agent of its own (BALP 1.1.4 §3:5.7.5.4) | Recorded |
| (c) the categories of data accessed | `categories` + `category_basis` + `category_evidence` + `category_map_digest`, classified by `ehds-logging` through the audit category register (beside `resource_class`, `domain` and `result_count`) | none (no such element) | the `entity` named `ehds-categories`, with one `ehds-category` detail per category and its basis, each id the classification read and the map digest; each category is also a `meta.security` label | Recorded |
| (d) the time and date of access | `recorded_at` | `EventIdentification/@EventDateTime` | `recorded` | Recorded |
| (e) the origin or origins of data | `origins` + `origin_count` (the `FEEDER_AUDIT` originating systems of the served version bodies, or the server that created them, derived at commit onto `version.origins`) | — (no such element) | one `entity` named `origin` per origin, its `system-id` detail the originating system | Recorded |

Read (a) beside (b): they are both about the accessing side, and the pair
distinguishes the **organisation** on whose behalf the access happened from the
**natural person** who made it. The principal answers (b); the `organisation`
column answers (a), resolved from the same access-token claim the ABAC layer
reads (`[authz.abac] organization_claim`) rather than from a second setting
that could name a different claim. Sharing the setting is not sharing the
switch: the organisation is recorded whether or not the ABAC gate is enabled,
because the trail describes a caller rather than deciding anything about them.
The FHIR rendering carries it as an `agent` of its own, a logical reference to
an `Organization`, with no participation code, because `AuditEvent.agent.type`
is optional and inventing a code would be worse than omitting one. The DICOM
rendering carries nothing: PS3.15 §A.5 defines no organisation attribute on
`ActiveParticipant`, and `AuditEnterpriseSiteID` is the reporting node's own
site rather than the caller's organisation. Where no claim is configured or the
caller authenticated with Basic, the field stays empty rather than being
guessed.

Element (b) asks for a **natural person**, and a bearer token can name a client
application instead. Each issuer's [`kind` and profile](security.md#assurance-level-and-the-natural-person-on-patient-data)
classify the caller, so every record says which it was and who stood behind it:

| Field | Meaning | DICOM PS3.15 | FHIR `AuditEvent` |
|---|---|---|---|
| `acting_mode` | `person` when the token names a natural person, `client` when it names a client application | the participant split below | the agent split below |
| `professional_id` | the natural person: the person's own claim, or the professional a client token acts for | `ActiveParticipant/@UserID` of the requesting participant | the `ihe-otherId` extension of the `agent:user`, typed `NPI` |
| `principal` | the token subject, as before: the client application in `client` mode | a second `ActiveParticipant` with `UserIsRequestor="false"` and the network address; `@AlternativeUserID` in `person` mode when it differs from the professional | `agent:user` `who.identifier`, with the issuer as its system; the acting mode is its first `role` |
| `assurance_level` | the eIDAS level (`low`, `substantial`, `high`) the token's assurance claim maps to under `[authz.assurance]` | none (no such attribute) | the `ihe-assuranceLevel` extension of the `agent:user` |
| `assurance_value` | the raw claim value the level was read from, usually `acr` | none | none |

[PS3.15 §A.5](https://dicom.nema.org/medical/dicom/current/output/chtml/part15/sect_A.5.html)
gives `ActiveParticipant` no assurance attribute, so the DICOM message and the
syslog feed carry no level. The FHIR export carries the level as the BALP
`ihe-assuranceLevel` extension (BALP 1.1.4 §3:5.7.5.4) and never the raw claim
value, which lives in the local store's column only. The tamper-evidence digest covers both columns, like every other column of
the record (see [Tamper evidence](#tamper-evidence) for records written before
the digest was versioned).

For a system caller the mode and the professional stay empty and (b) is
answered by `principal`, which names the client application. Without `[authz.assurance] levels` the level stays empty;
the raw value is still recorded when the token carries the claim. A Basic
credential records none of the four.

Element (c) asks for the **priority category** of the data, in the sense of
Art. 14(1): patient summaries, electronic prescriptions, electronic
dispensations, medical imaging, medical test results and discharge reports,
plus any category a Member State adds in national law. AQL selects archetypes
and templates, never categories, and no openEHR specification maps the one
onto the other, so the deployment declares the mapping in
[the audit category register](installation/config-audit.md#the-audit-category-register), an
[admin register](operational-policy.md#the-admin-registers): a
template id, or failing that the root archetype id, maps to a set of categories
(a priority category by its code in HL7 Europe's
`EEHRxFDocumentPriorityCategoryCS`, such as `Patient-Summaries` or
`Laboratory-Reports`, a national category as its `<system>|<code>`), or to
`none` for content that holds no data of any category. While you declare no
template and no archetype, the map FerroEHR ships applies: the published CKM
templates and COMPOSITION archetypes whose data sit in one priority category
([the shipped map](installation/config-audit.md#the-audit-category-register)). Your own
map replaces it, and `ehds-logging` classifies each access through the map in
force.

Each record is classified when the access happens, from what it served or
wrote, and never at commit, because the map is your reading of your templates
and can change:

- a composition read or write is classified by the version's template id, then
  by its root archetype id;
- an AQL execution is classified by the template and root archetype ids of the
  versions its result page came from; where the answer carries no version (a
  `DISTINCT` or aggregate projection, or zero rows) by the query's positive
  archetype and template constraints, so an operand under `NOT CONTAINS` or a
  predicate under `NOT` never counts. Every id the query constrains to must be
  mapped, and every content operand bound to an id, or the record is
  `unclassified` beside what did map. Each per-EHR record of a query carries
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
template and archetype ids as evidence, and is never refused; the FHIR record
says why (`unmapped`, `unbound`, `named-nothing`). A stored category is its
`<system>|<code>` token. `category_basis` says what the classification was read
from (`returned`, `written`, `queried` or `resource-kind`; records written
before 5.3.2 carry `template`, `archetype` or `query` and the earlier
spellings), and `category_map_digest` names the map that produced it, so a
later map never rereads an old record. The map's
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
requesting person's agent in the FHIR export. A declared code carries no
system: the vocabulary is the one you agree with your callers in `[audit]
purpose_codes`, which has no published code system URI, and attributing the
code to a system it does not come from would misdescribe it. A purpose the
access token states keeps the system the token gives it.
The
[DICOM Audit Message schema](https://dicom.nema.org/medical/dicom/current/output/chtml/part15/sect_A.5.html)
of PS3.15 §A.5 defines no purpose element at all — `EventIdentification`,
`ActiveParticipant`, `AuditSourceIdentification` and
`ParticipantObjectIdentification`, and none of them carries one — so on that
side it is a limit of the format, not something this code withholds.

### Where the purpose of use comes from

The purpose of use reaches the record from two places:

- **The access token.** An issuer with an `[[auth.attributes]]` table
  (`mapping = "iua"`) states its professionals' attributes in the token: the
  IHE IUA `purpose_of_use` and `subject_role` claims under
  `extensions.ihe_iua` (IHE ITI TF-2 §3.71.4.2.2.1.1), and with
  `authorization_details_purposes` the `purpose_of_use` of each RFC 9396
  `authorization_details` entry. The record stores them in
  `token_attributes`, and both renderings write them with their code systems:
  the purposes at the requesting agent's `purposeOfUse`, the roles at its
  `role`, beside the roles the caller held.
- **The `[audit] purpose_header`** (`x-purpose-of-use`), which the caller
  sets. It works alone for an issuer without an `[[auth.attributes]]` table,
  as before.

The token is the authorization server's statement, so it decides. A request
whose header names a purpose the token, which states purposes, does not
state is refused with `403`, and the refusal is recorded as a failed
authentication naming the caller and the token's purposes. A header naming
one of the token's purposes is admitted and recorded with both. Which source
wins is FerroEHR's own design; no openEHR specification governs the purpose
of use.

> [!NOTE]
> Every row of this table is asserted by a test
> (`app/ferroehr/tests/it/audit_ehds_mapping.rs`), **including the gaps**: the
> tests that record an absence fail the day the field arrives, so the table and
> the code cannot drift apart in either direction.

The common specifications the regulation's Article 36 provides for have not
been adopted. When they are, this mapping is re-verified against them rather
than assumed to still hold; see the
[EHDS readiness page](https://ferrohealth.eu/docs/ehds/readiness.html).

### The binding outcome

Every record of an authenticated request carries `binding`: how the caller
was bound to a demographic party, or why the binding refused it. The
committer the same request writes into a version is on
[Who is the committer](security/committer.md). It answers
Annex II 3.2(a) and (b) where the token's subject alone does not. The columns
beside it hold what each outcome needs:

| `binding` | The caller | Recorded beside it |
|---|---|---|
| `bound` | a person bound to a `complete` `PERSON` | `party`, `party_groups`, `provider`, `guest` for an admitted guest, `representation` for a representative, `actor_chain` for a delegated token |
| `provisional` | a person bound to an `incomplete` `PERSON` | `party`, `party_groups`, `provider` |
| `external` | a verified external person of a `federation` issuer | `external_fields`, `agreement`, `provider` |
| `organisation` | an organisation under an agreement with `accessor = "organisation"` | `external_fields`, `agreement` (with its legal ground and access class, and for a `chained` request `correlation_id` and `correlation_source`), `provider` |
| `system` | a client acting for nobody | `party`, the client's `ACTOR` in the policy parties; `provider`, the identifier of the `ORGANISATION` its acts-for relationship targets; `facilities` of the compositions it committed |
| `self` | a patient token of a `patients` issuer | the principal |
| `refused-*` | a refusal of the binding or of a caller kind: `unbound`, `ambiguous`, `inactive`, `abandoned`, `deprovisioned`, `wrong-marker`, `representation-unrecorded`, `key-claim-missing`, `groups-unavailable`, `guest`, `external-incomplete`, `trc-patient-mismatch`, `trc-missing`; and of a caller kind, `system-personal-scope` (a system caller holding a `user/` or `patient/` scope), `no-agreement` (no federation agreement admits the accessor) and `scope-beyond-agreement` (an external caller holding a scope its agreement does not read) and `correlation-missing` (a `chained` organisation request without an attested correlation id) | `party` for a party refused for its state or marker, `binding_detail` for what else the refusal names (`represented`, the subject a representative asserted; `claim`, the name of the key claim a token lacked, never its value) |

`acting_mode` is `person`, `delegated` for a person through a declared client
or actor, `external` for a verified external accessor and `system` for a client
acting for nobody (`client` remains on records written before). The FHIR
`AuditEvent` writes each mode as the person's `agent.role`. A record of a
request with no authenticated caller carries no binding. The columns are in the
chain digest: version 2 hashes them, version 3 adds `token_attributes`, a new
record is version 3, and a record written before keeps verifying under its own
version.

#### The binding fields in the renderings

Neither BALP 1.1.4 §3:5.7.5.4 nor DICOM PS3.15 §A.5 has an element for the
binding fields beyond the bound person, so `ehds-logging` writes them as
details of two entities it adds, and FerroEHR hands it every field. The FHIR
`AuditEvent` carries each entity as an `entity` with its `name` and one
`detail` per value. The DICOM message carries each as a
`ParticipantObjectIdentification` of type `4` (Other) with the entity name as
its `ParticipantObjectName` and one `ParticipantObjectDetail` per value
(RESTful ATNA Rev. 3.6 Table 3.20.4.2.2.1-1), and the ITI-20 syslog feed
sends that message. Both renderings take the detail list from
`ehds-logging`, so they never disagree.

| Recorded field | Rendered as |
|---|---|
| `binding` | `ehds-accessor`: `binding`, with `guest` for an admitted guest and `security-event` for `refused-trc-patient-mismatch` and `refused-trc-missing` |
| `party` | `ehds-accessor`: `party` as `<namespace>\|<type>\|<id>`; in the FHIR `AuditEvent` also the person's `otherId` typed `PRN` |
| `party_groups` | `ehds-accessor`: one `group` per group, in the party's namespace with type `GROUP` |
| `provider` | `ehds-accessor`: `acts-for` as `<issuer>\|<id>` |
| `facilities` | `ehds-accessor`: one `facility` per facility, as `<issuer>\|<id>` |
| `representation` | `ehds-accessor`: `represents`, `represents-ground` and `represents-relationship`; a refusal's `represented` as `represents` alone |
| `actor_chain` | `ehds-accessor`: one `actor` per actor, `<issuer>\|<subject>`, outermost first |
| `binding_detail` | `ehds-accessor`: `binding-claim` for `refused-key-claim-missing` |
| `external_fields` | `ehds-relayed`, marked `asserted` and naming the issuer by `relayed-issuer`: each 2026/2099 Annex field and eHDSI SAML attribute at its own detail (`hp-family-name`, `hp-identifier`, `provider-identifier` and the others), `consent.reference` as `agreement-consent`, and every other field as `asserted-field` `<name>=<value>` |
| `agreement` | `ehds-relayed`: `agreement-name`, `agreement-version`, `agreement-legal-ground` and `agreement-access-class` |
| `correlation_id`, `correlation_source` | the FHIR `AuditEvent`: `correlation-id` and `correlation-source` of the request id's entity; the DICOM message: the same two details of `ehds-relayed`, since it carries no request entity |
| `acting_mode` | the FHIR `AuditEvent`: the person's `agent.role`; DICOM: none |

Every value goes through the renderers' escape: a control character, a
bidirectional control or a line separator is written as its code point, and
no value is cut. A DICOM detail value is base64-encoded as given (PS3.15
§A.5.1.1). A binding record that lacks a field its code needs fails the
rendering, and the sender reports the failure, never a record without the
binding.

### Refusal records and their budgets

Every refused request is recorded as a failed user authentication, through the
same path as an operation record: under `[audit] fail_mode = "closed"` a
refusal whose record the system log cannot take is answered `503`. A refusal of
the login binding or of a caller kind carries its `refused-*` code. Refusals
are charged to one of two budgets, which never draw on each other, so a flood
of invalid tokens cannot hide a verified caller's probing:

| Budget | Charged to | Per caller in a window | Per window |
|---|---|---|---|
| Failed verification | the source network: the `/24` of an IPv4 address, the `/64` of an IPv6 one | 10 | 1000 |
| Verified caller | the token's `iss` and `sub` | 10 | 1000 |

A window lasts 60 seconds. Past a caller's 10 records the rest of its
refusals in that window become one summary record, written when the window
closes: it names the caller and carries `summary`, the count and the EHR ids
the refused requests named (at most 64, while the count stays exact). Past a
budget's 1000 records, the refusals of every caller not yet recorded in that
window become one summary record per issuer and outcome, naming no caller. A
security event (`refused-trc-patient-mismatch`, `refused-trc-missing`) is
recorded once for every answered request and never summarised. A federation
caller above its agreement's `caller_rate`, a contact point's included, is
answered `429` before any patient lookup, and every such refusal goes into
that caller's summary record from the first, with `binding_detail` naming
`caller-rate`, so the count of security events stays bounded. A refusal
record the system log cannot take is counted in
`atna_audit_refusals_unwritten` and logged at `error`.

> [!NOTE]
> The limits, the windows and the budgets are kept **per replica**. Behind a
> load balancer with three replicas, one caller can have up to 30 refusals
> recorded one by one in a window before the summaries begin, and each replica
> writes its own summary records.

### Access through FerroBRIDGE's FHIR facade

The openEHR REST API carries no end-user identity on a read, so when
FerroBRIDGE's FHIR facade reads or writes through FerroEHR, FerroEHR's record
names the bridge's credential under (b) and cannot name the natural person
behind the FHIR request. From FerroBRIDGE 0.0.16 the bridge records each
facade access itself when its `[audit]` section is on: it writes the Annex II
3.2 record with the person, the categories, the time and the origin through
the same `ehds-logging` crate, and sends it as an IHE BALP `AuditEvent`
through `ihe-atna` over the ITI-20 FHIR feed to the Audit Record Repository
you configure there. The bridge refuses a facade access whose record it
cannot take. Read the two records together: FerroEHR's says which data left
the CDR and to which client, and FerroBRIDGE's says which person asked for
it. FerroBRIDGE's
[access record](https://ferrobridge.eu/docs/operate/access-record.html) page
describes its record and its configuration.

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
- **The FHIR `AuditEvent`:** the `entity` named `ehds-emergency-access` that
  `ehds-logging` writes, whose `description` states the mark in plain words
  ("Emergency access: the accessor declared that the access was necessary to
  protect the vital interests of the data subject (Regulation (EU) 2025/327
  Art 11(5)).") and whose `ehds-emergency-purpose` details name the purposes
  that marked it. The declared code stays on `agent.purposeOfUse` and on
  `purposeOfEvent`. A patient portal keys on the entity and shows its
  description.
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
still logged with the mark. An EHDS Art. 8 restriction of access by health
professionals is different: a caller holding `authz.rbac.emergency_access_role`
that declares an emergency purpose reads the restricted data, and its record
carries the mark
([restriction of access](security.md#restriction-of-access-by-health-professionals-ehds-art-8)).
An emergency the caller does not declare is not marked.

### Retention, and who chooses it

`[audit.store] retention` decides how long the repository keeps a record:
`0`, the default, keeps them forever, and any other value sets the horizon.
Every record stores the first date it may be deleted on, and the reaper
deletes each record hourly on or after that date and never before it. An
access record takes its end from the retention its categories and origins
call for (Regulation (EU) 2025/327 Annex II 3.4), so two records written the
same day can end on different dates; every other record ends the day after
its date of access plus the horizon. A record written before FerroEHR stored
per-record ends is reaped by the horizon.

**An openEHR access-log entry states its own retention.** Each
`ACCESS_LOG_ENTRY` carries an `ACCESS_RETENTION` when it is written:
`retain_until`, the last day it is kept, and `grounds`, the provisions that
set the period. The period is the one the retention policy above computes from
the entry's categories and origins, so an entry whose categories call for
different periods keeps the longest, and an entry kept without end states no
`retain_until`. The grounds are coded in the `ehds-logging` terminology: the
legal ground of the floor (`eu-2025-327-art-9-2` for Art. 9(2)) and what called
for the period (`default`, `category:<system>|<code>` or `origin:<key>`). The
same hourly worker replaces every entry whose `retain_until` has passed by an
`ACCESS_LOG_TOMBSTONE` in place, in any order of expiry. The tombstone keeps the
entry's uid, `system_id`, `time_recorded` and chain link, names the grounds of
the period that ended, and holds nothing else of the entry, so the chain
verifies after the content is gone ([Tamper evidence](#tamper-evidence)). A
tombstone is never replaced again, and the database refuses any other change
to it. An erasure a law orders before the period ends takes the same path
through `POST {ext}/admin/access-log/erase`, with the law's provisions as the
tombstone's grounds; the erasure is an access record of its own
(`access-log-erasure:<uid>`) naming the administrator
([Admin APIs](operations-admin-apis.md)). No configuration key decides an
entry's period beyond `[audit.store] retention`, which stays the floor every
period is held to.

**The default is deliberate.** An access log is the evidence a data subject's right of access is served from, and a repository that silently
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
writes them: a horizon set as `[audit.store] retention` is compared
against the years exactly, and one set as `retention` against the most
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
A record takes about 1.2 KB on disk, so the horizon decides how far the log
grows ([Storage sizing](operations.md#audit-retention)).

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
What the product gives that portal is a grant of its own size: a
`read_subject_audit` grant in the CDR
([Operational policy](operational-policy.md#what-the-grants-decide)) reads
the log for one subject at a time, with
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
| `[audit.store]` | **on** | The local **Audit Record Repository**: records persist in the dedicated `audit` PostgreSQL schema (append-only and tamper-evident, strictly outside the EHR content), served back via the ITI-81 search below. `retention` prunes old records hourly (`0` = keep forever). |
| `[audit.syslog]` | off | The classic ATNA feed: DICOM PS3.15 XML as RFC 5424 syslog over RFC 5425 TLS to an external ARR, per IHE ITI-20. |
| `[audit.fhir_feed]` | off | The RESTful-ATNA feed (ITI-20 **ATX: FHIR Feed**): each FHIR `AuditEvent` is `POST`ed to an external FHIR ARR. With the local store on, delivery is **outbox-driven**: an ARR outage loses nothing, pending records ship on recovery. The outbox holds the records written while the feed is configured; with the feed off, no record is owed. Records stored before FerroEHR kept an outbox and never delivered ship once a feed runs. |

Emission never blocks the request path: a record is handed to a bounded queue
(`queue_capacity`) and written by a background drain.

### While a composition is held from the person

With [the access-delay register](security.md#delayed-access-by-the-person-ehds-art-33)
on, a new composition can be held from the person under
[EHDS](https://eur-lex.europa.eu/eli/reg/2025/327/oj) Art. 3(3) until a
professional releases it or its period passes. Only one route honours the
hold: the subject-scoped [ITI-81 read](#retrieving-audit-records-iti-81), the
one a patient portal serves the person's own trail from, leaves out every
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

The local store is the durability anchor. Under `fail_mode = "closed"`, an
operation whose audit record cannot be recorded answers
`503 Service Unavailable`: the deployment demanded an audit trail it cannot
currently deliver, so no un-audited PHI access happens, and the same holds for
the domain-level records the subject resolutions and the EHR-Extract
transfers write. `open`, the default, drops-and-meters instead,
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

Each record's digest covers every column of the record except the digest
itself (`row_hash`) and the two per-sink forwarding stamps
(`delivered_syslog_at`, `delivered_fhir_feed_at`). A column added by a later
release joins the digest in the same release.

The store keeps a record's facts in columns and renders its FHIR `AuditEvent`
from them when the record is read (ITI-81) or delivered (the FHIR feed). What
the rendering takes from the configuration (`source_id`, `enterprise_site_id`,
`server_host`, `value_if_missing`, the store horizon, the audit category register and
`emergency_purpose_codes`) is kept once per distinct configuration in
`audit.audit_render_policy`, keyed by the SHA-256 of its text, and every record
names the policy it was written under. A record therefore renders the same
document after the configuration changes, and the verification below reports a
policy whose text no longer matches its digest, as it reports a changed record.
Release 5.6.0 starts on an empty audit store: its migration refuses a store
that holds records written by an earlier release, and names the remedy.

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
  fully in force with `database.migrations = "verify"`; see
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
  content modified after it was written, a render policy changed or missing,
  records deleted with no retention record for the removal,
  or records removed from the end of the chain where no successor would have
  noticed.
- **The server runs the check itself, on a schedule.** With the local store on,
  the check runs one minute after boot and then every
  `[audit.store] verify_interval` (default `3600`, one hour; `0` turns
  it off). Each run verifies the records written since the newest link an
  earlier run found intact, and the chain head, so its cost follows the
  records written since rather than the size of the trail
  (`audit.verify_audit_chain_incremental()`). A full pass
  runs every `[audit.store] full_verify_interval` (default `604800`,
  one week; `0` leaves it to the query above) and after every run that found
  damage, so damage to an older record stays reported. A run that finds damage logs at `ERROR`, one summary line with the
  finding count and one line per finding (the first 20), and adds the count to
  the `atna_audit_chain_findings` counter with `kind = "damage"`. A run that
  cannot verify at all counts as `kind = "unverifiable"`, because a trail that
  cannot be checked is never a pass. The latest result is the `audit_chain`
  indicator on `GET /health/readiness`: `UP` while intact and `DEGRADED`
  when damaged or unverifiable; its detail on `GET /management/health` names
  the first finding. It never flips
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

## Checkpoints and the transfer of a log

The records of the openEHR access log (`audit.access_log_record`) are chained
with the `audit_event` records above. A chain proves order and completeness
against someone who cannot rewrite its whole tail. A **checkpoint** closes
that case: an `ACCESS_LOG_CHECKPOINT` names the position and digest of the
head of the chain and is signed with the deployment's OpenPGP key, in the
form an openEHR `ATTESTATION` carries its proof (RM common, Generic Package
§Attestation). A verifier that holds a checkpoint detects any later rewrite
of the records it covers. No openEHR specification governs log signing; this
is our own design.

- **The key** is the keyring of [version signing](signing/pgp.md):
  `[record.signing]` in `pgp` mode, with its `retired_key_files` still
  verifying what an earlier key signed. With that key every link is signed
  and the log is sealed; a deployment without it signs nothing, and
  `ferroehr access-log checkpoint` answers that the keyring holds no key.
- **Use an Ed25519 key.** Each access writes one entry per EHR it reached,
  and each entry's link is signed in the audit writer's transaction while
  the head of the chain is locked. RSA signing is far slower than Ed25519,
  and every other audit write waits on it.
- **The schedule.** The server seals the head every five minutes when the
  chain grew since the last checkpoint; a run on a log that did not grow
  writes nothing. There is no switch and no interval to set.
  `ferroehr access-log checkpoint` and `POST /admin/access-log/checkpoint`
  seal one on demand. Keep a copy of a checkpoint somewhere the database
  owner does not control (a ticket, a timestamping service, the data
  protection officer's mailbox): that copy is what makes a rewrite of the
  whole tail detectable.
- **Verification.** `ferroehr access-log verify` and
  `POST /admin/access-log/verify` recompute the whole chain (an entry
  replaced by its tombstone verifies through the content digest the
  tombstone keeps), check the checkpoint's signature and that the link at its
  position is the one it sealed, and check the signature of every signed
  link. Each finding is reported; the command exits 1 when the log does not
  verify, and the route answers `200` with `intact: false`.
- **Signed links.** Each link's digest is signed as the eHDSI Audit Trail
  Profile §2.2 asks of a national contact point. The server locks the head
  of the chain, computes the links the new records will get, signs them and
  writes each signature with its record in one `INSERT`; a stored signature
  never changes.
- **Moving the log with the EHR.** EHDS Annex II 2.6 and Art 9(2) keep the
  access information available when an EHR moves to another system.
  `ferroehr access-log export --ehr-id <id>` and
  `GET /admin/access-log/export?ehr_id=` write the EHR's records with their
  links, every link from the first of them to the checkpoint they verify
  against, and that checkpoint, sealing one when none covers them. The
  receiving system runs `ferroehr access-log import --input <file>`: it
  verifies the export against its checkpoint and the certificates in
  `[audit.access_log] trusted_key_files`, keeps the records with their
  `system_id`, links and `retain_until` apart from its own chain, and records
  the import as an `import` entry of its own log. An import repeated stores
  nothing twice. An export whose span holds `audit_event` records is refused,
  since only the database that wrote those records can recompute their
  links.

## Retrieving audit records (ITI-81)

The RESTful-ATNA **ITI-81 Retrieve ATNA Audit Event** transaction is a FHIR
search on `AuditEvent`, so it is served at FerroEHR's FHIR R4 base,
`/ferroehr/rest/fhir/r4`, beside the openEHR API rather than under it, where a
FHIR client or an IHE Audit Consumer expects `[base]/AuditEvent`. It is the one
FHIR route FerroEHR serves:

```text
GET /ferroehr/rest/fhir/r4/AuditEvent
```

It returns a FHIR `searchset` Bundle of the matching records' `AuditEvent`
documents, rendered from the store, newest first, with the full match `total`, a `self` link and a `next` link
while more pages follow. The query is read by `ihe-atna`, the family's audit
crate. At least one `date` parameter is required (RESTful ATNA
§3.81.4.1.2.1): a query without one is a `400`. Every search parameter
§3.81.4.1.2.2 says the repository shall support is applied. A parameter
with several comma-separated values matches any of them, and every
parameter of the query must match. A token takes the three forms of FHIR R4
Search §3.1.1.4.10: `code`, `|code` (recorded without a system) and
`system|code` (or `system|` for any code of that system). FerroEHR records
identifiers without a system, except the object of an access record, whose
system is `urn:openehr:object-id`: `entity.identifier=urn:openehr:object-id|<uid>`
and `entity.identifier=<uid>` find it, and a query that names another system
for an identifier matches nothing. Every parameter answers through an index. A
parameter on an element of the document (`address` and the `.identifier`,
`entity-type`, `entity-role`, `type` and `subtype` parameters) matches the
search tokens the store takes from the record's rendered document when it
writes the record (`audit.audit_search_token`, `audit.audit_search_set` and
the record's `search_tokens`), so the search finds what the served document
says and renders only the page it serves. An `entity.identifier` that carries
the record's request id or the object it names (the version uid of a create)
is read from the record's own column instead, so a request adds no row to
`audit.audit_search_token`. Retention deletes the tokens that no remaining
record names, and never one a kept record names.

| Parameter | Matches |
|---|---|
| `date` | **required**, repeatable: a FHIR date value with a prefix (`eq` when none, `gt`, `lt`, `ge`, `le`, `sa`, `eb`); a day such as `2026-07-12` covers the whole day, read in UTC |
| `patient` | the recorded patient (EHR subject) id |
| `patient.identifier` | `entity.what.identifier` of the entity whose role is Patient |
| `agent` | the authenticated principal |
| `agent.identifier` | `agent.who.identifier` |
| `entity` | the touched resource id |
| `entity.identifier` | `entity.what.identifier` of any entity |
| `entity-type` | `entity.type` |
| `entity-role` | `entity.role` |
| `source.identifier` | `source.observer.identifier`, the audit source id |
| `type` | `type` |
| `subtype` | `subtype` (the REST interaction and the openEHR operation) |
| `address` | a part of `agent.network.address`, compared without regard to case |
| `outcome` | the outcome indicator: `0`, `4`, `8` or `12` (FerroEHR records `0`, `4` and `8`) |
| `action` | the action code: `C`, `R`, `U`, `D` or `E` |
| `_count` / `_offset` | paging; page size defaults to 50, capped at 1000; `_count=0` returns the total alone |

A malformed value is a `400` carrying a FHIR `OperationOutcome` with
`invalid`; another FHIR R4 `AuditEvent` parameter, which §3.81.4.1.2.2 does
not ask for, is a `400` with `not-supported`; a parameter the crate does not
know is ignored
(§3.81.4.1.3). Under RBAC the
unscoped retrieval is **admin-only**; a caller holding a
`read_subject_audit` grant reads it **scoped to one subject**: one
`patient` or `patient.identifier` with one value is required, and the caller
is refused with a `403` that names the parameter when it omits
it. Reading a subject's log is itself recorded as an access naming that
subject and the record count served. An
[emergency access](#emergency-access-ehds-art-115) in the subject's log
carries its mark as the `ehds-emergency-access` entity. The surface answers `404` when the local
store is disabled.

```bash
# Who accessed patient-42's data this month?
curl -u admin:pw \
  "https://cdr.example.org/ferroehr/rest/fhir/r4/AuditEvent?patient=patient-42&date=ge2026-07-01T00:00:00Z"
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
certificate_file = "/etc/ferroehr/server.pem"
key_file = "/etc/ferroehr/server.key"
min_version = "1.3"               # 1.3 (default) | 1.2
client_auth = "required"          # none | optional | required
client_ca_file = "/etc/ferroehr/client-ca.pem"
```

The protocol floor is **TLS 1.3 only** by default, as RFC 9325 §3.1.1 has it
supported. Setting `min_version = "1.2"` enables
1.2 *alongside* 1.3, never instead of it, for a client that genuinely cannot
do 1.3, such as an older integration engine or a pinned Java runtime. TLS 1.1
and 1.0 are refused when the configuration is read (RFC 8996).

With `client_auth = "required"`, only clients presenting a certificate
chaining to your explicit trust anchor complete the handshake: the IHE
mutually-authenticated-node posture. Deployments terminating TLS at an
ingress leave `[server.tls]` unset and enforce mTLS there instead.

> [!WARNING]
> The separate-port management listener (`management.listen`) stays **plain HTTP**
> even with `[server.tls]` set, and it binds all interfaces. Treat it as an
> internal surface and keep it off any publicly routed port. Under
> `deployment.profile = "production"` setting it opens the `plaintext_listener`
> gap, so the server refuses to start unless the gap is accepted by name.

Complete the posture with time synchronisation (IHE Consistent Time): run
NTP/chrony on every node so audit timestamps align across systems.

## Against independent ATNA implementations

The transactions above are tested by FerroEHR's own suites, which read the
profiles the way FerroEHR's authors read them. EHDS Annex II 1.4 asks that a
component meant to work with other products does so reliably, so
[`scripts/interop/atna.sh`](https://github.com/FerroHEALTH/FerroEHR/blob/main/scripts/interop/atna.sh)
runs the quickstart against software FerroEHR's authors did not write, each
from a published image:

| Transaction | Independent implementation | What the run requires |
|---|---|---|
| ITI-20 FHIR feed | HAPI FHIR, an open-source FHIR R4 server, as the Audit Record Repository, with IHE BALP 1.1.4 loaded | It accepts every record, and its validator finds no error against base R4 and the BALP profile each record names |
| ITI-20 syslog | syslog-ng over RFC 5425 TLS | Every message parses as RFC 5424, and its payload is a well-formed DICOM PS3.15 A.5.1 `AuditMessage` (libxml2) |
| ITI-81 | HAPI FHIR's AuditEvent search over the records the feed delivered | The same searches answer the same records on both |

```shell
cargo build --release -p loadtest
bash scripts/interop/atna.sh ghcr.io/ferrohealth/ferroehr:<tag> \
  ghcr.io/ferrohealth/ferroehr-postgres:<tag> atna-out
```

It prints every disagreement and exits non-zero on any. A disagreement is
either FerroEHR's or a shared crate's defect, filed and fixed, or the other
side's, recorded with the profile text that shows it.

The run of 2026-10-29 found no disagreement: HAPI FHIR accepted and validated
all 71 records of the window, syslog-ng parsed all 72 messages, and the ten
ITI-81 searches answered the same records on both repositories
([the record](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/conformance/interop/atna/2026-10-29.md)).

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
