# SCIM provisioning

`[auth.scim]`: provisioning users and care teams from your identity provider.

<!-- toc -->

## `[auth.scim]`

FerroEHR is a SCIM 2.0 service provider (RFC 7644) at `/scim/v2/`, the
endpoint an identity provider provisions each professional through. It is off
by default.

| Key | Type | Default | Description |
|---|---|---|---|
| `filter_max_results` | integer | `100` | The most resources one list response returns, which `ServiceProviderConfig` declares as `filter.maxResults`. At least 1. |
| `max_deactivations` | integer | `100` | The most Users one client deactivates or deletes per window. Past it, a deactivation or deletion is **429** with `Retry-After` and logged at `error`. The count is kept in the database, so every replica shares one bound. At least 1. |
| `deactivation_window` | duration | `"1h"` | The length of that window. At least `"1min"`. |
| `contact_erasure_delay` | duration | `"0s"` | How long after a User's deletion the contacts of every version of its `PERSON` are erased (GDPR Art. 17). `"0s"` erases them in the deletion itself; at most `"30d"`. |
| `max_group_members` | integer | `5000` | The most members one Group holds. An add past it is **400** `invalidValue`. A membership change versions only the member's `PERSON`, so its cost does not grow with the Group. At least 1. |

Who provisions is CDR data, read on every request: a provisioning client is
an OAuth client-credentials client of your own authorization server whose
`ACTOR` holds a `scim_provision` grant in force. The grant's
`binding_issuer` is the identifier issuer of the binding keys the client
provisions: the `identifier_issuer` of an `[auth.issuer.<name>.party]` table
your professionals log in through. Its `token_scope` is the scope a token of the client
must hold; SMART on openEHR defines none for SCIM, so the value is yours to
choose. Each User belongs to the one client whose `binding_issuer` issues its
key, so no two clients' grants name one. `[authz.grants] allowed` must admit
`scim_provision`. The first such grant is written with
[`ferroehr party-binding grant`](../operational-policy.md#the-first-grant),
since no administrator can log in before the persons it provisions exist;
[Operational policy](../operational-policy.md#what-the-grants-decide) has the
grant's terms.

Every `meta.location` and discovery document names the resource by its URI
(RFC 7643 §3.1), under `<public_base_url>/scim/v2` with
[`server.public_base_url`](config-server.md#public_base_url-the-one-public-url)
and `server.base_path`, so the table needs that key.

The table's presence turns the endpoint on; it has no `enabled` key, and
setting one refuses the start. The start is refused when the table is present
without `server.public_base_url`, and when the [party mapping](config-authz.md#authzparty_binding)
lacks what a User is written with: a default person template with a
`user_name` selector, and `[authz.party_binding.professional_role]`.

```toml
[server]
public_base_url = "https://cdr.example.com"

[auth.scim]

[authz.grants]
allowed = ["commit_composition", "scim_provision"]
```

`/scim/v2/ServiceProviderConfig` answers without a token, so a client can read
the authentication scheme before it holds one. Every other path needs a
bearer token of a client that acts for nobody and holds a `scim_provision`
grant in force:

- no token, or one that does not verify, is **401** with
  `WWW-Authenticate: Bearer`;
- a person's token, a client with no grant in force, a grant whose
  `binding_issuer` is not the `identifier_issuer` of an `[auth.issuer.<name>.party]`
  table, or another client's grant names too, and a token without the grant's `token_scope` are **403**, the
  last with `WWW-Authenticate: Bearer error="insufficient_scope"` (RFC 6750
  §3.1).

Every refusal is an RFC 7644 §3.12 `Error` message in `application/scim+json`.
The client's requests count against
[`[server.caller_rate]`](config-server.md) like any caller's, so each
replica gives the client an allowance of its own, and past it a request is
**429** with `Retry-After`. `/scim/v2/Schemas` serves the core User schema
reduced to `userName`, `active` and the SCIM attributes
`[[authz.party_binding.person.attribute]]` maps, and FerroEHR's extension
`urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User` with
`loginIdentifier`, the attribute that feeds the binding key. `/ResourceTypes`
lists the `User` type at `/Users`, and the `Group` type at `/Groups` with
FerroEHR's Group extension
`urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:Group` and its
immutable boolean `careTeam` when Groups are served.

The two extension URNs are a deliberate deviation from RFC 7643. They sit
under `urn:ietf:params:scim:schemas:extension:`, the sub-namespace RFC 7643
§10.2 and RFC 3553 reserve for names IANA assigns, and neither name is in the
IANA "SCIM Schema URIs" registry (RFC 7643 §10.3). The reason is Microsoft
Entra ID: its provisioning client sends a custom attribute only under a name
of the form
`urn:ietf:params:scim:schemas:extension:CustomExtensionName:2.0:User:CustomAttribute`
("For SCIM applications, the attribute name must follow the pattern shown in
the example", *Customize Microsoft Entra attribute mappings*, vendored at
`docs/specs/microsoft-entra/docs/identity/app-provisioning/customize-application-attributes.md`
line 148). A schema id outside that namespace, which RFC 7643 §3 would allow,
leaves an Entra non-gallery application unable to send `loginIdentifier`, so
it could not provision anyone. The binding key fits no registered attribute:
`userName` is mutable, `externalId` is the client's own identifier for the
resource (RFC 7643 §3.1), and the Enterprise User extension (§4.3) carries no
login subject. The two URNs are the provisioning contract of the FerroHEALTH
products, fixed in the same way as the `/scim/v2` base path, and a client
writes them exactly as `/Schemas` lists them.

### Users

A User is a `PERSON` of the default person template performing the
professional `ROLE`: SCIM (RFC 7643, RFC 7644) provisions a workforce. A
patient needs no provisioning, since their own token's login binds their
`PERSON`. A representation is a `PARTY_RELATIONSHIP` of its own, written by
the patient, an administrator or a `write_representation` holder
(Regulation (EU) 2025/327 Art. 4(2)), and a User version that holds one in
its relationships is refused `422`. This is FerroEHR's own design. The client serves these operations (RFC 7644 §3):

- `POST /scim/v2/Users` creates the `PERSON` and its `ROLE` in one demographic
  CONTRIBUTION, under the client as committer, and answers **201** with the
  User, its `Location` and its `ETag`. The body carries `userName` and the
  binding key in `loginIdentifier`; `externalId` and every mapped attribute
  are optional. A version that fits its template in full is `complete`, and
  one that fits only with every lower limit at zero is `incomplete`, which the
  User's `lifecycleState` and `missingPaths` name. A User created with
  `active = false` is **400** `invalidValue`.
- `GET /scim/v2/Users/{id}` answers the User with its `ETag`, the weak tag of
  its version, and **304** to an `If-None-Match` naming that version.
- `GET /scim/v2/Users` answers a `ListResponse`, an empty one with **200**
  when nothing matches. `filter` takes `eq` and `and` over `userName`,
  `externalId` and `id`; names and operators are compared in any case, and
  anything else is **400** `invalidFilter`. `startIndex` and `count` page the
  result, at most `max_results` at a time.

The User's `userName`, `externalId` and binding key are `DV_IDENTIFIER`s at
their selectors, issued by the client's `binding_issuer`. A client sees only
its own Users, the ones whose key its `binding_issuer` issues: every other id
is **404** and absent from every list, as is a `PERSON` without a live
professional `ROLE`. The `userName` index spans every client, so a **409**
`uniqueness` tells one client that a `userName` exists under another;
`externalId` is unique within one client. Every SCIM request is an access
record of the client, with the `PERSON` as its object.

A User is changed and removed through these operations:

- `PUT /scim/v2/Users/{id}` replaces the User (RFC 7644 §3.5.1). A mapped
  attribute the body leaves out is cleared; the binding key and `externalId`
  are kept, and a left-out `active` keeps the lifecycle state.
- `PATCH /scim/v2/Users/{id}` applies the RFC 7644 §3.5.2 operations, value
  paths and a `replace` without `path` included, and commits the result as
  one version. The operation is read in any case, as the Entra ID client
  sends `Replace`.
- `DELETE /scim/v2/Users/{id}` logically deletes the `PERSON` and its
  professional `ROLE` in one demographic CONTRIBUTION and answers **204**.
  Every later request on the id is **404**, the User leaves every list, and a
  new User may take its `userName`. The binding key stays behind as a
  tombstone: a login on it is refused as deprovisioned until the owning
  client creates a User with it again.

Each write answers with the User and its new `ETag`. An `If-Match` naming
another version is **412**, a changed binding key is **400** `mutability`, and
a write without `If-Match` is accepted (RFC 7644 §3.14 makes entity tags a
MAY). `active` drives the lifecycle state of the `PERSON` (RM common
§Version Lifecycle):

| Current state | `active` in the request | Result |
|---|---|---|
| `complete` or `incomplete` | `true` or left out | `complete` when the template holds in full, else `incomplete` |
| `complete` | `false` | `inactive` |
| `incomplete` | `false` | `abandoned` |
| `inactive` | `true` | `complete`, or `incomplete` when the template does not hold |
| `abandoned` | `true` | `incomplete`, then `complete` in a second version when the template holds |
| `inactive` or `abandoned` | `false` or left out, with a change | **400** `invalidValue`: reactivate the User first |
| `inactive` or `abandoned` | `false` or left out, without a change | **200**, no new version |

An inactive or abandoned User is still read and found by a filter, so its
client can reactivate it. When the second version of an `abandoned` User
fails, the User stays `incomplete` and the same request completes it.

#### Roles

A User's `roles` (RFC 7643 §4.1.2) are the roles the access policy's
[controller rules](../access-policy.md#controller-rules) name. Each
value is one `ROLE` the User's `PERSON` performs, of the professional role
template, whose `name` is the value coded in the terminology the client's
`scim_provision` grant names, and whose `time_validity` starts on the day it
is added. The grant states which roles the client may hand out:

| Element | Value |
|---|---|
| `role_terminology` | The terminology id the client's role values are coded in, exactly one |
| `role` | A role code the client may assign, one element per code |

A request whose `roles` names a code the grant does not list, or any role
when the grant names no `role_terminology`, is **403** and changes nothing.
A `POST`, `PUT` or `PATCH` makes the User hold exactly the roles its result
lists: a new value commits a new `ROLE`, and a removed one ends its `ROLE`
the day before (its `time_validity` gains the date of the removal as its
exclusive end) and drops it from the `PERSON`'s `roles`. The change decides
the next request on the replica that took it, and on every other replica
once the commit's notification arrives, with no added transaction. A token's
role claims never drive a controller rule. These `ROLE`s carry no
capability, so a SCIM client writes them, and an administrator alone grants
a capability. RFC 7643 sets no vocabulary for `roles` and RFC 7644 §2 leaves
the authorization model out: the mapping is FerroEHR's own design.

The versions of a deleted User are kept for the record-keeping duties that
need the professional's identification (GDPR Art. 6(1)(c), laid down in law
as Art. 6(3) requires: in the Netherlands BW 7:454(3), twenty years, and the
logging duty of the Besluit elektronische gegevensverwerking door
zorgaanbieders). Outside the Netherlands, name your national retention
provision. Enable the identity provider's protection against accidental
deletions beside `max_deactivations`.

Only the attribution minimum waits for that horizon: the name, the binding key
and the identifiers, `userName` and `externalId` among them. The contacts
serve no attribution, so FerroEHR erases them (GDPR Art. 5(1)(e)): with
`auth.scim.contact_erasure_delay = 0` the deletion removes the `contacts` of every
version of the `PERSON` in the same transaction, and with a period a sweep
that runs every minute removes them once the period has passed. The sweep
reaches every deleted `PERSON` whose binding key the `binding_issuer` of a
`scim_provision` grant issues, one that has ended included,
an administrator's demographic `DELETE` included. Each version keeps its uid,
audit and contribution, and its node rows are derived again from the erased
body. A server signature on such a version is removed, because it was taken
over the content the erasure changed. Each erasure is an access record, a
delete of `party:<id>#contacts`. A login on the key is still refused as
deprovisioned.

### Groups

When `[authz.party_binding.group]` names a `display_name` and
`[authz.party_binding.membership]` is set, `/scim/v2/Groups` is served beside
`/Users`, and `/scim/v2/ResourceTypes` lists the Group. A Group is a
demographic `GROUP` of the group template:

- its `displayName` is the `GROUP`'s `PARTY_IDENTITY`, unique across the live
  Groups after case folding, so a duplicate is **409** `uniqueness`;
- its `externalId` is a `DV_IDENTIFIER` at the `external_id` selector under
  the client's group issuer: the `group_identifier_issuer` of the
  `[auth.issuer.<name>.party]` table whose `identifier_issuer` is the client's
  `binding_issuer`, so the group claim of that issuer's tokens matches it.
  When that table sets none, the group issuer is the `binding_issuer` with
  `/groups` appended (`urn:example:login` gives `urn:example:login/groups`),
  with any trailing `/` of the `binding_issuer` dropped first. A deployment
  whose tokens assert groups sets `group_claim` and a
  `group_identifier_issuer` in the table, and that value is the group issuer;
- each member holds one inline `PARTY_RELATIONSHIP` of the membership
  archetype, with its `PERSON` as `source` and the `GROUP` as `target`,
  stored in the member's own version (RM demographic §Party Relationships).
  No `ROLE` is created for a Group, and a `PERSON` in no Group has
  `relationships` Void.
- its `careTeam`, in FerroEHR's Group extension, says whether it is a care
  team: set at creation, `false` when left out, and never changed (**400**
  `mutability`). A care team is served with `careTeam = true` and the
  extension in its `schemas`; any other Group is served without either. A
  care team's `GROUP` is named by the coded `care_team` and each membership
  by the coded `member_of` of `openehr_access` (RM demographic `PARTY.type()` is the `name`), so
  its members are in the care of the patients under the team's care.
  Creating one and changing its members needs the client's
  `write_care_relationship` grant beside `scim_provision`, and is **403**
  without it ([Care teams provisioned over SCIM](../caller-facts.md#care-teams-provisioned-over-scim)).
  The group template and the membership archetype must admit a
  `DV_CODED_TEXT` name, as the default definitions do.

A Group belongs to the client that created it; every other client gets
**404**. A member is a User of the same client: a member naming another
client's User, an unknown id or a Group (nested Groups are not supported) is
**400** `invalidValue`. What a Group allows stays with the token's scopes and
roles; the access record lists the Groups the token names that the person
belongs to, and a Group in `inactive` or `abandoned` lists no member.

- `POST /scim/v2/Groups` creates the `GROUP` with its members, if any, and
  answers **201** with the Group, its `Location` and `ETag`.
- `GET /scim/v2/Groups/{id}` and `GET /scim/v2/Groups` filter with `eq` and
  `and` over `displayName`, `externalId` and `id`, and
  `excludedAttributes=members` leaves the members out. Each member is served
  with its `value`, `$ref` and `type`.
- `PATCH /scim/v2/Groups/{id}` answers **204** with the new `ETag` (RFC 7644
  §3.5.2). `add` of `members` adds the listed members; `remove` with the
  filter `members[value eq "<id>"]` removes that member, and `remove` of
  `members` with neither a filter nor a value list removes them all.
- `PUT /scim/v2/Groups/{id}` replaces the `displayName` and the members,
  keeps a left-out `externalId`, and answers **200** with the Group.
- `DELETE /scim/v2/Groups/{id}` logically deletes the `GROUP` with its
  memberships, and every later request on the id is **404**.

**Entra compatibility deviation.** A `remove` of `members` that carries a
`value` list removes only the listed members, the request Microsoft Entra ID
sends. RFC 7644 §3.5.2.2 removes "the attribute and all values" when no
filter is specified; FerroEHR follows Entra only when the `value` list is
present.

A membership change is a new version of each member's `PERSON` it touches,
in one CONTRIBUTION, and the `GROUP` gets no version; a change costs the same
whatever the Group's size. The Group's `ETag` is its `GROUP` version with a
membership revision that moves with every membership change, so it changes
whenever its members do (RFC 7644 §3.14). The member User's `ETag` changes
too, since its `PERSON` holds the membership (RFC 7643 §4.1.2); its body does
not, because the User serves no `groups` attribute. Deleting a User deletes
the memberships its `PERSON` holds with it. Deleting a Group drops the
membership of each active member in the CONTRIBUTION that deletes the
`GROUP`. A `GROUP` an administrator set `inactive` or `abandoned` keeps its
version, since the version lifecycle draws no update of either state, and a
SCIM write to such a Group is **400** `invalidValue`.

**Deactivated Users leave their Groups.** The version lifecycle draws no
update of an `inactive` or `abandoned` `PERSON` (RM common master06 §Version
Lifecycle), so such a `PERSON` cannot take a membership change, and an
inactive `PERSON` binds to nothing. Setting a User's `active` to `false`
therefore drops its memberships in the same version, and each Group's
`ETag` changes. A membership of a care-team Group stays, as does every other
care relationship, since SCIM never changes one ([Deprovisioning a team
member](../caller-facts.md#deprovisioning-a-team-member)). Microsoft Entra ID often disables a user and then removes it
from its groups; the later `remove` finds nothing to remove and answers
**204**. An `add` naming an inactive or abandoned User is **400**
`invalidValue` ("reactivate the User first"). A reactivated User belongs to
no plain Group until its client adds it again.

### Provision from Microsoft Entra ID

Microsoft Entra ID provisions through a non-gallery enterprise application.
FerroEHR validates the access token your own authorization server issues to
it, so the steps split between the two.

1. **Map the person template.** The default
   [party mapping](config-authz.md#the-party-mapping) maps `userName`, `externalId`, the
   formatted, given and family name and the work email Entra sends. Set the
   person section only to map more, or to key the binding on the object id
   as the worked example there does.

   A value path such as `emails[type eq "work"].value` serves the value and
   its `type`. FerroEHR holds one value per `type` and no preference among
   them, so `/scim/v2/Schemas` declares `primary` read-only and the `primary`
   Entra sends is ignored (RFC 7644 §3.3).
2. **Grant the client.** Grant its client `scim_provision`, with `ferroehr
   party-binding grant` or as an administrator through the Demographic API:
   the `--issuer` is your authorization server, the `binding_issuer` element the `identifier_issuer` of the
   `[auth.issuer.<name>.party]` your professionals log in through, and the `token_scope`
   element the scope its token holds. Set `server.public_base_url` to the URL
   Entra reaches. In the Helm chart, a `config.auth.scim` value also adds the
   `/scim/v2` path to the Ingress. `ferroehr config check` refuses a mapping
   the endpoint cannot serve.
3. **Federate Entra's credential at your authorization server.** Entra's
   Workload Identity Federation presents a signed assertion, which your
   authorization server exchanges for a client-credentials access token
   (RFC 7523 §2.2). These checks are your authorization server's, not
   FerroEHR's, so verify each one there, as Microsoft's "Workload Identity
   Federation for SCIM Provisioning" guide requires:
   - [ ] `iss` is pinned to your full tenant issuer,
     `https://login.microsoftonline.com/<tenant>/v2.0`, and the assertion
     verifies against that tenant's key set.
   - [ ] `aud` is pinned to `api://<workload-identity-app-id>`, with one
     workload-identity application per FerroEHR deployment.
   - [ ] `azp` is pinned to the application id of Microsoft's provisioning
     service.
   - [ ] `uti` is cached for the lifetime of the assertion, and an assertion
     whose `uti` was seen before is refused as a replay.
   - [ ] The access token it issues holds the grant's `token_scope`, names the
     client in `client_id`, and lives between 1 and 6 hours, the range
     Microsoft's guide asks for.

   The assertion's `sub` is the object id of a Microsoft application that
   every integration in the tenant shares, never your `client_id`, so it does
   not meet RFC 7523 §3 item 2.B. Accepting it is a deviation of your
   authorization server; record it there. Leaving Entra's **Secret Token**
   blank, so that Entra uses its own token, is not supported: Microsoft
   reserves it for testing, and every non-gallery application in the tenant
   receives a token with the same `iss` and `aud` (RFC 9700 §2.3). A
   long-lived bearer token is not supported either.
4. **Point Entra at FerroEHR.** Set the **Tenant URL** to
   `<server.public_base_url>/scim/v2` and run **Test Connection**. Entra queries
   `/Users` with `filter=externalId eq "<random guid>"`, which FerroEHR
   answers with an empty `ListResponse`, and fetches a random User and a
   random Group, each answered **404**.
5. **Map the attributes in Entra.** Map `userName`, `externalId`, `active`
   and the attributes of step 1. Map the attribute that carries the
   professional's login identifier to
   `urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:loginIdentifier`;
   Entra sends it without listing the extension in `schemas`, which FerroEHR
   accepts. An attribute the person template does not map is refused as
   undefined, so remove the rest of Entra's default mappings.
6. **Protect against mass deactivation.** Turn on Entra's accidental
   deletions prevention, with a threshold at or below `max_deactivations`.
7. **Provision groups.** Map the group's `objectId` to its `externalId`, so
   the object ids Entra puts in the `groups` claim match, and provision only
   the groups assigned to the application, which keeps a person's token under
   the groups overage limit. Entra creates each Group with no member, reads it
   with `excludedAttributes=members`, finds it by `displayName`, and adds and
   removes members with PATCH.

Entra writes `Replace` and `Add` with a capital letter, disables a User with
`active = false`, sends `DELETE` once the user is removed, and never sends a
null. A value cleared in Entra therefore stays in FerroEHR until the User is
replaced or deleted, or an administrator removes it.
