# Content negotiation & errors

A handful of HTTP mechanisms cut across every openEHR resource: choosing the
wire format (JSON or XML), controlling how much a write returns (the `Prefer`
header), versioned optimistic concurrency (`ETag` and `If-Match`), and the
request headers that enrich a commit (audit metadata and item tags). This
chapter explains them all, plus the shape of error responses, so the examples in
[Resource walkthroughs](resources.md) make sense in general.

<!-- toc -->

## JSON and XML

FerroEHR speaks **canonical JSON** and **canonical XML** for the RM-typed
resources. Choose with the standard HTTP headers:

- **Request body:** set `Content-Type: application/json` or `application/xml`
  (`text/xml` works the same as `application/xml`).
- **Response:** set `Accept: application/json` or `application/xml`. With no
  `Accept`, or with several acceptable formats at equal quality, JSON wins.

JSON is wired end to end for every operation. XML is supported for the
spec-typed RM objects whose canonical XML shape the openEHR ITS-XML schemas
define: a single composition, `EHR_STATUS`, `EHR`, `FOLDER`, the demographic
party types, and the version family (versioned objects and revision history).
Responses that are not a spec-typed RM value (collections, item tags, and the
query and terminology DTOs) are JSON-only, as is the CONTRIBUTION envelope.

```shell
# Commit a composition as XML, ask for XML back
curl -u ferroehr:ferroehr \
  -H 'Content-Type: application/xml' \
  -H 'Accept: application/xml' \
  --data-binary @composition.xml \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/composition
```

### Choosing the XML namespace

openEHR publishes its canonical XML schemas in two lineages, each declaring its
own namespace:

| Lineage | Root namespace | Status |
|---|---|---|
| v1 | `http://schemas.openehr.org/v1` | The schema release openEHR labels stable, frozen against an older Reference Model |
| v2 (**default**) | `http://schemas.openehr.org/v2` | The newer schema release, still marked *trial* by openEHR; the only one that models the current Reference Model |

Pick one per request with a `version` parameter on the XML media type:

```shell
# Read a composition in the v1 namespace
curl -u ferroehr:ferroehr \
  -H 'Accept: application/xml; version=1' \
  "http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/composition/$UID"

# Commit one whose payload already uses the v1 namespace
curl -u ferroehr:ferroehr \
  -H 'Content-Type: application/xml; version=1' \
  -H 'Accept: application/xml; version=1' \
  --data-binary @composition-v1.xml \
  "http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/composition"
```

What to expect:

- **No parameter (or `version=2`) means the v2 default**, and the response
  header stays `Content-Type: application/xml`.
- A v1 response says so: `Content-Type: application/xml; version=1`.
- **On requests the parameter is a courtesy, not a requirement.** The reader
  dispatches on element names and `xsi:type` and never inspects the root
  namespace, so both lineages parse identically whatever you declare; you only
  need the parameter when you want the declaration to be accurate.
- Asking for a lineage the server does not serve (`version=3`, say) is
  **406 Not Acceptable** on `Accept` and **415 Unsupported Media Type** on
  `Content-Type`. The refusal is deliberate: the parameter exists so you can be
  specific about the representation you can consume, and quietly substituting a
  different one would defeat that.
- **Operational templates are always v1.** `GET
  …/definition/template/adl1.4/{template_id}` returns OPT XML in the v1
  namespace and ignores the parameter; it is a template document, not a
  canonical RM resource.

The documents FerroEHR writes for the two lineages are byte-identical apart
from that root namespace: the same elements, the same order, the same `xsi:type`
dispatch. **The two schema bundles are a different story**, and that matters if
you intend to validate.

### If you validate responses against the published XSDs

Both published bundles have a problem today, and knowing which one you are
hitting saves a long debugging session.

**The v1 bundle is frozen at an older Reference Model**, so it does not describe
everything a current openEHR document may contain. It declares no schema at all
for EHR, EHR_STATUS, CONTRIBUTION or the demographic party types, and it omits
attributes that later RM releases added: `FOLDER.details`,
`ELEMENT.null_reason`, `DV_QUANTITY.units_system` and `units_display_name`,
`CODE_PHRASE.preferred_term`, and `ENTRY.workflow_id` among them. A perfectly
valid directory or blood-pressure response can therefore fail v1 validation.
FerroEHR does not drop clinical content to fit the older bundle. That content gap
is why **v2 is the default**: the lineage a schema-validating client receives
without asking has to be the one that models what the server actually emits.

