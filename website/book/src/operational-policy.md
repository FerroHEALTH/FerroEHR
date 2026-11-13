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
request resolves its caller's grants from them before any check runs
([Which checks read a grant](#which-checks-read-a-grant)).

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
- the password hashes of the Basic users, since a secret never becomes
  versioned openEHR data;
- the grant ceiling, `[authz.grants] allowed`: the operations the party store
  may grant at all, so a compromised administrator account cannot grant
  more than the operator allows ([The grant ceiling](#the-grant-ceiling)).

Everything else is CDR data: who may do what is party data (below), and the
policy with no demographic home is an admin register.

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
| `hold_roles` | Hold, as a Basic user, the roles its `role` elements name |
| `hold_scopes` | Hold, as a Basic user, the SMART scopes its `scope` elements name |
| `query_system_wide` | Count the client's `system/aql-*` scope, which reaches every query over every EHR (SMART on openEHR, master08 §Resource Scopes) |
| `access_demographic` | Reach the DEMOGRAPHIC API, which no SMART on openEHR scope covers |

The same terminology names the elements that qualify a grant, by the code
in the element's `name`:

| Code | Element |
|---|---|
| `not_before` | A `DV_DATE_TIME`: the instant before which the grant does not hold |
| `not_after` | A `DV_DATE_TIME`: the instant after which the grant does not hold |
| `template` | A template the grant writes |
| `facility` | A facility the grant covers, an element or a `CLUSTER` |
| `subject_namespace` | The `EHR_STATUS.subject.external_ref.namespace` of the EHRs it touches |
| `acts_for` | On a `PARTY_RELATIONSHIP`: the source acts for the target |
| `role` | A `DV_TEXT`: a role a `hold_roles` grant gives |
| `scope` | A `DV_TEXT`: a SMART scope a `hold_scopes` grant gives |

A code of another terminology grants nothing, and a code of
`ferroehr-operations` that the table does not define is refused at commit.
A capability that codes several operations grants each of them.

**Who acts.** A client is any `ACTOR` (`AGENT`, `PERSON`, `ORGANISATION` or
`GROUP`) that carries, anywhere in its `details` or `identities`, a
`DV_IDENTIFIER` whose `type` is `client_id`, whose `issuer` is the token
issuer and whose `id` is the OAuth 2.0 `client_id`. One party names a client:
a second party carrying the same issuer and `client_id` is refused with
`409`.

**Whose grants.** Every authenticated request resolves its caller's grants
once: a system client, a token of a `machines` issuer, by its issuer and
`client_id`; a person the login binding bound to a `PERSON`, by that party;
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

### Which checks read a grant

Each check below reads the grants of the request's caller, beside what the
configuration grants:

- **A Basic user's roles and scopes.** A Basic user bound to a `PERSON`
  holds, beside the roles and scopes of its `[[auth.basic.users]]` table,
  the roles the `role` elements of its `hold_roles` grants name and the
  scopes the `scope` elements of its `hold_scopes` grants name. Every gate
  reads them: the role gate, the SMART on openEHR scopes, ABAC and the
  access record. A token's roles and scopes come from the token alone, since
  its issuer is their authority.
- **Importing.** `message_import_ehr` admits a caller holding
  `[authz.rbac] import_role`, a `system/` caller, or a caller with an
  `import_ehr` grant; `message_import_ehr_extract` the same with an
  `import_ehr_extract` grant.
- **Reading protected identifiers.** `GET /admin/party/identifiers` admits an
  administrator who holds `[authz.rbac] identifier_reader_role` or a
  `read_identifiers` grant.

A grant is the second source of a privileged permission, beside the token
issuers `[authz.rbac] role_issuers` lists. The privileged roles,
`admin_role`, `emergency_access_role`, `import_role` and
`identifier_reader_role`, never come from a `hold_roles` grant: a `ROLE`
whose `hold_roles` grant names one is refused at commit with `403`, and one
stored before the role was configured is never honoured. The administrator
role and emergency access come from the configuration and the identity
provider alone; the import and identifier permissions come from their own
operations, under the grant ceiling.

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

### For a check that reads a grant

A check inside a request reads the caller's grants with
`ferroehr::party_binding::grants::capabilities::Capabilities::current()`,
which holds them for the duration of the request:

- `client()` is the `ACTOR` the client identifier names, with its state;
- `grants()`, `of(operation)` and `holds(operation)` are the grants in force,
  each an `Operation` with its `ROLE`;
- `credentials()` of a grant reads its credentials by the code of an
  element's name: `named`, `texts`, `codes`, `count`, `boolean`, and
  `clusters` for a `CLUSTER` such as a facility.

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
| Data migration `ROLE` | `openEHR-DEMOGRAPHIC-ROLE.data_migration.v1` |
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
identifier type make a party a policy party, in any archetype. The archetype
ids above and the node ids below are what FerroEHR reads the terms of an
example grant by, and every template and version built on an archetype keeps
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
| `ROLE.system_client` | Kind | `at10` relay, `at11` device | |
| `ROLE.system_client` | Template written | `/capabilities[id6]/credentials[id8]/items[id9]` | `DV_TEXT` |
| `ROLE.system_client` | Facility: issuer, identifier, identifier prefix | `/capabilities[id6]/credentials[id8]/items[id10]/items[id11]`, `[id12]`, `[id13]` | `DV_TEXT` |
| `ROLE.system_client` | Device identifier issuer | `/capabilities[id6]/credentials[id8]/items[id14]` | `DV_TEXT` |
| `ROLE.system_client` | Not before, not after, authorisation reference | `/capabilities[id6]/credentials[id8]/items[id15]`, `[id16]`, `[id17]` | `DV_DATE_TIME`, `DV_DATE_TIME`, `DV_TEXT` |
| `ROLE.data_migration` | Kind | `at10` data migration | |
| `ROLE.data_migration` | Operation | `/capabilities[id6]/credentials[id8]/items[id9]`: `at20` import EHR, `at21` import EHR extract, `at22` export EHR extract | `DV_CODED_TEXT` |
| `ROLE.data_migration` | Subject namespace | `/capabilities[id6]/credentials[id8]/items[id10]` | `DV_TEXT` |
| `ROLE.data_migration` | Rate: requests per second, burst | `/capabilities[id6]/credentials[id8]/items[id11]/items[id12]`, `[id13]` | `DV_COUNT` |
| `ROLE.data_migration` | Trusted creating system | `/capabilities[id6]/credentials[id8]/items[id14]` | `DV_TEXT` |
| `ROLE.data_migration` | Not before, not after, authorisation reference | `/capabilities[id6]/credentials[id8]/items[id15]`, `[id16]`, `[id17]` | `DV_DATE_TIME`, `DV_DATE_TIME`, `DV_TEXT` |
| `ROLE.federation_partner` | Kind (accessor) | `at10` person, `at11` organisation | |
| `ROLE.federation_partner` | Agreement name, version, jurisdiction, exchange | `/capabilities[id6]/credentials[id8]/items[id9]`, `[id10]`, `[id11]`, `[id12]` | `DV_TEXT` |
| `ROLE.federation_partner` | Purpose | `/capabilities[id6]/credentials[id8]/items[id13]`: `at20` care, `at21` keuring | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Legal ground: GDPR Art. 6(1), Art. 9(2), national provision | `/capabilities[id6]/credentials[id8]/items[id14]/items[id15]`, `[id16]`, `[id17]` | `DV_TEXT` |
| `ROLE.federation_partner` | Consent source | `/capabilities[id6]/credentials[id8]/items[id18]`: `at30` Mitz, `at31` local | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Read scope, template written | `/capabilities[id6]/credentials[id8]/items[id19]`, `[id20]` | `DV_TEXT` |
| `ROLE.federation_partner` | Discard, discard window | `/capabilities[id6]/credentials[id8]/items[id21]`, `[id22]` | `DV_BOOLEAN`, `DV_DURATION` |
| `ROLE.federation_partner` | MyHealth@EU service | `/capabilities[id6]/credentials[id8]/items[id23]`: `at40` Patient Summary, `at41` ePrescription, `at42` original clinical document | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Caller rate: requests per second, burst | `/capabilities[id6]/credentials[id8]/items[id24]/items[id25]`, `[id26]` | `DV_COUNT` |
| `ROLE.federation_partner` | Opt-out override | `/capabilities[id6]/credentials[id8]/items[id27]` | `DV_BOOLEAN` |
| `ROLE.federation_partner` | Access class | `/capabilities[id6]/credentials[id8]/items[id28]`: `at50` automated, `at51` chained | `DV_CODED_TEXT` |
| `ROLE.federation_partner` | Chained log retention | `/capabilities[id6]/credentials[id8]/items[id29]` | `DV_DURATION` |
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
`performer`. One capability per origin, here `relay`: the templates it
writes and the facility it writes at.

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
    "_type": "CAPABILITY", "archetype_node_id": "id6",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Relay",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
    "credentials": {
      "_type": "ITEM_TREE", "archetype_node_id": "id8", "name": { "_type": "DV_TEXT", "value": "Grant" },
      "items": [
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Commit composition",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "commit_composition" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9", "name": { "_type": "DV_TEXT", "value": "Template written" },
          "value": { "_type": "DV_TEXT", "value": "openEHR-EHR-COMPOSITION.report-result.v1" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id10", "name": { "_type": "DV_TEXT", "value": "Facility" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id11", "name": { "_type": "DV_TEXT", "value": "Issuer" },
              "value": { "_type": "DV_TEXT", "value": "urn:oid:2.16.528.1.1007.3.3" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id12", "name": { "_type": "DV_TEXT", "value": "Identifier" },
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
    "_type": "CAPABILITY", "archetype_node_id": "id6",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Relay",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
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

#### A data migration

A migration job imports the EHRs of a legacy system and its extracts over
one night. Its `AGENT` is written as above; its grant is a `ROLE` of the
data migration template, `POST /demographic/role`. The capability's
`time_validity` holds the migration to two days, and `Not before` and
`Not after` narrow it to the night between them.

<!-- policy-example: data-migration-role -->
```json
{
  "_type": "ROLE",
  "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1",
  "archetype_details": {
    "_type": "ARCHETYPED",
    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1" },
    "template_id": { "_type": "TEMPLATE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.data_migration.v1.0.0" },
    "rm_version": "1.1.0"
  },
  "name": { "_type": "DV_TEXT", "value": "Data migration" },
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
    "_type": "CAPABILITY", "archetype_node_id": "id6",
    "name": {
      "_type": "DV_CODED_TEXT", "value": "Data migration",
      "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at10" }
    },
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
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Import EHR",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "import_ehr" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id90", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Import EHR extract",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "import_ehr_extract" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Import EHR",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at20" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9", "name": { "_type": "DV_TEXT", "value": "Operation" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Import EHR extract",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at21" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id10", "name": { "_type": "DV_TEXT", "value": "Subject namespace" },
          "value": { "_type": "DV_TEXT", "value": "nl.legacy-his.patients" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id11", "name": { "_type": "DV_TEXT", "value": "Rate" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id12", "name": { "_type": "DV_TEXT", "value": "Requests per second" },
              "value": { "_type": "DV_COUNT", "magnitude": 50 }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id13", "name": { "_type": "DV_TEXT", "value": "Burst" },
              "value": { "_type": "DV_COUNT", "magnitude": 100 }
            }
          ]
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id14", "name": { "_type": "DV_TEXT", "value": "Trusted creating system" },
          "value": { "_type": "DV_TEXT", "value": "legacy-his.example.org" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id15", "name": { "_type": "DV_TEXT", "value": "Not before" },
          "value": { "_type": "DV_DATE_TIME", "value": "2026-12-05T22:00:00Z" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id16", "name": { "_type": "DV_TEXT", "value": "Not after" },
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

#### A federation partner

A partner organisation's professionals read patient summaries under a
data-sharing agreement. The grant is a `ROLE` of the federation partner
template performed by the partner's `ORGANISATION`, with one capability per
accessor, here `person`.

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
    "lower": { "_type": "DV_DATE", "value": "2027-01-01" },
    "upper": { "_type": "DV_DATE", "value": "2027-12-31" },
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
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Federated person access",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ferroehr-operations" }, "code_string": "federated_person_access" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id9", "name": { "_type": "DV_TEXT", "value": "Agreement name" },
          "value": { "_type": "DV_TEXT", "value": "Regional care network exchange" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id10", "name": { "_type": "DV_TEXT", "value": "Agreement version" },
          "value": { "_type": "DV_TEXT", "value": "2.1" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id11", "name": { "_type": "DV_TEXT", "value": "Jurisdiction" },
          "value": { "_type": "DV_TEXT", "value": "NL" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id12", "name": { "_type": "DV_TEXT", "value": "Exchange" },
          "value": { "_type": "DV_TEXT", "value": "patient summary" }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id13", "name": { "_type": "DV_TEXT", "value": "Purpose" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Care",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at20" }
          }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id14", "name": { "_type": "DV_TEXT", "value": "Legal ground" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id15", "name": { "_type": "DV_TEXT", "value": "GDPR Article 6(1) point" },
              "value": { "_type": "DV_TEXT", "value": "c" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id16", "name": { "_type": "DV_TEXT", "value": "GDPR Article 9(2) point" },
              "value": { "_type": "DV_TEXT", "value": "h" }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id17", "name": { "_type": "DV_TEXT", "value": "National provision" },
              "value": { "_type": "DV_TEXT", "value": "UAVG Art. 30(3)(a)" }
            }
          ]
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id18", "name": { "_type": "DV_TEXT", "value": "Consent source" },
          "value": {
            "_type": "DV_CODED_TEXT", "value": "Mitz",
            "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" }, "code_string": "at30" }
          }
        },
        {
          "_type": "ELEMENT", "archetype_node_id": "id19", "name": { "_type": "DV_TEXT", "value": "Read scope" },
          "value": { "_type": "DV_TEXT", "value": "patient/composition-openEHR-EHR-COMPOSITION.patient_summary.v1.r" }
        },
        {
          "_type": "CLUSTER", "archetype_node_id": "id24", "name": { "_type": "DV_TEXT", "value": "Caller rate" },
          "items": [
            {
              "_type": "ELEMENT", "archetype_node_id": "id25", "name": { "_type": "DV_TEXT", "value": "Requests per second" },
              "value": { "_type": "DV_COUNT", "magnitude": 5 }
            },
            {
              "_type": "ELEMENT", "archetype_node_id": "id26", "name": { "_type": "DV_TEXT", "value": "Burst" },
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
