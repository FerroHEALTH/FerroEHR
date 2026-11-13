# Operational policy

Policy that changes while the CDR runs is data the CDR holds, never a key in
`ferroehr.toml`: a hospital CDR cannot be stopped to edit a file. Each change
is versioned, carries who made it, is written to the access log, and decides
the next request on every replica without a restart. `ferroehr.toml` keeps
only what the server needs before it can reach its database or trust a
caller, such as the listeners, the issuers, `admin_role`, the deployment
profile and the security floors.

Operational policy takes two forms. Who may do what, which systems call
FerroEHR, what each may write, import or export, the terms of each
federation partner and the privileges a professional holds beyond clinical
access, is openEHR demographic data in the party store: the
[policy parties](#the-policy-parties), managed through the Demographic API,
with every change a version with its committer. Policy that has no home in
the openEHR demographic model is an [admin register](#the-admin-registers).
FerroEHR stores, validates and protects the policy parties, and every
request resolves its caller's grants from them before any check runs. The
checks that decide on a grant are listed under
[What the grants decide](#what-the-grants-decide) and
[What a system client may do](#what-a-system-client-may-do).

No openEHR specification governs operational policy or where it is kept, and
none gives a party an authorization meaning. The classes are the openEHR
Reference Model's (RM Demographic Information Model); the design on this page
is FerroEHR's own, and the points that are its own for the policy parties are
under [Own design](#own-design).

<!-- toc -->

## What stays in the configuration

The configuration file keeps what the server needs before it can reach its
store, or before it can trust anything:

- the listeners, the database, TLS, telemetry, the audit sinks and record
  signing;
- the token issuers with their keys and trust bounds, `[auth] product_scope`,
  and `[authz.rbac] admin_role` with `role_issuers`: the authority that
  writes policy parties;
- the party mapping of the deployment's own people (`[authz.party_binding]`),
  which is digest-locked. Each section defaults to FerroEHR's default
  demographic definitions, which the server stores at boot as it stores the
  policy definitions, so a deployment that uses them sets no mapping key and
  one with templates of its own sets the sections that name them; the
  policy parties take no key;
- the provider the instance acts for (`[authz.party_binding] provider`);
- the security floors: the assurance minimum, `ehr_access_default` and the
  cohort thresholds;
- break-glass: `[authz.rbac] emergency_access_role` and
  `[audit] emergency_purpose_codes`, which must work as configured when the
  party store does not answer;
- the Basic users as credentials alone, `user` with its password hash, since
  a secret never becomes versioned openEHR data, and `[authz.rbac]
  first_admin`, the one Basic user that holds `admin_role` and the product
  scope from the file: the first administrator, who writes the first policy
  parties;
- `[auth.smart] patient_issuers`, the issuers whose `ehrId` claim the server
  believes, which is issuer trust;
- the grant ceiling, `[authz.grants] allowed`: the operations the party store
  may grant at all, so a compromised administrator account cannot grant
  more than the operator allows ([The grant ceiling](#the-grant-ceiling)).

Everything else is CDR data: who may do what is party data (below), and the
policy with no demographic home is an admin register. The privileges beyond
clinical access have no configuration key: reading one subject's access log,
acting as an EHDS Art. 10 access service, importing, and reading protected
national identifiers are each a grant of its own operation
([What the grants decide](#what-the-grants-decide)).

## The policy parties

### The model

**The actor.** A system is an `AGENT` ("any kind of agent, including
devices, software systems", RM `AGENT`), an organisation is an
`ORGANISATION` and a person is a `PERSON`. Each is identified by a
`DV_IDENTIFIER` in `PARTY.details` (RM §Party Identification: identifiers
given by organisations "are treated as any other attribute of a Party").
A system's `AGENT` carries its OAuth 2.0 `client_id` under the token
issuer: the identifier's `issuer` is the issuer URL, its `id` the
`client_id` and its `type` `client_id`.

**What it may do.** The actor performs a `ROLE` (RM `ROLE.performer`), and
the role's `CAPABILITY` holds the grant in its `credentials`. The openEHR RM
and CKM use a capability for a performer's qualifications ("Capability
should be backed up by credentials", RM `CAPABILITY`), and the professional
`ROLE` of the default set records a clinician's registrations that way,
from the CKM `individual_credentials` archetype. Reading a capability as an
authorization grant is FerroEHR's own design, and the capability of each
policy `ROLE` is FerroEHR's own archetype. The role's
`time_validity` bounds the engagement and the capability's `time_validity`
bounds the grant. Both are `DV_INTERVAL<DV_DATE>`, so a bound finer than a
day goes in the credentials, as the `Not before` and `Not after` elements.

**On whose behalf.** A system holds an acts-for `PARTY_RELATIONSHIP` from its
`AGENT` to the provider's `ORGANISATION`. The RM stores a relationship in
the party at its source (RM §Party Relationships), so the acts-for
relationship is part of the `AGENT`'s version.

**Management.** The policy parties are written through the standard
ITS-REST 1.1.0 Demographic API, which marks itself `DEVELOPMENT`, and
FerroEHR adds no endpoint of its own for them: `POST`, `PUT` and `DELETE` on
`/demographic/agent`, `/demographic/organisation`, `/demographic/person` and
`/demographic/role`, and `POST /demographic/contribution` for a change of
several parties in one commit.
`GET /demographic/versioned_party/{id}/revision_history` is the history of a
grant. A `PARTY_RELATIONSHIP` changes through an update of its source party,
since the RM stores it there.

**Audit.** Every change carries its committer in
`AUDIT_DETAILS.committer` and is an access record of the demographic API, so
who granted what, and when, is recorded twice.

**Revocation.** A grant ends with a new version of the `ROLE` whose
`time_validity` ends before today, or with its deletion. An `AGENT` that committed
versions is set inactive, never deleted, because it is their committer.

### Who writes and reads a policy party

The rule keys on what a party holds, in any archetype or template you model
it with, never on its archetype id. A party is a policy party when a version
of it holds:

- a `CAPABILITY` whose `credentials` hold, anywhere in their tree, a
  `DV_CODED_TEXT` of FerroEHR's operation terminology, terminology id
  `ferroehr-operations`, such as `ferroehr-operations::import_ehr`;
- a `PARTY_RELATIONSHIP`, held inline or written on FerroEHR's own
  relationship routes, whose `name` or `details` hold the code
  `ferroehr-operations::acts_for`;
- a `DV_IDENTIFIER` of type `client_id`, which binds an actor to an OAuth 2.0
  client.

A deployment that creates an `AGENT` "Legacy migration" with a `ROLE` of its
own archetype, whose capability's credentials hold `import_ehr`, has written
a policy party; the same `ROLE` with a code of another terminology is an
ordinary one.

Only a caller holding `[authz.rbac] admin_role` creates, updates or deletes
one, through its own route or in a `POST /demographic/contribution`. Any
other caller is answered `403`, and the answer names what makes it one, for
example `the ROLE … is a policy party, as it holds a capability granting
ferroehr-operations::import_ehr: only an administrator writes it`. A
SCIM client never writes one, even for a `PERSON` it provisioned, and
neither does a holder of an `access_demographic` grant, so no client grants
itself a capability. The rule judges the version before the write as well
as the new one, so a non-administrator can neither change nor delete a
policy party, nor turn one into an ordinary party. Once a version of a party
is a policy party, the party is marked, in the commit's own transaction, and
the mark is never cleared: every later write of it is an administrator's.

Only an administrator reads one too: `GET` of the party, of
`/demographic/versioned_party/{id}` and its `revision_history` and
`version` reads, and of its tags answer `403` to any other caller when
the party carries the mark. A party
an administrator turns into an ordinary one stays an administrator's to
read, every version of it. Every other party stays readable as before.

AQL over the demographic store applies the same rule. For a caller without
`admin_role`, a query rooted at any demographic class (`PERSON`,
`ORGANISATION`, `GROUP`, `AGENT`, `ROLE`, the abstract `PARTY` and `ACTOR`,
or `PARTY_RELATIONSHIP`), alone or under `VERSION`, leaves out every version
of a party or relationship that carries the mark. The query answers as if
those parties were not stored, and it is not refused:

- The rule is a condition of the one SQL statement the query runs, so
  `COUNT` and the other aggregates, `LIMIT`, `OFFSET` and the REST `fetch`
  and `offset` count only the parties the caller may read. A page is never
  short because a policy party was taken out of it, and no row of a policy
  party leaves the database.
- A node the query reaches through `CONTAINS`, such as a `ROLE`'s
  `CAPABILITY`, or through a path belongs to its party and is left out with
  it. The party half of a query that joins a party to its `EHR` is filtered
  the same way.
- The commit that stores a policy version marks the party, in the same
  transaction, and nothing clears the mark. The rule reads that mark from
  the party's head row, which the query reaches by its key anyway, so it
  adds no statement and no transaction. The Demographic API read checks the
  same mark.
- An administrator's query returns the policy parties.

So `FROM AGENT` and `FROM PARTY` never return a system's `AGENT` to such a
caller, and `FROM PARTY_RELATIONSHIP` never returns an acts-for relationship
written as a `PARTY_RELATIONSHIP` of its own.

Every write and every read is an access record of the Demographic API, with
its committer, as any demographic access is.

### How a request reads its grants

FerroEHR reads a grant from coded content, never from an archetype or a
template id. You model your parties from any archetypes, and a grant holds
when the data carries the codes below. The terminology, its codes and the
client identifier type are the stable contract.

**The operation terminology.** Its `TERMINOLOGY_ID` is
`ferroehr-operations`. A `CAPABILITY` grants an operation when its
`credentials` hold, at any depth, an element whose value is a
`DV_CODED_TEXT` with that terminology and the operation's code:

| Code | Operation |
|---|---|
| `commit_composition` | Commit a `COMPOSITION` on the caller's own account, within the templates and facilities the grant names |
| `import_ehr` | Import an EHR |
| `import_ehr_extract` | Import an EHR extract |
| `export_ehr_extract` | Export an EHR extract |
| `federated_person_access` | Admit a federation partner's persons under a data-sharing agreement |
| `federated_organisation_access` | Admit a federation partner organisation itself under a data-sharing agreement |
| `read_subject_audit` | Read the access log of one subject |
| `access_service` | Act as an EHDS Article 10 access service, held to a subject's opt-out |
| `read_identifiers` | Read a party's protected national identifiers |
| `data_origin` | Be an origin of data: the grant's `origin` is the id the party's data carry |
| `read_change_feed` | Read the commit-event outbox as the reader the grant's `reader` names |
| `scim_provision` | Provision Users and Groups over SCIM, owning the binding keys of the grant's `binding_issuer` |
| `hold_roles` | Hold, as a Basic user, the roles its `role` elements name |
| `hold_scopes` | Hold, as a Basic user, the SMART scopes its `scope` elements name |
| `create_at_login` | Create a provisional `PERSON` at the first login of a person no party carries, under the admission conditions the grant names ([Just-in-time admission](#just-in-time-admission)) |
| `query_system_wide` | Count the client's `system/aql-*` scope, which reaches every query over every EHR (SMART on openEHR, master08 §Resource Scopes) |
| `access_demographic` | Reach the DEMOGRAPHIC API, which no SMART on openEHR scope covers |
| `sign_in_persons` | Be a client the persons of its token issuer sign in through: a person's token through it is the person's own, or their representative's |
| `act_for_persons` | Act for the persons of its token issuer, as a third-party client or as the RFC 8693 actor a person's token names in `act` |
| `access_without_scope` | Be admitted, as a system caller whose token carries no resource scope, to every SMART on openEHR requirement |
| `write_care_relationship` | Write the `member_of` and `under_care_of` care relationships through the Demographic API, as a hospital's ADT system does ([The access control terminology](#the-access-control-terminology)) |
| `write_access_policy` | Write the controller's `ACCESS_POLICY` |

The same terminology names the elements that qualify a grant, by the code
in the element's `name`:

| Code | Element |
|---|---|
| `not_before` | A `DV_DATE_TIME`: the instant before which the grant does not hold |
| `not_after` | A `DV_DATE_TIME`: the instant after which the grant does not hold |
| `template` | A template the grant writes |
| `facility` | A facility the grant covers: an element whose value is a `DV_IDENTIFIER` (its `issuer` and `id`), or a `CLUSTER` of `issuer` with `identifier` or `identifier_prefix` |
| `issuer` | In a `facility` `CLUSTER`: the issuer of the facility identifier |
| `identifier` | In a `facility` `CLUSTER`: the facility identifier |
| `identifier_prefix` | In a `facility` `CLUSTER`: a prefix every facility identifier it covers starts with |
| `origin` | `relay` or `device`, as a `DV_CODED_TEXT` of this terminology or as a `DV_TEXT` |
| `relay` | As an `origin`: the grant commits relayed records of a person's act, such as HL7 v2 |
| `device` | As an `origin`: the grant commits a device's registrations, which no person made |
| `device_identifier_issuer` | The issuer of a device identifier a `device` grant may name as its composer |
| `subject_namespace` | The `EHR_STATUS.subject.external_ref.namespace` of the EHRs it touches, as a `DV_TEXT`; or the `DV_CODED_TEXT` `no_subject_namespace` of this terminology for the EHRs whose subject has no `external_ref` |
| `no_subject_namespace` | As a `subject_namespace` value: the EHRs whose `EHR_STATUS.subject` carries no `external_ref` namespace, such as an anonymous record's `PARTY_SELF`, and no other EHR |
| `creating_system` | On an import grant: a `creating_system_id` whose wrapped committers the import may carry |
| `origin` | On `data_origin`: the origin id, exactly one |
| `reader` | On `read_change_feed`: the reader's name, exactly one, matching `^[a-z][a-z0-9-]{0,62}$` |
| `binding_issuer` | On `scim_provision`: the `identifier_issuer` of the binding keys the client owns, exactly one |
| `token_scope` | On `scim_provision`: a scope the client's access token must hold, at least one |
| `acts_for` | On a `PARTY_RELATIONSHIP`: the source acts for the target |
| `represents` | On a `PARTY_RELATIONSHIP`: the source represents the target, the subject, on the subject's own record ([Representatives in the patient portal](#representatives-in-the-patient-portal)); it makes no policy party |
| `role` | A `DV_TEXT`: a role a `hold_roles` grant gives |
| `scope` | A `DV_TEXT`: a SMART scope a `hold_scopes` grant gives |
| `login_issuer` | A text or `DV_URI`: a token issuer whose first logins a `create_at_login` grant admits |
| `admit_idp` | A text: an identity provider an admitted login comes from |
| `admit_group` | A text: a group an admitted login holds |
| `admit_role` | A text: a role an admitted login holds |
| `admit_assurance` | A text, `low`, `substantial` or `high`: the lowest assurance level an admitted login carries |
| `admit_guests` | A `DV_BOOLEAN`: whether a guest is admitted too |
| `sensitivity_clearance` | A `DV_CODED_TEXT` of the jurisdiction's sensitivity terminology: a sensitivity the capability clears |
| `permitted_purpose` | A `DV_CODED_TEXT` of the jurisdiction's purpose-of-use terminology: a purpose the capability lets its holder declare |
| `member_of` | On a `PARTY_RELATIONSHIP`: the source, a professional's `PERSON`, is a member of the target, a care-team `GROUP` |
| `under_care_of` | On a `PARTY_RELATIONSHIP`: the source, a patient's `PERSON`, is under the care of the target, a team `GROUP` or an organisational unit |

The terms of a federation agreement are named by codes of the same
terminology ([Federation agreements](#federation-agreements)), and the value of
an element any of these codes names is that element's value, never an
operation. A code of another terminology grants nothing, and a code of
`ferroehr-operations` that no table defines is refused at commit.
A capability that codes several operations grants each of them. A scope
element holds a `DV_TEXT`, a `DV_CODED_TEXT` or a `DV_IDENTIFIER`, whose `id`
is read. A `ROLE` whose `data_origin`, `read_change_feed` or `scim_provision`
capability names its element other than as the table says is refused with
`400` at its commit.

**Who acts.** A client is any `ACTOR` (`AGENT`, `PERSON`, `ORGANISATION` or
`GROUP`) that carries, anywhere in its `details` or `identities`, a
`DV_IDENTIFIER` whose `type` is `client_id`, whose `issuer` is the token
issuer and whose `id` is the OAuth 2.0 `client_id`. One party names a client:
a second party carrying the same issuer and `client_id` is refused with
`409`.

**Whose grants.** Every authenticated request resolves its caller's grants
once: a system client, a token of a `machines` issuer, and a verified
external caller of a `federation` issuer, by its issuer and `client_id`; a
person the login binding bound to a `PERSON`, by that party;
anyone else to no grants. Its grants are the operations the capabilities of
the `ROLE`s whose `performer` names that party grant, when:

- the client and the `ROLE` are `complete` or `incomplete`; a client set
  inactive holds no grant;
- the `ROLE`'s `time_validity` and the capability's `time_validity` both hold
  the request's date, read in UTC. An absent `time_validity` holds every
  date;
- the request's instant is not before `not_before` and not after
  `not_after`, when the credentials carry them. A `DV_DATE_TIME` without an
  offset is read in UTC, and one that does not read as a date-time makes the
  capability hold on no date;
- `[authz.grants] allowed` admits the operation.

A change takes effect on the next request with no restart. Each replica keeps
the grants in memory and reads them again when a commit changes one: the
commit notifies every replica through PostgreSQL, the request that made the
change reads them before it answers, and a replica whose notification
listener is down reads them on every request. When the grants must be read
and the database cannot answer, the request is answered `503`.

`ferroehr party-binding refill` rebuilds the grants from the party versions,
with the rest of the binding side table.

### The access control terminology

The access directives of an EHR, the controller's access policy and the
caller facts they are read against are coded in `ferroehr-operations` too.
The groups below are the stable contract: the access evaluator and the
access write checks read these codes and no archetype id, so you model
consent, restriction and care relationships with your own archetypes and
bind their coded attributes to these groups. A value of another terminology,
or a code of `ferroehr-operations` outside the group its attribute is bound
to, is refused. Each code is the group's rubric until codes are assigned.
openEHR defines no access control terminology (RM EHR IM,
`ACCESS_CONTROL_SETTINGS`: "Currently implementation dependent"), so every
group is FerroEHR's own design.

| Group | Codes | Read from |
|---|---|---|
| `access default` | `open`, `restricted`, `consent_required` | `ACCESS_DIRECTIVE_SETTINGS.default_access`: the disposition of an EHR where no directive applies |
| `access directive kind` | `consent`, `restriction`, `opt_out`, `delegation` | `ACCESS_DIRECTIVE.kind`. A `restriction` (EHDS Art. 8) is always a deny; an `opt_out` (EHDS Art. 10) is a deny of the whole record |
| `access effect` | `permit`, `deny` | `ACCESS_DIRECTIVE.effect`; a deny overrides a permit |
| `access relationship` | `care`, `representative`, `proxy` | `ACCESS_ACTOR_SCOPE.relationship`: the caller's relationship to the subject |
| `access caller kind` | `professional`, `access_service`, `subject`, `representative`, `system` | `ACCESS_ACTOR_SCOPE.caller_kind`. An `access_service` caller is one holding the `access_service` grant |
| `access operation` | `read`, `write`, `export`, `query` | `ACCESS_DIRECTIVE.operations`: the operation classes a directive applies to. No capability grants one |
| `access sensitivity clearance` | `sensitivity_clearance` | The `name` of a credentials element whose value is a sensitivity code |
| `access permitted purpose` | `permitted_purpose` | The `name` of a credentials element whose value is a purpose-of-use code |
| `care relationship` | `member_of`, `under_care_of` | The `name` of a `PARTY_RELATIONSHIP` |
| `access refusal` | `refused-opt-out`, `refused-restricted`, `refused-consent-denied`, `refused-sensitivity`, `refused-no-care-relationship`, `refused-purpose-missing`, `refused-purpose-not-permitted`, `refused-consent-required` | The access record of a refusal |

Two grants of the operation table belong to access control:
`write_care_relationship` and `write_access_policy`.

**Codes FerroEHR does not own.** Sensitivity, data category, purpose of use
and legal ground are coded in the terminology your jurisdiction names, such
as HL7 v3 `Confidentiality` and `ActCode`, the EHDS categories and HL7
`PurposeOfUse`. FerroEHR compares them by terminology id and code, and refuses
one coded in `ferroehr-operations`, which holds none.

**Refusals.** `refused-opt-out` and `refused-restricted` answer `404`, as if
the data did not exist, and appear only in the access record, so that a
restriction or an opt-out is not visible to the caller (EHDS Art. 8). Every
other refusal answers `403` and names its code.

### What the grants decide

Each check below reads the grants of the request's caller, beside what the
configuration grants. A change an administrator commits decides the next
request on every replica with no restart, and each operation must also be in
`[authz.grants] allowed`.

**The committers an import may carry.** An `IMPORTED_VERSION` keeps the
original's `commit_audit`, so an import could make a version read as a local
colleague's act. A wrapped committer naming a party of this CDR, or carrying a
login of its issuers, is refused with `403` unless the version's
`creating_system_id` is one a `creating_system` element of the caller's
`import_ehr` grant (for `POST /ehr` with an extract) or `import_ehr_extract`
grant (for an extract import into an existing EHR) names. This node's own
system id is never trusted that way.

**The origins of data** (EHDS Annex II 3.4). An origin is recognised when a
`data_origin` grant in force names it in its `origin` element: a retention
period may be keyed on it (`PUT {ext}/admin/retention/policy`), and an ABAC
rule may name it. A body with no `FEEDER_AUDIT`, committed by the holder of
such a grant, is stamped with that origin in `version.origins`; any other
direct commit keeps this server's `system_id`. A system that only sends data
through a feed, and never commits itself, holds the grant on an `AGENT` of
its own with no client identifier. A committer whose grants name two origins
commits no body without a `FEEDER_AUDIT` (`403`), since that body would come
from neither.

**The readers of the change feed.** A reader of the commit-event outbox is
named by the `reader` element of a `read_change_feed` grant in force. While
one is, every commit writes its outbox row; with none, no commit does. The
reader registry is reconciled with the grants at boot, on every feed call and
before every prune, so a reader no grant names any more is inactive and holds
the prune back no longer. `GET {ext}/admin/outbox/{reader}` and its
acknowledgement answer `404` for a reader no grant names, and `403` to a caller
whose own `read_change_feed` grant does not name it.

**The SCIM provisioning clients.** A request to `/scim/v2/` is admitted for a
client-credentials token whose client's `ACTOR` holds a `scim_provision` grant
in force. Its `binding_issuer` is the `identifier_issuer` of an
`[[auth.party]]` table that no other client's grant names, and the token must
hold one of its `token_scope` values (`403` with `insufficient_scope`
otherwise). The client owns the Users whose binding key that issuer issues.
The contacts of a deleted User are erased for every client that ever held the
grant, so ending a grant stops no erasure.

**A Basic user's roles and scopes.** A Basic user is a credential alone.
Bound to a `PERSON`, it holds the roles the `role` elements of its
`hold_roles` grants name and the scopes the `scope` elements of its
`hold_scopes` grants name; a privileged role is never held this way. The one
exception is `[authz.rbac] first_admin`, which holds `admin_role` and the
product scope from the configuration. Every gate reads them: the role gate,
the SMART on openEHR scopes, ABAC and the access record. A token's roles and
scopes come from the token alone, since its issuer is their authority.

**Which clients a person's token comes through.** While `ferrohealth-smart`
classifies a token, it asks the grants of the `ACTOR` carrying the client
identifier under the token's issuer: a person's token through a client
granted `sign_in_persons` is the person's, one through a client granted
`act_for_persons` is that client acting for the person, and one through any
other client is refused with `401`. An RFC 8693 actor a person's token names
is a delegate when the `ACTOR` carrying its `sub` under its issuer is granted
`act_for_persons`, and a system caller whose token carries no resource scope
meets every SMART on openEHR requirement when its `ACTOR` is granted
`access_without_scope`. The answer comes from the copy each replica holds, so
it adds no transaction, and a party store that cannot answer refuses the
token with `503`.

**Importing.** `message_import_ehr` admits a caller with an `import_ehr`
grant whose `subject_namespace` names the EHR's subject namespace;
`message_import_ehr_extract` the same with an `import_ehr_extract` grant
([What a system client may do](#what-a-system-client-may-do)).

**Committing and exporting as a system client.** A system client commits
under its `commit_composition` grants and exports under its
`export_ehr_extract` grants
([What a system client may do](#what-a-system-client-may-do)).

**Reading protected identifiers.** `GET /admin/party/identifiers` admits an
administrator who holds a `read_identifiers` grant. The administrator role
alone reveals no national identifier.

**Reading one subject's access log.** `GET /fhir/r4/AuditEvent` admits a
caller outside the administrator role that holds a `read_subject_audit`
grant, for one subject at a time: the `patient` parameter names exactly one
subject, and without it the call is `403`
([Audit trail](audit.md#retrieving-audit-records-iti-81)).

**Acting as an access service.** A caller with an `access_service` grant is
an EHDS Art. 10 access service: an EHR whose person opted out reads as
absent to it at every route, query and export, unless it declares one of
`[audit] emergency_purpose_codes` and acts for no person (Art. 10(2)).
While `[authz.grants] allowed` does not allow `access_service`, recording an
opt-out is refused with `400`, since it would hold for nobody
([Opt-out from the access services](security.md#opt-out-from-the-access-services-ehds-art-10)).

A role a token names never grants any of these four: each comes from its
grant alone, under the grant ceiling. The privileged roles of
`[authz.rbac]`, `admin_role` and `emergency_access_role`, come from the
configuration and the token issuers `role_issuers` lists, and never from a
`hold_roles` grant: a `ROLE` whose `hold_roles` grant names one is refused at
commit with `403`, and one stored before the role was configured is never
honoured.

For example, a ward clerk who signs in with Basic holds the clerk role
through this capability of a `ROLE` the clerk's `PERSON` performs:

```json
"capabilities": [{
  "_type": "CAPABILITY", "archetype_node_id": "at0020",
  "name": { "_type": "DV_TEXT", "value": "Ward clerk" },
  "credentials": {
    "_type": "ITEM_TREE", "archetype_node_id": "at0021", "name": { "_type": "DV_TEXT", "value": "Grant" },
    "items": [
      { "_type": "ELEMENT", "archetype_node_id": "at0022", "name": { "_type": "DV_TEXT", "value": "Operation" },
        "value": { "_type": "DV_CODED_TEXT", "value": "Hold roles",
          "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "hold_roles" } } },
      { "_type": "ELEMENT", "archetype_node_id": "at0023",
        "name": { "_type": "DV_CODED_TEXT", "value": "Role",
          "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "role" } },
        "value": { "_type": "DV_TEXT", "value": "WARD_CLERK" } }
    ]
  }
}]
```

A `hold_scopes` capability is built the same way, with one `scope` element
per SMART scope. Both operations are allowed by name in
`[authz.grants] allowed`, as every other operation.

A SCIM provisioning client's capability, on a `ROLE` of your own archetype
with no template (the element names carry the codes):

```json
{ "_type": "CAPABILITY", "archetype_node_id": "at0020",
  "name": { "_type": "DV_TEXT", "value": "Provisioning" },
  "credentials": { "_type": "ITEM_TREE", "archetype_node_id": "at0021",
    "name": { "_type": "DV_TEXT", "value": "Terms" },
    "items": [
      { "_type": "ELEMENT", "archetype_node_id": "at0022",
        "name": { "_type": "DV_TEXT", "value": "Operation" },
        "value": { "_type": "DV_CODED_TEXT", "value": "scim_provision",
          "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" },
            "code_string": "scim_provision" } } },
      { "_type": "ELEMENT", "archetype_node_id": "at0023",
        "name": { "_type": "DV_CODED_TEXT", "value": "binding_issuer",
          "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" },
            "code_string": "binding_issuer" } },
        "value": { "_type": "DV_TEXT", "value": "urn:example:login" } },
      { "_type": "ELEMENT", "archetype_node_id": "at0024",
        "name": { "_type": "DV_CODED_TEXT", "value": "token_scope",
          "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" },
            "code_string": "token_scope" } },
        "value": { "_type": "DV_TEXT", "value": "ferroehr/scim" } }
    ] } }
```

A change-feed reader's capability codes `read_change_feed` with a `reader`
element (`ferrobridge`), an origin's codes `data_origin` with an `origin`
element (`lab.example`), and an importer's `import_ehr` grant adds a
`creating_system` element (`legacy.example`) for each system it vouches for.

### The first grant

An administrator is a person bound to a `PERSON`, and the `PERSON`s of a new
deployment usually arrive through a SCIM client, which itself needs a
`scim_provision` grant. `ferroehr party-binding grant` breaks that circle on
the operator's authority: it commits, through the same path a Demographic
API write takes, an `AGENT` carrying the client identifier (unless an `ACTOR`
names the client already) and a `ROLE` it performs whose capability grants
the operation with the elements given. The commits carry the server's own
committer, the ceiling and the element rules apply, and a client that holds
the operation already gets nothing, so it runs again safely:

```console
ferroehr party-binding grant --issuer https://login.example.org/machines \
  --client-id entra-provisioning --operation scim_provision \
  --element binding_issuer=urn:example:login --element token_scope=ferroehr/scim
```

With `--basic-user <user>` in place of `--issuer` and `--client-id`, the
`ROLE` is performed by the `PERSON` the Basic user binds to, which must exist
already, for example the scopes a development lane's users hold:

```console
ferroehr party-binding grant --basic-user clinician --operation hold_scopes \
  --element 'scope=user/composition-*.cruds' --element 'scope=user/aql-*.cruds'
```

Every later grant is an administrator's write through the Demographic API.
`[authz.rbac] first_admin` is that administrator in a deployment whose people
sign in with HTTP Basic: it holds `admin_role` and the product scope before
any policy party exists.

### The grant ceiling

`[authz.grants] allowed` lists the operations the party store may grant. The
default is the narrowest set that still lets a system client write:
`["commit_composition"]`. That grant reaches only the templates it names,
while an import, an export, a federation agreement and every privilege reach
data beyond one system's own writes, so you admit each by name:

```toml
[authz.grants]
allowed = ["commit_composition", "import_ehr", "export_ehr_extract"]
```

The environment form is `FERROEHR__AUTHZ__GRANTS__ALLOWED`, comma-separated.
An empty list lets the party store grant nothing.

A `ROLE` committed with a capability that codes an operation the ceiling does
not admit is refused with `403`, and the problem names the operation and the
key, for example `the ROLE grants the operation import_ehr, which
[authz.grants] allowed does not admit, so this CDR may not grant it`. A grant
already stored, for example before you narrowed the ceiling, is never
honoured, and the server logs the operations it skips when it reads the
grants.

### Federation agreements

A federation agreement says what a partner's verified external persons, or
the partner organisation itself, may do here and on which legal ground. It is
a grant on the partner: the partner's `ACTOR` (usually an `ORGANISATION`)
carries a `client_id` identifier for each client its `federation` issuer
issues tokens to, and a `ROLE` it performs holds one capability per accessor.
A capability coding `federated_person_access` admits the partner's named
persons; one coding `federated_organisation_access` admits the organisation
itself, with no named person. The agreement holds while the grant does: its
`time_validity` and its `not_before` and `not_after` elements. An external
caller is admitted only under the agreement of its accessor kind that its
partner holds on the request, read from the CDR, so a new, changed or ended
agreement takes effect on the next request.

The agreement's terms are elements of the capability's `credentials`, each
named by its `ferroehr-operations` code:

| Code | Element |
|---|---|
| `agreement_name`, `agreement_version` | `DV_TEXT`: the agreement's name and version |
| `jurisdiction` | `DV_TEXT` or `DV_CODED_TEXT`: the ISO 3166-1 alpha-2 code of the jurisdiction |
| `exchange` | `DV_TEXT`: the kind of exchange |
| `purpose` | `DV_CODED_TEXT` or `DV_TEXT`: what the exchange is for |
| `gdpr_art_6_1`, `gdpr_art_9_2` | `DV_TEXT` or `DV_CODED_TEXT`: the point of GDPR Art. 6(1), `a` to `f`, and of Art. 9(2), `a` to `j` |
| `national_provision` | `DV_TEXT`: the provision of the law that applies |
| `consent` | `registry` (an external consent registry, whose question names the responsible professional) or `local`: the source of the consent a read is conditioned on; a read must then carry its consent decision reference |
| `read_scope` | `DV_TEXT`, one per scope: a `patient/` scope the callers may read |
| `template` | `DV_TEXT`, one per template: a template an external person may commit |
| `discard_window` | `DV_DURATION` of whole hours, such as `PT24H`: an external person may discard a dispensation of its own point of care within it |
| `ehdsi_service` | `patient_summary`, `eprescription` or `orcd`: the MyHealth@EU service a contact point's person agreement serves |
| `requests_per_second`, `burst` | `DV_COUNT`: the admission rate per caller |
| `opt_out_override` | `DV_BOOLEAN`: a professional may read an opted-out EHR under an emergency purpose |
| `access_class` | `automated` or `chained`: how an organisation's access is attributed |
| `requester_log_retention` | `DV_DURATION` of whole years or days, such as `P5Y`: how long a `chained` requester keeps its log |

A coded value is read by its `code_string` in any terminology, a text by its
`value`. A `ROLE` whose agreement breaks one of these rules is refused at its
commit with `422`, and the problem names every rule it breaks:

- the partner carries a `client_id` identifier, and each identifier's issuer
  is an `[[auth.issuer]]` of kind `federation`;
- a partner holds one agreement per accessor: a second capability for the same
  accessor, in this `ROLE` or another, is refused;
- every element holds a value of its kind;
- every agreement sets `requests_per_second` and `burst`, reads only
  `patient/` scopes, and codes its GDPR points within their articles;
- a contact point's person agreement names its `ehdsi_service`, and no other
  agreement does;
- an organisation agreement names its `agreement_name`, `agreement_version`,
  GDPR points and `access_class`, writes no template (EHDS Art. 13(4)), sets
  no `opt_out_override`, takes no consent from a `registry`, and its partner
  is no contact point;
- a `chained` organisation agreement sets a `requester_log_retention` at
  least as long as `[audit.store] retention`, and its issuer's
  `[[auth.federation]]` table maps `correlation_id` from the token, never from
  a constant.

The partner's client identifiers are read when the `ROLE` commits, so commit
the partner first; a contribution lists it before the `ROLE`. An agreement
that no longer keeps the rules, for example after its partner gained an
identifier of a contact point, admits no caller, and the server logs why.
`/management/info` lists every agreement in force under `binding.agreements`,
with its partner's issuer and client, its accessor, name and legal ground, and
the boot line names each one.

### For a check that reads a grant

A check inside a request reads the caller's grants with
`ferroehr::party_binding::grants::capabilities::Capabilities::current()`,
which holds them for the duration of the request:

- `client()` is the `ACTOR` the client identifier names, with its state;
- `grants()`, `of(operation)` and `holds(operation)` are the grants in force,
  each an `Operation` with its `ROLE`;
- `credentials()` of a grant reads its credentials by the code of an
  element's name: `named`, `texts`, `codes`, `count`, `boolean`, and
  `clusters` for a `CLUSTER` such as a facility;
- `covers(operation, namespaces)` is whether one grant of the operation names
  every subject namespace given; it never holds for none.

### What a system client may do

A system client is a token of a `machines` issuer. It resolves to the
`ACTOR` carrying its client identifier, and what it may do is what that
party's grants say. No key of `ferroehr.toml` declares a system client.

**Commit into an EHR.** A system client commits only under a
`commit_composition` grant; otherwise any commit into an EHR is answered
`403`. Its `ACTOR` acts for the provider its acts-for relationship targets,
or, with none, for the deployment's `[authz.party_binding] provider`. A `COMPOSITION` is admitted when one `commit_composition` grant:

- lists its template in a `template` element, or the commit is `403`;
- has the origin that fits the act. A composer that is a `PARTY_IDENTIFIED`
  whose identifier, of an issuer the grant lists as a
  `device_identifier_issuer`, is the `feeder_audit.originating_system_audit.system_id`
  is a device's registration and needs the origin `device`. Any other
  `COMPOSITION` records a person's act, needs the origin `relay`, and names
  the professional by an identifier, in the composer or in a performer
  participation (EHDS Art. 13(4)). The wrong origin is `403`, a person act
  naming nobody `422`;
- covers the facility the `COMPOSITION` names, in
  `EVENT_CONTEXT.health_care_facility` or, with no context, in
  `feeder_audit.originating_system_audit.location`; otherwise `422`. The
  server never writes the facility.

A deletion is held to the templates the same way. A composer that names an
`ORGANISATION` is `422`: the composer is a person.

**Import.** `message_import_ehr` needs an `import_ehr` grant and
`message_import_ehr_extract` an `import_ehr_extract` grant, either of which
names in `subject_namespace` the subject namespace of the EHR: of each
`EHR_STATUS` the extract carries and, for an extract into an existing EHR,
of that EHR. Any other import is `403`, whatever roles the caller holds. The
versions inside an extract keep their original committers only when a
`creating_system` element of the grant names their `creating_system_id`.

**An EHR whose subject names no namespace.** The RM lets
`EHR_STATUS.subject` be a `PARTY_SELF` with no `external_ref`, for an
anonymous record, and a legacy system often holds such EHRs. A grant
reaches them only through a `subject_namespace` element whose value is the
`DV_CODED_TEXT` `no_subject_namespace` of `ferroehr-operations`, for the
import, the extract import and the export alike. That value covers exactly
those EHRs: a grant naming real namespaces does not reach them, and the
code reaches no EHR whose subject names a namespace. A grant that names no
`subject_namespace` at all is still refused at its commit with `422`.

```json
{ "_type": "ELEMENT", "archetype_node_id": "at0023",
  "name": { "_type": "DV_CODED_TEXT", "value": "Subject namespace",
    "defining_code": { "_type": "CODE_PHRASE",
      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" },
      "code_string": "subject_namespace" } },
  "value": { "_type": "DV_CODED_TEXT", "value": "No subject namespace",
    "defining_code": { "_type": "CODE_PHRASE",
      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" },
      "code_string": "no_subject_namespace" } } }
```

**Export.** A system client exports an EHR with `message_export_ehrs` or
`message_export_ehr_extracts` only under an `export_ehr_extract` grant whose
`subject_namespace` names the EHR's subject namespace; any other export by a
system client is `403`. A person's export is decided by the access rules it
already meets.

**Attribution.** Every commit of a system client, an import included,
records as its committer a `PARTY_IDENTIFIED` named by the client, with the
client identifier under the token issuer and an `external_ref` to the client's
`ACTOR` (`PARTY_REF` type `ACTOR`). Every access record of a system client
names the `ACTOR` as its party and the provider by the first `DV_IDENTIFIER`
in the `details` of the `ORGANISATION` its acts-for relationship targets, or
the deployment's provider when it names none (EHDS Annex II 3.2(a)).

**Checked at commit.** A `ROLE` whose grant lacks the terms its operation is
checked by is refused with `422`, naming the capability: a
`commit_composition` grant without an `origin` coding `relay` or `device`, a
`device` grant without a `device_identifier_issuer`, a `facility` `CLUSTER`
without one `issuer` and exactly one of `identifier` and `identifier_prefix`,
and an import or export grant without a `subject_namespace`.

### The example policy archetypes

FerroEHR ships the archetypes below as examples of the model. They carry
the codes: each `ROLE`'s capability holds an `Operation` element whose value
is a `ferroehr-operations` code, the acts-for relationship holds
`ferroehr-operations::acts_for` in its details, and the `AGENT` holds a
`client_id` identifier. A party of one of them is a policy party because of
those codes, as a party of your own archetypes is. No configuration key
names them.

| Kind | Archetype |
|---|---|
| System `AGENT` | `openEHR-DEMOGRAPHIC-AGENT.system.v1` |
| Acts-for relationship | `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1` |
| System client `ROLE` | `openEHR-DEMOGRAPHIC-ROLE.system_client.v1` |
| Federation partner `ROLE` | `openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1` |
| Privileged `ROLE` | `openEHR-DEMOGRAPHIC-ROLE.privileged.v1` |

**Default definitions.** The binary carries a default template of each,
version `1.0.0` (for example `openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0`).
At boot FerroEHR stores each one through the normal definition path only
when no definition of its id is stored. It never overwrites a stored
definition, never refuses one you changed, and never seeds over your change.
After that they are ordinary definitions: upload a new version, a renamed or
a specialised template through the definition API, as for any other. The
[default demographic definitions](installation/default-demographic-definitions.md)
are stored at boot the same way, the healthcare provider `ORGANISATION` the
examples below write among them. Every write of a policy party is validated against its own template. The sources
and every change from them are in
`app/ferroehr/assets/definitions/policy/PROVENANCE.md`.

**The contract.** The `ferroehr-operations` codes and the `client_id`
identifier type make a party a policy party, in any archetype, and FerroEHR
reads the terms of a system client's grant by the codes in its elements'
names, never by a node id. The archetype ids and the node ids below describe
the examples, and every template and version built on an archetype keeps
them. A breaking change to them is a new archetype major version (`.v2`),
which FerroEHR does not take for the `.v1` kind. Each grant is a
`CAPABILITY` at `/capabilities[id6]`, its kind the coded `name`, its
credentials at `/capabilities[id6]/credentials[id8]`, and its `time_validity` the day bounds.

| Archetype | Field | Node path | Type |
|---|---|---|---|
| `AGENT.system` | Client identifier (`issuer` the token issuer, `id` the `client_id`, `type` `client_id`) | `/details[id2]/items[id3]` | `DV_IDENTIFIER` |
| `AGENT.system` | Name | `/identities[id4]/details[id5]/items[id6]` | `DV_TEXT` |
| `PARTY_RELATIONSHIP.acts_for` | Type (its `name`) | `at10` acts for | `DV_CODED_TEXT` |
| `PARTY_RELATIONSHIP.acts_for` | Agreement reference | `/details[id3]/items[id4]` | `DV_TEXT` |
| `PARTY_RELATIONSHIP.acts_for` | Acts-for code, `ferroehr-operations::acts_for` | `/details[id3]/items[id6]` | `DV_CODED_TEXT` |
| every policy `ROLE` | Kind of grant (the capability's `name`) | `/capabilities[id6]/name` | `DV_CODED_TEXT` |
| every policy `ROLE` | Operation, a `ferroehr-operations` code | `/capabilities[id6]/credentials[id8]/items[id90]` | `DV_CODED_TEXT` |
| `ROLE.system_client` | Template written, named `template` | `/capabilities[id6]/credentials[id8]/items[id9]` | `DV_TEXT` |
| `ROLE.system_client` | Facility, named `facility`: issuer, identifier, identifier prefix, named `issuer`, `identifier`, `identifier_prefix` | `/capabilities[id6]/credentials[id8]/items[id10]/items[id11]`, `[id12]`, `[id13]` | `DV_TEXT` |
| `ROLE.system_client` | Device identifier issuer, named `device_identifier_issuer` | `/capabilities[id6]/credentials[id8]/items[id14]` | `DV_TEXT` |
| `ROLE.system_client` | Not before, not after, authorisation reference | `/capabilities[id6]/credentials[id8]/items[id15]`, `[id16]`, `[id17]` | `DV_DATE_TIME`, `DV_DATE_TIME`, `DV_TEXT` |
| `ROLE.system_client` | Origin, named `origin`: `relay` or `device` | `/capabilities[id6]/credentials[id8]/items[id18]` | `DV_CODED_TEXT` |
| `ROLE.system_client` | Subject namespace, named `subject_namespace` | `/capabilities[id6]/credentials[id8]/items[id19]` | `DV_TEXT` |
| `ROLE.federation_partner` | Kind (accessor) | `at10` person, `at11` organisation | |
| `ROLE.federation_partner` | Agreement name, version, jurisdiction, exchange | `/capabilities[id6]/credentials[id8]/items[id9]`, `[id10]`, `[id11]`, `[id12]` | `DV_TEXT` |
| `ROLE.federation_partner` | Purpose | `/capabilities[id6]/credentials[id8]/items[id13]` | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Legal ground: GDPR Art. 6(1), Art. 9(2), the provision of the law that applies | `/capabilities[id6]/credentials[id8]/items[id14]/items[id15]`, `[id16]`, `[id17]` | `DV_TEXT` |
| `ROLE.federation_partner` | Consent source: `registry` or `local` | `/capabilities[id6]/credentials[id8]/items[id18]` | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Read scope, template written | `/capabilities[id6]/credentials[id8]/items[id19]`, `[id20]` | `DV_TEXT` |
| `ROLE.federation_partner` | Discard window | `/capabilities[id6]/credentials[id8]/items[id22]` | `DV_DURATION` |
| `ROLE.federation_partner` | MyHealth@EU service: `patient_summary`, `eprescription` or `orcd` | `/capabilities[id6]/credentials[id8]/items[id23]` | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Caller rate: requests per second, burst | `/capabilities[id6]/credentials[id8]/items[id24]/items[id25]`, `[id26]` | `DV_COUNT` |
| `ROLE.federation_partner` | Opt-out override | `/capabilities[id6]/credentials[id8]/items[id27]` | `DV_BOOLEAN` |
| `ROLE.federation_partner` | Access class: `automated` or `chained` | `/capabilities[id6]/credentials[id8]/items[id28]` | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Requester log retention | `/capabilities[id6]/credentials[id8]/items[id29]` | `DV_DURATION` |
| `ROLE.federation_partner` | Not before, not after, authorisation reference | `/capabilities[id6]/credentials[id8]/items[id30]`, `[id31]`, `[id32]` | `DV_DATE_TIME`, `DV_DATE_TIME`, `DV_TEXT` |
| `ROLE.privileged` | Kind (privilege) | `at10` subject audit, `at11` access service, `at12` identifier reader, `at13` import | |
| `ROLE.privileged` | Not before, not after, authorisation reference | `/capabilities[id6]/credentials[id8]/items[id9]`, `[id10]`, `[id11]` | `DV_DATE_TIME`, `DV_DATE_TIME`, `DV_TEXT` |

### Worked examples

The examples use the base path `/ferroehr/rest/openehr/v1`, send
`Content-Type: application/json` and `Prefer: return=representation`, and
run as an administrator. The party ids are examples: use the ids your
server returns in the `Location` header and the body.

The provider this instance acts for is an `ORGANISATION` named
`9d2f4a7e-3c1b-4e8a-9f60-2b7d5c8e1a40`, and the federation partner below is
the `ORGANISATION` `7c4e9a2b-1f6d-4b3e-9c8a-5d2e0f7b1a36`. The default set's
healthcare provider `ORGANISATION` (`openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0`,
on [Default demographic definitions](installation/default-demographic-definitions.md))
records both, identified by their provider identifier. Neither is a policy
party, so any demographic writer may create them.

#### A system client

A laboratory relay commits laboratory results into EHRs at one facility.
First its `AGENT`, `POST /demographic/agent`:

<!-- policy-example: agent -->
```json
{
  "_type": "AGENT",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.system.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.system.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "System agent" },
  "details": {
    "_type": "ITEM_TREE", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Identifiers" },
    "items": [{
      "_type": "ELEMENT", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Identifier" },
      "value": {
        "_type": "DV_IDENTIFIER",
        "issuer": "https://login.example.org/machines",
        "id": "lab-relay",
        "type": "client_id"
      }
    }]
  },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id5", "name": { "_type": "DV_TEXT", "value": "Items" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Name" },
        "value": { "_type": "DV_TEXT", "value": "Laboratory relay" }
      }]
    }
  }]
}
```

Its grant is a `ROLE` it performs, `POST /demographic/role`, with the
`AGENT` `5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73` the first call returned as
`performer`. Its capability grants `commit_composition` with the origin
`relay`, the template it writes and the facility it writes at, each element
named by its `ferroehr-operations` code. A system that also registers
device measurements holds a second capability with the origin `device` and
its `device_identifier_issuer`.

<!-- policy-example: system-client-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "System client" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Laboratory relay writes" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
    "id": { "_type": "HIER_OBJECT_ID", "value": "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73" }
  },
  "time_validity": {
    "_type": "DV_INTERVAL",
    "lower": { "_type": "DV_DATE", "value": "2026-12-01" },
    "lower_unbounded": false, "upper_unbounded": true,
    "lower_included": true, "upper_included": false
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Relay" },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Commit composition", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "commit_composition" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id18",
          "name": { "_type": "DV_CODED_TEXT", "value": "Origin", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "origin" } },
          "value": { "_type": "DV_CODED_TEXT", "value": "Relay", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "relay" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9",
          "name": { "_type": "DV_CODED_TEXT", "value": "Template written", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "template" } },
          "value": { "_type": "DV_TEXT", "value": "openEHR-EHR-COMPOSITION.report-result.v1" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id10",
          "name": { "_type": "DV_CODED_TEXT", "value": "Facility", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "facility" } },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id11",
              "name": { "_type": "DV_CODED_TEXT", "value": "Issuer", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "issuer" } },
              "value": { "_type": "DV_TEXT", "value": "urn:oid:2.16.528.1.1007.3.3" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id12",
              "name": { "_type": "DV_CODED_TEXT", "value": "Identifier", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "identifier" } },
              "value": { "_type": "DV_TEXT", "value": "01234567" }
            }
          ]
        }
      ]
    }
  }]
}
```

Then a new version of the `AGENT`, `PUT /demographic/agent/{uid}` with the
`If-Match` of its version, that names the `ROLE` it performs and the
provider it acts for. A relationship is stored in its source party, and the
`AGENT`'s id exists only once the first call returns, so the acts-for
relationship goes in this second version.

<!-- policy-example: agent-acts-for -->
```json
{
  "_type": "AGENT",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.system.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.system.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-AGENT.system.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "System agent" },
  "details": {
    "_type": "ITEM_TREE", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Identifiers" },
    "items": [{
      "_type": "ELEMENT", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Identifier" },
      "value": {
        "_type": "DV_IDENTIFIER",
        "issuer": "https://login.example.org/machines",
        "id": "lab-relay",
        "type": "client_id"
      }
    }]
  },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id5", "name": { "_type": "DV_TEXT", "value": "Items" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Name" },
        "value": { "_type": "DV_TEXT", "value": "Laboratory relay" }
      }]
    }
  }],
  "roles": [{
    "_type": "PARTY_REF", "namespace": "demographic", "type": "ROLE",
    "id": { "_type": "HIER_OBJECT_ID", "value": "e41c7b2a-6d3f-4a9e-8b05-2f7d1c9e3a68" }
  }],
  "relationships": [{
    "_type": "PARTY_RELATIONSHIP",
    "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Acts for",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Details" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Acts-for code" },
        "value": {
          "_type": "DV_CODED_TEXT", "value": "Acts for",
          "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "acts_for" }
        }
      }]
    },
    "source": {
      "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
      "id": { "_type": "HIER_OBJECT_ID", "value": "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73" }
    },
    "target": {
      "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
      "id": { "_type": "HIER_OBJECT_ID", "value": "9d2f4a7e-3c1b-4e8a-9f60-2b7d5c8e1a40" }
    }
  }]
}
```

#### A backend client and a demographic client

A reporting job queries every EHR with a `system/aql-*` scope, and a master
patient index reads and writes persons through the DEMOGRAPHIC API. Neither
is a key in `ferroehr.toml`: each is a grant of the client's `AGENT`, read on
every request. A `system/aql-*` scope "would grant access to all registered
and ad-hoc AQL queries system-wide" (SMART on openEHR, master08 §Resource
Scopes), so it counts only for a client granted `query_system_wide`; no
SMART on openEHR scope covers demographic data, so the DEMOGRAPHIC API admits
a client granted `access_demographic`, or a caller holding the
`[auth] product_scope`.

Both operations reach beyond one system's own writes, so the ceiling admits
each by name:

```toml
[authz.grants]
allowed = ["commit_composition", "query_system_wide", "access_demographic"]
```

The client's `AGENT` is written as above, and its grant is a `ROLE` it
performs, `POST /demographic/role`. One `ROLE` can carry both operations; a
deployment that runs the two as separate clients gives each its own `AGENT`
and `ROLE`.

<!-- policy-example: backend-client-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "System client" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Reporting and patient index" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
    "id": { "_type": "HIER_OBJECT_ID", "value": "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73" }
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Reporting" },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Query system-wide",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "query_system_wide" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Access demographic",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "access_demographic" }
          }
        }
      ]
    }
  }]
}
```

The client's next request runs under the grant, on every replica, with no
restart. The grant counts for the client's own app-only token, a token of a
`machines` issuer whose `client_id` the `AGENT` carries. A person signed in
through the same client holds the person's own grants, never the client's.
To withdraw it, end the `ROLE`'s `time_validity` or delete the `ROLE`
([Ending a grant](#ending-a-grant)).

#### A migration

A migration is a system client like any other: its `AGENT` is written as
above, and its `ROLE` holds the import and export operations. Here a job
imports the EHRs of a legacy system and its extracts over one night, and may
export them again to check the result. The capability's `time_validity`
holds the migration to two days, `Not before` and `Not after` narrow it to
the night between them, and `subject_namespace` limits it to the legacy
system's patients. `[authz.grants] allowed` must admit `import_ehr`,
`import_ehr_extract` and `export_ehr_extract`.

<!-- policy-example: migration-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.system_client.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "System client" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Legacy HIS migration" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "AGENT",
    "id": { "_type": "HIER_OBJECT_ID", "value": "5b8e2c91-7d4a-4f3e-8a1c-6e9f0b2d4c73" }
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Migration" },
    "time_validity": {
      "_type": "DV_INTERVAL",
      "lower": { "_type": "DV_DATE", "value": "2026-12-05" },
      "upper": { "_type": "DV_DATE", "value": "2026-12-06" },
      "lower_unbounded": false, "upper_unbounded": false,
      "lower_included": true, "upper_included": true
    },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Import EHR", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "import_ehr" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Import EHR extract", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "import_ehr_extract" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Export EHR extract", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "export_ehr_extract" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id19",
          "name": { "_type": "DV_CODED_TEXT", "value": "Subject namespace", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "subject_namespace" } },
          "value": { "_type": "DV_TEXT", "value": "nl.legacy-his.patients" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id15",
          "name": { "_type": "DV_CODED_TEXT", "value": "Not before", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "not_before" } },
          "value": { "_type": "DV_DATE_TIME", "value": "2026-12-05T22:00:00Z" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id16",
          "name": { "_type": "DV_CODED_TEXT", "value": "Not after", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "not_after" } },
          "value": { "_type": "DV_DATE_TIME", "value": "2026-12-06T05:00:00Z" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id17", "name": { "_type": "DV_TEXT", "value": "Authorisation reference" },
          "value": { "_type": "DV_TEXT", "value": "CHG-2026-1182" }
        }
      ]
    }
  }]
}
```

The job then calls `POST /message/import` (`message_import_ehr`) and
`POST /message/import/{ehr_id}` (`message_import_ehr_extract`) with its own
token, and `POST /message/export` to read the result back. Each
import is committed under the job's `ACTOR` and is an access record that names
it and the provider it acts for. The versions of an extract keep their legacy
committers when a `creating_system` element of the grant names the legacy
system's `creating_system_id`; otherwise an extract naming a local party is
refused.
When the night ends the grant holds no longer, with no change to anything.

#### A federation partner

A partner organisation's professionals read patient summaries under a
data-sharing agreement. The partner is an `ORGANISATION` that carries a
`client_id` identifier for each client its `federation` issuer issues tokens
to, for example `{ "_type": "DV_IDENTIFIER", "issuer":
"https://federation.example", "id": "remote-portal", "type": "client_id" }`
in its `details`; committing it is an administrator's write, because the
identifier makes it a policy party. The agreement is a `ROLE` of the
federation partner template the partner performs, with one capability per
accessor, here `person`. Each term is an element whose `name` is its
`ferroehr-operations` code, which is how FerroEHR reads it in any archetype
(see [Federation agreements](#federation-agreements)).

<!-- policy-example: federation-partner-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.federation_partner.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "Federation partner" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Regional care network" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
    "id": { "_type": "HIER_OBJECT_ID", "value": "7c4e9a2b-1f6d-4b3e-9c8a-5d2e0f7b1a36" }
  },
  "time_validity": {
    "_type": "DV_INTERVAL",
    "lower": { "_type": "DV_DATE", "value": "2026-01-01" },
    "upper": { "_type": "DV_DATE", "value": "2036-12-31" },
    "lower_unbounded": false, "upper_unbounded": false,
    "lower_included": true, "upper_included": true
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Person",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Federated person access", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "federated_person_access" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9",
          "name": { "_type": "DV_CODED_TEXT", "value": "Agreement name", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "agreement_name" } },
          "value": { "_type": "DV_TEXT", "value": "Regional care network exchange" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id10",
          "name": { "_type": "DV_CODED_TEXT", "value": "Agreement version", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "agreement_version" } },
          "value": { "_type": "DV_TEXT", "value": "2.1" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id11",
          "name": { "_type": "DV_CODED_TEXT", "value": "Jurisdiction", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "jurisdiction" } },
          "value": { "_type": "DV_TEXT", "value": "DE" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id12",
          "name": { "_type": "DV_CODED_TEXT", "value": "Exchange", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "exchange" } },
          "value": { "_type": "DV_TEXT", "value": "patient summary" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id13",
          "name": { "_type": "DV_CODED_TEXT", "value": "Purpose", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "purpose" } },
          "value": { "_type": "DV_CODED_TEXT", "value": "treatment", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_PurposeOfUse" }, "code_string": "TREAT" } }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id14", "name": { "_type": "DV_TEXT", "value": "Legal ground" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id15",
              "name": { "_type": "DV_CODED_TEXT", "value": "GDPR Article 6(1) point", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "gdpr_art_6_1" } },
              "value": { "_type": "DV_TEXT", "value": "e" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id16",
              "name": { "_type": "DV_CODED_TEXT", "value": "GDPR Article 9(2) point", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "gdpr_art_9_2" } },
              "value": { "_type": "DV_TEXT", "value": "h" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id17",
              "name": { "_type": "DV_CODED_TEXT", "value": "National provision", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "national_provision" } },
              "value": { "_type": "DV_TEXT", "value": "the provision the agreement names" }
            }
          ]
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id18",
          "name": { "_type": "DV_CODED_TEXT", "value": "Consent source", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "consent" } },
          "value": { "_type": "DV_CODED_TEXT", "value": "Local", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "local" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id19",
          "name": { "_type": "DV_CODED_TEXT", "value": "Read scope", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "read_scope" } },
          "value": { "_type": "DV_TEXT", "value": "patient/composition-openEHR-EHR-COMPOSITION.patient_summary.v1.r" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id24", "name": { "_type": "DV_TEXT", "value": "Caller rate" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id25",
              "name": { "_type": "DV_CODED_TEXT", "value": "Requests per second", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "requests_per_second" } },
              "value": { "_type": "DV_COUNT", "magnitude": 5 }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id26",
              "name": { "_type": "DV_CODED_TEXT", "value": "Burst", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "burst" } },
              "value": { "_type": "DV_COUNT", "magnitude": 20 }
            }
          ]
        }
      ]
    }
  }]
}
```