**The v2 bundle currently cannot be compiled at all.** One of its base
simple types constrains `archetypeNodeId` with a Perl-flavoured pattern (using
`(?:…)` non-capturing groups), and the XML Schema pattern language admits no
such construct, so a conformant XSD processor rejects the schema set rather
than the document. The defect sits in every v2 Reference Model folder, not just
the newest, and it is upstream: nothing in a response can work around it.

> [!WARNING]
> As a result there is no bundle today that both compiles *and* models current
> RM content. If you must schema-validate right now, request `version=1` and
> accept that it will reject valid content it never modelled; otherwise validate
> structurally against your own profile and treat the published XSDs as
> documentation. FerroEHR tracks the v2 defect and will say so here when
> upstream fixes the facet.

> [!NOTE]
> The `version` parameter is a FerroEHR extension: the openEHR REST
> specification predates the two schema lineages and says nothing about
> selecting one. It never changes the media type itself (responses are always
> `application/xml`), so a client that ignores it behaves exactly as the
> specification describes.

## Simplified formats (FLAT and STRUCTURED)

Beyond the canonical formats, the server implements the openEHR **Simplified
Formats**: template-driven JSON representations that use friendly field
identifiers (`vital_signs/body_temperature:0/any_event:0/temperature|magnitude`)
instead of full RM paths. Select them the same way as JSON/XML, with these media
types:

| Media type | Meaning |
|---|---|
| `application/openehr.wt.flat+json` | FLAT — one flat JSON object of `path: value` pairs |
| `application/openehr.wt.structured+json` | STRUCTURED — the same data as nested JSON |
| `application/openehr.wt+json` | A template rendered as Web Template JSON (template endpoints only) |

Where they work:

- **Compositions:** full round-trip. Commit with
  `Content-Type: application/openehr.wt.flat+json` (or `…structured…`) and read
  back with the matching `Accept`.
- **Template examples:** `GET …/definition/template/adl1.4/{id}/example` (and
  the ADL2 form) return the generated example in canonical JSON or XML, FLAT, or
  STRUCTURED, chosen via `Accept`.
- **Template definitions:** `GET …/definition/template/adl1.4/{id}` with
  `Accept: application/openehr.wt+json` returns the Web Template document.
  `Accept: application/json` returns the same document (it is the only JSON
  representation of a template) under `Content-Type: application/json`: the
  response always carries the media type you asked for.
- **Contributions:** the CONTRIBUTION envelope itself stays canonical JSON; a
  simplified media type applies only to each composition payload inside
  `versions[].data`.

Two rules to know when committing a composition in a simplified format:

- A FLAT/STRUCTURED payload cannot carry its own template id, so the
  **`openehr-template-id` request header is required**; the commit is rejected
  with **422** without it.
- There is **no `?format=` query parameter**: format selection is done
  exclusively through the standard `Accept` and `Content-Type` headers.

Requests naming a media type the endpoint does not support are answered with
**415 Unsupported Media Type** (request body) or **406 Not Acceptable** (response
format), with a body naming the formats that endpoint does support. EHR,
EHR_STATUS, directory, and demographic resources have no simplified
representation (the format is generated from an operational template, which
those resources do not have). They speak canonical JSON/XML only.

The query API is **JSON only**: it does not accept XML or the simplified media
types.

## The `Prefer` header

Write operations (create/update) accept a `Prefer` header controlling the
response body. Its default is `return=minimal`:

| `Prefer` value | Effect |
|---|---|
| `return=minimal` (default) | Empty body; the identifier is in `ETag`/`Location`. Status **204** on update, **201** on create. |
| `return=representation` | The full created/updated resource in the body, status **200**/**201**. |
| `return=identifier` | Just the resource identifier object — `{"uid": "…"}` (templates: `{"template_id": "…"}`). Status **200**/**201**, never 204. |

Use `return=representation` when you want the server-completed object back (with
its assigned version id and any server-set audit fields); use `return=minimal`
for throughput when you only need the id.

`return=identifier` always comes back with a body, so it never uses 204: an
update that would answer 204 under `return=minimal` answers **200** with the
identifier object instead. Under an XML `Accept` the identifier body is the
equivalent single element, `<uid>…</uid>`.

