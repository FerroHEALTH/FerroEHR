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

## The openEHR access-log entry

Every access to EHR data is recorded as an openEHR access-log entry: an
`ACCESS_LOG_ENTRY` of the draft `org.openehr.rm.access_log` 0.1.0 model,
FerroEHR's proposal to openEHR, which no openEHR release defines yet. Reads,
writes, AQL queries, exports, refusals and reads of the access log itself each
write one. An access that reaches several EHRs writes one entry per EHR, and
the entries of one request share its request id, so a population query writes
one entry for each EHR that contributed a row, a statement that projects
only the EHR (`SELECT e/ehr_id/value FROM EHR e`) included.

The administrative routes that touch an EHR write entries the same way, one
per EHR they touched: the physical EHR delete, one EHR or in bulk; the
restriction, research-objection, access-delay and access-repair registers and
the `EHR_ACCESS` settings commit that carries a restriction or an opt-out;
archiving an EHR and restoring it; its retention anchor and the retention hold
on one of its objects; the repository dump and load; and the EHR Extract
export and import. An export's entry names the request it answered, the bound
party and the provider, as every other access entry does. The import of another system's access log (`ferroehr
access-log import`) writes its `import` entry through the same path. SCIM
provisioning touches party data only and writes none.

The entry names who accessed (the bound party, the groups, the provider, the
binding outcome and the credential by its issuer, subject, client and token
id), what (the EHR, the version or object, its categories and origins), why
(the declared purposes), under which authorisation (the agreement, the
representation, a data permit, the emergency mark, and the grounds of the
access decision below), the outcome (`granted`, `refused` or `failed`) and
the request. It carries no data value of the EHR:
a query is named by its stored name or the SHA-256 of its text, never the
text, and a read of the access log through ITI-81 is named by the digest of
its search.

**The grounds of the access decision.** The access evaluator's decision on
the EHR is in the entry's `authorisations`, each an `ACCESS_AUTHORISATION`
whose `name` and `version` name the document the ground is defined in:

| Kind | When | What it names |
|---|---|---|
| `role` | always | each grant the caller held, by its operation code (`name`) and the `ROLE` that carries it (`reference`); each role the caller held |
| `subject of care` | the person reads their own record | |
| `consent` | a consent permit admitted the access, or the caller is a gate-keeper of the settings | `EHR_ACCESS` and the uid of its version; the directive by its uid, its first identifier or its position from 1, and its legal grounds |
| `representation` | a `delegation` permit admitted a representative | as `consent` |
| `treatment relationship` | the decision read a live care relationship | `ACCESS_POLICY` and the uid of its version, and in `care` the relationship the access rested on: its `role`, its `path` (`direct`, `team` or `self_declared`), its target (`carer`: the professional's `PERSON` or `ROLE`, or the care team or unit), the uid of its version (`relationship`), whether the grace after its end admitted the access (`in_grace`), the carer it covers for (`covers_for`) and its `provider`. Where several admitted it, `direct` comes before `team` before `self_declared`, then the order of the roles in the rule that matched |
| `emergency` | an emergency lifted a directive, a controller rule or steps 6 to 8 | the directive it lifted, or the `ACCESS_POLICY` version with the rule it lifted by its uid, its first identifier or its position from 1; an entry naming the objects it delivered names only the directives and rules whose data scope covers one of them |

The entry of each EHR an AQL query served names the same grounds a read of
that EHR alone names. The query's condition is built from this replica's copy
of every EHR's settings, which holds each EHR's subject and the uid of its
`EHR_ACCESS` version beside the settings, so the record adds no database read.
For an EHR with no settings, which the copy does not hold, a care relationship
is named when only a caller in care could have been served its rows, without
the relationship itself. In the FHIR `AuditEvent` the care relationship's
role is an `agent.role` coding of the requestor, its HL7 code system named by
its canonical URL (`HL7_ParticipationType` as
`http://terminology.hl7.org/CodeSystem/v3-ParticipationType`), and the
policy version that named it is among the requestor's `agent.policy`.

`accessor.binding` says how FerroEHR bound the login to the party, or why
it could not, in the `credential binding` group of the `openehr_access`
terminology: `bound`, `provisional`, `subject of care`, `external`,
`organisation` and `system` for a login it bound, and the cause for one it
refused, such as `unbound`, `party inactive`, `no agreement` or `treatment
relationship confirmation missing`. The draft terminology page lists every
code.

A refusal names its code in `outcome.reason`, in the `openehr_access`
terminology, and in `outcome.description` the step that refused and the
directive or policy of that step, such as `step 2 (restriction): directive 1
of EHR_ACCESS 5a1c…::cdr.example.org::2`. This holds for the `404` of an
opt-out or a restriction too: the response says nothing (EHDS Art. 8), and
the entry says why. An override in an emergency sets `is_emergency`, so the
person sees it in their access log (EHDS Art. 9 and 11(5)). The EHDS record
writes the versions the grounds name after the data permit at
`agent[user].policy`, "the policy or plan that authorized the activity being
recorded" (FHIR R4 `AuditEvent.agent.policy`), read from the entry's grounds
when the record is rendered. Which grounds are named is FerroEHR's own design.