#### A privileged role

A professional covers a colleague's access-log reviews for one week. The
grant is a `ROLE` of the privileged template performed by the professional's
`PERSON` `3e7a1d5c-9b2f-4c8e-a6d0-1f4b8e2c7a95`, with one capability per
privilege, here `subject audit`. The privileged role never grants the
administrator role or emergency access: the identity provider and the
configuration grant those.

<!-- policy-example: privileged-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "Privileged role" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Access log review cover" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
    "id": { "_type": "HIER_OBJECT_ID", "value": "3e7a1d5c-9b2f-4c8e-a6d0-1f4b8e2c7a95" }
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "id6",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Subject audit",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
    "time_validity": {
      "_type": "DV_INTERVAL",
      "lower": { "_type": "DV_DATE", "value": "2026-12-07" },
      "upper": { "_type": "DV_DATE", "value": "2026-12-13" },
      "lower_unbounded": false, "upper_unbounded": false,
      "lower_included": true, "upper_included": true
    },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Read subject audit",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "read_subject_audit" }
          }
        },
        {
        "_type": "ELEMENT", "archetype_node_id": "id11", "name": { "_type": "DV_TEXT", "value": "Authorisation reference" },
        "value": { "_type": "DV_TEXT", "value": "Cover for the privacy officer, approved by the CMIO" }
      }]
    }
  }]
}
```

#### Just-in-time admission

A first login of a person no party carries creates a provisional `PERSON`
only when a `create_at_login` grant admits it. The grant is a `ROLE` like
any other, and FerroEHR reads it on every first login, whoever performs the
`ROLE`: here the provider `ORGANISATION` performs it, which reads as "the
hospital admits its own staff". Its capability names the issuer by
`login_issuer` and declares the admission conditions:

- `admit_idp`: the identity providers whose logins are created, read from
  the issuer's `[[authz.party_binding.issuer]] idp_claim` (`idp` by
  default);
- `admit_group`: the groups, read from the issuer's `[[auth.party]]
  group_claim`, one of which an admitted login holds;
- `admit_role`: the roles, read from `[auth] role_claims`, one of which an
  admitted login holds;
- `admit_assurance`: the lowest `[authz.assurance]` level an admitted login
  carries;
- `admit_guests`: true to admit a guest the issuer's `[[auth.party]]`
  table admits.

A login is admitted when it meets every condition the grant declares; each
list admits any one of its values. Several grants in force admit a login
when any one of them does. A grant names at least one `login_issuer` and
at least one of `admit_idp`, `admit_group`, `admit_role` and
`admit_assurance`, so no grant admits every login of an issuer: a grant
without a condition is refused with `422`, as is an element whose value
does not read. `[authz.grants] allowed` must list `create_at_login`, which
the default does not.

The provider admits the logins of its Microsoft Entra tenant from members of
one group at assurance substantial or above, from 1 December 2026,
`POST /demographic/role`:

<!-- policy-example: login-admission -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.login_admission.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.login_admission.v1" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "Login admission" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "at0001", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "at0002", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "at0003", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Staff first login" }
      }]
    }
  }],
  "performer": {
    "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
    "id": { "_type": "HIER_OBJECT_ID", "value": "9d2f4a7e-3c1b-4e8a-9f60-2b7d5c8e1a40" }
  },
  "time_validity": {
    "_type": "DV_INTERVAL",
    "lower": { "_type": "DV_DATE", "value": "2026-12-01" },
    "lower_unbounded": false, "upper_unbounded": true,
    "lower_included": true, "upper_included": false
  },
  "capabilities": [{
    "_type": "CAPABILITY", "archetype_node_id": "at0010", "name": { "_type": "DV_TEXT", "value": "Admission" },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "at0011", "name": { "_type": "DV_TEXT", "value": "Terms" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "at0012", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Create at login",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "create_at_login" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "at0013",
          "name": {
            "_type": "DV_CODED_TEXT", "value": "Login issuer",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "login_issuer" }
          },
          "value": { "_type": "DV_URI", "value": "https://login.microsoftonline.com/0f1e2d3c-4b5a-4968-8776-655443322110/v2.0" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "at0014",
          "name": {
            "_type": "DV_CODED_TEXT", "value": "Admitted group",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "admit_group" }
          },
          "value": { "_type": "DV_TEXT", "value": "5f2a9c1e-0000-4000-8000-00000000c11a" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "at0015",
          "name": {
            "_type": "DV_CODED_TEXT", "value": "Admitted assurance",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "admit_assurance" }
          },
          "value": { "_type": "DV_TEXT", "value": "substantial" }
        }
      ]
    }
  }]
}
```

