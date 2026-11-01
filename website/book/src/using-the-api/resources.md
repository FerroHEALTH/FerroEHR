# Resource walkthroughs

This chapter walks through the core openEHR resources (**EHR**, **EHR_STATUS**,
**COMPOSITION**, **DIRECTORY**, **CONTRIBUTION** and **ITEM_TAG**) with real
`curl` examples you can adapt. For each resource it shows the operations, the
headers they need, and the status codes they return. Paths are relative to the
base `/ferroehr/rest/openehr/v1` (see [Using the API](index.md)); examples use
Basic auth (`-u ferroehr:ferroehr`) and JSON. Content negotiation, the `Prefer`
header, and `ETag`/`If-Match` versioning are cross-cutting and get their own
chapter, [Content negotiation & errors](content-negotiation.md); this chapter
uses them in context.

**Datetime parameters.** Several operations below take a point in time
(`version_at_time`, the CONTRIBUTION `time_range` bounds). Write it in the
_extended_ ISO 8601 form: `YYYY-MM-DDThh:mm:ss.sss[Z|±hh:mm]`, e.g.
`2016-06-23T13:42:16.117+02:00`. The timezone is optional: leave it off
(`2016-06-23T13:42:16`) and the value is read in the **server's** local
timezone, so supply `Z` or an explicit offset whenever the client's timezone may
differ from the server's. The time itself is required: a bare date
(`2016-06-23`), the compact "basic" ISO form (`20160623T134216Z`), a
zone-annotated form (`2016-06-23T13:42:16[Europe/Amsterdam]`), and anything
unparseable all return **400 Bad Request**.

<!-- toc -->

## EHR

An **EHR** is the top-level container for one subject's health record.

### Create an EHR

```shell
curl -u ferroehr:ferroehr -X POST -i \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr
```

`POST /ehr` — the body is optional; you may supply an `EHR_STATUS` to set the
subject and flags at creation. Returns **201 Created** with the new EHR id in
`ETag` and a `Location` header. With `Prefer: return=representation` the body is
the full `EHR`; otherwise it is empty. Supplying an `EHR_STATUS` whose subject
already has an EHR returns **409 Conflict**.

With no body the server mints the spec's default `EHR_STATUS`
(`is_queryable: true`, `is_modifiable: true`, a `PARTY_SELF` subject, and an
`archetype_details` block naming `openEHR-EHR-EHR_STATUS.generic.v1`). A
supplied `EHR_STATUS` must be a complete, RM-valid instance: `EHR_STATUS` is
always an archetype root, so `archetype_details` (with its `archetype_id`) is
mandatory, and a body without it is refused with **422** naming the violated
invariant. Some servers accept such a partial status and fill the gap
silently; FerroEHR validates the supplied resource exactly as the Reference
Model defines it, so either send the complete status or omit the body and let
the server mint the default.

To create with a specific id, use `PUT /ehr/{ehr_id}` (also 201; 409 if that id
is already used).

Once the deployment declares `[privacy] subject_namespaces`, a supplied
`EHR_STATUS.subject.external_ref` must name one of those namespaces and carry a
UUID; anything else is a **422** naming the RM path and never echoing the value.
Left undeclared, that rule is out of force and whatever you send is stored. See
[Privacy & data minimisation](../installation/config-privacy.md).

### Retrieve an EHR

```shell
curl -u ferroehr:ferroehr \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID
```

`GET /ehr/{ehr_id}` returns **200** with the `EHR`, or **404** if unknown. You
can also look one up by subject: `GET /ehr?subject_id=…&subject_namespace=…`
(both parameters required).

The `EHR` root object is not itself versioned, so its reads carry a weak `ETag`
built from `EHR.ehr_id.value` but no `Last-Modified`.

## EHR_STATUS

**EHR_STATUS** holds the record's metadata: the link to the subject, and the
`is_queryable` / `is_modifiable` flags. It is itself versioned.

Setting `is_modifiable` to `false` **deactivates** the EHR: any attempt to
create, update, or delete its content (a composition, the directory, or a
folder) is refused with **409 Conflict**, through *every* write path including a
CONTRIBUTION commit. The EHR_STATUS itself stays writable (so you can set the
flag back to `true` to reactivate), and reads and queries are unaffected.