Every write response names the preference the server actually applied in a
**`Preference-Applied`** header (`return=representation`, `return=identifier`, or
`return=minimal`), so a client can tell what it got without sniffing the body.
The header reports what the response *did*: a request with no `Prefer` gets
`return=minimal` (the default behaviour), and where an identifier cannot be
produced at all (an item-tag collection has no uid, for instance) the server
applies and reports `return=minimal` rather than claiming an identifier response
it did not send.

### `Prefer: resolve_refs`

Contribution reads return their `versions` as `OBJECT_REF`s by default. Add
`resolve_refs` to the `Prefer` header (it combines with the `return=…` token,
e.g. `Prefer: return=representation, resolve_refs`) and the response carries the
full VERSION objects instead: one round trip instead of one per version. A
version created here resolves to an `ORIGINAL_VERSION`; one this server received
from another system resolves to the `IMPORTED_VERSION` that wraps it (see
[Imported versions](#imported-versions)).

## `ETag` and `If-Match` — optimistic concurrency

openEHR objects are versioned, and updates use HTTP preconditions to prevent
lost updates:

- Every read and successful write returns an **`ETag`** header carrying the
  object or version identifier as a *weak* ETag, `W/"…"`.
- Updating or deleting a versioned object requires an **`If-Match`** header set
  to the **current** version id. Both the weak form and a bare quoted value are
  accepted; echoing the `ETag` you received works either way:

  ```text
  If-Match: W/"8849182c-82ad-4088-a07f-48ead4180515::your.system::2"
  If-Match: "8849182c-82ad-4088-a07f-48ead4180515::your.system::2"
  ```

- If the object has moved on since you read it, the write fails with **412
  Precondition Failed** and the *current* version id in the response `ETag`.
  Re-read, reconcile, and retry against the new version.

A **`Location`** header is emitted only when a resource is **created**. Reads
and deletes identify the version through `ETag` alone, so do not expect
`Location` on them; the `ETag` is the authoritative identifier.

## `Last-Modified` — when the version was committed

Alongside the `ETag`, versioned responses carry a **`Last-Modified`** header in
the standard HTTP-date form (`Wed, 22 Jul 2009 19:15:56 GMT`). Its value is the
commit time of the version being served (the audit `time_committed` of that
`VERSION`), so it changes exactly when the `ETag` does.

You get it on:

- every `VERSION` read (`…/versioned_composition/{uid}/version[/{version_uid}]`,
  `…/versioned_ehr_status/version[/{version_uid}]`) and revision history read;
- every COMPOSITION, `EHR_STATUS`, and DIRECTORY read, including the FLAT and
  STRUCTURED representations, which describe the same version;
- every write of those resources (create, update, and the delete `204`), and the
  EHR create `201`;
- every CONTRIBUTION, both the read and the commit `201`, where the value is
  the contribution audit's commit time. On a contribution commit you get the
  header under either `Prefer` setting; with `return=minimal` there is no
  response body, so the header is the only place the commit time appears.

Resources that are not versioned do not carry it: `GET /ehr/{ehr_id}` returns the
weak `ETag` (built from `EHR.ehr_id.value`) but no `Last-Modified`, because the
`EHR` root object has no commit audit of its own.

Template responses (ADL 1.4 and ADL2) carry a weak `ETag` keyed on the template
identifier; for ADL2 it is the **resolved** artefact id, so requesting a template
by a partial id or a major-version prefix still gives you an `ETag` that changes
when the served artefact changes.

> [!NOTE]
> Version ids normally end in a plain trunk number (`…::2`), but openEHR version
> trees can **branch**: when a version that was created on another system is
> modified locally, the server forks a branch and the new version id ends in a
> three-part tree id (`…::2.1.1`). Treat the version id as an opaque token
> (echo it back in `If-Match` exactly as received) and it works the same for
> trunk and branch versions. `ALL_VERSIONS` queries and version reads return
> branch versions alongside trunk ones; the *latest* version of an object is
> always the latest trunk version.

## Imported versions

Most versions are created here, and a `VERSION` read returns them as an
`ORIGINAL_VERSION`. A version that arrived from **another** system (through an
EHR Extract import) is a copy, and openEHR wraps a copy: the server commits it
as an **`IMPORTED_VERSION`** and the version resource serves that wrapper. Both
shapes appear at the same URLs
(`…/versioned_composition/{uid}/version[/{version_uid}]`,
`…/versioned_ehr_status/version[/{version_uid}]`) and in a `resolve_refs`
contribution read, so branch on `_type`:

```json
{
  "_type": "IMPORTED_VERSION",
  "contribution": { "_type": "OBJECT_REF", "type": "CONTRIBUTION", "…": "…" },
  "commit_audit": { "_type": "AUDIT_DETAILS", "…": "…" },
  "item": {
    "_type": "ORIGINAL_VERSION",
    "uid": { "_type": "OBJECT_VERSION_ID", "value": "…::source.example.org::1" },
    "contribution": { "_type": "OBJECT_REF", "type": "CONTRIBUTION", "…": "…" },
    "commit_audit": { "_type": "AUDIT_DETAILS", "…": "…" },
    "data": { "…": "…" }
  }
}
```

Two acts, kept apart on purpose:

- the **wrapper's** `contribution` and `commit_audit` are *this* server's act of
  importing: our system id, the importing user, the instant the import landed
  here, change type `249|creation|`;
- **`item`** is the received `ORIGINAL_VERSION`, byte-for-byte as it was sent,
  keeping the source system's contribution reference, commit audit and
  signature.

Three consequences worth designing for:

- **An `IMPORTED_VERSION` has no `uid` of its own.** It shares the wrapped
  version's identity, so read the version id from `item.uid.value`. The `ETag`
  still carries that id, so the `If-Match` round trip is unchanged.
- **Times are local.** `VERSIONED_OBJECT.time_created`, `Last-Modified`, the
  revision history and every as-of-instant read report when the version became
  available *here*, never the source system's earlier clock, so a query for the
  record's state at a past instant returns what this repository actually held
  then. The original committal is still there, inside `item.commit_audit`.
- **An export unwraps.** EHR Extracts carry `ORIGINAL_VERSION`s, so exporting an
  imported version ships the wrapped original verbatim; the receiving system
  creates its own wrapper. Wrappers never nest.

## Commit metadata headers

When you commit through the direct resource endpoints (EHR creation,
composition, EHR_STATUS, directory), the server builds the version's audit for
you. Two request headers let you set parts of it: **`openehr-version`** for the
version's own attributes and **`openehr-audit-details`** for the commit audit.
The value is a comma-separated list of `attribute.subkey="value"` pairs (quoted
values may contain commas; the header may repeat, and repeats are merged):

```text
# Commit a composition as a draft (lifecycle state "incomplete", code 553)
openehr-version: lifecycle_state.code_string="553"

# Name the committer, describe the change, and stamp the source system
openehr-audit-details: committer.name="John Doe",description.value="Corrected dosage",system_id="pas.example.org"
```

The attributes the server merges:

| Header | Attribute | Sub-keys |
|---|---|---|
| `openehr-version` | `lifecycle_state` | `code_string` |
| `openehr-audit-details` | `change_type` | `code_string` |
| `openehr-audit-details` | `description` | `value` (or a bare value) |
| `openehr-audit-details` | `committer` | `name`, `external_ref.id`, `external_ref.namespace`, `external_ref.type` (defaults to `PERSON`) |
| `openehr-audit-details` | `system_id` | (bare value) |

A client-supplied `system_id` is merged into the commit audit, useful when a
gateway commits on behalf of a source system; when absent, the server stamps its
own system id.

Both EHR creates (`POST /ehr` and `PUT /ehr/{ehr_id}`) accept the headers too:
creating an EHR commits its EHR_STATUS and EHR_ACCESS in a contribution, so the
supplied description, committer, and system id land on that commit, and
`openehr-version` sets the new EHR_STATUS version's lifecycle state. The
`change_type` on a create is constrained to `249|creation|` (a create commits a
first version), so restating `249` is accepted while any other change type is
rejected. A `DELETE` accepts the headers as well: a logical delete commits a
`523|deleted|` version, and the audit metadata rides with it.

> [!NOTE]
> openEHR itself deprecated an earlier spelling of these headers in
> Release-1.0.3: the attribute sat in the header *name*
> (`openEHR-VERSION.lifecycle_state: code_string="553"`,
> `openEHR-AUDIT_DETAILS.committer: name="John Doe"`, and the bare
> `openEHR-AUDIT_DETAILS`). The specification keeps them available for backward
> compatibility and so does this server, so an older client keeps working. If
> both forms appear, the value-form header above wins.

## Item tags via headers

Item tags (small `key`/`value` annotations, optionally pointing at a node inside
the data via `target_path`) can ride the same request as a write, so tagging
does not need a second round trip (the dedicated endpoints are in
[Resource walkthroughs](resources.md#item_tag)). Two headers carry them:

- **`openehr-item-tag`:** tags targeting the versioned object;
- **`openehr-version-item-tag`:** tags targeting the version being committed.

The value is a `;`-separated list of tags, each a comma-separated set of
`key="…"`, `value="…"`, and optional `target_path="…"` pairs:

```text
openehr-version-item-tag: key="diagnosis",value="confirmed",target_path="/content[0]"; key="reviewed",value="true"
```

They are accepted on the EHR-group change-controlled writes (composition
create/update, EHR_STATUS update, directory create/update) and on demographic
party writes. Sending the header with an **empty value** removes all tags. A
defective tag refuses the whole request before the content is committed, so a bad
tag never leaves a half-applied write behind.

The two headers address **different targets**, so the response echo keeps them
apart: `openehr-item-tag` confirms the tags now stored on the versioned object,
`openehr-version-item-tag` those stored on the version just committed, and a
header you did not send is not echoed at all. (Demographic parties store tags
against the versioned object only, so both headers carry the same list there.)

## Error responses

Errors use conventional HTTP status codes (see the summary in
[Resource walkthroughs](resources.md#status-code-summary)) with **one uniform
JSON body shape**: the openEHR `Error` object's members plus a
machine-readable `error` reason phrase:

- **Validation errors** (a composition that fails its template) populate the
  list:

  ```json
  {
    "message": "Composition validation failed",
    "validationErrors": [
      "/content[0]/data/events[0]/data/items[1]/value/magnitude: value out of range",
      "/content[0]/data/events[0]/data/items[2]/value/defining_code: code not in group"
    ],
    "error": "Unprocessable Entity"
  }
  ```

  Each entry is `"<path>: <message>"`, so a client can point the user at the
  exact offending node. A refusal from the
  [data-minimisation rules](../installation/config-privacy.md) uses the same
  list and the same shape, with one entry per finding, and deliberately never
  echoes the value that matched: repeating it would carry the identifier into
  the response body, the access log and the traces.

- **All other errors** carry the same shape with an empty list:

  ```json
  { "message": "No EHR with id …", "validationErrors": [], "error": "Not Found" }
  ```

  This shape is used consistently, including for `405 Method Not Allowed` and
  `501 Not Implemented`, which some servers leave bodyless, and for the two
  refusals that come from the transport layer rather than a handler:
  `408 Request Timeout` (the request exceeded the server's request-execution
  limit) and `413 Payload Too Large` (the request body exceeds the accepted
  size).

Match on the HTTP status first; read the body for the human-readable detail and,
for validation, the per-node list.

### `405` always names the allowed methods

Every `405 Method Not Allowed` carries an `Allow` header listing the methods the
target resource currently supports, as
[RFC 9110 §15.5.6](https://www.rfc-editor.org/rfc/rfc9110.html#section-15.5.6)
requires, so a client can discover the right method without guessing:

```http
HTTP/1.1 405 Method Not Allowed
Allow: GET,HEAD,PUT
Content-Type: application/json

{ "message": "the request method is not allowed on this resource", "validationErrors": [], "error": "Method Not Allowed" }
```

When a resource is switched off by configuration (the
[admin API](../operations-admin-apis.md) with `FERROEHR__ADMIN__ENABLED=false`),
the header is present but **empty**, which
[RFC 9110 §10.2.1](https://www.rfc-editor.org/rfc/rfc9110.html#section-10.2.1)
defines as "the resource allows no methods": nothing you can send to that path
will be served until the gate is opened.

A method the server does not recognize at all is answered `405` as well (with
`Allow`), not `501`. The openEHR spec suggests `501` there, but the two rules it
states overlap for any method outside its own list, and a blanket `501` would
also mislabel requests to paths that simply do not exist and are owed a `404`.
`501 Not Implemented` remains reserved for a recognized operation this server
does not implement.
