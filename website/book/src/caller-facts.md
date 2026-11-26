# Roles, care relationships and representatives

The facts the access evaluator reads about a caller: the roles it performs, its care
relationships and the persons it represents. How they take part in a decision is on
[How FerroEHR decides access](explanation/access-model.md).

<!-- toc -->

## Caller facts: roles and care relationships

What FerroEHR stores and decides on, and what it leaves to your applications
and your deployment, is listed in
[What the CDR leaves to the application](application-boundary.md).

Beside its grants, a caller carries the facts the access evaluator reads:
the roles it performs, the care teams it belongs to and the providers it
acts for. A patient carries its care relationships. FerroEHR reads them in
any archetype (a membership by `openehr_access` `member_of`, care by
`under_care_of`), never by an archetype id, and holds them
on every replica beside the grants, so a request reads them with no
transaction of its own.

**Roles.** A `ROLE` whose `name` is a `DV_CODED_TEXT` of the terminology
your jurisdiction names, such as a national professional role code, is a
role its `performer` holds while the `ROLE` is in force: its `time_validity`
holds the date of the request and it is not deleted. The role's type is its
name (RM Demographic IM, `PARTY.type()`), so a nurse is one `ROLE` per nurse,
each named by the nurse code. A [controller rule](access-policy.md#controller-rules) names a
role in an `applies_to` entry, compared by terminology and code. A `ROLE`
named in text, or by a code of `openehr_access`, names no role. A
token's role claims never count for a rule: they decide only the coarse RBAC
class gate ([Security](security.md)). The one exception is a caller admitted
under a [federation agreement](operational-policy.md#federation-agreements): the professional
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
rule](access-policy.md#controller-rules) names them in an `applies_to` entry's
`organisation_type`, compared by terminology and code.

**Memberships.** A `PARTY_RELATIONSHIP` in a `PERSON`'s `relationships`
whose `name` or `details` carry `member_of` of `openehr_access` makes the
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
| `basis` | 0..1 | `assigned` (the default); `self_declared` where the policy sets `self_declared_care` ([Writing a care relationship](access-policy.md#writing-a-care-relationship)) |

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

**Care scoped to an episode.** A care relationship that names an `episode`
admits on care only the data of that episode, the data placed in no
episode and the data your access policy's `episode_shared` names. A
relationship that names no episode covers the whole record, as before. A
patient admitted to cardiology and seen by dermatology has two episodes: a
cardiologist whose one live relationship names the cardiology episode reads
the cardiology data and the unplaced data, is refused the dermatology data
with `refused-outside-episode` (`403`, step 7 of the access record), and no
query, count or page shows it. An access service caller also reads the data
of the EHDS priority categories (Art. 14(1)) through the category map. An
emergency, the patient, a permit directive and a gate-keeper decide as they
do without episodes. A query can narrow your caseload further to named
episodes with `caseload_episodes` ([Narrowing a query to your
caseload](querying-aql.md#narrowing-a-query-to-your-caseload)).

**Group memberships of a provider.** A `PARTY_RELATIONSHIP` in an
`ORGANISATION`'s `relationships` carrying `member_of` makes that provider a
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
  `member_of` relationship to it; a `GROUP` that belongs to none, such as a
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
allows it ([Writing a care relationship](access-policy.md#writing-a-care-relationship)).
FerroEHR compares what each relationship says (its code, target, role,
provider and validity), never its position in `relationships`, so removing
another relationship before it changes none.

The care delivery environment reaches FerroEHR on two channels, and a
deployment uses either or both:

- **The ADT or rostering system** writes memberships (`member_of`) and `under_care_of`
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
coded `member_of`, and the evaluator counts the member in the team's care on
the next request. FerroEHR reads the kind by that code, never by the template
or a path. `careTeam` is immutable (RFC 7643 §2.2): a
Group never changes kind with its members in it.

Creating a care-team Group, and every membership change of one, needs the
client's `write_care_relationship` grant in the CDR beside its
`scim_provision` grant; without it the request is refused `403` naming the
grant. The Group belongs to the provider the client acts for: its `GROUP`
carries a `member_of` relationship to the client's `acts_for` `ORGANISATION`,
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

## Representatives

A parent, a guardian or a proxy the patient chose reads the patient's record
through an app as the patient's representative. Regulation (EU)
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
for (`[authz.party_binding.provider]`), and that the representative did not
write itself. A representative with no such representation is refused `403`
with `refused-representation-unrecorded`. The access record of
every admitted request names the representation's container, which the
access log's `representation` authorisation carries as its reference.

The representative's `PERSON` is of a template
`[[authz.party_binding.person.template]]` maps and carries a binding key: a
`DV_IDENTIFIER` whose `issuer` is the `identifier_issuer` of the party table
of the issuer they sign in at, and whose `id` is their login there (their
`sub`). An administrator or an identity feed writes it through the
Demographic API; SCIM provisions a workforce only. The `PERSON` carries the
representative marker once it is the source of a representation, and no
professional `ROLE`. EHDS Art. 4(2) gives the authorisation to the patient
(a) and to the legal representative under national law (b), so the patient,
an administrator or a `write_representation` holder records it.

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

A representative signs in at a trusted issuer one of two ways:

- **As a `RelatedPerson`.** Their token's `fhirUser` is a `RelatedPerson`
  (SMART App Launch 2.2.0, Scopes and Launch Context, `fhirUser`), and its
  launch context names the patient's EHR (`ehrId`).
- **As a user whose `PERSON` is a representative's alone.** Their token
  names no `RelatedPerson`, and their login binds a `PERSON` carrying the
  representative marker and no professional `ROLE`.

Either way FerroEHR binds the login, by the token's `sub` under its
issuer's party table, to the `PERSON` whose binding key it is, and admits
the request while that `PERSON` holds a live representation of the subject:
the subject of the launch EHR, else of the EHR the request names, else the
`PERSON` an EHR lookup by subject names. The request is confined to that
subject's EHRs, or to the launch EHR when the token names one. The
representation is read from the CDR, never from the token (EHDS Art. 4(2)).
A representative with no representation yet, one whose representation ended
or was deleted, and one who represents someone else are refused `403` with
`refused-representation-unrecorded`, so a former representative never becomes
the patient. A representative reaching their own record reaches it as the
person themself (EHDS Art. 3). The patient themself is the one whose login
is a binding key of the `PERSON` the EHR's subject names: the party binding
is the one source of own access, and the token's claims are never trusted on
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

### Where this is our own design

No openEHR specification governs these points; each is FerroEHR's own
design:

- **A representation is recognised by its code**, as the acts-for
  relationship is, so a deployment models it in any archetype.
- **Legal capacity is the recorded representations and their validity.**
  The rules are national law (EHDS Art. 4(2)), so FerroEHR holds no rule of
  its own.
- **A login binds by the token issuer.** The representative's binding key
  is a `DV_IDENTIFIER` under the identifier issuer of the party table of the
  issuer that signs their login.
- **A patient writes the representation of themselves**, and the party
  protection holds them to it.