**The entry is the record.** The store keeps an access to EHR data as its
entry alone, in `audit.access_log_record`, chained under one hash chain (see
[Tamper evidence](#tamper-evidence)), and each entry states the date it is
kept until and why. The FHIR `AuditEvent` the ITI-81 search and the ITI-20
FHIR feed serve, the DICOM message the syslog sink sends and the EHDS access
record are projections of the stored entry: each is written from what the
log keeps and from nothing else, and none is stored as a second record.
`audit.audit_event` keeps the records of accesses outside the access log: an
authentication, a template or party operation, an application activity.

The entry is written with every other record of its request in one database
transaction before the response leaves, and an access whose entry cannot be
written is refused with `503` ([how a record is written](#how-a-record-is-written)).

### The access log details

The draft's core attributes are openEHR's: the action, the outcome, the
accessor, the items, the query, the authorisations, the channel. The facts
the BALP `AuditEvent`, the DICOM message and the EHDS access record carry
that the core holds no attribute for are kept in the entry's
`other_details`, which the draft gives to the fields a jurisdiction adds: one
`ITEM_TREE` under the archetype `openEHR-EHR-ITEM_TREE.access_log_details.v1`,
one `ELEMENT` per fact, named by its node id. A fact the core states as the
access holds it, such as the principal the credential names or the count the
outcome delivered, is not repeated there: read the core first and the
details beside it. They are part of the entry, so the hash chain covers
them, and a tombstone removes them with the rest of its content.

The archetype is published, so another system reads the details of an
exported log by it. FerroEHR ships it among its default definitions
(`openEHR-EHR-ITEM_TREE.access_log_details.v1.0.0.adls`, stored at the
first boot like the [operational policy](operational-policy.md)
definitions, with its digest in `deploy/definitions/published.sha256`), and
you read it with `GET /definition/template/adl2/openEHR-EHR-ITEM_TREE.access_log_details.v1.0.0`.
The node ids are two-digit id-codes, so their byte order is their numeric
order, which is the order the elements of a tree are kept in. No openEHR
specification governs the details: they are FerroEHR's own.

| Node id | Name | Value | Holds |
|---|---|---|---|
| `id11` | event action | `DV_TEXT` | the DICOM `EventActionCode`: `C`, `R`, `U`, `D` or `E` |
| `id12` | resource class | `DV_TEXT` | the class of resource the access addressed |
| `id13` | outcome indicator | `DV_COUNT` | the DICOM `EventOutcomeIndicator`: `0`, `4` or `8` |
| `id14` | principal | `DV_TEXT` | the login the access was made under |
| `id15` | issuer | `DV_TEXT` | the issuer of the caller's token |
| `id16` | client id | `DV_TEXT` | the client the token was issued to |
| `id17` | token id | `DV_TEXT` | the token's `jti` |
| `id19` | role | `DV_TEXT` | a role the caller held, one element each |
| `id20` | domain | `DV_TEXT` | the pseudonymisation domain, when it is not the resource class's own |
| `id21` | ehr id | `DV_TEXT` | the EHR the request named |
| `id22` | subject | `DV_IDENTIFIER` | the subject of care of the EHR, as `EHR_STATUS.subject` names it, with its namespace as the issuer |
| `id23` | subject lookup failed | `DV_BOOLEAN` | set when the subject could not be looked up |
| `id24` | object id | `DV_TEXT` | the object the request named; none for a read of the log, whose search the entry names by digest |
| `id25` | purpose | `DV_TEXT` | the purpose of use the caller declared |
| `id26` | legal basis | `DV_TEXT` | the legal basis the deployment attributes |
| `id27` | result count | `DV_COUNT` | how many records the access served |
| `id28` | origin | `DV_TEXT` | an origin of the data, one element each |
| `id29` | origin count | `DV_COUNT` | how many distinct origins the data had |
| `id30` | category map digest | `DV_TEXT` | the category map the access was classified through |
| `id31` | category content | `DV_PARSABLE` (`application/json`) | what the classification read |
| `id32` | acting mode | `DV_TEXT` | how the credential acted |
| `id33` | assurance level | `DV_TEXT` | the assurance level of the authentication |
| `id34` | assurance value | `DV_TEXT` | the claim value the level was read from |
| `id35` | professional id | `DV_TEXT` | the identifier of the natural person who accessed |
| `id36` | token attributes | `DV_PARSABLE` (`application/json`) | the purposes and roles the token stated |
| `id37` | binding | `DV_PARSABLE` (`application/json`) | the party binding with every field |
| `id38` | summary | `DV_PARSABLE` (`application/json`) | the count and EHRs of a summary of refusals |
| `id39` | event type | `DV_TEXT` | the DICOM `EventTypeCode` of an authentication record: `110122` Login, `110123` Logout |

The subject of care is kept so the BALP `entity:patient` and the
`patient` search parameter name the patient as they did when the access
was recorded, whatever happens to the EHR after. An entry imported from
another system ([Checkpoints and the transfer of a log](#checkpoints-and-the-transfer-of-a-log))
keeps that system's details, and FerroEHR projects none of it.

To read the trail with SQL, `audit.access_trail` holds every record of
either form with the facts it states, in the audit vocabulary
(`operation`, `principal`, `binding`, `party` and the rest), an entry's read
from its content. `audit.access_detail(content::jsonb, 'id32')` reads one
access detail of an entry. The categories a classification computes are no
column there: the projection writes them.

### The projections and their losses

Each projection reads one stored entry and writes one target record.
"Loss" means the target has no place for the value; the entry keeps it. The
FHIR R4 `AuditEvent` follows IHE BALP 1.1.4; an access the category map did
not classify is a BALP exchange without the EHDS entities. Each row is a test
(`access_log::projection` in the source).

**To the FHIR R4 `AuditEvent` (IHE BALP 1.1.4).**

| Entry | `AuditEvent` | Loss |
|---|---|---|
| `action` | `action` (`C`, `R`, `U`, `D`, `E`), `type` `rest` and the FHIR interaction as the first `subtype`; an export or import is `type` Export or Import | the openEHR action has no code system of its own: `export` and `import` fold into `R` and `C` |
| `operation` | a `subtype` in `urn:openehr:its-rest:operation` | none |
| `time_recorded` | `recorded` | `occurred`: no `period` is written |
| `outcome.result` | `outcome`: `0` granted, `4` refused or a minor failure, `8` a serious failure | refused and a minor failure share `4`; `outcomeDesc` states the outcome in words and no reason code |
| `outcome.delivered` | the `delivered` detail of the `ehds-categories` entity | none, as a detail |
| `accessor.party` | the `ihe-otherId` typed `PRN` of `agent:user` (BALP 1.1.4 §3:5.7.5.4) | none |
| `accessor.credential` | `agent:user` `who.identifier` (issuer as `system`, subject as `value`), the token id as its first `policy`, the client as an agent of its own, the assurance as `ihe-assuranceLevel` | the authentication method has no element |
| `accessor.provider` | an agent naming the provider of the caller's party binding | none |
| `accessor.roles`, `accessor.kind` | `agent:user` `role`, the acting mode first | none |
| `accessor.groups`, `accessor.binding`, `accessor.delegation` | details of the `ehds-accessor` entity, one `actor` per actor in the chain's order | no element; details |
| `ehr_id` | the `ehr` entity, with the subject of care as the patient entity (`entity:patient`, role `1`) | none |
| `items` | `entity:data` (type `2`, role `4`) naming the version in `urn:openehr:object-id`; the categories and the template and archetype ids as details of `ehds-categories` | the RM type and the paths have no element |
| `items.origins` | one `origin` entity each | no element; entities |
| `query` | `entity:query` (role `24`), a stored query by its name | the text digest is not written |
| `purposes` | `purposeOfEvent` and `agent:user` `purposeOfUse` | none |
| `authorisations` | a permit as a `policy` of `agent:user` after the token id; an agreement in the `ehds-relayed` entity; a representation in `ehds-accessor` | legal grounds have no element; details |
| `is_emergency` | the `ehds-emergency-access` entity naming the purposes that marked it | the mark becomes an entity |
| `channel.request_id`, `channel.client_address` | the `X-Request-Id` entity; the client agent's `network.address` | none |
| `system_id` | `source.observer.identifier`; `source.site` from `[audit] enterprise_site_id` | none |
| `security_labels` | the `securityLabel` of the entity they label | no `meta.security` is written |
| `retention` | details of `ehds-categories` | no element; details |
| `link`, checkpoints | nothing | the chain does not travel; verify it on the openEHR side |
| `other_details` | the values it holds, where the rows above put them | the tree itself is not written |

**To the DICOM PS3.15 A.5 audit message (the syslog sink).**

| Entry | DICOM element | Loss |
|---|---|---|
| `action` | `EventActionCode`; `EventID` Patient Record (110110), Query (110112), Export (110106), Import (110107) | as in R4 |
| `operation` | `EventTypeCode` in `openEHR-ITS-REST` | none |
| `time_recorded` | `EventDateTime` | `occurred` |
| `outcome` | `EventOutcomeIndicator`, `EventOutcomeDescription` | the reason's code |
| `accessor.credential` | the requestor's `UserID` (the subject, or the professional a client acts for) | the issuer, the token id, the assurance |
| `accessor.roles` | `RoleIDCode` in the `codeSystemName` of `[audit] role_code_system`, by default the deployment's `[api] system_id` | none |
| `accessor.party`, `binding`, `groups`, `delegation` | details of the `ehds-accessor` participant object | no element; details |
| `accessor.provider` | nothing | lost |
| `system_id` | `AuditSourceID`, with `AuditEnterpriseSiteID` from the deployment | none |
| `channel.client_address` | `NetworkAccessPointID`, type `2` | none |
| `ehr_id` | the patient participant (type `1`, role `1`) names the subject of care | the `ehr_id` itself is not written |
| `items` | a System Object (type `2`, ID type URI) naming the version | the categories, origins, template and archetype |
| `query` | the query object of the Query event | the text digest |
| `purposes`, `authorisations`, `is_emergency` | nothing in the A.5.1 schema | lost |
| `retention`, `security_labels`, `link`, `other_details` | nothing | lost |

**To the EHDS access record (`ehds-logging` `AccessRecord`).**

| `AccessRecord` | Entry | Loss |
|---|---|---|
| `action` | `action` | export and import fold into `Read` and `Create` |
| `recorded` | `time_recorded` | none |
| `outcome`, `outcome_description` | `outcome.result` and the outcome indicator | a refusal shares `MinorFailure` with a minor failure |
| `accessor` (issuer, subject, client, token id, acting, roles, assurance, provider, party, groups, binding, actors, represents, relayed, purposes, policies) | `accessor`, `purposes`, `authorisations` and the access details | none |
| `subject.ehrs` | `ehr_id`, with the subject of care found | one record per entry; the entries of a request share its request id |
| `categories` | `items.categories`, computed again through the category map that classified the access | none |
| `delivered`, `origins`, `origin_count` | `outcome.delivered`, `items.origins`, the origin count | none |
| `request` (id, operation, resource, stored query, client address, correlation) | `channel`, `operation`, `items`, `query` | the query text is never carried |
| `retention`, `labels`, `emergency` | `retention`, the categories and origins, `is_emergency` | none |
| none | `link` | the chain |

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
| `token_attributes` | The purposes of use and the professional roles the access token states, each with its code system, for an issuer with an `[auth.issuer.<name>.attributes]` table ([where the purpose comes from](#where-the-purpose-of-use-comes-from)). |
| `emergency_access` | `true` when the declared purpose, or a purpose the token states, is one of `[audit] emergency_purpose_codes`: an [emergency access](#emergency-access-ehds-art-115) (EHDS Art. 11(5)). |
| `legal_basis` | The basis the deployment processes under, from `[audit] legal_basis`. |
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
| (a) the healthcare provider or other individuals having accessed | `provider` of the binding (the provider of the caller's party binding, read from the CDR), or the `organisation` record of a verified external organisation | — (no such attribute) | the `acts-for` detail of the `ehds-accessor` entity | Recorded |
| (b) the specific natural person or persons having accessed | `principal` (+ `token_id`, `client_ip`), and `professional_id` + `acting_mode` for a caller its issuer classifies as a person or a delegate | `ActiveParticipant/@UserID` (the professional; a client application acting for one is a second, non-requesting participant; the token subject is `@AlternativeUserID` when it differs) | the `agent:user` slice, `who` the token's issuer and subject, the professional's identifier as its `ihe-otherId`, and the client application as an agent of its own (BALP 1.1.4 §3:5.7.5.4) | Recorded |
| (c) the categories of data accessed | `categories` + `category_basis` + `category_evidence` + `category_map_digest`, classified by `ehds-logging` through the audit category register (beside `resource_class`, `domain` and `result_count`) | none (no such element) | the `entity` named `ehds-categories`, with one `ehds-category` detail per category and its basis, each id the classification read and the map digest; each category is also a `meta.security` label | Recorded |
| (d) the time and date of access | `recorded_at` | `EventIdentification/@EventDateTime` | `recorded` | Recorded |
| (e) the origin or origins of data | `origins` + `origin_count` (the `FEEDER_AUDIT` originating systems of the served version bodies, or the server that created them, derived at commit onto `version.origins`) | — (no such element) | one `entity` named `origin` per origin, its `system-id` detail the originating system | Recorded |

Read (a) beside (b): they are both about the accessing side, and the pair
distinguishes the **organisation** on whose behalf the access happened from the
**natural person** who made it. The principal answers (b); the provider of the
caller's party binding answers (a). It is read from the CDR, never from a token
claim, so the provider a record names is the one the deployment bound the
caller to. A caller with no binding, such as a Basic user with no `PERSON`,
leaves it empty rather than guessed.

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

- **The access token.** An issuer with an `[auth.issuer.<name>.attributes]` table
  (`mapping = "iua"`) states its professionals' attributes in the token: the
  IHE IUA `purpose_of_use` and `subject_role` claims under
  `extensions.ihe_iua` (IHE ITI TF-2 §3.71.4.2.2.1.1), and with
  `authorization_details_purposes` the `purpose_of_use` of each RFC 9396
  `authorization_details` entry. The record stores them in
  `token_attributes`, and both renderings write them with their code systems:
  the purposes at the requesting agent's `purposeOfUse`, the roles at its
  `role`, beside the roles the caller held.
- **The `[audit] purpose_header`** (`x-purpose-of-use`), which the caller
  sets. It works alone for an issuer without an `[auth.issuer.<name>.attributes]` table,
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
| `self` | the record's person, whose login the party binding binds to the `PERSON` an EHR's subject names, through a `persons` or a `patients` issuer | the principal; `party`, that `PERSON` |
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
same path as an operation record: a refusal whose record cannot be written is
answered `503`. A refusal of
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

The controller decides how long the log keeps a record, in its access policy
(`ACCESS_POLICY.log_retention`, written through
[`/admin/access_policy`](operational-policy.md)). FerroEHR provides the
mechanism and names no national law of its own. `log_retention` holds:

- `in_union`: whether Regulation (EU) 2025/327 binds the controller. When it
  does, Art. 9(2) keeps the access information "available for at least three
  years from each date of access to the data", and FerroEHR refuses a shorter
  period.
- `period`: how long each record is kept from the date of access, in whole
  years (`P5Y`) or whole days (`P1826D`). Absent keeps every record without
  end.
- `floors` and `ceilings`: the minimum and maximum periods the controller's
  law sets, each with the provision that sets it, coded in the terminology the
  jurisdiction names (an `ACCESS_RETENTION_BOUND`).

A policy whose period breaks a floor or ceiling it declares is refused when it
is written, with `400`, and the version in force stays. So is a ceiling below
a floor, a ceiling with no period (a record kept without end outlasts every
ceiling), a controller outside the Union with no floor, and a period in any
unit other than years or days. Years are counted on the calendar: three years
admit `P1096D` as a floor but not as a ceiling, since 1 096 days can outlast
three calendar years. A new version applies to every record written after it,
on every replica, with no restart.

**With no `log_retention`, every record is kept without end.** That meets
every floor, so the log never forgets evidence a data subject's right of access
is served from. It is also undecided: GDPR Art. 5(1)(e) makes the period the
controller's to set. The `log_retention` readiness indicator reads `degraded`
until the access policy states it, and the instance stays in rotation. `ferroehr
config check` reads only the configuration file, so it cannot see the policy;
check the indicator on `GET /management/health` after the first policy write.

A stored policy version whose `log_retention` does not read (a damaged store;
the write check refuses one) is named and held loud. Each replica keeps the
last retention that read and records every access under it, so the access log
never stops, while the reaper purges nothing and the `log_retention`
indicator reads `down`, naming the version, until a version that reads is
committed.

Every record stores the first date it may be deleted on, and the reaper
deletes each record hourly on or after that date and never before it. A
record outside the access log ends the day after its date of access plus the
period in force when it was written.

**An openEHR access-log entry states its own retention.** Each
`ACCESS_LOG_ENTRY` carries an `ACCESS_RETENTION` when it is written:
`retain_until`, the last day it is kept; `grounds`, the provisions that set
the period; and `policy`, the version of the access policy it was taken from.
An entry kept without end states no `retain_until`. The grounds are Art. 9(2)
for a controller in the Union, coded by the regulation's European Legislation
Identifier with the subdivision as the code
(`http://data.europa.eu/eli/reg/2025/327/oj`, `art_9.par_2`), then each
declared floor and ceiling as the controller coded it. The ELI coding is our
own design (Council conclusions 2012/C 325/02 define the identifier). The
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
([Admin APIs](operations-admin-apis.md)).

**A tombstone is kept as long again, then folds into a gap.** A tombstone
holds no EHR, no accessor and no purpose, but its content digest is a hash
of the entry it replaced, and a copy of that entry elsewhere (a forwarded
BALP `AuditEvent`, an export, a backup) can be matched against it. It is
kept only while the records after it need it to verify (GDPR Art. 5(1)(e)):
for one more retention horizon, its entry's own, so a tombstone stays as long
after its removal as its entry was kept before it. After that, the same
hourly run folds each run of consecutive tombstones that are all past their
end into one gap of the chain (`audit.access_log_chain_gap`). The gap keeps
the positions the run held, the link of its last record, and every ground
its tombstones named, and it records when the run was
folded. The record after the run links to the gap's link, so the chain
verifies across it. The gap row is the record of the fold: it is never
changed or removed, and a record that leaves the log any other way is
reported as tampering. A run folds whole. A checkpoint inside the run
that was never published leaves with it, since the checkpoints after it seal
every link below them through the chain. A published checkpoint stays, and
the gap records the link it sealed and the digest of its proof, so a
checkpoint you kept outside the database verifies after the fold as it did
before. A checkpoint at the run's last position verifies against the gap's
link. An entry kept longer pins
the runs on both sides apart, and each folds on its own. An erasure a law
orders is a tombstone like any other and folds the same way. Several
replicas fold on a PostgreSQL advisory lock, so no tombstone is folded twice.
The metrics are `access_log_folded_total` and `access_log_gaps_total`
([Access path](operations-access-path.md)). No configuration key sets the
second horizon. No openEHR specification governs audit retention: our own
design.

**An imported entry ends at its own `retain_until` too.** An entry imported
with an EHR from another system
([Checkpoints and the transfer of a log](#checkpoints-and-the-transfer-of-a-log))
keeps the `retain_until` its recording system stated, and the same hourly
run replaces it by its tombstone once that day has passed, on the entry's
own grounds. Its tombstone is kept as long again, as a local one is. An
imported log usually holds one checkpoint, at its end, and the links of the
other EHRs' records between the moved EHR's own. A gap does not commit to
the link before it, so a record kept before a gap would be tied to its
checkpoint only by its own link signature, and by nothing when the
exporting system signed no link. The fold of an imported log therefore
takes, per exporting system and within each span between two of its
imported checkpoints, only the positions before the first record still
kept: the tombstones past their end and the other EHRs' links alike, one
gap (`audit.access_log_imported_gap`) per run that holds a tombstone. Every
kept imported record stays hash-chained to the checkpoint after it whether
or not its links were signed, and a tombstone after a kept record waits
until that record has folded too. A later import from the same system can
bring another EHR's records at positions a gap already holds: they are
kept, and verify through their own links. A repeated import does not bring
back a tombstone or an expired entry at a position a gap holds. The
metrics count both logs, with `chain` = `local` or `imported`. No openEHR
specification governs this: our own design (#5664).

Choosing the period is a legal question for the controller. NEN 7513 is the
Dutch reference for this log and the Besluit vaststelling bewaartermijn
logging sets its five years; SGB V § 309 Abs. 1 caps the German
telematics-infrastructure log at three years; DSV Art. 4 Abs. 5 keeps a Swiss
log at least one year. A controller bound by one of them declares it as a
floor or ceiling with its provision. Art. 9 applies from 26 March 2029 for
patient summaries, prescriptions and dispensations and from 26 March 2031 for
the other priority categories (Art. 105). FerroEHR holds the Art. 9(2) floor
from this release on, so a log kept today already meets it when Art. 9
applies. One instance serves one access policy, so two controllers needing
different periods run two instances.
The retention in force is the access policy's (`GET {ext}/admin/access_policy`).
A record takes about 1.2 KB on disk, so the period decides how far the log
grows ([Storage sizing](operations.md#audit-retention)).

### The access log at steady state

With entries written at a rate of `r` a day and kept for `P` days, the log
levels off once the first tombstones fold, after `2P`:

- the entries of the last `P` days: `r × P × 3.9 kB` (2.5 kB of row and 1.4
  kB of indexes each, with signed links);
- their tombstones, kept another `P` days: `r × P × 1.1 kB`;
- the gaps: about 0.6 kB each, one for each run an entry kept longer cut
  apart. A checkpoint inside a run leaves with it; gaps and the remaining
  checkpoints are kept without end.

These figures predate the storage of #5703, which keeps the part an
entry's caller shares once and compresses each row in place; the
performance gate's summary states the bytes per entry of the tree it
measures. The access-path record of #5590 measures 2 518 bytes of row and 3 887 bytes
stored per entry, 1 079 bytes per tombstone, and 2.08 entries per request at
20 requests a second, which is 3.59 million entries a day. With the EHDS
floor of three years (`P` = 1 096 days) that is about 14 GB of entries a day,
and the log levels off near 15.3 TB of entries and 4.2 TB of tombstones,
where before it grew by 3.9 GB of tombstones a day for as long as the
deployment ran. The 0.6 kB of a gap was measured on the same schema with
signed links (#5619). `VACUUM` reuses the space a fold frees for later rows
(PostgreSQL 18 docs, Routine Vacuuming, Recovering Disk Space).

### Getting the log out

Three routes, and they answer different questions.

- **The access log over ITS-REST:** the [access-log
  resources](#reading-the-access-log-over-its-rest) serve the openEHR
  access-log entries: one EHR's log to the person it is about and their
  representative, and the full log to a reviewer. This is the person's own
  route (EHDS Art. 9(2)).
- **ITI-81 retrieval:** the [audit search](#retrieving-audit-records-iti-81)
  serves the records back as FHIR `AuditEvent` resources, filtered by patient,
  agent, date and outcome. This is the operator's route and the one an IHE
  Audit Consumer uses: everything Annex II 3.2 records is reachable through
  it, including the purpose and the organisation, which the syslog feed's
  DICOM form cannot carry.
- **The BALP feed:** records fan out to syslog and to a FHIR feed as they are
  written, for a SIEM or a central Audit Record Repository.

## Reading the access log over ITS-REST

The openEHR access-log entries are served under the extensions root,
`/ferroehr/rest/extensions/v1`. No openEHR ITS-REST release defines these
resources: they are part of FerroEHR's access-log proposal to openEHR, and they
follow the ITS-REST 1.1.0 conventions, `offset` and `fetch` paging as the Query
API's and ISO 8601 date-times.

| Method and path | Who | What |
|---|---|---|
| `GET /ehr/{ehr_id}/access_log` | the subject of care, their representative, a reviewer | The entries of one EHR, newest first. Filters: `from`, `to`, `action`, `outcome`, `accessor_kind` (each a term by its coded content, the token `terminology_id\|code`, such as `HL7_AuditEventAction\|R`, `openehr_access\|refused` or `HL7_SecurityRoleType\|datasubject`; a rubric or a bare code is a `400`), `emergency`, `include_self`; `offset`, `fetch` (1 to 1000, default 100). |
| `GET /ehr/{ehr_id}/access_log/{uid}` | as above | One entry. |
| `GET /admin/access-log` | a reviewer | Records across EHRs, entries and tombstones. Filters: `ehr_id`, `party_namespace`, `party_type`, `party_id`, `issuer`, `subject`, `request_id`, `from`, `to`, `outcome`, `emergency`; `offset`, `fetch`. |
| `GET /admin/access-log/chain` | a reviewer | The `uid` and `link` of each record from `from_sequence` to `to_sequence` (at most 10000), without content, and the `gaps` of folded tombstones between, for verification. |
| `GET /admin/access-log/checkpoint` | a reviewer | The newest stored checkpoint, or the one at `sequence`. |

A list answers `offset`, `fetch` and `entries`, and no total count, since a
count over a large log costs a scan. A parameter a resource does not take, or a
value outside its form, is a `400`.

**The caller's authorisation selects the view, and no parameter widens it.**

- The **subject view** goes to the person the EHR is about and to their
  representative, bound through the party binding: a patient-portal token
  reads the EHR of its launch context, and a representative, through a
  `persons` issuer or as the actor of the patient's token, reads the EHR of
  the subject the representation names
  ([Operational policy](operational-policy.md)). It shows who accessed (the
  provider and the party), when, the action and outcome, the items and their
  categories, the purposes and the emergency mark, and leaves out the
  credential, the delegation chain, the binding outcome, the channel, the
  query text and its digest, the security labels, `other_details` and the
  link, and the accessor members a
  [restriction](#restricting-the-subject-view-ehds-art-93) withholds. An entry about a composition held from the person under EHDS Art.
  3(3) is left out until the hold ends, as in the [subject-scoped ITI-81
  read](#while-a-composition-is-held-from-the-person). Any other EHR answers
  `404`, the same as an unknown one, so its existence is not disclosed.
- The **full view**, each record in its ITS-JSON form with its link, goes to
  a reviewer: a caller the admin class admits (`admin_role` under RBAC), the
  same caller `GET /admin/access-log` admits.
- Every other caller is refused with `403`. The log has its own access
  control: a clinical grant such as `patient/composition-*.r` or
  `user/composition-*.r` reaches none of it.

The person's own reads of their log are recorded like every other read and
are left out of their default view; `include_self=true` adds them. Every read
of these resources, refused and failed ones included, writes an access-log
entry whose item names the log it read (`ACCESS_LOG_RECORD` for an entry,
`ACCESS_LOG` for an EHR's whole log), with the filter named by its digest. A
tombstone keeps no EHR, so a removed entry is read through the review.

### Restricting the subject view (EHDS Art. 9(3))

EHDS Art. 9(3) lets a Member State restrict the person's right to see who
accessed their data "in exceptional circumstances". If your law does, declare
the restriction in the controller's
[access policy](operational-policy.md), never in `ferroehr.toml`: each entry
of `log_restrictions` names the accessor kinds it applies to (the `accessor
kind` terms of the log, such as `person` or `external person` of
`openehr_access`, or `datasubject` of `HL7_SecurityRoleType` for the
subject of care), the
accessor members the subject view withholds (`accessor_party`,
`accessor_provider`, `accessor_roles`, `accessor_groups`, in
`openehr_access`), and the provisions that lay it down, coded in your
jurisdiction's terminology. FerroEHR ships no country's restriction.

```json
"log_restrictions": [{
  "_type": "ACCESS_LOG_RESTRICTION",
  "name": { "_type": "DV_TEXT", "value": "External accessor withheld" },
  "archetype_node_id": "id11",
  "accessor_kinds": [{ "_type": "DV_CODED_TEXT", "value": "external person",
    "defining_code": { "_type": "CODE_PHRASE",
      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
      "code_string": "external person" } }],
  "withheld": [{ "_type": "DV_CODED_TEXT", "value": "accessor_party",
    "defining_code": { "_type": "CODE_PHRASE",
      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
      "code_string": "accessor_party" } }],
  "legal_grounds": [{ "_type": "DV_CODED_TEXT", "value": "your national provision",
    "defining_code": { "_type": "CODE_PHRASE",
      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "urn:example:national-law" },
      "code_string": "art-x" } }]
}]
```

- The subject and their representative get the subject view of an entry
  whose accessor is of a named kind without the withheld members. A reviewer
  reads the full entry, and the stored entry never changes.
- A policy write applies on the next request on every replica, with no
  restart.
- A write that changes the restrictions changes what the log means, so it
  writes an access-log entry of its own (operation
  `admin_access_policy_write`), and the policy history keeps every version.
  When the entry cannot be written, the write is answered `503`.

```bash
# Who accessed my record this month? (the patient portal's token)
curl -H "Authorization: Bearer $PATIENT_TOKEN" \
  "https://cdr.example.org/ferroehr/rest/extensions/v1/ehr/$EHR_ID/access_log?from=2026-11-01T00:00:00Z"
```

The same entries are queryable with AQL as the top-level class
`ACCESS_LOG_ENTRY`, under the same rules of who reads which view, and each such
query writes its own entry ([Querying the access log](querying-aql.md#querying-the-access-log)).

## Sinks

Records fan out to independently configured sinks (`[audit]` in
`ferroehr.toml`; see the
[configuration reference](installation/config-audit.md#audit)):

| Sink | Default | What it does |
|---|---|---|
| `[audit.store]` | **always on** | The local **Audit Record Repository**: records persist in the dedicated `audit` PostgreSQL schema (append-only and tamper-evident, strictly outside the EHR content), served back via the ITI-81 search below. Every record is written here before its access is reported. |
| `[audit.syslog]` | off | The classic ATNA feed: DICOM PS3.15 XML as RFC 5424 syslog over RFC 5425 TLS to an external ARR, per IHE ITI-20. |
| `[audit.fhir_feed]` | off | The RESTful-ATNA feed (ITI-20 **ATX: FHIR Feed**): each FHIR `AuditEvent` is `POST`ed to an external FHIR ARR. Delivery is **outbox-driven** from the store: an ARR outage loses nothing, pending records ship on recovery. The outbox holds the records stored while the feed is configured; with the feed off, no record is owed. Records stored before FerroEHR kept an outbox and never delivered ship once a feed runs. |

syslog delivers from the store the same way, from its own outbox, so both
feeds are projections of the stored records and neither holds a record in
memory.

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

The records go into an **intake**: two append-only tables, one for the
access-log entries and one for the other records, that no request waits on.
A request whose records are all access-log entries hands them to a small set
of intake writers. Each writer writes everything waiting in one statement that
commits on its own, with the entries' search tokens, and answers each request
once that commit is durable, so under load one commit serves many requests.
A **chainer** on every replica links the intake into the hash chains
moments later: it takes the head of the access log without waiting, so one
replica chains at a time and the others skip, computes the links, writes the
records with their index rows, the checkpoint and the outbox rows of the
feeds, and commits. The intake refuses every update and delete except the
chainer's move, which writes each row into its chain in the same statement.
A request wakes its replica's chainer, which gathers what the requests of the
next fifth of a second write into one round; every replica also looks at the
intake every two seconds for the rows a stopped replica left, and a graceful
stop chains what the intake holds. The window between the intake and the chain
is therefore about a fifth of a second; the
`audit_sender` readiness indicator reads `DEGRADED` when the oldest intake row
has waited longer than 10 seconds. A record in the intake is durable: a
restart, a crash or a lost replica leaves it for the next chainer.

The reference host sustains about 15 000 chained access-log entries per
second in one chain ([capacity](capacity.md)).

A write's records live in the audit cluster, which production keeps apart
from the clinical and party ones, so they cannot join the write's own
transaction. They are written first, into the intake, **held** under a
reservation id the chainer skips; the id goes into the write's own domain in
the write's transaction, and the write commits. Once the response is built,
the reservation goes to the chainer, whose next round releases the held
records in the same transaction that chains them, so a write costs two
commits. A replica that stops before that round leaves the records held:
two minutes later the reconciler of any running replica
releases those records, exactly as they were written and once, and discards
the held records of a write that never committed. The reservation holds the
id alone, no identity of the caller. While a reservation waits past those two
minutes the `audit_sender` indicator reads `DEGRADED` with their count and
age, and the metrics `audit_reservations_held`,
`audit_reservation_age_seconds` and `audit_reservations_reconciled_total`
show them.

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

## Tamper evidence

An audit trail that anything with the application's database password can
quietly rewrite is a log, not an accountability record. The local store is
therefore tamper-**evident**: every record is linked into a SHA-256 hash chain
maintained inside PostgreSQL, so each record commits to its predecessor and to
its own content. The chain is built by the database itself, not by the server,
which means it covers every writer: the chainer and any statement typed by
hand.

Each record's digest covers every column of the record except the digest
itself (`row_hash`) and the two per-sink forwarding stamps
(`delivered_syslog_at`, `delivered_fhir_feed_at`). A column added by a later
release joins the digest in the same release.

The store keeps a record's facts in columns and renders its FHIR `AuditEvent`
from them when the record is read (ITI-81) or delivered (the FHIR feed). What
the rendering takes from the configuration (`source_id`, `enterprise_site_id`,
`server_host`, `value_if_missing`, the access policy's log retention, the audit category register and
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
  re-walks every link, and checks both ends of the chain, one query per
  chain:

  ```sql
  SELECT * FROM audit.verify_audit_chain();       -- the system chain
  SELECT * FROM audit.verify_access_log_chain();  -- the access log
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
  records written since rather than the size of the trail. It walks each
  chain from its own cursor to the head it started with, one statement per
  step (`audit.verify_audit_chain_step()` and
  `audit.verify_access_log_chain_step()`), each sized to take about half a
  second, and each intact step moves the cursor, so a backlog of any size is
  verified in one run and no statement nears the
  `[database] statement_timeout`. After each step the run pauses at least as
  long as the step took, and longer the more the step's cost per record rose
  above the lowest the server has measured for that chain: a step that cost
  twice as much pauses four times as long, up to 16 times. A catch-up
  therefore takes about half a core of a quiet database and yields to the
  requests under load, where the lag can grow until the load falls. How far
  a cursor is behind is
  the `audit_chain_verification_lag` and `audit_chain_unverified_age_seconds`
  metrics; once the oldest unverified record has waited longer than
  `[audit.store] max_verify_lag` (default four hours), the `audit_chain`
  indicator reads `DEGRADED` and names the chain.
  A full pass
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

The records of the openEHR access log (`audit.access_log_record`) form a
chain of their own: the access log holds only the accesses to EHR data, one
chain from position 1 with no gaps, apart from the `audit_event` records
above, which form the system chain (an authentication, a template, party or
application operation). Each chain has its own head, its own verification
and its own signing; writers of one chain wait on its head in the database,
so every replica writes the same chain. `SELECT * FROM
audit.verify_access_log_chain();` verifies the access log as
`audit.verify_audit_chain()` verifies the system chain, and the scheduled
check runs both. A chain proves order and completeness
against someone who cannot rewrite its whole tail. A **checkpoint** closes
that case: an `ACCESS_LOG_CHECKPOINT` names the position and digest of the
head of the chain and is signed with the deployment's OpenPGP key, in the
form an openEHR `ATTESTATION` carries its proof (RM common, Generic Package
§Attestation). A verifier that holds a checkpoint detects any later rewrite
of the records it covers. No openEHR specification governs log signing; this
is our own design. No law requires a signature on each entry: EHDS Art. 9
and Annex II 3.2 ask for a complete log each person can read, and GDPR
Art. 32(1)(b) for integrity appropriate to the risk. The eHDSI Audit Trail
Profile's per-event signature binds the MyHealth@EU national contact
points, which sign with their own certificate.

- **The key** is the keyring of [version signing](signing/pgp.md):
  `[record.signing]` in `pgp` mode, with its `retired_key_files` still
  verifying what an earlier key signed. With that key the log is sealed; a
  deployment without it seals nothing, and `ferroehr access-log checkpoint`
  answers that the keyring holds no key.
- **A checkpoint a second, and one at rest.** While requests keep writing,
  the chainer seals the last link of a round once the last checkpoint is a
  second old, and when it comes to rest it seals the head of the chain, so
  every chained entry is covered by a signature within a second under load
  and at once when the load stops. A write that settles its own entries
  (an import, an erasure) seals them in the same transaction. A change,
  removal, insertion or reordering of a record below a checkpoint breaks the
  chain under that checkpoint's signature. The links carry no signature of
  their own. Within the second before a checkpoint, someone who can write
  the database but holds no key could change the newest entries unseen; a
  checkpoint per round closed that second at the cost of one signature and
  its wait in every round, about 20 a second at 3000 requests a second.
  There is no switch and no interval to set. `ferroehr access-log checkpoint` and
  `POST /admin/access-log/checkpoint` answer the checkpoint of the head, and
  seal one only for a head appended while the keyring held no key.
- **Use an Ed25519 key.** An RSA signature is far slower than an Ed25519
  one, and every other audit write waits while one append holds the head.
- **Publish the checkpoints outside the database.** Someone who can write
  the database can remove the newest entries together with the checkpoints
  that seal them, and nothing left in the database shows it. A checkpoint
  kept where that person cannot write does. Set
  `[audit.access_log] publish_directory` to a directory on write-once
  storage (an object-lock bucket mounted as a file system, a WORM volume, a
  directory another system collects from): every 30 seconds the server
  writes the newest checkpoint there as its own file,
  `access-log-checkpoint-<sequence>.json`, created once and never
  rewritten. Replicas publish the same files. This is the recommended
  setup; without it the `access_log_checkpoints` health indicator reads
  `DEGRADED`, and it reads `DEGRADED` while a publication fails. To check
  the log against a published checkpoint, run
  `ferroehr access-log verify --checkpoint <file>` with the newest file.
- **Verification.** `ferroehr access-log verify` and
  `POST /admin/access-log/verify` recompute the whole chain (an entry
  replaced by its tombstone verifies through the content digest the
  tombstone keeps, and a gap of folded tombstones as one link whose digest
  is its link), and check the checkpoint's signature and that the link at
  its position, a record's or a gap's, is the one it sealed. A checkpoint
  published outside the database whose link the log no longer holds is
  reported as `the link at <n> is not the link the checkpoint seals`. They
  also verify every log
  imported from another system against each of its checkpoints: the stored
  links, gaps and records from the checkpoint before it, through the same
  verifier the import runs, under the keyring and
  `[audit.access_log] trusted_key_files`, reported under `imported`. Each finding is reported; the command exits 1 when the log does not
  verify, and the route answers `200` with `intact: false`.
- **Moving the log with the EHR.** EHDS Annex II 2.6 and Art 9(2) keep the
  access information available when an EHR moves to another system.
  `ferroehr access-log export --ehr-id <id>` and
  `GET /admin/access-log/export?ehr_id=` write the EHR's records with their
  links, every link and gap from the first of them to the checkpoint they
  verify against, and that checkpoint, sealing one when none covers them. The
  receiving system runs `ferroehr access-log import --input <file>`: it
  verifies the export against its checkpoint and the certificates in
  `[audit.access_log] trusted_key_files`, keeps the records with their
  `system_id`, links, gaps and `retain_until` apart from its own chain, and records
  the import as an `import` entry of its own log, built like every other
  entry and naming the system as its accessor. An import repeated stores
  nothing twice. The span holds the links of the other EHRs' entries between
  the EHR's own, without their content, and never a record outside the
  access log, so an export is never refused for what else the server
  recorded meanwhile.
- **The system chain is sealed too.** The same schedule and key seal the
  head of the system chain (`audit.audit_chain_seal`).
  `ferroehr audit-chain seal` seals one on demand and prints it, and
  `ferroehr audit-chain verify [--seal <file>]` recomputes the system chain,
  checks the seal's signature and that the link at its position is the one
  it fixes, and exits 1 when the chain does not verify.
- **The erasure is recorded once.** `POST /admin/access-log/erase` writes the
  tombstone and the entry that records the erasure in one transaction; the
  erasure is not written a second time as an `audit_event` record.

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
documents, rendered from the store, newest first, with a `self` link and a
`next` link while more pages follow. The page carries the full match `total`
only when the query asks for it with `_total=accurate` or `_total=estimate`
(FHIR R4 Search §3.1.1.5.2): counting every match reads every record the
filter selects, and a page alone reads one record past what it serves. The query is read by `ihe-atna`, the family's audit
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
the record's `search_tokens`, for an entry a column of its own row in
`audit.access_log_record`, which goes with the entry's content when a
tombstone replaces it), so the search finds what the served document
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
| `_total` | `accurate` or `estimate` adds the full match `total`; `none`, or no `_total`, serves the page without it; any other value is a `400` |

A malformed value is a `400` carrying a FHIR `OperationOutcome` with
`invalid`; another FHIR R4 `AuditEvent` parameter, which §3.81.4.1.2.2 does
not ask for, is a `400` with `not-supported`; a parameter the crate does not
know is ignored
(§3.81.4.1.3). Under RBAC the
unscoped retrieval is **admin-only**; a caller holding a
`read_subject_audit` grant, a patient portal acting for the people it
serves, reads it **scoped to one subject**: one
`patient` or `patient.identifier` with one value is required, and the caller
is refused with a `403` that names the parameter when it omits
it. Reading a subject's log is itself recorded as an access naming that
subject and the record count served. An
[emergency access](#emergency-access-ehds-art-115) in the subject's log
carries its mark as the `ehds-emergency-access` entity. The surface answers `404` when the local
store is disabled.

The subject-scoped ITI-81 read stays beside the [access-log
resources](#reading-the-access-log-over-its-rest). It serves the ATNA records
as FHIR `AuditEvent`s to a portal that holds a `read_subject_audit` grant and
names the subject itself; `GET /ehr/{ehr_id}/access_log` serves the openEHR
access-log entries of the same accesses to the person, or their
representative, under the person's own credential.

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
