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
- each issuer's `patient_context`, whether the server believes its `ehrId`
  claim, which is issuer trust;
- the grant ceiling, `[authz.grants] allowed`: the operations the party store
  may grant at all, so a compromised administrator account cannot grant
  more than the operator allows ([The grant ceiling](#the-grant-ceiling)).

Everything else is CDR data: who may do what is party data (below), and the
policy with no demographic home is an admin register. The privileges beyond
clinical access have no configuration key: reading one subject's access log,
acting as an EHDS Art. 10 access service, and importing are each a grant of
its own operation
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
  `DV_CODED_TEXT` of the draft openEHR access terminology, terminology id
  `openehr_access`, such as `openehr_access::import_ehr`;
- a `PARTY_RELATIONSHIP`, held inline or written on FerroEHR's own
  relationship routes, whose `name` or `details` hold the code
  `openehr_access::acts_for`;
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
openehr_access::import_ehr: only an administrator writes it`. A
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
- A party belongs to no EHR, so the EHR access settings and the rest of the
  access decision a clinical query applies to each row do not apply to a
  party. The mark decides which parties a caller reads, for AQL as for the
  Demographic API. A demographic query never reads the EHR access settings,
  so it answers while the settings store is unavailable, when a clinical
  query answers `503`.

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

**The access terminology.** Its `TERMINOLOGY_ID` is `openehr_access`, the
draft terminology the access control change request proposes to openEHR
([The access control terminology](#the-access-control-terminology)). The
operations and the elements that qualify a grant are groups of FerroEHR's
own in it, which the change request does not propose. A `CAPABILITY` grants an operation when its
`credentials` hold, at any depth, an element whose value is a
`DV_CODED_TEXT` with that terminology and the operation's code:

| Code | Operation |
|---|---|
| `commit_composition` | Commit a `COMPOSITION` on the caller's own account, within the templates and facilities the grant names |
| `compose_for` | Name, as a bound professional, another `PERSON` of the provider the caller acts for as the composer of a `COMPOSITION` the caller commits; the committer stays the caller ([Who is the committer](security/committer.md#the-composer-of-a-professionals-composition)) |
| `import_ehr` | Import an EHR |
| `import_ehr_extract` | Import an EHR extract |
| `export_ehr_extract` | Export an EHR extract |
| `federated_person_access` | Admit a federation partner's persons under a data-sharing agreement |
| `federated_organisation_access` | Admit a federation partner organisation itself under a data-sharing agreement |
| `read_subject_audit` | Read the access log of one subject |
| `access_service` | Act as an EHDS Article 10 access service, held to a subject's opt-out |
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
| `write_care_relationship` | Write the membership (`MBR` of `HL7_RoleClass`) and `under_care_of` care relationships through the Demographic API, as a hospital's ADT system does ([The access control terminology](#the-access-control-terminology)) |
| `write_representation` | Write a representation (`represents`) through the Demographic API, for the providers its `provider` elements name ([Representatives in the patient portal](#representatives-in-the-patient-portal)) |
| `write_access_policy` | Write the controller's `ACCESS_POLICY` |

The same terminology names the elements that qualify a grant, by the code
in the element's `name`:

| Code | Element |
|---|---|
| `not_before` | A `DV_DATE_TIME`: the instant before which the grant does not hold |
| `not_after` | A `DV_DATE_TIME`: the instant after which the grant does not hold |
| `template` | On a federation agreement: a template an external person may commit ([Federation agreements](#federation-agreements)) |
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
| `role_terminology` | On `scim_provision`: the terminology the client's User `roles` are coded in, exactly one when it names a `role` |
| `provider` | On `write_care_relationship` and `write_representation`: a `DV_IDENTIFIER` (its `issuer` and `id`) naming a provider whose care relationships and memberships, or representations, the grant writes; none named means none written |
| `target_kind` | On `write_care_relationship`: `PERSON`, `ROLE`, `GROUP` or `ORGANISATION`, a kind of care relationship target the grant writes; none named means none written |
| `acts_for` | On a `PARTY_RELATIONSHIP`: the source acts for the target |
| `represents` | On a `PARTY_RELATIONSHIP`: the source represents the target, the subject, on the subject's own record ([Representatives in the patient portal](#representatives-in-the-patient-portal)); it makes no policy party |
| `role` | A `DV_TEXT`: a role a `hold_roles` grant gives, or a role code a `scim_provision` grant lets its client assign its Users ([SCIM roles](installation/config-auth.md#roles)) |
| `scope` | A `DV_TEXT`: a SMART scope a `hold_scopes` grant gives |
| `login_issuer` | A text or `DV_URI`: a token issuer whose first logins a `create_at_login` grant admits |
| `admit_idp` | A text: an identity provider an admitted login comes from |
| `admit_group` | A text: a group an admitted login holds |
| `admit_role` | A text: a role an admitted login holds |
| `admit_assurance` | A text, `low`, `substantial` or `high`: the lowest assurance level an admitted login carries |
| `admit_guests` | A `DV_BOOLEAN`: whether a guest is admitted too |
| `under_care_of` | On a `PARTY_RELATIONSHIP` of its own: the source, a patient's `PERSON`, is under the care of the target, a professional's `PERSON`, a `ROLE`, a team `GROUP` or an organisational unit, with its `care_role` and `provider` |

The terms of a federation agreement are named by codes of the same
terminology ([Federation agreements](#federation-agreements)), and the value of
an element any of these codes names is that element's value, never an
operation. A code of another terminology grants nothing, and a code of
`openehr_access` that no table defines is refused at commit.
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
caller facts they are read against are coded in the draft `openehr_access`
terminology, which the access control change request proposes to openEHR
as "openEHR access (draft)". Where an HL7 code system publishes a code of
the same meaning, the group's term is that code, under the code system's
id: `HL7_ConsentProvisionType` (`http://hl7.org/fhir/consent-provision-type`),
`HL7_SecurityRoleType`
(`http://terminology.hl7.org/CodeSystem/extra-security-role-type`) and
`HL7_RoleClass` (`http://terminology.hl7.org/CodeSystem/v3-RoleClass`). The
groups below are the stable contract: the access evaluator and the access
write checks read these codes and no archetype id, so you model consent,
restriction and care relationships with your own archetypes and bind their
coded attributes to these groups. A value of a terminology the group holds
no term of, or a code outside the group, is refused. Each code of
`openehr_access` is its own rubric until openEHR assigns codes. openEHR
defines no access control terminology (RM EHR IM, `ACCESS_CONTROL_SETTINGS`:
"Currently implementation dependent"), so every group is the draft's own
design.

| Group | Codes | Read from |
|---|---|---|
| `access default` | `open`, `restricted`, `consent_required` | `ACCESS_DIRECTIVE_SETTINGS.default_access`: the disposition of an EHR where no directive applies |
| `access directive kind` | `consent`, `restriction`, `opt_out`, `delegation`, `controller_rule`, `exclusion` | `ACCESS_DIRECTIVE.kind`. A `restriction` (EHDS Art. 8) is always a deny; an `opt_out` (EHDS Art. 10) is a deny of the whole record; a `controller_rule` is a rule of the access policy and never a directive of an EHR; an `exclusion` is the controller's deny of a named party in the settings' `exclusions` alone, with no exception |
| `access effect` | `permit`, `deny` of `HL7_ConsentProvisionType` | `ACCESS_DIRECTIVE.effect`; a deny overrides a permit |
| `access relationship` | `care`, `representative`, `proxy` | `ACCESS_ACTOR_SCOPE.relationship`: the caller's relationship to the subject |
| `access caller kind` | `professional`, `access_service`, `datasubject` of `HL7_SecurityRoleType` (the subject), `representative`, `system` | `ACCESS_ACTOR_SCOPE.caller_kind`. An `access_service` caller is one holding the `access_service` grant |
| `access operation` | `read`, `write`, `export`, `query` | `ACCESS_DIRECTIVE.operations`: the operation classes a directive applies to. No capability grants one |
| `care relationship` | `MBR` of `HL7_RoleClass`, `under_care_of` | The `name` or `details` of a `PARTY_RELATIONSHIP`: with `MBR`, the source, a professional's `PERSON`, is a member of the target, a care-team `GROUP`, or the source, a provider `ORGANISATION`, is a member of the target `GROUP`; with `under_care_of`, the source, a patient's `PERSON`, is under the care of the target, a team `GROUP` or an organisational unit |
| `representation` | `represents` | The `name` of a `PARTY_RELATIONSHIP` of its own: the source represents the target, the subject ([Representatives in the patient portal](#representatives-in-the-patient-portal)) |
| `representation element` | `basis`, `provider`, `evidence` | The `name` of an `ELEMENT` in the `details` of a `represents` relationship |
| `kinship`, `kinship element` | `related_to`; `kinship` | The `name` or `details` of a `PARTY_RELATIONSHIP` of its own: its source and target, two `PERSON`s, are kin, and its `kinship` element codes how ([Exclusions and conflicts of interest](#exclusions-and-conflicts-of-interest)) |
| `group kind` | `care_team` | The `name` of a `GROUP`: the `GROUP` is a care team ([Care teams provisioned over SCIM](#care-teams-provisioned-over-scim)) |
| `access refusal` | `refused-opt-out`, `refused-restricted`, `refused-consent-denied`, `refused-policy-denied`, `refused-sensitivity`, `refused-confidentiality`, `refused-no-care-relationship`, `refused-outside-episode`, `refused-purpose-missing`, `refused-consent-required`, `refused-not-subject`, `refused-not-represented`, `refused-consent-reference-missing`, `refused-excluded`, `refused-conflict` | The access record of a refusal |
| `access log withheld` | `accessor_party`, `accessor_provider`, `accessor_roles`, `accessor_groups` | `ACCESS_LOG_RESTRICTION.withheld`: the accessor members the access log's subject view withholds under EHDS Art. 9(3) |

Three grants of the operation table belong to access control:
`write_care_relationship`, `write_representation` and `write_access_policy`.

**Codes FerroEHR does not own.** Role, sensitivity, data category, purpose
of use and legal ground are coded in the terminology your jurisdiction names, such
as HL7 `HL7_Confidentiality` and `HL7_ActCode`, the EHDS categories and the
HL7 `HL7_ActReason` purposes of use. FerroEHR compares them by terminology id and code, and refuses
one coded in `openehr_access`, which holds none.

**Refusals.** `refused-opt-out` and `refused-restricted` answer `404`, as if
the data did not exist, and appear only in the access record, so that a
restriction or an opt-out is not visible to the caller (EHDS Art. 8). Every
other refusal answers `403` and names its code. The access record of every
refusal names its code, the step that refused and the directive or policy of
that step, and the record of an admitted access names the grounds it rested
on ([The openEHR access-log entry](audit.md#the-openehr-access-log-entry)).

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
period may be keyed on it (`PUT {ext}/admin/retention/policy`), and an access
directive's data scope may name it. A body with no `FEEDER_AUDIT`, committed by the holder of
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
`[auth.issuer.<name>.party]` table that no other client's grant names, and the token must
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
the SMART on openEHR scopes, the access evaluator and the access record. A token's roles and
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

**Composing for another professional.** A professional's `COMPOSITION`
names the professional as its composer. Under a `compose_for` grant in force
on a `ROLE` the professional's `PERSON` performs, its composer may name
another `PERSON` of the same provider by its `external_ref`, as a scribe or
a resident records for a physician; a correction that keeps the composer
of the version it replaces needs no grant. The committer is still the
professional who committed
([Who is the committer](security/committer.md#the-composer-of-a-professionals-composition)).
The grant names no further element.

**Committing and exporting as a system client.** A system client commits
under its `commit_composition` grants and exports under its
`export_ehr_extract` grants
([What a system client may do](#what-a-system-client-may-do)).

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
opt-out is refused with `422`, since it would hold for nobody
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
          "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "hold_roles" } } },
      { "_type": "ELEMENT", "archetype_node_id": "at0023",
        "name": { "_type": "DV_CODED_TEXT", "value": "Role",
          "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "role" } },
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
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
            "code_string": "scim_provision" } } },
      { "_type": "ELEMENT", "archetype_node_id": "at0023",
        "name": { "_type": "DV_CODED_TEXT", "value": "binding_issuer",
          "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
            "code_string": "binding_issuer" } },
        "value": { "_type": "DV_TEXT", "value": "urn:example:login" } },
      { "_type": "ELEMENT", "archetype_node_id": "at0024",
        "name": { "_type": "DV_CODED_TEXT", "value": "token_scope",
          "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
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
sign in with HTTP Basic and no `[auth.issuer.<name>.party]` table binds logins: it holds
`admin_role` and the product scope before any policy party exists.

No HTTP route grants an operation before an administrator exists. A
deployment whose staff sign in through a `persons` issuer bound by an
`[auth.issuer.<name>.party]` table starts in this order:

1. The operator grants the SCIM client `scim_provision` with
   `ferroehr party-binding grant`, as above.
2. The SCIM client provisions the staff, the administrator among them.
3. The administrator signs in in one of two ways. The first is a token of an
   issuer `[authz.rbac] role_issuers` lists, carrying `admin_role`. The
   second is `first_admin`, mapped with `[[authz.party_binding.basic_user]]`
   to the key of the `PERSON` the SCIM client provisioned. A Basic user beside
   an `[auth.issuer.<name>.party]` table must map to a key, so an unmapped `first_admin`
   refuses the start, and a mapped one is refused as unbound until its
   `PERSON` exists.
4. The administrator writes every other policy party through the Demographic
   API.

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
named by its `openehr_access` code:

| Code | Element |
|---|---|
| `agreement_name`, `agreement_version` | `DV_TEXT`: the agreement's name and version |
| `exchange` | `DV_TEXT`: the kind of exchange |
| `purpose` | `DV_CODED_TEXT` or `DV_TEXT`: what the exchange is for |
| `gdpr_art_6_1`, `gdpr_art_9_2` | `DV_TEXT` or `DV_CODED_TEXT`: the point of GDPR Art. 6(1), `a` to `f`, and of Art. 9(2), `a` to `j` |
| `national_provision` | `DV_TEXT`: the provision of the law that applies |
| `consent` | `registry` (an external consent registry, whose question names the responsible professional) or `local`: the source of the consent a read is conditioned on; a read must then carry its consent decision reference, and the access evaluator refuses one that carries none with `403 refused-consent-reference-missing` |
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
  is an `[auth.issuer.<name>]` of kind `federation`;
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
  least as long as the period of the access policy's `log_retention`, and its issuer's
  `[auth.issuer.<name>.federation]` table maps `correlation_id` from the token, never from
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

A composer that names an `ORGANISATION` is `422`: the composer is a person.
Which templates a client commits is a [controller rule](#controller-rules),
as it is for every other caller: a deny rule whose `applies_to` names the
client's `AGENT`, with a permit exception listing the templates it may write.

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
`DV_CODED_TEXT` `no_subject_namespace` of `openehr_access`, for the
import, the extract import and the export alike. That value covers exactly
those EHRs: a grant naming real namespaces does not reach them, and the
code reaches no EHR whose subject names a namespace. A grant that names no
`subject_namespace` at all is still refused at its commit with `422`.

```json
{ "_type": "ELEMENT", "archetype_node_id": "at0023",
  "name": { "_type": "DV_CODED_TEXT", "value": "Subject namespace",
    "defining_code": { "_type": "CODE_PHRASE",
      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
      "code_string": "subject_namespace" } },
  "value": { "_type": "DV_CODED_TEXT", "value": "No subject namespace",
    "defining_code": { "_type": "CODE_PHRASE",
      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" },
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
is a `openehr_access` code, the acts-for relationship holds
`openehr_access::acts_for` in its details, and the `AGENT` holds a
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
The privileged `ROLE` also ships as `1.0.1`, which adds the kind `Compose for`
(`at14`) for a `compose_for` grant and drops the kind `Identifier reader`
(`at12`), which grants nothing; `1.0.0` stays as published, and a `ROLE`
committed against it still validates.
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

**The contract.** The `openehr_access` codes and the `client_id`
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
| `PARTY_RELATIONSHIP.acts_for` | Acts-for code, `openehr_access::acts_for` | `/details[id3]/items[id6]` | `DV_CODED_TEXT` |
| every policy `ROLE` | Kind of grant (the capability's `name`) | `/capabilities[id6]/name` | `DV_CODED_TEXT` |
| every policy `ROLE` | Operation, a `openehr_access` code | `/capabilities[id6]/credentials[id8]/items[id90]` | `DV_CODED_TEXT` |
| `ROLE.system_client` | Template written, named `template` | `/capabilities[id6]/credentials[id8]/items[id9]` | `DV_TEXT` |
| `ROLE.system_client` | Facility, named `facility`: issuer, identifier, identifier prefix, named `issuer`, `identifier`, `identifier_prefix` | `/capabilities[id6]/credentials[id8]/items[id10]/items[id11]`, `[id12]`, `[id13]` | `DV_TEXT` |
| `ROLE.system_client` | Device identifier issuer, named `device_identifier_issuer` | `/capabilities[id6]/credentials[id8]/items[id14]` | `DV_TEXT` |
| `ROLE.system_client` | Not before, not after, authorisation reference | `/capabilities[id6]/credentials[id8]/items[id15]`, `[id16]`, `[id17]` | `DV_DATE_TIME`, `DV_DATE_TIME`, `DV_TEXT` |
| `ROLE.system_client` | Origin, named `origin`: `relay` or `device` | `/capabilities[id6]/credentials[id8]/items[id18]` | `DV_CODED_TEXT` |
| `ROLE.system_client` | Subject namespace, named `subject_namespace` | `/capabilities[id6]/credentials[id8]/items[id19]` | `DV_TEXT` |
| `ROLE.federation_partner` | Kind (accessor) | `at10` person, `at11` organisation | |
| `ROLE.federation_partner` | Agreement name, version, exchange | `/capabilities[id6]/credentials[id8]/items[id9]`, `[id10]`, `[id12]` | `DV_TEXT` |
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
| `ROLE.privileged` | Kind (privilege) | `at10` subject audit, `at11` access service, `at13` import, `at14` compose for (`1.0.1`); `1.0.0` also lists `at12` identifier reader, which grants nothing | |
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
named by its `openehr_access` code. A system that also registers
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
          "value": { "_type": "DV_CODED_TEXT", "value": "Commit composition", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "commit_composition" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id18",
          "name": { "_type": "DV_CODED_TEXT", "value": "Origin", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "origin" } },
          "value": { "_type": "DV_CODED_TEXT", "value": "Relay", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "relay" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9",
          "name": { "_type": "DV_CODED_TEXT", "value": "Template written", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "template" } },
          "value": { "_type": "DV_TEXT", "value": "openEHR-EHR-COMPOSITION.report-result.v1" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id10",
          "name": { "_type": "DV_CODED_TEXT", "value": "Facility", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "facility" } },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id11",
              "name": { "_type": "DV_CODED_TEXT", "value": "Issuer", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "issuer" } },
              "value": { "_type": "DV_TEXT", "value": "urn:oid:2.16.528.1.1007.3.3" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id12",
              "name": { "_type": "DV_CODED_TEXT", "value": "Identifier", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "identifier" } },
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
          "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "acts_for" }
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
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "query_system_wide" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Access demographic",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "access_demographic" }
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
          "value": { "_type": "DV_CODED_TEXT", "value": "Import EHR", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "import_ehr" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Import EHR extract", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "import_ehr_extract" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": { "_type": "DV_CODED_TEXT", "value": "Export EHR extract", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "export_ehr_extract" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id19",
          "name": { "_type": "DV_CODED_TEXT", "value": "Subject namespace", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "subject_namespace" } },
          "value": { "_type": "DV_TEXT", "value": "nl.legacy-his.patients" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id15",
          "name": { "_type": "DV_CODED_TEXT", "value": "Not before", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "not_before" } },
          "value": { "_type": "DV_DATE_TIME", "value": "2026-12-05T22:00:00Z" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id16",
          "name": { "_type": "DV_CODED_TEXT", "value": "Not after", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "not_after" } },
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
`openehr_access` code, which is how FerroEHR reads it in any archetype
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
          "value": { "_type": "DV_CODED_TEXT", "value": "Federated person access", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "federated_person_access" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9",
          "name": { "_type": "DV_CODED_TEXT", "value": "Agreement name", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "agreement_name" } },
          "value": { "_type": "DV_TEXT", "value": "Regional care network exchange" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id10",
          "name": { "_type": "DV_CODED_TEXT", "value": "Agreement version", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "agreement_version" } },
          "value": { "_type": "DV_TEXT", "value": "2.1" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id12",
          "name": { "_type": "DV_CODED_TEXT", "value": "Exchange", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "exchange" } },
          "value": { "_type": "DV_TEXT", "value": "patient summary" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id13",
          "name": { "_type": "DV_CODED_TEXT", "value": "Purpose", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "purpose" } },
          "value": { "_type": "DV_CODED_TEXT", "value": "treatment", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_ActReason" }, "code_string": "TREAT" } }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id14", "name": { "_type": "DV_TEXT", "value": "Legal ground" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id15",
              "name": { "_type": "DV_CODED_TEXT", "value": "GDPR Article 6(1) point", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "gdpr_art_6_1" } },
              "value": { "_type": "DV_TEXT", "value": "e" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id16",
              "name": { "_type": "DV_CODED_TEXT", "value": "GDPR Article 9(2) point", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "gdpr_art_9_2" } },
              "value": { "_type": "DV_TEXT", "value": "h" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id17",
              "name": { "_type": "DV_CODED_TEXT", "value": "National provision", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "national_provision" } },
              "value": { "_type": "DV_TEXT", "value": "the provision the agreement names" }
            }
          ]
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id18",
          "name": { "_type": "DV_CODED_TEXT", "value": "Consent source", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "consent" } },
          "value": { "_type": "DV_CODED_TEXT", "value": "Local", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "local" } }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id19",
          "name": { "_type": "DV_CODED_TEXT", "value": "Read scope", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "read_scope" } },
          "value": { "_type": "DV_TEXT", "value": "patient/composition-openEHR-EHR-COMPOSITION.patient_summary.v1.r" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id24", "name": { "_type": "DV_TEXT", "value": "Caller rate" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id25",
              "name": { "_type": "DV_CODED_TEXT", "value": "Requests per second", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "requests_per_second" } },
              "value": { "_type": "DV_COUNT", "magnitude": 5 }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id26",
              "name": { "_type": "DV_CODED_TEXT", "value": "Burst", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "burst" } },
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
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "read_subject_audit" }
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

#### A scribe composing for physicians

A scribe records the notes the physicians of the same provider dictate. The
grant is a `ROLE` of the privileged template, version `1.0.1`, whose kind
`at14` is `Compose for`, performed by the scribe's `PERSON`
`3e7a1d5c-9b2f-4c8e-a6d0-1f4b8e2c7a95`. Each `COMPOSITION` the scribe
commits names the physician as its composer, and the committer is the scribe.

<!-- policy-example: compose-for-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.privileged.v1.0.1" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "Privileged role" },
  "identities": [{
    "_type": "PARTY_IDENTITY", "archetype_node_id": "id2", "name": { "_type": "DV_TEXT", "value": "Role name" },
    "details": {
      "_type": "ITEM_TREE", "archetype_node_id": "id3", "name": { "_type": "DV_TEXT", "value": "Name" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id4", "name": { "_type": "DV_TEXT", "value": "Full name" },
        "value": { "_type": "DV_TEXT", "value": "Medical scribe" }
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
      "_type": "DV_CODED_TEXT", "value": "Compose for",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at14" }
    },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [{
        "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
        "value": {
          "_type": "DV_CODED_TEXT", "value": "Compose for",
          "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "compose_for" }
        }
      }]
    }
  }]
}
```

Allow the operation by name: `[authz.grants] allowed = ["commit_composition", "compose_for"]`.

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
- `admit_group`: the groups, read from the `group_claim` of the issuer's
  party table, one of which an admitted login holds;
- `admit_role`: the roles, read from `[auth] role_claims`, one of which an
  admitted login holds;
- `admit_assurance`: the lowest `[authz.assurance]` level an admitted login
  carries;
- `admit_guests`: true to admit a guest the issuer's `[auth.issuer.<name>.party]`
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
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "create_at_login" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "at0013",
          "name": {
            "_type": "DV_CODED_TEXT", "value": "Login issuer",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "login_issuer" }
          },
          "value": { "_type": "DV_URI", "value": "https://login.microsoftonline.com/0f1e2d3c-4b5a-4968-8776-655443322110/v2.0" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "at0014",
          "name": {
            "_type": "DV_CODED_TEXT", "value": "Admitted group",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "admit_group" }
          },
          "value": { "_type": "DV_TEXT", "value": "5f2a9c1e-0000-4000-8000-00000000c11a" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "at0015",
          "name": {
            "_type": "DV_CODED_TEXT", "value": "Admitted assurance",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "admit_assurance" }
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

## Caller facts: roles and care relationships

What FerroEHR stores and decides on, and what it leaves to your applications
and your deployment, is listed in
[What the CDR leaves to the application](application-boundary.md).

Beside its grants, a caller carries the facts the access evaluator reads:
the roles it performs, the care teams it belongs to and the providers it
acts for. A patient carries its care relationships. FerroEHR reads them in
any archetype (a membership by HL7 v3 `RoleClass` `MBR`, care by
`openehr_access` `under_care_of`), never by an archetype id, and holds them
on every replica beside the grants, so a request reads them with no
transaction of its own.

**Roles.** A `ROLE` whose `name` is a `DV_CODED_TEXT` of the terminology
your jurisdiction names, such as a national professional role code, is a
role its `performer` holds while the `ROLE` is in force: its `time_validity`
holds the date of the request and it is not deleted. The role's type is its
name (RM Demographic IM, `PARTY.type()`), so a nurse is one `ROLE` per nurse,
each named by the nurse code. A [controller rule](#controller-rules) names a
role in an `applies_to` entry, compared by terminology and code. A `ROLE`
named in text, or by a code of `openehr_access`, names no role. A
token's role claims never count for a rule: they decide only the coarse RBAC
class gate ([Security](security.md)). The one exception is a caller admitted
under a [federation agreement](#federation-agreements): the professional
roles its token attests in `professional.hp_professional_role` are its roles,
each a code of the system the value names, because no `ROLE` of this store
describes a partner's professional. Which sensitivities a caller may read
and which purposes it may declare are rules of the access policy too, never
elements of a capability. A SCIM client writes these `ROLE`s from a User's
`roles`, within the roles its grant admits
([SCIM roles](installation/config-auth.md#roles)), so your identity
provider assigns roles and the access policy says what each may do.

**Organisation types.** An `ORGANISATION` whose `name` is a `DV_CODED_TEXT`
of the terminology your jurisdiction names, such as a national care
provider type, is of that type, read as a `ROLE`'s name is. A system client
acting for that provider acts for an organisation of that type, and a
caller admitted under a federation agreement acts for one of each type its
token attests in `provider.facility_type`. A [controller
rule](#controller-rules) names them in an `applies_to` entry's
`organisation_type`, compared by terminology and code.

**Memberships.** A `PARTY_RELATIONSHIP` in a `PERSON`'s `relationships`
whose `name` or `details` carry `MBR` of `HL7_RoleClass` makes the
professional a member of the care team or unit its `target` names, for as
long as its `time_validity` holds. A membership of a party other than a
`PERSON` or an `ORGANISATION`, and one whose target is no party of this
store, is refused `422`.

**Care relationships.** A care relationship is a `PARTY_RELATIONSHIP` of
its own, written at `/demographic/party_relationship`, each versioned with
its own audit details (draft access control package, Care Relationships and
The Relationship Store). Its `name` is the code `under_care_of` of
`openehr_access`, its `source` is the patient's `PERSON`, and its `target`
is the carer: a professional's `PERSON`, a `ROLE` a professional performs, a
care team's `GROUP` or an `ORGANISATION` unit. Its `details` hold elements
named by codes of `openehr_access`:

| Element | Occurrences | Value |
| --- | --- | --- |
| `care_role` | 1..1 | A `DV_CODED_TEXT`: the part the carer plays in the patient's care, of the value set the access policy binds for `care_role`, or of `HL7_ParticipationType` when it binds none, such as `RESP`, `ATND` or `PART` when the role is not stated |
| `provider` | 1..1 | A `DV_IDENTIFIER` of type `ORGANISATION`: the healthcare provider in whose care the relationship is, by the namespace of its reference and its id, or by the issuer and id of its identifier |
| `covers_for` | 0..1 | A `DV_IDENTIFIER` in the provider's form: the carer the target replaces, its `issuer` the namespace of the carer's reference, its `id` the carer's container and its `type` the carer's kind (`PERSON`, `ROLE`, `GROUP` or `ORGANISATION`), never the target itself |
| `episode` | 0..1 | A `DV_IDENTIFIER`: the episode of care the relationship is scoped to, by the `issuer` and `id` the care delivery environment gives it |
| `basis` | 0..1 | `assigned` (the default); `self_declared` where the policy sets `self_declared_care` ([Writing a care relationship](#writing-a-care-relationship)) |

A caller holds a live care relationship when the relationship's `target`
names its `PERSON` or a `ROLE` it performs (the `direct` path), or a care
team or unit it is a member of (the `team` path), or, for a `self_declared`
relationship, its own `PERSON` (the `self_declared` path), the access
declares no purpose or one of the policy's `care_purposes`, and it acts for
the relationship's `provider`: a bound professional acts for the provider the party binding
names (`[authz.party_binding.provider]`, or the table of its tenant when the
binding names one per tenant), and a system client for the `ORGANISATION`
its `AGENT` acts for, matched by that organisation's own reference (its
namespace and id) or by its identifier (issuer and id), and for the
`GROUP`s that provider is a member of. Its `time_validity` is the period of the care: the end is the
upper bound, and the relationship stays, so the evaluator can apply a grace
period after it. A relationship without a `care_role` or a `provider`, with a
role outside its value set, or with a target of another kind is refused
`422`, as is one whose `provider` names no `ORGANISATION` of the store by
the identifier it carries, or whose `covers_for` names no party of its
`type`, names the target, or lacks its issuer, container id or type. A care relationship held in a `PERSON`'s `relationships` is refused
`422`, naming `/demographic/party_relationship`. The default definitions ship
`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.member_of.v1.0.1` and
`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.under_care_of.v2.0.0` as examples
([Default demographic definitions](installation/default-demographic-definitions.md)).

**Placing a composition in an episode of care.** A COMPOSITION names the
episodes of care it belongs to with an `ELEMENT` anywhere in its
`context.other_context` whose `name` is the code `episode` of
`openehr_access` and whose value is the episode's `DV_IDENTIFIER` (`issuer`
and `id`), as the care delivery environment identifies it. FerroEHR keeps
the episodes of each composition's current version. A directive, a rule of
your access policy, a classification or a delegation names an episode's
data with `episodes` in its `ACCESS_DATA_SCOPE`, each a `DV_IDENTIFIER`
compared by `issuer` and `id`: a patient's restriction of one episode hides
that episode's compositions and nothing else, and data placed in no episode
match no entry. A persistent composition (`openehr::431`) that names an
episode, and an `episode` element without a `DV_IDENTIFIER` carrying an
issuer and an id, are refused `422` on a commit, a CONTRIBUTION, an EHR
Extract import and an archive load. So is a composition placed in an episode
that no care relationship of its EHR's subject names, live or ended: write
the relationship with its `episode` before the data placed in it. An import
is judged against the relationships this CDR holds and those in the
extract's demographics chapter, an archive load against those it holds and
those in the archive. Your template must admit the element, for example in
a CLUSTER of an `other_context` slot.

A `relationship_validity` entry of the access policy with
`episode_required` set refuses `422` a care relationship of the roles and
provider types it covers that names no `episode`.

**Group memberships of a provider.** A `PARTY_RELATIONSHIP` in an
`ORGANISATION`'s `relationships` carrying `MBR` of `HL7_RoleClass` makes that provider a
member of the `GROUP` its `target` names, such as the providers of a
regional health network, for as long as its `time_validity` holds. A system
client acting for the `ORGANISATION` then acts for the `GROUP` too, so a
consent permit naming the regional `GROUP`, such as one copied from the
region's IHE BPPC opt-in policy, admits it. FerroEHR resolves one level: a
`GROUP` that is itself a member of another `GROUP` adds nothing. An
`ORGANISATION` carrying `under_care_of` is refused `422`. The same
`write_care_relationship` rule guards a change of an `ORGANISATION`'s
memberships.

**Who writes them.** The care delivery environment does, as openEHR's
security overview asks: "the evaluation of a role category such as 'care
deliverer' into particular identities ... must be done in each care
delivery environment, not in the EHR" (BASE Architecture Overview, §The
Problem of Roles). A care relationship, its change or its deletion, and a
version of a `PERSON` that adds, changes or removes a membership, commits
only for a caller holding a `write_care_relationship` grant that covers it,
such as your hospital's ADT system, or for an administrator; anyone else is
refused `403`. A version that leaves the memberships as they were needs no
grant. The grant covers what its elements name (draft access control
decision procedure, Writing Relationships):

- **A care relationship** is covered when one `provider` element names its
  `provider` and one `target_kind` element names the RM type of its target.
- **A membership** is covered when one `provider` element names a provider
  of the `ORGANISATION` it targets, or of the `ORGANISATION` the `GROUP` it
  targets belongs to. A `GROUP` belongs to an `ORGANISATION` by its own
  `MBR` relationship to it; a `GROUP` that belongs to none, such as a
  region, takes memberships from an administrator alone.

A grant with no `provider` or no `target_kind` element writes nothing it
would need them for. The operator grants one with typed flags:

```sh
ferroehr party-binding grant --issuer https://machines.example --client-id adt \
  --operation write_care_relationship \
  --provider-issuer urn:oid:2.999.7 --provider-id sint-example \
  --element target_kind=GROUP --element target_kind=PERSON
```

**A writer never admits itself.** Each caller fact and each row of the care
projection names the party that wrote it: the `PERSON` of a professional or
the client `ACTOR` of a system, as the commit audit's committer names it in
`external_ref`. A caller is never admitted on a care relationship or a
membership that it, or a party among its principals, wrote, so a
professional who rosters themselves onto a ward reads nothing through it.
The one exception is a `self_declared` care relationship where the policy
allows it ([Writing a care relationship](#writing-a-care-relationship)).
FerroEHR compares what each relationship says (its code, target, role,
provider and validity), never its position in `relationships`, so removing
another relationship before it changes none.

The care delivery environment reaches FerroEHR on two channels, and a
deployment uses either or both:

- **The ADT or rostering system** writes memberships (`MBR` of `HL7_RoleClass`) and `under_care_of`
  through the Demographic API. On the `PERSON` of a SCIM User, which the
  party binding protects, a version whose only change is to its care
  relationships commits under the grant; any other change it makes stays
  `403`, so the ADT system rosters a professional and touches nothing the
  identity provider owns. It reads the `PERSON` to write it, so it sees the
  professional's binding key and SCIM attributes.
- **The identity provider** marks a SCIM Group as a care team
  ([Care teams provisioned over SCIM](#care-teams-provisioned-over-scim)).

The two never undo each other: a SCIM Group change touches only
memberships of the membership archetype, and the ADT system's relationships
are of its own archetype.

**When a change applies.** An admission, a transfer and a discharge decide
the next request on every replica. The commit stamps the patient whose facts
changed, and each replica reads only the facts of the parties stamped since
its copy, in one indexed read.

No openEHR specification gives a role or a relationship an access meaning:
reading a `ROLE`'s coded name as the role a rule names, reading
relationships as care relationships and group memberships, the
`write_care_relationship` rule, and holding them in memory are FerroEHR's
own design.

### The care relationship resources

These resources are an extension of the openEHR REST API: ITS-REST 1.1.0
has no resource for a relationship, so FerroEHR serves them under the
extensions root (`/ferroehr/rest/extensions/v1`), after the draft access
control package (ITS-REST Resources, Care Relationships). Each is the
`under_care_of` `PARTY_RELATIONSHIP` above in a narrower form, so a write
needs the same `write_care_relationship` grant and commits a version with its
audit details in a contribution.

| Resource | What it does |
| --- | --- |
| `GET /care_relationship?subject_id=&subject_namespace=` | The care relationships of the subject that reference names, live ones by default; `at=YYYY-MM-DD` the day they are live on, `all=true` ended ones too |
| `GET /demographic/person/{uid_based_id}/care_relationship` | The same, for the subject this `PERSON` is |
| `POST` on either | Adds a relationship whose `source` is that subject's `PERSON`: `201` with `ETag` and `Location`; `422` for a body that is no care relationship of that subject |
| `GET /care_relationship/{relationship_uid}` | One relationship, its current version or the version the uid names; `204` once it is deleted |
| `PUT /care_relationship/{relationship_uid}` | Changes the role or the period, with `If-Match` naming the current version (`400` without it, `412` with `ETag` naming the current one when it is stale); closing `time_validity` ends it |
| `DELETE /care_relationship/{relationship_uid}` | Removes one written in error, with `If-Match`; its earlier versions keep it |
| `GET /versioned_care_relationship/{relationship_uid}`, `.../revision_history`, `.../version/{version_uid}` | The versioned object, its revision history with the audit details of each version, and one version |
| `GET /care_relationship?carer={uid}` | The relationships that reach a carer's party, `role=terminology_id\|code` and `path=direct` or `team` to narrow them, `at` as above, paged by `offset` and `fetch` (100 by default, at most 1000) |

That a person is under the care of a given professional or service is data
concerning health (GDPR Art 4(15), Art 9), so every read is decided per
subject it reveals. A relationship is served only to a caller who may read
the subject's record, judged as a read of each of the subject's EHRs, or to a
caller who writes care relationships for the relationship's provider (an
administrator, or a `write_care_relationship` grant naming that provider). A
list leaves every other relationship out, and a read of one answers `404`, so
its role and context never reach that caller. A subject with no EHR has no
record to read, so only a writer for the provider sees its relationships.
Each read writes one access record per subject it reveals: an access-log
entry on the subject's EHR, or, for a subject with no EHR, an access record
naming its `PERSON`. The resources carry the Demographic API's SMART
requirement: a client granted `access_demographic`, or the product scope.

```sh
curl "$BASE/ferroehr/rest/extensions/v1/demographic/person/$PATIENT/care_relationship" \
  -H "Authorization: Bearer $ADT_TOKEN"
```

The resources, the per-subject decision and the writer's exception are
FerroEHR's own design over its draft access control package; no openEHR
specification defines them.

### Care teams provisioned over SCIM

A SCIM Group is a care team when its client creates it with
`careTeam = true` in FerroEHR's Group extension
`urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:Group`. RFC 7643
§4.2 leaves what a Group means to the service provider, and the openEHR
Demographic IM names "the specialist care team, e.g. cardiology team" as
the typical `GROUP`, so this reading is FerroEHR's own design. The Demographic
IM makes a party's type its name (`PARTY.type()`, invariant
`Type_valid: type = name`, and `PARTY_RELATIONSHIP.type()` likewise), so its
`GROUP` is named by the coded `care_team` and each member's membership by the
coded `MBR` of `HL7_RoleClass`, and the evaluator counts the member in the team's care on
the next request. FerroEHR reads the kind by that code, never by the template
or a path. `careTeam` is immutable (RFC 7643 §2.2): a
Group never changes kind with its members in it.

Creating a care-team Group, and every membership change of one, needs the
client's `write_care_relationship` grant in the CDR beside its
`scim_provision` grant; without it the request is refused `403` naming the
grant. The Group belongs to the provider the client acts for: its `GROUP`
carries an `MBR` relationship to the client's `acts_for` `ORGANISATION`,
written when the Group is created, so the grant's `provider` element names
that provider's identifier. A client that acts for no provider creates no
care-team Group (`403`). A membership from SCIM has no `time_validity`, since RFC 7643 has
none: it holds until the client removes the member or the Group. Plain
Groups, with `careTeam` left out or `false`, carry no care relationship and
need no grant.

### Deprovisioning a team member

A SCIM client deactivates or deletes a professional who holds care
relationships without the `write_care_relationship` grant. Revoking access
is never blocked: removing a leaver's access rights is a control that has to
work at once (ISO/IEC 27001:2022 Annex A 5.18), and neither request changes
what the care relationships say.

- **Deactivation** (`active = false`, RFC 7643 §4.1.1) writes an `inactive`
  version of the `PERSON` that keeps every care relationship, whoever wrote
  it: the ADT system's rosters and the memberships of care-team Groups. Plain
  Group memberships are dropped ([Groups](installation/config-auth.md#groups)).
  **Reactivation** (`active = true`) restores the login, and the care
  relationships are as they were.
- **Deletion** writes a logical `deleted` version of the `PERSON` and its
  professional `ROLE`s (RM common §Logical Deletion). The earlier versions
  stay, with the teams the professional belonged to, so the record still
  answers "was this nurse on Ward 3 on that date". The User answers `404`
  from then on (RFC 7644 §3.6). SCIM never erases the identity; only the
  erasure process does, under the exemptions of GDPR Art. 17(3).

The care check compares what each care relationship says (its kind, target
and `time_validity`), and each record subject the `PERSON` declares, in the
new version with the last version that holds a body. A lifecycle state is no relationship content, and a deletion holds no
body, so neither needs the grant.

The login is refused on its next request on every replica, because each
request reads the binding with no cache: an inactive `PERSON` with
`refused-inactive`, and a login whose key only a deleted `PERSON` carried
with `refused-deprovisioned`. The access evaluator never sees the caller, so
the care relationships it would read do not matter.

The ADT system writes no version of a deactivated `PERSON`: an `inactive`
version takes no update (RM common master06 §Version Lifecycle: editing
resumes only through `reactivate` or `retrieve`), and a roster cannot change
the lifecycle state, so such a write is refused `403` and the `PERSON` stays
`inactive`. That costs nothing: a deactivated person's memberships decide no
access, since the login is refused at binding. A leaver's roster is cleaned
up by the SCIM deletion, a logical `deleted` version that ends every
relationship while the earlier versions keep the history (master06
§Logical Deletion), or by the ADT system after the User is reactivated.
Apart from the cited lifecycle rules, this section is FerroEHR's own design.

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

A representation is a `PARTY_RELATIONSHIP` of its own, written at
`POST /demographic/party_relationship` (FerroEHR's extension), from the
representative's `PERSON`, its source, to the subject's `PERSON`, its
target. Its `name` is `openehr_access::represents`, and its `details` hold
the elements of the `representation element` group, each named by its code:

| Element | Occurrences | Value |
|---|---|---|
| `basis` | 1..1 | `DV_CODED_TEXT`: why the representative may act for the subject, a code of the value set the access policy binds for `representation_basis`, or of `HL7_RoleCode` (such as `GUARD`, `RESPRSN`, `DELEGATEE`) when it binds none |
| `provider` | 1..1 | `DV_IDENTIFIER` of type `ORGANISATION` with an `issuer` and an `id`: the healthcare provider for which the representation is recorded |
| `evidence` | 0..1 | `DV_IDENTIFIER`: the court order, register entry or written authorisation |

Any archetype may model it; FerroEHR reads the codes, never the archetype
id. It may also name how the two are related with a `DV_CODED_TEXT` of the
openEHR `subject_relationship` group (`254|parent|`, `264|guardian|`), which
the committer and the access log carry. The shipped
`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v2` is an example.
A representation held in a `PERSON`'s `relationships` is refused `422`,
naming the resource to write it at, and so is one without a readable
`basis` or `provider`, or with a `basis` outside its value set. Each
representation is versioned on its own, with the commit audit of whoever
wrote it, and is ended by closing its `time_validity`.

It holds while its `time_validity` covers today's date in UTC. These
callers write one:

- an administrator;
- a caller holding a `write_representation` grant whose `provider`
  elements name the representation's provider, before and after the write;
  a grant that names no provider writes none;
- the patient it represents, on the basis `HL7_RoleCode::DELEGATEE` alone:
  the patient authorises a proxy of their choice (EHDS Art. 4(2)). Any other
  basis, such as a parent's or a court-appointed guardian's, is written by an
  administrator or a `write_representation` holder.

FerroEHR admits a representative only on a representation that is live on
the day of the request, whose `provider` is the provider the deployment acts
for (`[authz.party_binding.provider]`, or the table of the tenant the token
names), and that the representative did not write itself. A representative
whose party source reports a representation the CDR does not store is
refused `403` with `refused-representation-unrecorded`. The access record of
every admitted request names the representation's container, which the
access log's `representation` authorisation carries as its reference.

The representative's `PERSON` is of a template
`[[authz.party_binding.person.template]]` maps and carries a binding key: a
`DV_IDENTIFIER` whose `issuer` is the token issuer of the patient portal and
whose `id` is the representative's login there (their `sub`). The portal's
own SCIM client provisions it: a `scim_provision` grant whose
`binding_issuer` is the portal's `patients` issuer creates each
representative as a `PERSON` of the patient template, with no professional
`ROLE`, and deactivates or deletes it ([Users](installation/config-auth.md#users)).
SCIM provisions the identity only. The client never writes a
representation: EHDS Art. 4(2) gives the authorisation to the patient (a)
and to the legal representative under national law (b), so the patient, an
administrator or a `write_representation` holder records it.

### Legal capacity

How old a minor must be before their parents no longer see their record, and
when someone may act for an adult, is national law. It reaches FerroEHR as
the representations the deployment records and the `time_validity` it gives
them: the deployment, or the national identity or authorisation service it
follows, ends a parent's representation on the date the child decides
alone, and ends or adds a representation when a court or the patient changes
who may act. FerroEHR checks only that a live representation for its
provider covers today, so a change is effective on the next request, and
holds no age band, no capacity rule, no country table and no archetype id of
its own (draft access control package, The Representation as a Stored
Fact).

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
request is confined to that EHR. A login a live `PERSON` carries is held to
that `PERSON`: it is the patient only when that `PERSON` is the EHR's
subject, a representative only while it holds a live representation of the
subject, and refused otherwise. A representative with no representation yet,
one whose representation ended or was deleted, and one who represents
someone else are all refused, so a former representative never becomes the
patient. When FerroEHR knows who signs in, its own records decide, not the
issuer's claim of which EHR the login names. A login no `PERSON` carries
binds no one: on the first path such an actor is refused, and on the second
path the token is refused `403` with `refused-unbound`, as is a token that
names no `ehrId`. These outcomes are decided at binding, when FerroEHR resolves who
the login is, before the access evaluator decides what that caller may
reach, and each is recorded as a binding outcome: a missing representation
with `403` (`refused-representation-unrecorded`), a representative's
`PERSON` that is `inactive` or `abandoned` with `refused-inactive`, and a
login two `PERSON`s carry with `refused-ambiguous`. The patient themself is
the one whose login is a binding key of the `PERSON` the EHR's subject
names: the party binding is the one source of own access, for a `patients`
issuer as for a `persons` one, and the token's claims are never trusted on
their own. This is FerroEHR's own design.

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
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "represents" } } },
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

Two kinds of operational policy have no home in the openEHR demographic
model, so each is an admin register:

| Register | What it holds | Default, before anyone writes it |
|---|---|---|
| `audit-categories` | The EHDS category map every access is classified through ([the audit category register](installation/config-audit.md#the-audit-category-register)) | The category map FerroEHR ships |
| `access-delay` | The EHDS Art. 3(3) delay of the person's own access ([the access-delay register](installation/config-auth.md#the-access-delay-register)) | Off |

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
  `400` and never reaches a request: a category map that does not compile or
  a delay with an unusable limit.
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

A deployment provides its registers once the server runs, through the admin
API; neither has a key in `ferroehr.toml`.

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

### The security floor that stays in `ferroehr.toml`

A floor that must hold before a register can be read stays configuration. With
`deployment.profile = "production"` and auditing on, the audit category
register refuses a map that classifies nothing (no template, no archetype and
`shipped_map = false`) as the `no_category_map` gap, unless
`deployment.accepts` names it.

## The access policy

The controller's access policy states what holds for every EHR this FerroEHR
keeps: which kinds of data carry which sensitivity (`classifications`),
whether a professional's clinical access needs a care relationship
(`care_relationship_required`, with a `relationship_grace`) and a declared
purpose (`purpose_required`), the controller's own `rules` on which callers
may do what with which kinds of data (each an `ACCESS_DIRECTIVE` of kind
`controller_rule`, EHDS Art. 11(3)), the `catalogue` of named directives a
patient can accept by reference, such as an IHE BPPC policy, and the
`log_restrictions` of the access log's subject view your Member State's law
lays down under EHDS Art. 9(3) ([Audit](audit.md#restricting-the-subject-view-ehds-art-93)).
It is an `ACCESS_POLICY` of FerroEHR's draft access control package, written
against the default archetype `openEHR-EHR-ACCESS_POLICY.controller.v1`,
which every boot stores, or against one of your own. Break-glass and the default floor
stay in `ferroehr.toml`; the policy can only tighten them. Until you write
one, nothing is tightened.

- **Versioned.** Every write is a new version in its own contribution, with
  you as committer; the history route lists them all.
- **Whole, over a version.** Name the version you replace in `If-Match`, or
  create the first with `If-None-Match: *`. A stale condition is `412`.
- **Checked when written.** A body that is no canonical-JSON `ACCESS_POLICY`
  is refused whole with `400`. A policy that reads but breaks a rule is
  refused whole with `422`, naming the rule or the attribute: an invariant
  of the draft model, a rule of another kind than `controller_rule`, a
  catalogue entry of that kind, a rule naming a role, purpose, sensitivity,
  category or legal ground in `openehr_access`, the template its
  `archetype_details.template_id` names, or a code outside a value set it
  binds.
- **Effective without a restart**, as the registers above.
- **Read by the access evaluator** on every request: `purpose_required` is
  step 4, checked before any rule, the rules are step 5, a classification
  gives the data it covers its sensitivities (step 6), and
  `care_relationship_required` is step 7 of the
  [decision procedure](security.md#access-directives).
  A `relationship_grace` that is no ISO 8601 duration admits nothing after a
  care relationship ends.

The routes are on
[Admin & messaging APIs](operations-admin-apis.md#the-access-policy). A write
needs `admin_role` and a policy grant of `write_access_policy`.

### Care relationship facts

The policy also states how care relationships are read. Each attribute you
leave out reads as its strictest default, so a policy that sets none admits
least.

| Attribute | What it says | Left out |
| --- | --- | --- |
| `self_declared_care` | The longest period of a care relationship a professional states for themself | no self-declared relationship |
| `attested_care` | The agreements under which a caller's verified credential may attest a care relationship | no attested relationship |
| `caseload` | Whether a query may be narrowed to the caller's caseload | `true` |
| `conflict_rules` | The conflicts of interest (`self`, `related`, with the kinships) allowed through the professional path, and for whom | none: a professional who is the patient, or is related to them, is refused |
| `concealed_confidentialities` | The `HL7_Confidentiality` levels whose refused records answer as absent | every refused record answers as forbidden |
| `value_sets` | The value set on your terminology server that each coded attribute (`care_role`, `representation_basis`, `kinship`, `organisation_type`, `care_purpose`, `emergency_purpose`) is bound to | the package's default code system |
| `relationship_validity` | The default and maximum period of a care relationship, by its `care_role` and the type of its provider | every care relationship is written with an end |
| `relationship_writers` | The writers that may write a care relationship to a record marked `R` or `V` | none |
| `team_targets` | The `ORGANISATION` units a care relationship may name as its target | none: a care team is a `GROUP` |

FerroEHR ships no value set and no country content. A code of a bound
attribute is checked against its value set on the
[terminology server](templates-validation.md) when you write the policy;
with no terminology server configured, a policy that binds a value set the
check needs is refused. An unbound kinship is a code of the openEHR
`subject relationship` group. An organisation type has no default code
system, so a rule, an exception or a conflict rule that names an
`organisation_type` is refused `422`, naming it, until the policy binds an
`organisation_type` value set. A template of the default archetype that
fixes some of these attributes for many controllers is a profile: write the
policy with its `template_id`, and FerroEHR validates it against the
template.

Two more attributes say when a relationship is a ground at all:

| Attribute | What it says | Left out |
| --- | --- | --- |
| `care_purposes` | The purposes of use for which a care relationship is a ground: an access declaring another purpose, such as research, rests on no relationship and matches no rule scope naming `care` | `TREAT`, `ETREAT` and `CAREMGT` of `HL7_ActReason` |
| `time_zone` | The IANA time zone in which the dates of a relationship's `time_validity` are read, such as `Europe/Amsterdam`; a name the zone database does not hold is refused `422` | UTC |

#### The attested care path

A caller from another organisation, admitted under a
[federation agreement](#federation-agreements), can hold a care
relationship that is never stored: its verified credential attests a
treatment context for the patient. This is how a national contact point or a
national infrastructure reaches the record for cross-border care. The path is
off until `attested_care` names the agreement:

```json
"attested_care": [
  { "_type": "DV_IDENTIFIER",
    "issuer": "https://federation.example",
    "id": "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f99" }
]
```

- `id` is the container uid of the agreement `ROLE` (the part of its
  version uid before `::`), so a new version of the agreement keeps the
  reference.
- `issuer`, when you set it, must be the federation issuer the partner's
  client identifiers are held under. A policy that names another issuer, or
  names under an issuer an id that is no agreement `ROLE`, is refused `422`,
  naming `/attested_care/<n>/issuer`.

Which claims carry the treatment context is your data, in the issuer's
`[auth.issuer.<name>.federation]` table. FerroEHR reads these fields:

| Field | What it carries |
| --- | --- |
| `patient.id` | The patient, as `system\|value`: the identifier an EHR's subject names |
| `trc.id` | The id of the assertion that attests the treatment context |
| `trc.not_on_or_after` | The end of its window, in seconds since the epoch |
| `trc.not_before` | The start of its window, when the credential states one |
| `assertion.issuer` | The issuer of the assertion, when the credential states one |
| `trc.care_role` | The care role the treatment relationship attests for the professional, one code with its code system |

A credential that lacks `patient.id`, `trc.id` or `trc.not_on_or_after`
attests nothing. On every access FerroEHR checks that the policy names the
agreement, that the window holds the moment of the access, and that the
record's subject is the attested patient. The caller then holds a live care
relationship for the care step, the default disposition and any rule whose
actor scope names `relationship: care`. The relationship is never stored and
never in a caseload. The access log names it as a `treatment relationship`
ground whose `reference` is the assertion (`id` and `issuer`) and whose
`name` and `version` are the agreement's.

The attested relationship holds the role `PART` unless the credential
attests a role in `trc.care_role` of the `care_role` value set the policy
binds (unbound, a code of `HL7_ParticipationType`). The code system of the
attested role is the terminology its code is read in, so a credential whose
role carries the system `HL7_ParticipationType` names a code of that
terminology, and a field mapped as `{ constant = "ATND", system =
"HL7_ParticipationType" }` attests that role for every credential of the
issuer. A role outside that value set, or one no terminology server could
check, counts as no role, and the relationship keeps `PART`.
`ferrohealth-smart` refuses a token whose `trc.care_role` is not one code
with its system.

A rule limits attested callers like any other: give its actor scope
`relationship: care` with `care_path: attested`. A deny rule's `data` keeps
attested callers from the templates or categories it names, and on a record
the controller marks `R` or `V` an attested caller is refused
`refused-confidentiality` until a permit rule for that path names the level
in its `data` `confidentialities`:

```json
{ "_type": "ACCESS_DIRECTIVE", "kind": "controller_rule", "effect": "permit",
  "applies_to": [ { "_type": "ACCESS_ACTOR_SCOPE",
    "relationship": "care", "care_path": "attested" } ],
  "data": [ { "_type": "ACCESS_DATA_SCOPE", "confidentialities": [ "R" ] } ] }
```

The codes are shown short here; write each as a `DV_CODED_TEXT`. A permit
that names `R` clears `R` and nothing above it, and one written for the
`direct` path clears no attested caller.

#### Writing a care relationship

FerroEHR checks each care relationship against the policy when it is
written, and refuses one that breaks a bound with `422`, naming
`/time_validity` or `/target`:

- **Validity.** A relationship written without an end takes the
  `default_period` of the first `relationship_validity` entry that covers its
  role and its provider's type, counted from its start (or from the day of
  the write when it names no start). One longer than that entry's
  `max_period` is refused. One that no entry covers, or whose entry has a
  `max_period` and no `default_period`, must be written with an end. An entry
  with neither period lets the relationship stay open.
- **Target.** An `ORGANISATION` target must be a unit `team_targets` names,
  and is never the controller or the relationship's own provider.
- **Writers.** The level is the highest marking of the subject's records:
  every EHR whose subject reference names the relationship's source
  `PERSON`, directly or through a record subject the `PERSON` declares.
  FerroEHR reads those markings exactly before the commit takes its
  connection. At `N` every writer the `write_care_relationship` grant
  admits writes. At `R` or `V` only a writer a `relationship_writers` entry
  names for that level writes, and its access-log record is flagged for
  review with the level: the `review` detail (`id40`) of the entry's
  [details](audit.md), the `review` column of `audit.access_trail`, and a
  `review` detail on the object of the BALP `AuditEvent` and the DICOM
  audit message. Every other write is refused `403`, naming the level and
  the missing entry. The administrator is no exception: a
  policy that wants the administrator to write such relationships names the
  administrator's `PERSON` (or a role it performs) for the level. An entry
  for `V` does not cover `R`; name each level the writer may write.
- **Self-declared.** A relationship whose `basis` is `self_declared` is
  refused `422` unless the policy sets `self_declared_care`. Its target is
  the writer's own `PERSON` (`403` otherwise), and it ends within that period
  of its start, by its own end or the one `default_period` gives it. While
  the policy sets `self_declared_care`, it admits the professional who wrote
  it; once the policy no longer sets it, it admits no one.

For example, this `relationship_writers` lets the admission system's role
write relationships to records at `R`, and the administrator's `PERSON`
write them at `R` and `V` (the role code and the identifiers are your own):

```json
"relationship_writers": [
  { "_type": "ACCESS_RELATIONSHIP_WRITER",
    "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
      "role": { "_type": "DV_CODED_TEXT", "value": "admission system",
        "defining_code": { "terminology_id": { "value": "urn:example:roles" },
          "code_string": "adt" } } }],
    "confidentialities": [
      { "_type": "DV_CODED_TEXT", "value": "restricted",
        "defining_code": { "terminology_id": { "value": "HL7_Confidentiality" },
          "code_string": "R" } }] },
  { "_type": "ACCESS_RELATIONSHIP_WRITER",
    "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
      "party": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
        "id": { "_type": "HIER_OBJECT_ID", "value": "<the administrator's PERSON>" } } }],
    "confidentialities": [
      { "_type": "DV_CODED_TEXT", "value": "restricted",
        "defining_code": { "terminology_id": { "value": "HL7_Confidentiality" },
          "code_string": "R" } },
      { "_type": "DV_CODED_TEXT", "value": "very restricted",
        "defining_code": { "terminology_id": { "value": "HL7_Confidentiality" },
          "code_string": "V" } }] }
]
```

The end a relationship takes from `default_period` is never written into the
relationship: the version you committed stays exactly as you sent it, with
its digest and signature. FerroEHR computes the end once, in the commit's
transaction, from the policy version the write read, and keeps it beside the
relationship with the uid of that policy version. Every access decision and
query reads that end. A later policy version does not move it: a changed
period applies to the relationships written after the change. To change the
end of a relationship, write a new version of it.

Each write reads the newest policy version from the store before it takes
its connection to the demographic store, because the policy lives in the
clinical store. A policy version committed between that read and the commit
applies from the next write; the kept end names the version it came from.

An archive load (`load_ehrs`) carries no kept ends, so it checks each care
relationship it loads against the policy in force on the server it loads
into, counting a missing start from the day the loaded version was
committed. A relationship the policy refuses is reported in the load's
failure report and is not loaded; the rest of the archive loads. Put the
access policy in place before you load an archive that holds open care
relationships.

### Exclusions and conflicts of interest

**An exclusion keeps a named person out of one record.** It is a deny of
kind `exclusion` in the `exclusions` of the EHR's `EHR_ACCESS` settings,
naming the parties it keeps out by `applies_to` (a party, or the
`representative` relationship), with an optional data scope, validity and
legal grounds. It is decided before step 1: it binds a professional, a
representative and a gate-keeper, never the person, and no permit of the
person or a representative lifts it. A declared emergency lifts it only
where its `emergency_override` holds and the caller is no representative.
The caller is answered `403 refused-excluded`, and the answer names neither
the exclusion nor its reason; the access log names the exclusion and the
`EHR_ACCESS` version. A law that lets a provider decline to treat a person
as a representative, for example in a case of abuse, is carried by one.

Only the controller (a caller holding `admin_role`) reads and writes the
exclusions. Every read of an `EHR_ACCESS` by anyone else, the person and a
gate-keeper included, leaves them out; a version envelope, a contribution
or an AQL row that would carry them reads as absent to such a caller, and
an EHR Extract for the person carries no `EHR_ACCESS` that ever held one. A
version a gate-keeper or the person commits carries the current exclusions
forward unchanged, and one that names other exclusions is refused `403`.

**A restriction binds a representative it names.** A `restriction` the
person gives binds a representative whose `applies_to` names it by party or
by the `representative` relationship, answered as absent, as it is for a
professional. A controller rule over a label that names representatives
refuses them at step 5.

**A conflict of interest refuses the professional path.** A professional is
in conflict with the person when it performs a professional `ROLE` and is
bound to the person's `PERSON` (a staff member on their own record), or
when a `related_to` relationship in force joins its `PERSON` to the
person's. It is refused `403 refused-conflict` at step 5, in a read and in a
query, unless an entry of the policy's `conflict_rules` for that conflict
names it (and, for `related`, the kinship); an access such an entry admits
is marked in the access log with a `conflict allowance` ground. A staff
member reads their own record through the patient portal. Kinship refuses
and never grants: a relative passes the other steps as anyone else.

A kinship is a `PARTY_RELATIONSHIP` of its own named `related_to`, written at
`/demographic/party_relationship` under the `write_care_relationship` grant
or by an administrator, whose source and target are two `PERSON`s and whose
`kinship` element is a code of the value set bound for `kinship` (by default
the openEHR `subject relationship` group). Your demographic service or
master patient index decides who is kin; FerroEHR stores the fact and
decides on it. A query leaves out the EHRs of every person the caller is in
conflict with; a conflict rule that names the `care` relationship admits a
conflict in a direct read of a record the caller cares for, and in a query
reads as not admitting it.

### Controller rules

Regulation (EU) 2025/327 Art. 11(3) asks each Member State to lay down
which categories of health data different categories of health
professionals may reach. The access policy's `rules` hold those rules for
every EHR you keep. Each rule is an `ACCESS_DIRECTIVE` of kind
`controller_rule`, with the `applies_to`, `operations`, `purposes`, `data`,
`time_validity`, `emergency_override`, `exceptions` and `legal_grounds` a
directive has. The `ROLE` parties say who holds a role; the rules say what
the role may do.

- **A deny rule refuses** what it covers with `403 refused-policy-denied`,
  whatever the patient's directives permit (step 5). It binds every caller
  but the patient on their own record, representatives included.
- **A permit rule clears** the callers it names for the sensitivities its
  `data` names (step 6) and decides nothing else.
- **A rule's `purposes` match only a declared purpose.** A request that
  declares none is outside every rule that names purposes. Set
  `purpose_required` to refuse such a request: it is refused
  `refused-purpose-missing` before any rule is read, whatever rules you
  hold.
- **A deny rule with a permit exception says "only"**: the exception carves
  what it covers out of the deny.
- **A scope naming `care` can name care roles and a path.** With
  `relationship` `care`, an actor scope's `care_roles` lists the roles a
  caller's live care relationship must have, compared by terminology and
  code, and `care_path` says how it must reach the caller: `direct`, `team`,
  `self_declared` or `attested`. Left out, any role and the paths the policy
  turns on. No role implies another:
  `RESP` does not cover `ATND`. The same scope decides a read and narrows a
  query inside its one statement, so a `COUNT` agrees with the reads.
- **For a write**, the data are the version being committed: its template,
  root archetype, origins and the sensitivities your classifications give it.

Four rules, as you write them in the policy body (a role code of
`urn:example:roles` stands for your jurisdiction's role terminology):

```json
"rules": [
  { "_type": "ACCESS_DIRECTIVE", "name": { "_type": "DV_TEXT", "value": "Nurses do not record diagnoses" },
    "archetype_node_id": "id15",
    "kind": { "_type": "DV_CODED_TEXT", "value": "controller_rule", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "controller_rule" } },
    "effect": { "_type": "DV_CODED_TEXT", "value": "deny", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "HL7_ConsentProvisionType" }, "code_string": "deny" } },
    "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "role": { "_type": "DV_CODED_TEXT", "value": "Nurse", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "urn:example:roles" }, "code_string": "NURSE" } } }],
    "operations": [{ "_type": "DV_CODED_TEXT", "value": "write", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "write" } }],
    "data": [{ "_type": "ACCESS_DATA_SCOPE", "templates": ["Diagnosis"] }],
    "emergency_override": false }
]
```

- **A nurse may not record a diagnosis:** the rule above.
- **Only psychiatry reads mental-health notes:** classify the mental-health
  templates `PSY` (HL7 v3 `ActCode`), and write a `permit` rule whose
  `applies_to` names the psychiatrist role and whose `data` names the
  sensitivity `PSY`. A patient's consent permit naming their GP clears the
  GP in the same step; everyone else is refused `refused-sensitivity`.
- **Professionals may declare only treatment or emergency treatment:** a
  `deny` rule whose `applies_to` names the caller kind `professional`, with
  one `permit` exception whose `purposes` are `TREAT` and `ETREAT`. A request
  declaring another purpose is refused `refused-policy-denied`. A request
  declaring none is refused `refused-purpose-missing` where the policy sets
  `purpose_required`, and otherwise `refused-policy-denied`, since the
  exception names no request without a purpose.
- **Only the responsible practitioner, directly, signs the discharge
  summary:** a `deny` rule on `write` whose `data` names the discharge
  summary template, with one `permit` exception whose `applies_to` is
  `relationship` `care`, `care_roles` `[RESP]` and `care_path` `direct`. A
  second in chain (`SPRF`), a consultant (`CON`) and a team member are
  refused `refused-policy-denied`; a professional covering for the
  responsible one holds a `RESP` relationship of their own for the period
  and is admitted.
- **A system client writes only its laboratory templates:** a `deny` rule on
  `write` whose `applies_to` names the client's `AGENT` by `party`, with one
  `permit` exception whose `data` lists the templates.
- **Only care providers reach records through the exchange:** a `deny` rule
  whose `applies_to` names the caller kind `access_service`, with one
  `permit` exception whose `applies_to` lists one `organisation_type` per
  care provider type, and the provision in its `legal_grounds`. A partner's
  professional whose token attests an insurer's facility type, or none, is
  refused `refused-policy-denied`. A second `deny` rule naming a `role`, such
  as an occupational physician, or `purposes`, such as a medical
  examination, keeps those out too. A rule naming `purposes` refuses only a
  request that declares one of them, so set `purpose_required` to refuse a
  request that declares none. Wabvpz Art. 15f is a rule of this kind: you commit it,
  FerroEHR holds no copy of it.

A rule whose `emergency_override` is true is lifted for the holder of
`authz.rbac.emergency_access_role` declaring an emergency purpose, and the
access record names the rule. The access record of a refusal names the
rule by its uid, its first identifier or its position in `rules`, and a
`legal obligation` ground carrying the rule's `legal_grounds`. The `403`
problem body carries them in its `legal_grounds` member beside `refusal`,
each with its `terminology`, `code` and `value`, under
`Prefer: return=representation`.

**A federation agreement and the rules.** A federation agreement is the
contract with a partner: its `template` elements say what the partner's
persons may send. The rules are your own limits, and they hold for a
partner's persons too: a deny rule refuses a template the agreement allows,
because a deny overrides.

No openEHR specification has a controller rule: the rules, their place as
step 5 after the purpose check, matching a rule's purposes only on a
declared purpose, reading a role from the coded name of a `ROLE` and an organisation
type from the coded name of an `ORGANISATION`, reading a partner's attested
roles and facility types as the same facts, and naming a refusing rule's
legal grounds as a `legal obligation` ground are FerroEHR's own design,
proposed in the draft access control specification.