The `ROLE` is a policy party, so only an administrator writes or reads it.
The next first login of that tenant that meets both conditions creates its
`PERSON`, on every replica and with no restart. To stop creating, end the
`ROLE` as below; a `PERSON` already created stays. The configuration keeps
only how the issuer's tokens are read
([Just-in-time creation](installation/config-auth.md#just-in-time-creation)).

#### Ending a grant

To end a grant, an administrator commits a new version of the `ROLE` whose
`time_validity` ends before today (an upper bound of yesterday, or of today
with `upper_included` false), with `PUT /demographic/role/{uid}` and the
`If-Match` of its current version, or deletes it with
`DELETE /demographic/role/{uid}`. Both are versions with their committer, so
the revision history keeps the grant and who ended it, and the change decides
the caller's next request.

### Own design

No openEHR specification governs these points; each is FerroEHR's own
design:

- **A capability is a grant.** The RM defines `CAPABILITY.credentials` as
  "the qualifications of the performer of the role for this capability";
  FerroEHR reads them as what the performer may do.
- **Bounds finer than a day go in the credentials.** `time_validity` is
  `DV_INTERVAL<DV_DATE>` on `ROLE`, `CAPABILITY` and `PARTY_RELATIONSHIP`,
  so `Not before` and `Not after` are `DV_DATE_TIME` elements.
- **Only an administrator writes and reads a policy party.** The RM's
  `ACCESS_CONTROL_SETTINGS` is implementation dependent, and the Demographic
  API defines no access rule for its own operations.
- **AQL leaves the policy parties out for anyone else.** AQL over a
  demographic store is FerroEHR's own extension: QUERY §FROM gives semantics
  to the clinical repository only.
- **The grant ceiling.** `[authz.grants] allowed` bounds what the party store
  may grant.
- **A grant is held in memory on every replica** and read again when a commit
  changes one, so the request path adds no database read.
- **A `ROLE`'s `performer` ties a grant to its party**; the performer's
  `roles` list need not name the `ROLE`.
- **Break-glass stays in the configuration**, so emergency access works when
  the party store does not answer.
- **Trust in imported committers is the importer's.** The RM keeps an
  imported version's audit as received (`IMPORTED_VERSION`); which systems'
  committers may name a local party is a scope of the importer's grant.
- **An origin is a party's grant.** RM `FEEDER_AUDIT_DETAILS.system_id` names
  where data came from as free text; which ids a deployment recognises, and
  which origin a direct commit takes, is the `data_origin` grant.
- **The change feed and SCIM are grants.** No openEHR spec defines a change
  feed or SCIM provisioning, so who reads the one and who provisions through
  the other is FerroEHR's own reading of a capability.
- **The first grant is the operator's.** `ferroehr party-binding grant`
  commits it offline with the server's own committer, since no
  administrator can log in before the persons a provisioning client creates.
- **A `create_at_login` grant admits logins by its credentials, whoever
  performs its `ROLE`,** and must declare a condition: the RM and the
  Demographic API say nothing about creating a party at a login.

- **A federation agreement is a grant on the partner**, keyed by its client
  identifiers, and its rules are judged when it is written.

## Representatives in the patient portal

A parent, a guardian or a proxy the patient chose reads the patient's record
through the patient portal as the patient's representative. Regulation (EU)
2025/327 Art. 4(2) asks for proxy services that let natural persons
authorise others to access their personal electronic health data on their
behalf, and let legal representatives access the data of the persons whose
affairs they administer, "in accordance with national law"; Member States
set the rules for both. Who represents whom, and until when, is therefore
data the deployment keeps in the CDR. FerroEHR builds in no country's rules.

### The representation

A representation is a `PARTY_RELATIONSHIP` from the representative's
`PERSON`, its source, to the subject's `PERSON`, its target, whose `name` or
`details` carry `ferroehr-operations::represents`. Any archetype may model
it; FerroEHR reads the code, never the archetype id. It may also name the
relationship with a `DV_CODED_TEXT` of the openEHR `subject_relationship`
group (`254|parent|`, `264|guardian|`, `253|unknown|` for a proxy), which the
committer and the access log carry. The shipped
`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1` is an example: its
name is the type (parent, guardian or authorised proxy) and its details hold
both codes.

It holds while its `time_validity` covers today's date in UTC. It is either
inline, in the representative's `PERSON.relationships`, which an
administrator writes, or a `PARTY_RELATIONSHIP` of its own
(`POST /demographic/party_relationship`, FerroEHR's extension), which an
administrator or the patient writes.

The representative's `PERSON` is of a template
`[[authz.party_binding.person.template]]` maps and carries a binding key: a
`DV_IDENTIFIER` whose `issuer` is the token issuer of the patient portal and
whose `id` is the representative's login there (their `sub`).

### Legal capacity

How old a minor must be before their parents no longer see their record, and
when someone may act for an adult, is national law. It reaches FerroEHR as
the representations the deployment records and the `time_validity` it gives
them: the deployment, or the national identity or authorisation service it
follows, ends a parent's representation on the date the child decides
alone, and ends or adds a representation when a court or the patient changes
who may act. FerroEHR checks only that a live representation covers today,
so a change is effective on the next request, and holds no age band, no
country table and no archetype id of its own.

### Signing in

A `patients` issuer names its patient by the EHR of the token's launch
context (`ehrId`). A representative signs in one of two ways:

- **As the actor of the patient's token.** The issuer sets
  `act_represents_patient = true` and puts the representative in the
  token's `act` claim (RFC 8693 §4.1): `sub` is the patient, `act.sub` the
  representative.
- **Under their own login.** The token's `sub` is the representative and its
  launch context names the patient's EHR.

Either way FerroEHR binds the login, under the token's issuer, to the
`PERSON` whose binding key it is, and admits the request while that `PERSON`
holds a live representation of the subject of the EHR's `EHR_STATUS`. The
request is confined to that EHR. A login whose `PERSON` holds, or has ever
held, a representation of the EHR's subject is judged only as a
representative: it is admitted while a representation is live, and refused
once none is, whether the representation ended or was deleted, so a former
representative never becomes the patient. On the second path a login no
representative's `PERSON` has ever carried for that subject is the patient
themself; on the first path such an actor is refused. A representation that
is not recorded, has ended or was deleted is refused with `403`
(`refused-representation-unrecorded`); a representative's `PERSON` that is
`inactive` or `abandoned` with `refused-inactive`, and a login two `PERSON`s
carry with `refused-ambiguous`. FerroEHR trusts a `patients` issuer for the
patient its token names, so the issuer, not FerroEHR, decides which EHR a
login may name; FerroEHR holds every login it knows as a representative to a
live representation.

Each access is recorded bound to the representative's `PERSON`, naming the
subject they represent and the relationship (EHDS Annex II 3.2), and a commit
records the representative as a `PARTY_RELATED` committer.

### The patient authorises a proxy

The patient authorises a proxy with a `PARTY_RELATIONSHIP` of its own, from
the proxy's `PERSON` to their own:

```sh
curl -X POST "$BASE/ferroehr/rest/extensions/v1/demographic/party_relationship" \
  -H "Authorization: Bearer $PATIENT_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{
    "_type": "PARTY_RELATIONSHIP",
    "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1",
    "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.1.0",
      "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1" } },
    "name": { "_type": "DV_CODED_TEXT", "value": "Authorised proxy",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at12" } },
    "details": { "_type": "ITEM_TREE", "archetype_node_id": "id3",
      "name": { "_type": "DV_TEXT", "value": "Details" },
      "items": [
        { "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Representation code" },
          "value": { "_type": "DV_CODED_TEXT", "value": "represents",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "represents" } } },
        { "_type": "ELEMENT", "archetype_node_id": "id6", "name": { "_type": "DV_TEXT", "value": "Relationship" },
          "value": { "_type": "DV_CODED_TEXT", "value": "unknown",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "253" } } } ] },
    "time_validity": { "_type": "DV_INTERVAL", "lower": { "_type": "DV_DATE", "value": "2026-11-13" },
      "lower_unbounded": false, "upper_unbounded": true, "lower_included": true, "upper_included": false },
    "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
      "id": { "_type": "HIER_OBJECT_ID", "value": "<the proxy PERSON>" } },
    "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
      "id": { "_type": "HIER_OBJECT_ID", "value": "<the patient PERSON>" } }
  }'
```

The patient's own credential reaches the writes of
`/demographic/party_relationship`, and FerroEHR admits only a representation
whose target is the subject of the patient's launch EHR, before and after
the write. The committer is the patient, a `PARTY_IDENTIFIED` naming their
`PERSON`. The patient revokes the authorisation with
`DELETE /demographic/party_relationship/{uid}`, or ends it with a `PUT` whose
`time_validity` ends; the proxy's next request is refused. The patient
changes only a representation they inserted themself: one an administrator
recorded, such as a court-appointed guardian's, is the administrator's
(EHDS Art. 5).

### Where the portal is our own design

No openEHR specification governs these points; each is FerroEHR's own
design:

- **A representation is recognised by its code**, as the acts-for
  relationship is, so a deployment models it in any archetype.
- **Legal capacity is the recorded representations and their validity.**
  The rules are national law (EHDS Art. 4(2)), so FerroEHR holds no rule of
  its own.
- **A portal login binds by the token issuer.** The representative's binding
  key is a `DV_IDENTIFIER` under the issuer that signs their login, as a
  client's `client_id` is.
- **A patient writes the representation of themselves**, and the party
  protection holds them to it.

## The admin registers

Three kinds of operational policy have no home in the openEHR demographic
model, so each is an admin register:

| Register | What it holds | Default, before anyone writes it |
|---|---|---|
| `audit-categories` | The EHDS category map every access is classified through ([the audit category register](installation/config-audit.md#the-audit-category-register)) | The category map FerroEHR ships |
| `access-delay` | The EHDS Art. 3(3) delay of the person's own access ([the access-delay register](installation/config-auth.md#the-access-delay-register)) | Off |
| `cedar-policies` | The policy set of the embedded Cedar engine ([below](#the-cedar-policy-register)) | Empty, so Cedar permits nothing |

You read and write them under `{ext}/admin/registers/{register}` with a token
or Basic user holding `admin_role`, with `[api] admin = true`; the routes are
on [Admin & messaging APIs](operations-admin-apis.md#operational-policy-registers).

- **Versioned.** Every write appends a version that names its committer and
  when it was committed, and never overwrites one; the history route lists
  them all. Version 0 is the default no one committed.
- **Whole, over a version.** A write replaces the whole document and names the
  version it replaces in `If-Match`. A write over a version that is no longer
  the newest is refused with `412`, so two operators never overwrite each
  other's change unseen.
- **Checked when written.** A document that does not hold is refused whole with
  `400` and never reaches a request: a category map that does not compile, a
  delay with an unusable limit, a Cedar set that does not parse or validate.
  The register keeps its version in force.
- **Effective without a restart.** Each replica keeps the registers in
  memory and is told of every write through PostgreSQL `LISTEN`/`NOTIFY` when
  the write commits; its next request after that reads the new version. The
  replica that took the write applies it to its own next request. While a
  replica's listener is not connected, that replica reads the registers on
  every request, so it never serves an outdated policy. A database that cannot
  answer that read fails the request with `503`; it is never served under a
  guess.
- **Audited.** Every read and write of a register is an access record.

The three keys these registers replace, `[audit.categories]`,
`[privacy.access_delay]` and `[authz.abac.cedar]`, are unknown keys in
`ferroehr.toml` and refuse boot. A deployment provides its registers once the
server runs, through the admin API.

### A worked example: turning the access delay on

Read the register, then write the new document over the version it answered:

```sh
curl -s -u admin:… https://cdr.example/ferroehr/rest/extensions/v1/admin/registers/access-delay
# ETag: "0"
# {"register":"access-delay","version":0,"committer":null,"committed_at":null,
#  "document":{"enabled":false,"categories":[],"max_delay":null}}

curl -s -u admin:… -X PUT \
  -H 'Content-Type: application/json' -H 'If-Match: "0"' \
  -d '{"enabled": true, "categories": ["Laboratory-Reports"], "max_delay": "72h"}' \
  https://cdr.example/ferroehr/rest/extensions/v1/admin/registers/access-delay
# ETag: "1"
```

The next laboratory report committed on any replica is held from the person
for up to 72 hours.

### The Cedar policy register

The embedded Cedar engine (`[authz.abac] engine = "cedar"`) decides with the
policy set of this register, in the Cedar policy language:

```json
{ "policies": "permit(principal, action, resource) when { principal has organization };" }
```

A write is parsed and validated strictly against the schema FerroEHR builds
from its ABAC request model, and a set that does not parse or validate is
refused with `400`; the set in force keeps deciding. Until the register is
written the set is empty, and Cedar, deny-by-default, permits nothing, so a
deployment that turns ABAC on writes its policies before it serves clinical
traffic. The example policies under
[`app/ferroehr-rest/examples/policies`](https://github.com/FerroHEALTH/FerroEHR/tree/main/app/ferroehr-rest/examples/policies)
are a starting point.

### The security floor that stays in `ferroehr.toml`

A floor that must hold before a register can be read stays configuration. With
`deployment.profile = "production"` and auditing on, the audit category
register refuses a map that classifies nothing (no template, no archetype and
`shipped_map = false`) as the `no_category_map` gap, unless
`deployment.accepts` names it.
