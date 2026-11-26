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
the openEHR demographic model is an [admin register](access-policy.md#the-admin-registers).
FerroEHR stores, validates and protects the policy parties, and every
request resolves its caller's grants from them before any check runs. The
checks that decide on a grant are listed under
[What the grants decide](#what-the-grants-decide) and
[What a system client may do](#what-a-system-client-may-do).

No openEHR specification governs operational policy or where it is kept, and
none gives a party an authorization meaning. The classes are the openEHR
Reference Model's (RM Demographic Information Model); the design on this page
is FerroEHR's own, and the points that are its own for the policy parties are
under [Own design](#own-design). How the policy parties, the access policy
and the patient's settings decide a request together is on [How FerroEHR
decides access](explanation/access-model.md).

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
- the Basic users as credentials alone, `user` with its password hash, since
  a secret never becomes versioned openEHR data, and `[authz.rbac]
  first_admin`, the one Basic user that holds `admin_role` and the product
  scope from the file: the first administrator, who writes the first policy
  parties;
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
issuer, in an `ELEMENT` whose `name` is `openehr_access::client_id`: the
identifier's `issuer` is the issuer URL and its `id` the `client_id`.

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
- an `ELEMENT` named `openehr_access::client_id` holding a `DV_IDENTIFIER`,
  which binds an actor to an OAuth 2.0 client. A `DV_IDENTIFIER` whose
  `type` is the text `client_id`, in an element of your own naming, is your
  own data: it marks nothing and binds no client.

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
| `create_at_login` | Create a provisional `PERSON` at the first login of a person no party carries, under the admission conditions the grant names ([Just-in-time admission](policy-examples.md#just-in-time-admission)) |
| `query_system_wide` | Count the client's `system/aql-*` scope, which reaches every query over every EHR (SMART on openEHR, master08 §Resource Scopes) |
| `access_demographic` | Reach the DEMOGRAPHIC API, which no SMART on openEHR scope covers |
| `write_care_relationship` | Write the membership (`member_of`) and `under_care_of` care relationships through the Demographic API, as a hospital's ADT system does ([The access control terminology](#the-access-control-terminology)) |
| `write_representation` | Write a representation (`represents`) through the Demographic API, for the providers its `provider` elements name ([Representatives](caller-facts.md#representatives)) |
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
| `represents` | On a `PARTY_RELATIONSHIP`: the source represents the target, the subject, on the subject's own record ([Representatives](caller-facts.md#representatives)); it makes no policy party |
| `role` | A `DV_TEXT`: a role a `hold_roles` grant gives, or a role code a `scim_provision` grant lets its client assign its Users ([SCIM roles](installation/config-scim.md#roles)) |
| `scope` | A `DV_TEXT`: a SMART scope a `hold_scopes` grant gives |
| `login_issuer` | A text or `DV_URI`: a token issuer whose first logins a `create_at_login` grant admits |
| `admit_idp` | A text: an identity provider an admitted login comes from |
| `admit_group` | A text: a group an admitted login holds |
| `admit_role` | A text: a role an admitted login holds |
| `admit_assurance` | A text, `low`, `substantial` or `high`: the lowest assurance level an admitted login carries |
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
`GROUP`), of any archetype, that carries, anywhere in its `details` or
`identities`, an `ELEMENT` whose `name` is the `DV_CODED_TEXT`
`openehr_access::client_id` and whose value is a `DV_IDENTIFIER` whose
`issuer` is the token issuer and whose `id` is the OAuth 2.0 `client_id`.
That element makes the party a policy party, so only an administrator's
write binds the client. A `DV_IDENTIFIER` merely typed `client_id` in an
element of your own naming is plain data: FerroEHR neither marks the party
nor binds a client from it. One party names a client:
a second party carrying the same issuer and `client_id` is refused with
`409`.

**Whose grants.** Every authenticated request resolves its caller's grants
once: a system client's client credentials token, and a verified external
caller of a `federation` issuer, by its issuer and `client_id`; a
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
and `HL7_SecurityRoleType`
(`http://terminology.hl7.org/CodeSystem/extra-security-role-type`). A care-team
membership is the exception: it is `member_of` of `openehr_access`, and
`MBR` of `HL7_RoleClass` (`http://terminology.hl7.org/CodeSystem/v3-RoleClass`)
is only its broader mapping, so a relationship your own archetypes code
`HL7_RoleClass::MBR` keeps its HL7 meaning and FerroEHR attaches no access
rule to it. The groups below are the stable contract: the access evaluator and the access
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
| `care relationship` | `member_of`, `under_care_of` | The `name` or `details` of a `PARTY_RELATIONSHIP`: with `member_of`, the source, a professional's `PERSON`, is a member of the target, a care-team `GROUP`, or the source, a provider `ORGANISATION`, is a member of the target `GROUP`; with `under_care_of`, the source, a patient's `PERSON`, is under the care of the target, a team `GROUP` or an organisational unit |
| `representation` | `represents` | The `name` of a `PARTY_RELATIONSHIP` of its own: the source represents the target, the subject ([Representatives](caller-facts.md#representatives)) |
| `representation element` | `basis`, `provider`, `evidence` | The `name` of an `ELEMENT` in the `details` of a `represents` relationship |
| `kinship`, `kinship element` | `related_to`; `kinship` | The `name` or `details` of a `PARTY_RELATIONSHIP` of its own: its source and target, two `PERSON`s, are kin, and its `kinship` element codes how ([Exclusions and conflicts of interest](access-policy.md#exclusions-and-conflicts-of-interest)) |
| `client identifier` | `client_id` | The `name` of an `ELEMENT` holding a `DV_IDENTIFIER` in an `ACTOR`'s `details` or `identities`: the identifier is the OAuth 2.0 client the actor is ([Who acts](#how-a-request-reads-its-grants)) |
| `group kind` | `care_team` | The `name` of a `GROUP`: the `GROUP` is a care team ([Care teams provisioned over SCIM](caller-facts.md#care-teams-provisioned-over-scim)) |
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
on ([The openEHR access-log entry](access-log.md#the-openehr-access-log-entry)).

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

**Which clients a person's token comes through.** The authorization
server's registry is the one list of clients: a token whose `aud` names
FerroEHR admits the client its `client_id` names (RFC 9068 §2.2), and
FerroEHR keeps no client list of its own. An RFC 8693 §4.1 `act` names
someone acting for the user, which the access record carries as the
delegated acting mode.

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
([Audit trail](audit-integrity.md#retrieving-audit-records-iti-81)).

**Acting as an access service.** A caller with an `access_service` grant is
an EHDS Art. 10 access service: an EHR whose person opted out reads as
absent to it at every route, query and export, unless it invokes an
emergency justification of the access policy with a reason and acts for no
person (Art. 10(2)).
While `[authz.grants] allowed` does not allow `access_service`, recording an
opt-out is refused with `422`, since it would hold for nobody
([Opt-out from the access services](access-control.md#opt-out-from-the-access-services-ehds-art-10)).

A role a token names never grants any of these four: each comes from its
grant alone, under the grant ceiling. The privileged role of
`[authz.rbac]`, `admin_role`, comes from the
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
deployment whose staff sign in through an issuer bound by an
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
carries a client identifier (an element named `openehr_access::client_id`)
for each client its `federation` issuer issues tokens to, and a `ROLE` it performs holds one capability per accessor.
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

- the partner carries a client identifier, and each identifier's issuer
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

A system client is a client credentials token, whose `sub` is its
`client_id` (RFC 9068 §2.2). It resolves to the
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
Which templates a client commits is a [controller rule](access-policy.md#controller-rules),
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
`openehr_access::acts_for` in its details, and the `AGENT` holds its client
identifier in an element named `openehr_access::client_id`. A party of one of them is a policy party because of
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

**The contract.** The `openehr_access` codes, `openehr_access::client_id`
among them, make a party a policy party, in any archetype, and FerroEHR
reads the terms of a system client's grant by the codes in its elements'
names, never by a node id. The archetype ids and the node ids below describe
the examples, and every template and version built on an archetype keeps
them. A breaking change to them is a new archetype major version (`.v2`),
which FerroEHR does not take for the `.v1` kind. Each grant is a
`CAPABILITY` at `/capabilities[id6]`, its kind the coded `name`, its
credentials at `/capabilities[id6]/credentials[id8]`, and its `time_validity` the day bounds.

| Archetype | Field | Node path | Type |
|---|---|---|---|
| `AGENT.system` | Client identifier (the element's `name` `openehr_access::client_id`, `issuer` the token issuer, `id` the `client_id`) | `/details[id2]/items[id3]` | `DV_IDENTIFIER` |
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
- **Emergency access is a justification of the access policy**, held in
  memory on every replica, so it works while the policy store does not
  answer once the replica has read the policy.
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