### Read the current status

```shell
curl -u ferroehr:ferroehr \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/ehr_status
```

`GET /ehr/{ehr_id}/ehr_status` returns the current `EHR_STATUS`, its version id
in `ETag`. Add `?version_at_time=<ISO 8601>` to read it as of a point in time.
`GET …/ehr_status/{version_uid}` reads a specific version.

### Update the status

Updates require the current version id in an `If-Match` header (optimistic
concurrency):

```shell
curl -u ferroehr:ferroehr -X PUT \
  -H 'Content-Type: application/json' \
  -H 'If-Match: "<current-version-uid>"' \
  -H 'Prefer: return=representation' \
  --data-binary @ehr-status.json \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/ehr_status
```

`PUT /ehr/{ehr_id}/ehr_status` returns **200** (with representation) or **204**
(minimal), plus the new `ETag`. A stale or wrong `If-Match` returns **412
Precondition Failed** with the current version id in `ETag`.

### Status version history

The `versioned_ehr_status` sub-resource exposes the full version history:

- `GET …/versioned_ehr_status`: the `VERSIONED_EHR_STATUS` object,
- `GET …/versioned_ehr_status/revision_history`: the revision history,
- `GET …/versioned_ehr_status/version` (optionally `?version_at_time=`) and
  `…/version/{version_uid}` — a specific version.

## COMPOSITION

A **COMPOSITION** is a committed clinical document, validated against its
template.

### Create a composition

```shell
curl -u ferroehr:ferroehr \
  -H 'Content-Type: application/json' \
  -H 'Prefer: return=representation' \
  --data-binary @composition.json \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/composition
```

`POST /ehr/{ehr_id}/composition` returns **201 Created** with the version id in
`ETag`. A body that cannot be **constructed** as a COMPOSITION (malformed JSON,
an undeclared or repeated member, a missing mandatory attribute, an empty list
the model requires non-empty, a `_type` foreign to its slot) is **400 Bad
Request**: parsing is the shape check, so structural defects never reach
validation. A body that constructs but fails **semantic** validation (template
constraints, RM invariants, terminology bindings) returns **422 Unprocessable
Entity** with the errors; an unknown EHR, **404**.

### Retrieve a composition

```shell
curl -u ferroehr:ferroehr \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/composition/$UID
```

`GET /ehr/{ehr_id}/composition/{uid_based_id}` accepts either a full version id
(`<uuid>::<system>::<n>`) or a bare object uuid (in which case add
`?version_at_time=` to pick a point in time). It returns **200** with the
composition, **204** if the composition was (logically) deleted at that time, or
**404**.

### Update and delete

```shell
# Update — If-Match is the CURRENT version id; the URL uses the bare object uuid
curl -u ferroehr:ferroehr -X PUT \
  -H 'Content-Type: application/json' \
  -H 'If-Match: "<current-version-uid>"' \
  --data-binary @composition.json \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/composition/$OBJECT_UUID

# Delete — the URL uses the FULL version id
curl -u ferroehr:ferroehr -X DELETE \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/composition/$VERSION_UID
```

