# The access log

The openEHR access-log entry FerroEHR writes for every access, and how a subject or an
administrator reads it over ITS-REST. The sinks and the integrity checks are on
[Audit trail](audit.md).

<!-- toc -->

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
representation, a data permit, a justification with the clinician's reason,
and the grounds of the access decision below), the outcome (`granted`, `refused` or `failed`) and
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
| `treatment relationship` | the decision read a live care relationship, also when only a controller rule's exception reads it | in `care` the relationship the access rested on, with its `role` and `path`. A stored relationship is an `ACCESS_STORED_CARE_GROUND` under `ACCESS_POLICY` and the uid of its version: its target (`carer`: the professional's `PERSON` or `ROLE`, or the care team or unit), the uid of its version (`relationship`), whether the grace after its end admitted the access (`in_grace`), the carer it covers for (`covers_for`, with the carer's kind as its `type`), its `provider` and the episode of care it is scoped to (`episode`, absent for a relationship that names none). One a verified credential attests is an `ACCESS_ATTESTED_CARE_GROUND` on the `attested` path under the agreement's name and version: the `assertion` (`id`, `issuer`) and the `agreement` (the container uid of its `ROLE`, with the partner's federation issuer); the ground's `reference` names the assertion too. Where several admitted it, `direct` comes before `team` before `self_declared` before `attested`, then the order of the roles in the rule that matched |
| `justification` | a [justification](access-control.md#justified-access-outside-the-care-relationships) of the access policy admitted the caller outside its care relationships | `ACCESS_POLICY` and the uid of its version, the justification by its position from 1 (`reference`, type `justification`), and the clinician's own words in `reason`; an entry naming the objects it delivered names it only when one of them rested on it |
| `emergency` | an emergency justification admitted the access, or lifted a directive, a controller rule or steps 6 to 8 | for the justification itself, as `justification` with its `reason`; for each directive or rule it lifted, the directive, or the `ACCESS_POLICY` version with the rule by its uid, its first identifier or its position from 1, and no `reason`; an entry naming the objects it delivered names only the directives and rules whose data scope covers one of them |

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
policy version that named it is among the requestor's `agent.policy`. In
the DICOM message the same role follows the held roles as a `RoleIDCode` in
that code system. Both forms carry it, and a projection written from the log
reads the ground back, so it carries the role as the live record does.

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
the entry says why. A justified access and an emergency name their
justification with the reason, so the person sees them in their access log
(EHDS Art. 9 and 11(5)). The EHDS record
writes the versions the grounds name after the data permit at
`agent[user].policy`, "the policy or plan that authorized the activity being
recorded" (FHIR R4 `AuditEvent.agent.policy`), read from the entry's grounds
when the record is rendered. Which grounds are named is FerroEHR's own design.

**The entry is the record.** The store keeps an access to EHR data as its
entry alone, in `audit.access_log_record`, sealed in windows (see
[Tamper evidence](audit-integrity.md#tamper-evidence)), and each entry states the date it is
kept until and why. The FHIR `AuditEvent` the ITI-81 search and the ITI-20
FHIR feed serve, the DICOM message the syslog sink sends and the EHDS access
record are projections of the stored entry: each is written from what the
log keeps and from nothing else, and none is stored as a second record.
`audit.audit_event` keeps the records of accesses outside the access log: an
authentication, a template or party operation, an application activity.

The entry is written with every other record of its request in one database
transaction before the response leaves, and an access whose entry cannot be
written is refused with `503` ([how a record is written](audit.md#how-a-record-is-written)).

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
details beside it. They are part of the entry, so its window's seal covers
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
| `id40` | review | `DV_CODED_TEXT` (`HL7_Confidentiality`) | the level, `R` or `V`, of the record a care relationship was written to, when the write is recorded for review |

The subject of care is kept so the BALP `entity:patient` and the
`patient` search parameter name the patient as they did when the access
was recorded, whatever happens to the EHR after. An entry imported from
another system ([Seals, checkpoints and the transfer of a log](audit-integrity.md#seals-checkpoints-and-the-transfer-of-a-log))
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
| the `emergency` authorisation with a `reason` | the `ehds-emergency-access` entity naming the declared purpose that invoked it | the mark becomes an entity |
| an authorisation's `reason` | the `text` of `purposeOfEvent` and of `agent:user` `purposeOfUse` | none |
| `channel.request_id`, `channel.client_address` | the `X-Request-Id` entity; the client agent's `network.address` | none |
| `system_id` | `source.observer.identifier`; `source.site` from `[audit] enterprise_site_id` | none |
| `security_labels` | the `securityLabel` of the entity they label | no `meta.security` is written |
| `retention` | details of `ehds-categories` | no element; details |
| checkpoints | nothing | the seals do not travel; verify them on the openEHR side |
| `other_details` | the values it holds, where the rows above put them | the tree itself is not written |

**To the DICOM PS3.15 A.5 audit message (the syslog sink).**

| Entry | DICOM element | Loss |
|---|---|---|
| `action` | `EventActionCode`; `EventID` Patient Record (110110), Query (110112), Export (110106), Import (110107) | as in R4 |
| `operation` | `EventTypeCode` in `openEHR-ITS-REST` | none |
| `time_recorded` | `EventDateTime` | `occurred` |
| `outcome` | `EventOutcomeIndicator`, `EventOutcomeDescription` | the reason's code |
| `accessor.credential` | the requestor's `UserID` (the subject, or the professional a client acts for) | the issuer, the token id, the assurance |
| `accessor.roles` | `RoleIDCode` in the `codeSystemName` of `[audit] role_code_system`, by default the deployment's `[api] system_id`; the role of the care relationship the access rested on in its HL7 code system | none |
| `accessor.party`, `binding`, `groups`, `delegation` | details of the `ehds-accessor` participant object | no element; details |
| `accessor.provider` | nothing | lost |
| `system_id` | `AuditSourceID`, with `AuditEnterpriseSiteID` from the deployment | none |
| `channel.client_address` | `NetworkAccessPointID`, type `2` | none |
| `ehr_id` | the patient participant (type `1`, role `1`) names the subject of care | the `ehr_id` itself is not written |
| `items` | a System Object (type `2`, ID type URI) naming the version | the categories, origins, template and archetype |
| `query` | the query object of the Query event | the text digest |
| `purposes`, `authorisations` | nothing in the A.5.1 schema | lost |
| `retention`, `security_labels`, `other_details` | nothing | lost |

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
| `retention`, `labels`, `emergency` | `retention`, the categories and origins, the `emergency` authorisation with a `reason` | none |

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
| `GET /admin/access-log/seals` | a reviewer | The seal of each window that covers an entry id from `from_id` to `to_id` (less than 10000 apart), with whether the window was folded, for verification. |
| `GET /admin/access-log/checkpoint` | a reviewer | The newest stored checkpoint, or the one at `sequence`. |

A list answers `offset`, `fetch` and `entries`, and no total count, since a
count over a large log costs a scan. A parameter a resource does not take, or a
value outside its form, is a `400`.

**The caller's authorisation selects the view, and no parameter widens it.**

- The **subject view** goes to the person the EHR is about and to their
  representative, bound through the party binding: the person reads the EHRs
  whose subject is their `PERSON`, and a representative reads the EHR of the
  subject the representation names
  ([Operational policy](operational-policy.md)). It shows who accessed (the
  provider and the party), when, the action and outcome, the items and their
  categories, the purposes and the emergency mark, and leaves out the
  credential, the delegation chain, the binding outcome, the channel, the
  query text and its digest, the security labels and `other_details`, and the
  accessor members a
  [restriction](#restricting-the-subject-view-ehds-art-93) withholds. An entry about a composition held from the person under EHDS Art.
  3(3) is left out until the hold ends, as in the [subject-scoped ITI-81
  read](audit.md#while-a-composition-is-held-from-the-person). Any other EHR answers
  `404`, the same as an unknown one, so its existence is not disclosed.
- The **full view**, each record in its ITS-JSON form, goes to
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
# Who accessed my record this month? (the person's own token)
curl -H "Authorization: Bearer $PATIENT_TOKEN" \
  "https://cdr.example.org/ferroehr/rest/extensions/v1/ehr/$EHR_ID/access_log?from=2026-11-01T00:00:00Z"
```

The same entries are queryable with AQL as the top-level class
`ACCESS_LOG_ENTRY`, under the same rules of who reads which view, and each such
query writes its own entry ([Querying the access log](querying-aql.md#querying-the-access-log)).
