# Authorization keys

`[authz]`: the coarse role gate, the assurance level, the party binding and the grant
ceiling. The other authentication keys are on [Authentication & access](config-auth.md).

<!-- toc -->

## `[authz]`

Role-based (RBAC) authorization, the party binding, the grant ceiling and the
patient-data assurance gate. Which caller may read which data is the access
evaluator's, over each EHR's `access_directives` and the controller's access
policy, and takes no key here. How the parts fit together and the order a
request is decided in are on [How FerroEHR decides
access](../explanation/access-model.md).

### `[authz.assurance]`: the assurance level patient data requires

Off by default. What it refuses and the `401` challenge it sends are described
in [Security](../security.md#assurance-level-and-the-natural-person-on-patient-data).

| Key | Type | Default | Description |
|---|---|---|---|
| `claim` | string | `acr` | The token claim carrying the authentication context class; a dotted path reaches into a JSON object. |
| `levels` | table of string → `low` \| `substantial` \| `high` | empty | Maps each value your identity provider puts in `claim` to an eIDAS level of assurance. A value the table does not name carries no level. File-only: the keys are usually URIs. |
| `minimum` | `low` \| `substantial` \| `high` | unset | The level a patient-data request must carry. Unset refuses nothing. |

```toml
[authz.assurance]
minimum = "substantial"

[authz.assurance.levels]
"http://eidas.europa.eu/LoA/low" = "low"
"http://eidas.europa.eu/LoA/substantial" = "substantial"
"http://eidas.europa.eu/LoA/high" = "high"
```

These are the values FerroSMART writes into `acr`. Behind FerroSMART, set
the floor to substantial at least: the xShare Yellow Button that FerroBRIDGE
serves reads the patient's record with their own exchanged token, which
this gate holds to the floor
([Behind FerroSMART](../behind-ferrosmart.md#the-patients-own-token-through-ferrobridge)).

Boot refuses a `minimum` with an empty `levels` table or with no value mapped at
or above it (every patient-data request would be refused), a blank `claim`
beside a declared table, and a `levels` value that is blank or carries
whitespace, a quote or a backslash, because the refusal names the accepted
values in one quoted, space-separated `acr_values` parameter (RFC 9470 §3).

A Basic credential carries no level, so with `minimum` set every Basic user,
mapped to a person or not, is refused on patient data. While `minimum` is unset
and a Basic user is mapped in `[[authz.party_binding.basic_user]]`, boot logs a
`warn` naming those users.

The access record carries the acting mode and the professional the caller
classification names; see [Audit trail](../ehds-logging.md#the-ehds-logging-elements-mapped).
`[authz.professional]` is refused at boot: the token's `fhirUser` classifies
the caller, and its RFC 8693 `act` names who acts for them.

### `[authz.party_binding]`

How a person is attributed. There is no switch: binding is how FerroEHR
attributes a person, and `enabled` is refused at boot.

| Key | Type | Default | Description |
|---|---|---|---|
| `provider` | `{ issuer, id }` | **required** once `[auth]` admits a credential | The healthcare provider this deployment acts for, as an identifier issuer and id (in the Netherlands the URA). Every access record of a bound or provisional person and every version a person commits name it (EHDS Art. 13(4), Annex II 3.2(a)). |
| `namespace` | string | `demographic` | The `PARTY_REF.namespace` a bound party is written under, the value of the ITS-REST overview example (`committer.external_ref.namespace="demographic"`), held to the `OBJECT_REF.namespace` pattern `[a-zA-Z][a-zA-Z0-9_.:/&?=+-]*`. |
| `basic_user` | `[[authz.party_binding.basic_user]]` tables | `[]` | One per Basic user: `user`, `key_issuer` (the `identifier_issuer` of an `[auth.issuer.<name>.party]` table) and `key`, the binding key the user's `PERSON` carries. A Basic user with no table, a table naming no Basic user and a `key_issuer` no table names refuse the start. |
| `issuer` | `[[authz.party_binding.issuer]]` tables | `[]` | How the tokens of one issuer are read when a first login creates a provisional `PERSON`; see [Just-in-time creation](#just-in-time-creation). |
| `provisional_report_after` | duration | `"30d"` | The age past which `ferroehr party-binding check` reports a provisional `PERSON`. |

```toml
[authz.party_binding]
provider = { issuer = "urn:oid:2.16.528.1.1007.3.3", id = "12345678" }

[[authz.party_binding.basic_user]]
user = "clinician"
key_issuer = "urn:example:staff-login"
key = "3f2b0c1e-0000-4000-8000-000000000001"
```

#### The party mapping

SCIM provisioning, just-in-time creation and the login binding read and
write the same demographic parties under one mapping. It names the
templates each party is valid against, the identifier selectors of the
binding key, `userName` and `externalId`, and where each SCIM attribute and
OpenID Connect claim goes. Names go in `PARTY.identities` and identifiers
in `PARTY.details` (RM demographic §Names and Addresses, §Party
Identification), and boot refuses a name mapped into `details` or an
identifier mapped into `identities`. No openEHR specification governs this
mapping: it is FerroEHR's own design, and its targets are RM demographic
paths.

| Section | Keys | Description |
|---|---|---|
| `[[authz.party_binding.person.template]]` | `template_id`, `key`, `user_name`, `external_id`, `default` | One per template a professional's `PERSON` may use, so a migrated template can stay beside a new one. `key`, `user_name` and `external_id` are selectors, each `{ path, type }`: the path of the `ELEMENT` holding the `DV_IDENTIFIER` and its `type`. Every identifier can sit at one path (the repeated element of an identifier cluster), so two selectors at one path are told apart by `type`, and the key by its issuer, the `identifier_issuer` of an `[auth.issuer.<name>.party]` table. Exactly one table sets `default = true`: SCIM and just-in-time creation create new persons with it. |
| `[[authz.party_binding.person.attribute]]` | `scim`, `oidc`, `jit`, `name`, `path` | One per mapped value: a SCIM attribute path (the RFC 7644 `valuePath` form, `emails[type eq "work"].value`, and the FerroEHR extension `urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:loginIdentifier` that feeds the binding key), an OpenID Connect claim, whether just-in-time creation writes it (`jit`, default `false`), whether it is the name the committer reads (`name`, at most one), and `path`, one path per template id. `id`, `schemas`, `meta`, `active`, `userName` and `externalId` are SCIM's own and are never mapped. |
| `[authz.party_binding.professional_role]` | `template_id` | The template of the professional `ROLE`, rooted at `ROLE`. A `PERSON` is a professional while it lists a `ROLE` of this template that names it as `performer`, is `complete` or `incomplete`, and whose `time_validity` holds today. |
| `[authz.party_binding.group]` | `template_id`, `external_id`, `display_name` | The template of a SCIM Group's `GROUP`, its `externalId` selector and the path of its `displayName`. |
| `[authz.party_binding.membership]` | `archetype_id` | The `PARTY_RELATIONSHIP` archetype of a group membership, as each membership's `archetype_node_id` carries it (the interface form of an ADL 2 id, `openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1`). SCIM serves `/Groups` when the group table names a `display_name` and this table is set. |
| `[authz.party_binding.patient]` | `template_id`, `identifier`, `name` | The template of a patient's `PERSON`, the subject of an EHR and the target of a representation: the path of the `ELEMENT` holding its identifiers (each a `DV_IDENTIFIER` of any type and issuer) and the path of its name. A `PERSON` written under this template is validated against it. |

With no mapping key set, every section maps FerroEHR's [default
demographic definitions](default-demographic-definitions.md), which the
server stores as it boots when the definition store holds no definition of
their id. An empty configuration has this mapping:

```toml
[[authz.party_binding.person.template]]
template_id = "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0"
default = true
key = { path = "/details[id2]/items[id3]", type = "login" }
user_name = { path = "/details[id2]/items[id3]", type = "scim_user_name" }
external_id = { path = "/details[id2]/items[id3]", type = "scim_external_id" }

[[authz.party_binding.person.template]]
template_id = "openEHR-DEMOGRAPHIC-PERSON.patient.v3.0.0"
key = { path = "/details[id2]/items[id3]", type = "login" }
user_name = { path = "/details[id2]/items[id3]", type = "scim_user_name" }
external_id = { path = "/details[id2]/items[id3]", type = "scim_external_id" }

[[authz.party_binding.person.attribute]]
scim = "name.formatted"
name = true
path = { "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0" = "/identities[id4]/details[id5]/items[id6]", "openEHR-DEMOGRAPHIC-PERSON.patient.v3.0.0" = "/identities[id6]/details[id7]/items[id8]" }

[[authz.party_binding.person.attribute]]
scim = "name.givenName"
path = { "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0" = "/identities[id4]/details[id5]/items[id7]", "openEHR-DEMOGRAPHIC-PERSON.patient.v3.0.0" = "/identities[id6]/details[id7]/items[id9]" }

[[authz.party_binding.person.attribute]]
scim = "name.familyName"
path = { "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0" = "/identities[id4]/details[id5]/items[id8]", "openEHR-DEMOGRAPHIC-PERSON.patient.v3.0.0" = "/identities[id6]/details[id7]/items[id10]" }

[[authz.party_binding.person.attribute]]
scim = 'emails[type eq "work"].value'
path = { "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0" = "/contacts[id9]/addresses[id10]/details[id11]/items[id12]" }

[authz.party_binding.professional_role]
template_id = "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0"

[authz.party_binding.group]
template_id = "openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0"
external_id = { path = "/details[id2]/items[id3]", type = "scim_external_id" }
display_name = "/identities[id4]/details[id5]/items[id6]"

[authz.party_binding.membership]
archetype_id = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1"

[authz.party_binding.patient]
template_id = "openEHR-DEMOGRAPHIC-PERSON.patient.v3.0.0"
identifier = "/details[id2]/items[id3]"
name = "/identities[id6]/details[id7]/items[id8]"
```

The template ids are the ADL 2 ids of the stored archetypes, and the
membership archetype is named in the interface form each membership's
`archetype_node_id` carries. The patient template is a person template too,
never the default: a patient's login binds their `PERSON` in it. A
representation needs no section: FerroEHR recognises it by its coded name, in
any archetype (see
[Representatives](../caller-facts.md#representatives)). The default
key is of the type `login`, so an `[auth.issuer.<name>.party]` table whose
`identifier_type` is `login` needs no mapping key.

A section you set replaces that section's default whole: a deployment that
sets `[[authz.party_binding.person.template]]` maps its own attributes too.
A deployment with templates of its own sets the sections that name them and
stores its templates before boot (`ferroehr definitions load`); the server
stores nothing under its ids, and still stores the default definitions,
which no section of its mapping then names.

A worked example over the default definitions, with the Microsoft Entra
object id as the binding key. It sets the person section, and every other
section keeps its default:

```toml
[[authz.party_binding.person.template]]
template_id = "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0"
default = true
key = { path = "/details[id2]/items[id3]", type = "guid" }
user_name = { path = "/details[id2]/items[id3]", type = "scim_user_name" }
external_id = { path = "/details[id2]/items[id3]", type = "scim_external_id" }

[[authz.party_binding.person.attribute]]
scim = "name.formatted"
oidc = "name"
jit = true
name = true
path = { "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0" = "/identities[id4]/details[id5]/items[id6]" }
```

With Microsoft Entra ID, map the object id to `externalId` (Entra's
matching) and to the binding-key attribute (the binding): two attributes,
two selectors. The key's party table's `identifier_type` is `guid`, so a
key differing only in case binds one login.

Boot refuses, naming the key: no or two `default` tables; a selector path
that does not compile; two equal selectors; any selector under the Dutch
BSN namespace (OID `2.16.840.1.113883.2.4.6.3`); a key fed by an email
address, a phone number, `preferred_username`, `name`, `upn` or
`unique_name`; a template that is absent, rooted at another class or lacks
a mapped path; and a path whose type cannot carry its source. Deleting a template
the mapping names is refused with `409`.

The parties the binding relies on are protected: a `PERSON` of a mapped
template carrying a selector identifier, its professional `ROLE`, a
representation (a `PARTY_RELATIONSHIP` of its own named
`openehr_access::represents`), and a `GROUP` of the group template. Only an
administrator (`authz.rbac.admin_role`) writes one through the demographic
API, including the creation of a party that carries a selector identifier;
a `write_representation` holder writes a representation for the providers
its grant names, and the patient a representation targets writes one on the
basis `HL7_RoleCode::DELEGATEE`.
The policy parties (a party holding a capability coded in
`openehr_access`, an acts-for relationship or an `openehr_access::client_id` client identifier)
are recognised by what they hold and take no key of this mapping; they are an administrator's to write and to read,
never a SCIM client's: see [Operational policy](../operational-policy.md#who-writes-and-reads-a-policy-party).
An administrator's version that leaves a protected identifier out is refused
with `422`. A present binding key is immutable (RFC 7643 §2.2): a version
that changes it is refused with `403`, and
[`POST /admin/party/binding-key`](../operations-admin-apis.md) is the one
change. Each selector value binds one live party: a commit that gives a
binding key, a `userName` (compared after RFC 8265 width mapping, lower case
and NFC), an `externalId` or a group `displayName` a second party is refused
with `409`. A deleted `PERSON` leaves its key as a tombstone that refuses
the login until an administrator binds the key again.

#### Changing the mapping

The side table is filled from the configured mapping and the party
versions. The first boot fills it and stores the mapping's digest, which
covers the templates, the selectors, the role, the group and the membership.
It leaves out the key issuers: the `identifier_issuer` of each
`[auth.issuer.<name>.party]` table. Each running replica holds
a lease it renews every 20 seconds, and the lease lists the key issuers the
replica derives keys for.

**Adding or removing a key issuer needs no stop and no refill.** Each key
issuer is a key space with a state:

| State | Its keys bind a login | What happens next |
|---|---|---|
| `pending` | No: the login is answered `503` with a `Retry-After` | Once every live replica names the issuer, a replica fills its keys from the party versions |
| `filling` | No: `503` with a `Retry-After` | The fill runs online in pages and the issuer turns `active` |
| `active` | Yes | A replica that starts without the issuer turns it `retiring` |
| `retiring` | No: the login is refused with `403 refused-unbound` | Once no live replica names it, its keys are deleted; once every live replica names it again, it is filled anew |

So a rolling upgrade that adds an issuer serves every other issuer
throughout, and the new issuer's logins bind on every replica once the last
replica without it is replaced. A replica restarted without an issuer stops
that issuer's logins binding on every replica at once, so revoking an issuer
needs no downtime. A fill that finds two parties carrying one
key under the new issuer keeps it `pending`, logs both parties, and looks
again after five minutes; the other issuers keep binding.

**A change to the templates, selectors, role, group or membership is a
stop-the-world step.** A replica whose mapping digest differs from the stored
one refuses to start, naming both digests:

1. Stop every replica. With Helm, scale the deployment to zero first: a
   rolling upgrade would start new replicas beside old ones.
2. Run `ferroehr party-binding refill` with the new configuration. It
   refuses while a replica's lease was renewed within the last minute, and
   refuses a store in which two parties carry one selector value, naming
   both.
3. Start the replicas with the new mapping. A replica still holding the old
   mapping refuses to start.

#### How a login is looked up

Every request of a person is looked up in the binding side table of the
party store, on the party database role, with no cache, so a change to a
party takes effect on the next request on every instance. The table holds
the binding key of each `PERSON` of a mapped template that performs a
professional `ROLE` (a `ROLE` of the mapped role template that its `roles`
list, that names it as `performer`, whose state is `complete` or
`incomplete` and whose `time_validity` holds today's date in UTC) or that
is the source of a representation. A key of type `guid` or
`uuid` is compared in lower case. The table is written in each party
commit's transaction and can be rebuilt from the party versions alone.

The request is refused with `403` and the binding code in the problem body
when the login matches two parties (`refused-ambiguous`), when its party is
`inactive` (`refused-inactive`) or `abandoned` (`refused-abandoned`), when
its key belongs to a deleted party (`refused-deprovisioned`), when the token
carries no `sub` (`refused-key-claim-missing`), when the party lacks the
marker the caller's kind needs (`refused-wrong-marker`), when the groups a
token asserts cannot be read (`refused-groups-unavailable`), and when a
representative reaches a subject its party has no live
representation of, stored in the CDR for the provider the deployment acts
for and not written by the representative itself
(`refused-representation-unrecorded`). A party
store that cannot answer is answered `503`.

A person whose login matches no party is refused with `403` and
`refused-unbound` on every request, reads and writes alike; only the public
endpoints outside the auth layer answer. The problem names the identifier
issuer to provision the person under, and the refusal of a token without
`sub` names that claim instead. No setting serves an unbound person
under a login-only committer, so provision every professional (through
[SCIM](config-scim.md#authscim) or the demographic API) before you add an issuer, or
let the issuer create a provisional `PERSON` at the first login
([just-in-time creation](#just-in-time-creation)). Each
refusal writes one `warn` line naming the issuer and the outcome, never the
claim value, and counts into the `binding_refusals` metric by `issuer` and
`outcome`.

A representative reaches only the EHRs of a subject their `PERSON` holds a
live representation of, stored in the CDR for the provider
`[authz.party_binding.provider]` names and read from the store, never from
the token (EHDS Art. 4(2)). The access evaluator holds every request that
names an EHR to those EHRs, whatever the token's scopes cover, and refuses
any other with `403` `refused-not-represented`; a query must name one with
`ehr_id`, and an operation that names no EHR is refused the same way.
Templates stay readable. A professional's token whose only clinical scopes
are `patient/` is confined to its launch EHR, and its commits record the
professional, never `PARTY_SELF`. See
[Representatives](../caller-facts.md#representatives).

#### Just-in-time creation

Just-in-time creation lets a professional with no party work under their own
account at their first login, before SCIM or an administrator completes the
record. When a professional's login matches no party, and a
`create_at_login` grant in the CDR admits it, FerroEHR commits one `PERSON`
of the default template with lifecycle state `553|incomplete|`, which RM
common §Incomplete Content allows for "incomplete demographic details", and
its professional `ROLE` in the same demographic contribution, then serves the
request bound to it. The next request binds to that `PERSON` and creates
nothing. When a login creates a party is FerroEHR's own design: no openEHR
specification governs it.

Which logins are created is operational policy, so it is CDR data and no
configuration key: an administrator writes a `ROLE` whose capability grants
`openehr_access::create_at_login`, naming the issuer and the admission
conditions, through the Demographic API. The grant is read on each first
login, so a change applies to the next one without a restart. The
[Operational policy](../policy-examples.md#just-in-time-admission) page
documents the grant and has a worked example. `[authz.grants] allowed`
must list `create_at_login`, which the default does not, or no such grant
can be written or honoured.

The `[[authz.party_binding.issuer]]` table only says how an issuer's tokens
are read, and an issuer without one reads the defaults:

| Key | Type | Default | Description |
|---|---|---|---|
| `issuer` | string | required | An issuer an `[auth.issuer.<name>.party]` table binds. |
| `user_name_claim` | string | `sub` | The claim the `userName` is read from. It must be one the issuer controls: an email address, a phone number, `preferred_username`, `name`, `upn` and `unique_name` refuse the start. |
| `idp_claim` | string | `idp` | The dotted claim path an `admit_idp` condition reads the identity provider from (the Microsoft Entra `idp` claim). |

Creation also needs a default person template with a `user_name` selector,
`[authz.party_binding.professional_role]`, and an attribute that maps an
OpenID Connect name claim (`name`, `given_name`, `family_name`,
`middle_name` or `nickname`) to the default template. The default mapping
has them. When a grant admits a login and the mapping lacks one, the login
is not created and the server logs what is missing.

```toml
[[authz.party_binding.issuer]]
issuer = "https://login.microsoftonline.com/<tenant-id>/v2.0"
user_name_claim = "oid"
```

What it writes is the attribution minimum: the binding key, the `userName`
and the name claims the token carries. Any other mapped attribute is written
only where its `[[authz.party_binding.person.attribute]]` sets `jit = true`
(GDPR Art. 25(2), data protection by default), and `externalId` is never
written, since RFC 7643 §3.1 leaves it to the provisioning client. The
version is committed with change type `249|creation|`, a description naming
just-in-time creation and the issuer, and the person's own committer: the
name, the login identifier and an `external_ref` naming the new `PERSON`.
FerroEHR reads every value from the access token only (RFC 9068 §2.2.2), so
the issuer must put the name claims in the access token.

It runs only for a person or a delegated caller of the issuer, never for a
representative, a patient, a system or a `federation` caller, and never for a
login whose outcome is anything other than unbound: an ambiguous, inactive,
abandoned or deprovisioned party, a token without the key claim and a party
with the wrong marker are refused as before. A `PERSON` a SCIM client deleted
is never created again by a later login, whatever the identity provider still
issues. A login that no grant in force admits, or whose token carries no
mapped name claim, is refused with `403` and `refused-unbound`.

Concurrent first requests of one login commit one `PERSON`: the binding key's
unique index refuses the second commit, and that request binds to the first
one's `PERSON`. A login whose `userName` another party already holds is
refused with `403` and binds nothing. Just-in-time creation only creates: it
never updates, deactivates or deletes a `PERSON`, so a later login with
changed claims changes nothing.

A `PERSON` whose latest version is `incomplete` is provisional. The access
record of each of its requests carries `binding = provisional`, a demographic
read and an AQL query over `VERSION/lifecycle_state` show it, and
`ferroehr party-binding check` reports the provisional persons older than
`provisional_report_after` under `side_table.provisional`. What a provisional person may do is decided by
their SMART scopes and roles, as for anyone. A SCIM client whose
`binding_issuer` is the issuer's `identifier_issuer` completes the `PERSON`:
it finds it by `userName`, with a binding key equal to its own, writes the
`externalId` with a `PATCH`, and a complete `PUT` completes it. SCIM
`active = false` abandons it.

The lawful basis for the attribution minimum (key, `userName`, name) is GDPR
Art. 6(1)(c): EHDS Art. 13(4) and the logging duties need the professional
identified. An operator who opts a contact attribute in with `jit = true`
names its own basis, Art. 6(1)(b) or (f). Because the data come from the
identity provider and not from the person, tell the professionals what Art. 14
asks: what is copied, from which source, and why. On a realm or tenant that
creates persons, make the name attributes administrator-controlled, as the key
is; [Enterprise identity providers](../identity-providers.md#just-in-time-creation)
shows the settings per identity provider.

### Imports and system clients

The systems whose wrapped committers an import may carry are no key: a
`creating_system` element of the importer's import grant names each one
([What the grants decide](../operational-policy.md#what-the-grants-decide)).

The system clients are not configuration. An OAuth client-credentials client
is granted what it may commit, import or export in the CDR, as an `ACTOR`
carrying its `client_id` that acts for a provider and performs a `ROLE` whose
capabilities grant the operations; see
[What a system client may do](../operational-policy.md#what-a-system-client-may-do).
The former `[[authz.system_client]]` table is an unknown key and refuses the
start.

### Federation agreements

A federation agreement is CDR data, not a configuration key: a grant of
`federated_person_access` or `federated_organisation_access` on the partner's
party, read on every request and judged when it is written. The configuration
keeps what the agreements are judged against: the `federation` issuers, their
`[auth.issuer.<name>.federation]` tables, `[authz.grants] allowed` (which must name both
codes before an agreement can be written). The period a `chained` agreement
is held to is the access log retention of the controller's access policy
(`ACCESS_POLICY.log_retention`). The model, the codes and the rules are on
[Operational policy](../operational-policy.md#federation-agreements). The
`[[authz.federation.agreement]]` table is removed, and a configuration that
still holds it is refused at boot as an unknown key.

### `[authz.grants]`

| Key | Type | Default | Description |
|---|---|---|---|
| `allowed` | list of operation codes | `["commit_composition"]` | The operations of the `openehr_access` terminology the policy grants may grant. A `ROLE` committed with a capability that codes an operation this list does not name is refused with `403`, and one already stored is never honoured, so a compromised administrator account grants no more than you allow. Allow `compose_for` to let a professional name another `PERSON` of the same provider as the composer of what they commit ([Who is the committer](../security/committer.md#the-composer-of-a-professionals-composition)). An empty list lets the party store grant nothing; an unknown code refuses the start. Environment: `FERROEHR__AUTHZ__GRANTS__ALLOWED`, comma-separated. The codes and how a request reads its grants are on [Operational policy](../operational-policy.md#how-a-request-reads-its-grants). |

### `[authz.rbac]`

| Key | Type | Default | Description |
|---|---|---|---|
| `enabled` | bool | `true` | The coarse role gate (active when authentication is enabled). Off, every authenticated caller may use every enabled surface, reported as the `rbac_off` deployment gap. `ehr_access_default` and `admin_role` apply either way. |
| `admin_role` | string | `ADMIN` | Role required for admin-class operations. A blank value is a boot error. |
| `readonly_role` | string | `READONLY` | Role marking a principal read-only: refused on every write operation (create/update/delete/upload), even alongside granting roles. Reads and AQL queries are still allowed. |
| `role_issuers` | list of string | `[]` | The issuers whose role claims may grant `admin_role`. A token of any other issuer never holds it, a Basic user keeps the roles its own table grants, and a `hold_roles` policy grant never gives one. A `federation` issuer refuses the start. Empty, no token grants them. |
| `ehr_access_default` | enum{open,restricted} | `open` | What an EHR carrying no `ACCESS_CONTROL_SETTINGS` admits. `restricted` is object-level default-deny: only `admin_role` reaches a setting-less EHR, so an operator can still author the settings that open it. It is enforced whether `enabled` is on or off. With `[auth] enabled = false` nothing could enforce `restricted`, so that combination is a boot error. See [Security](../access-control.md#per-ehr-access-control-ehr_access). |

Reading one subject's access log, acting as an EHDS Art. 10 access service,
and importing an EHR or an extract have no key here: each is a grant in the
CDR (`read_subject_audit`, `access_service`, `import_ehr` and
`import_ehr_extract`),
under `[authz.grants] allowed`. The keys that named a role for them before
5.7.1 are unknown keys, and the server refuses to start with one
([What the grants decide](../operational-policy.md#what-the-grants-decide)).

> [!NOTE]
> The management surface is **not** configured under `[authz.rbac]`.
> [`[management.endpoints]`](config-auth.md#management) owns it, one level per endpoint, with
> no global default beside it: an endpoint you do not name is `off` and is not
> mounted. Only the `admin_only` level consults `authz.rbac.admin_role`.