`PUT` returns **200**/**204** (per `Prefer`) with the new version id, **412** on
an `If-Match` mismatch, **422** on validation failure. `DELETE` is a *logical*
delete (the history is retained), returning **204** with the new deleted
version's `ETag`; deleting something already deleted returns **400**, and a
version id that is not the latest returns **409**.

> [!WARNING]
> Watch the id you pass. `PUT` takes the **object uuid** (the versioned object),
> while `DELETE` takes the **full version id** (the version you are
> superseding). `GET` accepts either.

> [!NOTE]
> A version's lifecycle state (set through the `openehr-version:
> lifecycle_state.code_string` header; see
> [Content negotiation & errors](content-negotiation.md); the default on a
> commit is `532|complete|`) must follow the openEHR version-lifecycle state
> machine. An illegal transition is rejected with **422 Unprocessable Entity**
> naming the states. In particular, a version left in the `801|abandoned|`
> state cannot be updated straight to `complete`: you must first retrieve it
> back to `553|incomplete|`, then complete it.
>
> Two further rules apply to the state a commit may claim:
>
> - **`523|deleted|` belongs to `DELETE` alone.** Deleting is one act (a new
>   version whose data is removed and whose state is `deleted`), so a `PUT`
>   or `POST` that carries content may not claim it. Such a request is
>   rejected **422**. Conversely, a `DELETE` that supplies a lifecycle other
>   than `523|deleted|` is rejected **400**: the value would have to be
>   discarded, and the server tells you rather than pretending to honour it.
>   A `DELETE` with no lifecycle header at all is the normal case and is
>   unaffected.
> - **`553|incomplete|` relaxes what a commit must contain.** Content
>   committed as `incomplete` may leave mandatory attributes absent and
>   `1..*` containers empty, for compositions, folders, and demographic
>   parties and relationships alike. Everything else is still checked: types,
>   terminology codes, patterns and archetype constraints. Missing content is
>   allowed here; *wrong* content is still rejected **422**. The
>   `EHR_STATUS` resource is the one exception: it does not accept the
>   `incomplete` state.

### Composition version history

`GET …/versioned_composition/{versioned_object_uid}` and its `revision_history`,
`version`, and `version/{version_uid}` sub-resources mirror the EHR_STATUS
history endpoints.

## DIRECTORY

The **DIRECTORY** is an optional `FOLDER` tree for organising compositions
within an EHR. The `/directory` endpoints manage the EHR's primary hierarchy
(the openEHR `EHR.directory`, which is always the first member of
`EHR.folders`).

An EHR can also index **additional folder hierarchies** beyond the directory:
commit further root `FOLDER`s through the CONTRIBUTION endpoint (the openEHR
REST API defines no dedicated endpoint for them). The EHR resource then lists
every live hierarchy in its `folders` attribute, in creation order, with
`directory` always equal to the first member; deleting the directory promotes
the next live hierarchy.

```shell
# Create the directory
curl -u ferroehr:ferroehr \
  -H 'Content-Type: application/json' \
  --data-binary @folder.json \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/directory

# Read it (optionally at a time, or a sub-path)
curl -u ferroehr:ferroehr \
  'http://localhost:8080/ferroehr/rest/openehr/v1/ehr/'$EHR_ID'/directory?path=episodes/2024'
```

- `POST /ehr/{ehr_id}/directory`: create the root folder; **201**.
- `PUT /ehr/{ehr_id}/directory`: update it; requires `If-Match`; **200**/**204**.
- `DELETE /ehr/{ehr_id}/directory`: logical delete; requires `If-Match`;
  **204**.
- `GET /ehr/{ehr_id}/directory`: the current folder tree, optionally filtered
  by `?version_at_time=` and `?path=` (slash-separated folder names). **204** if
  deleted at that time.
- `GET /ehr/{ehr_id}/directory/{version_uid}`: a specific version, optionally
  `?path=`.

Folder `items` are `OBJECT_REF`s, and the server validates the ones that claim
**this system**: a reference whose `namespace` is `local` (or the server's
configured system id) must resolve to a versioned object in that EHR, or the
commit is refused with **422** naming each unresolvable reference at its tree
path. References into other namespaces (another system's id, or `unknown`)
are stored verbatim without a resolvability check, since openEHR object
references are explicitly allowed to point outside the current system. The
same rule applies to folder hierarchies committed through the CONTRIBUTION
endpoint.

## CONTRIBUTION

A **CONTRIBUTION** is an atomic change-set: a group of versioned-object changes
(compositions, statuses, folders) committed together with one shared audit. Use
it when several changes must land as a unit.

```shell
curl -u ferroehr:ferroehr \
  -H 'Content-Type: application/json' \
  --data-binary @contribution.json \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/contribution
```

`POST /ehr/{ehr_id}/contribution` takes a contribution whose `versions` array
each describe a change (the RM object, its `lifecycle_state`, and a
per-version `commit_audit` with its `change_type`) plus a shared `audit`. The
body is read through the released `NewContribution` schema, and a body that
does not match it is refused with a **400**. It returns **201** with the
contribution id in `ETag`, or **400**/**404**/**409**/**422** on invalid input,
unknown EHR, a uid conflict, or a change set that is well-formed but cannot be
followed.

A minimal entry that creates a composition looks like this:

```json
{
  "versions": [
    {
      "lifecycle_state": {
        "value": "complete",
        "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "532" }
      },
      "commit_audit": {
        "_type": "UPDATE_AUDIT",
        "change_type": {
          "value": "creation",
          "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "249" }
        },
        "description": { "_type": "DV_TEXT", "value": "first draft" },
        "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" }
      },
      "data": { "_type": "COMPOSITION", "…": "…" }
    }
  ],
  "audit": {
    "change_type": {
      "value": "creation",
      "defining_code": { "terminology_id": { "value": "openehr" }, "code_string": "249" }
    },
    "committer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Jones" }
  }
}
```

These things about the payload are worth calling out:

- **Every audit carries its own `change_type` and `committer`**, the shared
  `audit` and each `commit_audit` alike. They are your account of the change
  and are never derived or invented by the server: a contribution without an
  `audit`, or an audit that omits either one, is refused with a **400**. The
  `change_type` is a `DV_CODED_TEXT` naming a code of the openEHR *audit change
  type* group, such as `249` (creation), `251` (modification), `523` (deleted)
  or `666` (attestation); a code outside the group is a **422**. The server
  fills in `time_committed`, and `system_id` when you do not supply one. A
  `system_id` that is not a string is a **400**, and an empty one a **422**.
- **An audit is sent as `UPDATE_AUDIT`**, `AUDIT_DETAILS` or with no `_type`.
  An `ATTESTATION` or `UPDATE_ATTESTATION` audit is a **400**: attestations go
  in the version's `attestations` list (see below).
- **Every `commit_audit` names its own `committer`.** The server never copies
  the shared audit's committer into a version that leaves it out: that version
  is refused with a **400**. A committer that is not a `PARTY_PROXY` object at
  all, such as a bare string, is a **400** too; one that is a `PARTY_PROXY` but
  breaks its own rules, such as a `PARTY_IDENTIFIED` with no name, identifiers
  or reference, is a **422**. A version's committer is stored as you send it,
  even when it differs from the shared audit's. A version that omits
  `system_id` gets the shared audit's, and every audit in the contribution
  carries the same `time_committed`.
- **A committer that names a login must name you.** On every write, a
  contribution or a direct write of a composition, EHR status, directory,
  EHR or demographic object, the server compares the committer with the
  login that authenticated the request where it can. A committer whose
  `identifiers` hold a `DV_IDENTIFIER` with the `issuer` of your token (or
  `ferroehr`, for a login this server holds), or whose `external_ref` is a
  `GENERIC_ID` in that `scheme`, must carry your own login, or the write is
  refused with a **403**. A `PARTY_SELF` committer states that the record's
  person committed, so it is a **403** unless the request comes from the
  person, whose writes the server records as `PARTY_SELF` itself. A committer
  the server cannot compare with a login, such as a name only or an
  identifier from a professional register, is stored as you send it. The
  access record of the write names the login that made it. With
  authentication off, nothing is compared. The committer's own rules (the
  **422** above) apply on the direct writes as well.
- **A `description` is a `DV_TEXT` or `DV_CODED_TEXT` object that names its
  `_type`.** A plain string, or an object without `_type`, is a **400**. A
  coded description keeps its `defining_code`. The same holds for an
  attestation's `reason` and `description`.
- **Members an audit or the contribution does not declare are accepted and
  ignored.** The released schemas leave both objects open, so a `reason` or
  `is_pending` on a plain audit, or an extra top-level member, changes nothing.
- **`lifecycle_state` is required on every version** and is a
  `DV_CODED_TEXT`, as in the example. Omitting it, or sending it as a plain
  string or as `{"terminology_id": …, "code_string": …}`, is a **400**, on an
  attestation entry (see below) as well, even though an attestation commits no
  new version. A code outside the openEHR *version lifecycle state* group is a
  **422**.
- **A version entry may carry members the schema does not declare.** The
  released `UPDATE_VERSION` schema leaves the object open, so an extra member
  that means nothing on a locally created `ORIGINAL_VERSION`, such as `item`
  (an imported version's attribute) or an extension of your own, is accepted
  and ignored. An imported version comes only through the import route.
- **A version entry records its merge inputs.** When the version merges other
  versions, list them in `other_input_version_uids` as `OBJECT_VERSION_ID`
  objects (`{"value": "…::…::…"}`, `_type` optional). The committed
  `ORIGINAL_VERSION` stores them as given and serves them on every read and in
  the signed form. They may name versions this server does not hold. An empty
  list, an entry that is not an `OBJECT_VERSION_ID`, or the list on an
  attestation entry is a **422**.
- **A version entry never states what the server assigns.** The server assigns
  the identity of every version it commits and links it to the contribution
  being created, so a `uid` or a `contribution` on a version entry is a
  **400** naming `versions[i]/uid` or `versions[i]/contribution`. The optional
  `_type` self-tag is `UPDATE_VERSION` or `ORIGINAL_VERSION`; any other class
  is a **400**.

A version's `attestations` list carries `UPDATE_ATTESTATION` objects, each with
its own `change_type`, `committer`, `reason` (required) and `is_pending`
(required), and optionally `proof`, `items`, `attested_view` and `system_id`.
On an entry that commits content, this is how content is committed already
signed, or marked as awaiting signature (`is_pending: true`); the attestations
are part of the version the server signs, and are read back on the version
envelope, in the revision history, and in exports. A coded `reason` must be a
member of the openEHR *attestation reason* group, `items`, when present, must
be non-empty, and the committer follows the same rules as an audit's. An
attestation without `system_id` gets its version audit's.

An **attestation entry** signs or reviews a version that is already stored,
without committing a new one. Give its `commit_audit` the change type `666`
(*attestation*), name the attested version in `preceding_version_uid`, leave
`data` out, and put one or more `UPDATE_ATTESTATION`s in its `attestations`; an
attestation entry without any is a **400**. The entry still needs a
`lifecycle_state`, which the server checks against the *version lifecycle
state* group (a **422** when it is outside it) but does not store, because no
new version carries it.

`GET /ehr/{ehr_id}/contribution/{contribution_uid}` returns **200** with the
contribution, or **404**. Add `Prefer: resolve_refs` to get full VERSION objects
instead of `OBJECT_REF`s (see
[Content negotiation & errors](content-negotiation.md#prefer-resolve_refs)).

`GET /ferroehr/rest/extensions/v1/ehr/{ehr_id}/contribution` (no uid) lists the
EHR's contributions, newest first. It is a FerroEHR extension (the openEHR REST
API defines only the by-uid read), so it sits under the extensions root beside
the openEHR API. Paginate with `?offset=` (default 0) and `?fetch=` (default 20, capped at
100). It returns **200** with a JSON summary, or **404** for an unknown EHR:

```json
{
  "rows": [
    {
      "uid": "…",
      "time_committed": "…",
      "committer": "…",
      "change_type": "251",
      "change_type_rubric": "amendment"
    }
  ],
  "total": 123
}
```

`committer` is the audit committer's *name* only; the by-uid read returns the
full `PARTY_PROXY`. `change_type` is the openEHR audit-change-type code and
`change_type_rubric` its display rubric from the same terminology the by-uid
read uses, so a client never maps codes locally. `total` counts **all** of the
EHR's contributions, not just the returned window.

> [!NOTE]
> The contribution envelope is **canonical JSON only**: openEHR publishes no
> CONTRIBUTION XML document, so an XML `Accept` on these routes is a **406** and
> an XML `Content-Type` a **415**. The FLAT and STRUCTURED formats, when used,
> apply only to the inner composition `data` of each version, never to the
> envelope.

## ITEM_TAG

**ITEM_TAG**s are small `key`/`value` annotations on a versioned object or on
one specific version, optionally pointing at a node inside the data through
`target_path`. They carry no clinical meaning and do not create a new version.
Use them for workflow state, review flags, and integration bookkeeping.

```shell
# Every tag in an EHR, optionally filtered
curl -u ferroehr:ferroehr \
  'http://localhost:8080/ferroehr/rest/openehr/v1/ehr/'$EHR_ID'/tags?tag_key=flag'

# Replace the tag list of one composition (container-wide, by object uuid)
curl -u ferroehr:ferroehr -X PUT \
  -H 'Content-Type: application/json' \
  -H 'Prefer: return=representation' \
  --data-binary '[{"key":"flag","value":"follow-up"}]' \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/composition/$OBJECT_UUID/tags

# Delete every tag under one key
curl -u ferroehr:ferroehr -X DELETE \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/composition/$OBJECT_UUID/tags/flag
```

- `GET /ehr/{ehr_id}/tags`: every tag in the EHR, whatever it targets. The
  optional `tag_key`, `tag_value` and `tag_target_path` filters are exact,
  case-sensitive, and AND-combined; an omitted filter constrains nothing. An EHR
  with no matching tag answers **200** with `[]`, never 404.
- `GET|PUT /ehr/{ehr_id}/composition/{uid_based_id}/tags` and the same pair
  under `…/ehr_status/{uid_based_id}/tags` — read or **replace** the addressed
  collection. The `PUT` body is a bare JSON array of tags (`key` required,
  `value` and `target_path` optional; `target` and `owner_id` come from the
  route, and a body that supplies them, or any other undeclared member, is a
  **400**). An empty array `[]` is the clear-all form, never an error. `PUT`
  answers **204** by default, **200** with the resulting list under
  `Prefer: return=representation`.
- `DELETE …/tags/{key}`: delete every tag under one key on the addressed
  collection; **204**.

Two properties shape how you address them:

- **The id you pass selects *which* collection.** A full version id
  (`OBJECT_VERSION_ID`) addresses that one version's tags; a bare object uuid
  (`HIER_OBJECT_ID`) addresses the versioned-object container's tags.
- **The two collections are disjoint.** A tag has exactly one target, so
  replacing the container's list never touches any version's list, and
  replacing one version's list never touches the container's or a sibling
  version's.

Every returned tag carries a server-assigned `target` (the bare uid of what it
tags) and `owner_id` (an `OBJECT_REF` to the owning EHR). Tags can also ride
along with a content write instead of taking a second round trip; see
[Item tags via headers](content-negotiation.md#item-tags-via-headers).

## Status-code summary

| Code | Meaning across these resources |
|---|---|
| 200 | Retrieved, or updated with `Prefer: return=representation` / `return=identifier`. |
| 201 | Created (EHR, composition, directory, contribution). |
| 204 | Success with no body (`return=minimal`), or deleted / deleted-at-time. |
| 400 | Malformed request, missing required header/parameter, an undeclared payload member, or already-deleted. |
| 404 | Unknown EHR, object, version, or no version at the requested time. |
| 405 | Method not allowed on this resource (always with an `Allow` header), or the resource is switched off by configuration. |
| 406 | The `Accept` header names no representation this resource has. |
| 409 | Conflict: a duplicate subject or id, a version that is not the latest, or a content write to a deactivated (`is_modifiable = false`) EHR. The deactivation check runs before any content is validated, so a defective body on a deactivated record reports the state conflict rather than a `422`. |
| 412 | `If-Match` did not match the latest version (current id returned in `ETag`). |
| 415 | The request `Content-Type` names a format this resource cannot process. |
| 422 | Well-formed but unfollowable: failed template or semantic validation, a body the [data-minimisation rules](../installation/config-privacy.md) refuse, an illegal version-lifecycle transition, or a change set the server cannot apply. |
| 429 | Too many requests, with `Retry-After` and the `x-ratelimit-*` headers the limiter computed. |
| 503 | The server cannot decide or cannot serve right now, with `Retry-After`: the admission cap is full, the token issuer is unreachable, or (under `[audit] fail_mode = "closed"`) the access record could not be taken. Retry; none of these is a statement about your request. |

The [Content negotiation & errors](content-negotiation.md) chapter covers the
error body shape and the headers referenced above in full.
