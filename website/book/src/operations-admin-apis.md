# Admin & messaging APIs

Two groups of routes sit beside the clinical API for operators rather than for
clinical clients: the **admin API**, which physically deletes, reports on, and
moves repository content, and the **messaging API**, which exports and imports
whole records as openEHR EHR Extracts and accepts documents in the
template-data (TDD) form. This page is the operator's reference for both:
every path, what gates it, what it accepts, and every status code it answers.

<!-- toc -->

## Where these routes live, and what gates them

Both groups are mounted under the API base path, so every path below is
relative to it. With the default `server.base_path`, `{base}` reads
`/ferroehr/rest/openehr/v1`, and `{base}/admin/dump` is
`/ferroehr/rest/openehr/v1/admin/dump`.

The two groups are gated differently, and the difference matters
operationally:

| Group | Switch | Authorization class | While switched off |
|---|---|---|---|
| `{base}/admin/…` | `admin.enabled` (`FERROEHR__ADMIN__ENABLED`), default `false` | admin: the configured `authz.rbac.admin_role` (`ADMIN` by default) | every route answers **405 Method Not Allowed** with an **empty** `Allow` header: the resource exists but currently serves no method |
| `{base}/message/…` | none (always mounted) | ordinary clinical, exactly like the composition API | n/a |

Two consequences worth planning around:

- **The messaging routes are not admin-gated.** They read and write the same
  clinical content the composition API does, so they carry the same ordinary
  authentication and the same coarse clinical class. A principal holding the
  configured read-only role (`authz.rbac.readonly_role`, `READONLY` by
  default) is refused **403** on every messaging *import*, before the body is
  read; the exports stay available to it.
- **RBAC has to be on for the admin role to mean anything.** With
  `authz.rbac.enabled = false` (the posture the Compose quickstart ships)
  authentication is the only gate, so *any* authenticated caller reaches every
  enabled admin route. Turn RBAC on before enabling the admin API anywhere
  that is not a laptop.

While the admin API is enabled, the server also advertises `/admin` in the
`OPTIONS` conformance manifest it serves at the API base-path root; with the
API off, that entry is absent.

> [!WARNING]
> `DELETE {base}/admin/ehr/all` with no parameter empties the repository. There
> is no confirmation step and no undo. Keep `admin.enabled` off unless a
> workflow needs it, and gate the admin role tightly.

## Physical deletion

Normal openEHR deletes are *logical*: the version history is retained. The
admin API is the exception: **physical, irreversible** removal, for legal
erasure requests and test-data cleanup.

| Route | Success | Refusals |
|---|---|---|
| `DELETE {base}/admin/ehr/{ehr_id}` | **204**: the EHR and every resource it owns (compositions, `EHR_STATUS`, item tags, contributions, and all their historical versions) are physically gone | **400** malformed id (rejected before any deletion), **404** no EHR with that id |
| `DELETE {base}/admin/ehr/all` | **204**: bulk delete | **400** any id in the list is not a well-formed UUID; the whole request is refused before anything is deleted |
| `DELETE {base}/admin/template/{template_id}` | **204** | **404** unknown template, **409** a committed version still references it |
| `DELETE {base}/admin/query/{qualified_query_name}/{version}` | **204** | **404** unknown name, or a known name with no such version |
| `GET {base}/admin/config` | **200**: the effective configuration as a redacted JSON tree | — |

Every route additionally answers **401** unauthenticated, **403** for an
authenticated caller outside the admin class, and **405** while the group is
switched off.

**What an EHR delete reaches**, in the order it runs: one clinical
transaction removes the EHR row and, through the foreign-key graph, its
versions over both storage tiers, their nodes, attestations, contributions,
commit audits, item tags, folder memberships, the restriction and retention
marks, the multimedia blob references and every pending change event of the
EHR; an erasure tombstone is appended to the change-event stream in the same
transaction, so a consumer that derived anything from the EHR is told to
delete it (GDPR Art. 19). The cross-reference row that names the EHR's subject
is then erased in the linkage domain, and the externalized multimedia blobs no
surviving version still references are removed from the object store.

**A party delete announces itself the same way.** The SM
`physical_party_delete` operation, which this release exposes on the service
layer and on no REST route, writes one erasure tombstone on the party domain's
own outbox before its transaction commits, naming the party and every
`PARTY_RELATIONSHIP` erased with it. A consumer of the party stream has no
other way to learn the identity is gone.

**What it keeps:** the audit records naming the `ehr_id`. Art. 17(3)(b)
withholds erasure where processing is necessary for compliance with a legal
obligation, and the national access-logging periods are that obligation. Data
outside the running database (backups, replicas, exports) is reached by your
own rotation, not by this call.

Details that decide behaviour:

- **The bulk delete's parameter is optional, and its absence means
  everything.** `DELETE {base}/admin/ehr/all` with no `ehr_id` deletes every
  EHR on the server. To delete a subset, pass `?ehr_id=<uuid>`, repeatable
  (`?ehr_id=a&ehr_id=b`) or comma-separated (`?ehr_id=a,b`); blank entries are
  dropped. An id that names no EHR deletes nothing and is not an error, so the
  bulk route has no `404`.
- **Deleting a template never orphans clinical data.** The **409** is a
  deliberate guard: while any stored composition was committed against the
  template, the delete is refused and the message says how many committed
  versions still hold the reference. Delete those compositions first.
- **A stored-query delete removes exactly one `(name, version)` row.** The
  query's other versions survive.
- **The config view is redacted structurally, not by key name.** Passwords and
  password hashes, HMAC and signing-key secrets and S3 secret keys render as
  `***`; every URL that can carry credentials (database, AMQP, the FHIR audit
  feed, terminology servers and their token endpoints, the remote policy
  server, the OTLP collector, the object store, the usage-report endpoint)
  keeps its host and path and masks the embedded credentials
  (`postgres://***@host:5432/db`). The OIDC issuer and the SMART URLs are
  refused at boot when they carry credentials, so they render as set. Non-secret
  identifiers (usernames, roles, an OIDC issuer) stay visible. Redaction is
  a property of the configuration's secret types, so no secret value can reach
  this response.

> [!NOTE]
> The template delete, the stored-query delete and the config view are FerroEHR
> extensions: the openEHR admin API defines only the two EHR deletes. They
> share the same switch and the same authorization as those deletes.

## The activity report

Four **read-only** counters over the repository's change history, behind the
same switch and role as the deletes.

Every route takes two query parameters:

| Parameter | Required | Meaning |
|---|---|---|
| `a_service` | yes | the service whose versioned content to report on: one of `Admin`, `Definitions`, `Ehr`, `Ehr_index`, `Demographic`, `Message`, `Query`, `System_log`, matched case-insensitively |
| `time_interval` | no | `<lower>/<upper>`, two ISO 8601 date-times matched **inclusively** against each commit time |

Either bound may be left empty for an open interval
(`?time_interval=2026-01-01T00:00:00Z/`); an absent parameter reports over all
time. A service that holds no versioned content reports an empty list or `0`
rather than failing.

| Route | **200** body |
|---|---|
| `GET {base}/admin/report/contribution` | the matching CONTRIBUTION ids as a JSON array, ordered by commit time then id |
| `GET {base}/admin/report/contribution/count` | how many there are, as a bare JSON number |
| `GET {base}/admin/report/versioned_composition/count` | how many distinct COMPOSITION version containers had a version committed in the interval |
| `GET {base}/admin/report/composition_version/count` | how many individual COMPOSITION versions were committed in the interval |

**400** covers an absent `a_service`, one that names no known service, a
`time_interval` that is not `<lower>/<upper>`, a bound that is not a valid ISO
8601 date-time, and an interval bounded on **both** sides whose lower bound is
after its upper bound: that is not an interval, and answering it with the
empty result it would select would hand back a truthful-looking count for a
window nobody asked for. Equal bounds are a legitimate single-instant interval
and are reported normally.

## Archiving

Four routes that move a selected set of records to the server's cold storage
tier and back, behind the same switch and role. Archiving is **not** a delete:
the archived records stay fully readable through the normal API, with their
whole revision history intact.

| Route | Body | Success |
|---|---|---|
| `POST {base}/admin/archive/ehrs` | `{"ehr_ids": ["…"]}` | **204**: every named EHR and all its versioned content is archived |
| `POST {base}/admin/archive/parties` | `{"party_ids": ["…"]}` | **204**: every named demographic party is archived |
| `POST {base}/admin/archive/ehrs/restore` | `{"ehr_ids": ["…"]}` | **204**: every named EHR's archived content is back in the primary tier |
| `POST {base}/admin/archive/parties/restore` | `{"party_ids": ["…"]}` | **204**: every named party's archived content is back in the primary tier |

All four are **all-or-nothing and idempotent**: a body of the wrong shape or a
malformed id is **400** and an id that names nothing is **404**, in both cases
before anything is moved; re-archiving an already-archived record, or restoring
one that is not archived, changes nothing. An empty list succeeds and moves
nothing. A body sent without `Content-Type: application/json` is **415**. A
party that is currently archived is still found by its restore call (the
existence check spans both tiers) so an archived party is never reported
missing.

A party's `PARTY_RELATIONSHIP`s are **not** carried along: each is an
independently addressable versioned object, archived in its own right.

What "cold storage tier" means here: the archived rows are physically moved out
of the primary tables into a separate schema in the same database, so the
tables and indexes that serve everyday traffic shrink by exactly what was
archived: no extra tablespace, volume, or external service to operate. Reads
of unarchived records never touch the cold tier; a read that addresses an
archived record is served from it. Writing to an archived record brings it
back to the primary tier first, so a versioned object is never split across
tiers; a physical delete clears both tiers; and `{base}/admin/dump` still
exports archived content.

Two consequences to plan for:

- **AQL queries see the primary tier only**, so an archived record stops
  appearing in query results; that is exactly what shedding the query tables'
  rows and indexes buys you. Everything addressed by id (an EHR, a
  composition, a folder, a party, a version, a revision history) keeps working
  as before. Restoring puts the record back in query results.
- **There are three ways back, and only one of them is deliberate.** The
  `…/restore` routes above reverse a whole set on request; a write to an
  archived record thaws just that record as a side effect, whichever route the
  write came in on; and a physical delete removes it from both tiers. Query
  visibility is the effect to plan around, and the restore routes are how you
  get it back.

## Legal marks: restriction, objection and retention

Eight routes behind the same switch and role, carrying the three marks the law
asks a repository to hold beside its clinical content. None of them deletes
anything.

| Route | Body or parameter | Success |
|---|---|---|
| `POST {base}/admin/restriction` | `{"ehr_id": "…", "vo_id": "…"?, "ground": "…", "note": "…"?}` | **204**: the restriction is recorded and in force |
| `GET {base}/admin/restriction` | `?ehr_id=…` | **200**: the register, newest request first, lifts included |
| `POST {base}/admin/restriction/lift` | `{"ehr_id": "…", "vo_id": "…"?}` | **204**: every in-force restriction at that grain is lifted |
| `POST {base}/admin/research-objection` | `{"ehr_id": "…", "objected": true, "ground": "…"?}` | **204**: the objection, its override, or its withdrawal is recorded |
| `PUT {base}/admin/retention/policy` | `{"kind": "…", "jurisdiction": "…", "category": "…"?, "period": "…", "anchor": "…", "source": "…"}` | **204**: the period is declared, keyed on one EHDS priority category when `category` is set |
| `GET {base}/admin/retention/policy` | — | **200**: the whole retention register, each period with its `category` (`null` for every object of the kind) |
| `PUT {base}/admin/retention/anchor` | `{"ehr_id": "…", "jurisdiction": "…", "anchored_at": "…"?, "hold_at": "…"?, "hold_ground": "…"?}` | **204**: the EHR's anchor and any whole-record hold |
| `POST {base}/admin/retention/hold` | `{"vo_id": "…", "held": true}` | **204**: the per-object exemption is placed or released |
| `GET {base}/admin/retention/due` | `?limit=100` | **200**: what has run out, oldest first, per period and category, with the due and held object counts |

A body of the wrong shape or a malformed id is **400**, an id that names nothing
is **404**, and a body without `Content-Type: application/json` is **415**, in
each case before anything is recorded. Setting or lifting a mark is an access
record of its own.

A `category` is spelled as `[audit.categories]` spells it (`patient-summary`,
`eprescription`, `edispensation`, `imaging`, `test-results`, `discharge-report`,
`national:<code>` or `none`; an unknown spelling is **400**), which serves EHDS
Annex II 3.4: retention periods that take the categories of the data into
account. The server mirrors the map into the register at every boot. A period
keyed on a category lists the objects the map places in it; an object the map
cannot classify is listed under the longest period configured for its kind,
and a category row with no object is left out of `due`. A period with no
category lists every object of its kind, as before.

Restricting a record changes what every other route answers: reads of a
restricted object become **403**, it leaves AQL results at every scope, exports
skip it and the event stream withholds it, and writes to it are refused. On a
write the restriction is the LAST gate: content validation answers **422** and
the preconditions that run before the commit transaction, a non-modifiable EHR
or a second directory, answer **409**, while the mark is read inside the
transaction. A malformed write to a restricted object therefore answers 422
rather than 403. Both refuse the write and neither changes what is stored, so
read the register itself to learn whether a restriction is in force. What
each mark means, which provision it serves and what the deploying organisation
still has to decide are on [Retention, restriction and
objection](compliance/retention.md).

## Storage integrity

Two routes that check the stored data against itself and repair what they
find, behind the same switch and role. FerroEHR stores every version's content twice: once as the
materialized document a point read serves, and once as the decomposed rows the
AQL engine queries. A commit writes both in the same transaction, so they
always agree; anything that changes one of them behind the server's back
breaks that agreement.

| Route | **200** body |
|---|---|
| `POST {base}/admin/integrity/verify` | the sweep report |
| `POST {base}/admin/integrity/rebuild-nodes` | the rebuild report |

The sweep covers **both pseudonymisation domains**, clinical and demographic,
in one pass. Every finding names the domain it came from, so a report can never
describe half the store while looking like it described all of it. It re-derives
every stored version from its decomposed rows and compares the result with the
stored document, then decomposes that document again and compares the rows it
would write, which is what catches rows an older release left in a shape this
one no longer produces. It reads the archived tier as well, takes
no lock, and runs outside the request path of any clinical call, so it is safe
to run on a live server. It is also a full scan of what it covers, so schedule
it rather than calling it per request.

Four optional query parameters narrow the scan, and they compose. Both routes
take the same set:

| Parameter | Effect |
|---|---|
| `ehr_id` | cover only versions belonging to that EHR |
| `vo_id` | cover only versions of that versioned object |
| `sys_version` | cover only that version of it; the ordinal is per-object, so it needs `vo_id` and is a **400** on its own |
| `committed_since` | cover only versions whose validity begins at or after that RFC 3339 instant |

Use them. Verifying one record after a support incident, or everything written
since a known point, costs a fraction of a full repository scan.

### Verifying a whole large repository

By default the route computes the whole report before it answers, so it has to
finish inside the server's 30-second request timeout. Send
`Accept: application/x-ndjson` and it streams the same sweep instead, writing
each finding as it is made. Nothing bounds that response, so this is how you
verify a repository too large to scope:

```bash
curl -sN -X POST \
  -H 'Accept: application/x-ndjson' \
  "$BASE/admin/integrity/verify"
```

The body is one JSON object per line, each carrying a `type`:

| `type` | When | Carries |
|---|---|---|
| `mismatch` | as each disagreement is found | the same five fields the report's `mismatches` entries carry, `domain` included |
| `progress` | once per page of versions read | the counts so far |
| `summary` | once, at the end | the final counts and `elapsed_ms` |
| `error` | instead of `summary`, if the sweep failed part-way | a short message; the detail is in the server log |

Two consequences are worth planning around. The status code is sent before the
work is done, so a sweep that fails half-way still answered `200`: read to the
end and check that the last line is a `summary`, not an `error`. And the stream
carries no reporting cap, so every mismatch reaches you rather than the first
thousand.

The stream has to be asked for by name. A request sending `*/*`, or no `Accept`
at all, gets the aggregated document below, unchanged.

```json
{
  "versions_checked": 128,
  "versions_with_body": 126,
  "versions_without_body": 2,
  "mismatch_count": 1,
  "mismatches": [
    {
      "domain": "clinical",
      "vo_id": "8849182c-82ad-4088-a07f-48ead4180515",
      "sys_version": 2,
      "kind": "COMPOSITION",
      "defect": "content_differs"
    }
  ],
  "truncated": false,
  "elapsed_ms": 431
}
```

`domain` is `clinical` or `demographic`, naming the schema the damaged version
lives in. It is also what the repair below uses to reach it.

`defect` is one of five values:

- `content_differs`: both copies exist and hold different content.
- `nodes_missing`: the version has a stored document but no decomposed rows.
- `nodes_unreadable`: the decomposed rows exist but no longer form one tree.
- `unexpected_nodes`: the version is a logical delete, which stores no
  document, yet decomposed rows exist for it.
- `stale_decomposition`: the rows hold exactly the stored document, but not in
  the shape this version of the server decomposes it into. An older release
  wrote them. The content is intact and every read serves it correctly; what is
  stale is the index over it, so a query that depends on the current shape does
  not reach this version until the repair below rewrites its rows.

`mismatch_count` is the full count. `mismatches` is capped at 1000 entries and
`truncated` says whether the cap was reached; every mismatch is logged at
`warn` level with its identifiers whatever the cap does, so a truncated report
never loses a finding. The log line and the response carry identifiers only,
never content.

A finding is **not** a request failure: the sweep ran and is telling you what
it saw, so the status stays **200** and the report is the body. Check
`mismatch_count`, not the status code.

> [!NOTE]
> This is the companion check to [digest signing](signing/digest.md). A stored
> digest covers the document a point read serves; it says nothing about the
> decomposed rows, which no read-path check recomputes. Together the two cover
> both copies.

### Repairing what the sweep found

The sweep is the diagnosis. `POST {base}/admin/integrity/rebuild-nodes` is the
repair: it re-derives the decomposed rows of every damaged version from that
version's stored document, one transaction per version.

```bash
curl -s -X POST "$BASE/admin/integrity/rebuild-nodes?ehr_id=$EHR_ID"
```

It runs the same sweep first and writes only the versions that sweep reports
damaged, so a run over healthy data writes nothing at all. The scope
parameters are the same four, so a repair can be as narrow as one version:

```bash
curl -s -X POST \
  "$BASE/admin/integrity/rebuild-nodes?vo_id=$VO_ID&sys_version=2"
```

Which copy wins is not a choice the route makes. The stored document is the
version's canonical serialized form, the bytes a point read serves and the
bytes a digest was taken over; the decomposed rows are an index derived from
it. So the repair only ever runs in that direction, and a version whose
**document** is the damaged copy is refused rather than having the damage
copied into the index.

Each version is repaired inside one transaction: the document is read under a
row lock, decomposed, the old rows deleted, the new set inserted, and the
version re-derived from what was just written and compared with the document
before the commit. Anything that fails rolls that transaction back, so the
version's rows are left exactly as they were. One refusal never stops the run
that found it.

An archived object is thawed for the repair and re-archived in the same
transaction, marker and all, because a write to a versioned object always
happens in the primary tier.

A logically deleted version stores no document, so it rebuilds to no rows,
which is the repair for `unexpected_nodes`.

The same route is the upgrade step after a release that changes how content is
decomposed. Rows written by the older release hold the right content, so
nothing is damaged, but they are the wrong shape for the new one and the sweep
reports them `stale_decomposition`. Rebuilding writes them again from the
stored document, which is all the upgrade is. Run it unscoped once and the next
sweep is clean:

```bash
curl -s -X POST "$BASE/admin/integrity/rebuild-nodes"
```

The changelog names the affected object type at each such release.

```json
{
  "versions_checked": 128,
  "versions_damaged": 2,
  "versions_rebuilt": 1,
  "versions_refused": 1,
  "records": [
    {
      "domain": "clinical",
      "vo_id": "8849182c-82ad-4088-a07f-48ead4180515",
      "sys_version": 2,
      "kind": "COMPOSITION",
      "defect": "content_differs",
      "outcome": "rebuilt",
      "node_rows": 41
    },
    {
      "domain": "demographic",
      "vo_id": "1f0b7d64-6b2a-4a1f-9f0e-6b1d2a3c4d5e",
      "sys_version": 1,
      "kind": "PERSON",
      "defect": "content_differs",
      "outcome": "refused",
      "reason": "the stored body does not decompose: ..."
    }
  ],
  "truncated": false,
  "elapsed_ms": 812
}
```

`defect` is the sweep verdict that selected the version, from the five values
above. `outcome` is `rebuilt`, carrying the `node_rows` the version now has, or
`refused`, carrying the `reason`. `records` is capped at 1000 entries with
`truncated` saying whether the cap was reached; every record is logged at
`info` level with its identifiers whatever the cap does.

A refusal is **not** a request failure, so the status stays **200**. Check
`versions_refused`: anything above zero means a stored document is itself
damaged, and that is a restore-from-backup question rather than a rebuild one.

Unlike the sweep, this route writes, so a server in read-only mode refuses it.

## Dump and load

Two routes that move the whole repository to and from an archive on the
**server's** file system, behind the same switch and role. Both answer **200**
with a JSON array of per-entity failure reports; an **empty** array means
everything succeeded.

### `POST {base}/admin/dump`

Writes an archive of every EHR **and of every standalone demographic
container**: the parties and party relationships that live outside any EHR.
The body is `{"file_sys_loc": "…"}` plus the optional export settings:

| Field | Values | Default |
|---|---|---|
| `logical_format` | `openehr_canonical_json` or `openehr_canonical_xml` | canonical JSON |
| `compression_format` | `zip` or `7z` (omit for loose files) | uncompressed |
| `segment_split_size` | segment size in kb (a positive integer) | `1024` |

`logical_format` chooses how the clinical **content** is serialized, not how
the archive is packaged:

- `openehr_canonical_json` (the default) keeps each version's content inline
  in the segment files, exactly as this server stores it.
- `openehr_canonical_xml` writes each version to its own
  `versions/<version_uid>.xml` entry instead: a complete `ORIGINAL_VERSION`
  document under the openEHR-published `<version>` root, ready to hand to any
  tool that reads canonical openEHR XML.

The archive is a directory holding a `manifest.json`, one or more
`segment-NNNN.json` files, and a `blobs/` subdirectory for any externalized
multimedia. When the repository holds standalone demographic containers, the
archive additionally carries a `demographic-commons.json` (their shared
audits and contributions) and one or more `demographic-NNNN.json` segments,
one record per party or relationship, in the same version-record shape the
EHR segments use. With `compression_format` set, those same entries are
packed into a single `archive.zip` or `archive.7z` inside the location
instead. The
archive's own bookkeeping (the manifest and the segment skeleton) stays JSON
in both logical formats, because openEHR publishes no XML document form for
it.

### `POST {base}/admin/load`

Populates the repository from an archive. It takes the location and nothing
else: the container (loose files, a single `archive.zip`, or a single
`archive.7z`) is detected from what the location holds, and the logical format
is read from the archive's own manifest, so a load never has to be told how
the dump was written. Archives written before the demographic wave existed
simply carry no demographic entries and load unchanged.

The repository being loaded into **need not be empty**. An EHR whose id is
already present is reported and skipped rather than failing the load, so the
response array names each one; and a standalone demographic container that
already exists is reported the same way, under its own kind (`PERSON`,
`ORGANISATION`, `GROUP`, `AGENT`, `ROLE`, or `PARTY_RELATIONSHIP`) as the
`entity_type`:

```json
[ { "entity_type": "EHR",
    "entity_id": "7d44b88c-4199-4bad-97dc-d78268e01398",
    "dump_status": false,
    "error": "an EHR with this id already exists" } ]
```

Both directions are lossless: a dump and a load reproduce every record,
whichever format and container you choose.

### Refusals

| Status | When |
|---|---|
| **400** | a missing or blank `file_sys_loc`; a format value outside the lists above; a non-positive `segment_split_size`; an `encoding` field (the openEHR service model declares that enumeration with no members, so no value a client could send names one); on load, an archive carrying externalized multimedia this server has no store for |
| **415** | the request `Content-Type` is not `application/json` |
| **500** | a location that holds **no** archive, and one holding an archive that is corrupt (a mangled or truncated container, manifest, or segment) are the same fact and answer the same way: the service model's single `file_not_writable` error for these operations. Nothing is loaded either way. On dump, the same status covers a location, segment, payload entry or manifest that could not be created or written |

A single unreadable `versions/*.xml` entry is *not* in that family: it belongs
to one EHR, so that EHR is reported in the response array and skipped whole
while the rest of the archive loads.

> [!NOTE]
> The activity report, the archive routes and the dump/load pair are FerroEHR
> extensions: the openEHR *service model* defines these operations, but the
> released REST API surfaces no endpoint for them, so their URLs are our own.
> The two `…/restore` routes go one step further: the service model declares
> the archive calls and no un-archive counterpart, so both the operation and its
> URL are ours. They gate no openEHR conformance claim; see
> [Conformance](conformance.md).

## EHR Extract and TDD import

Six routes under `{base}/message` that move whole records between systems and
accept documents in the template-data form. Unlike the admin extensions above,
these are **not** admin-gated; see
[the gate table](#where-these-routes-live-and-what-gates-them).

### EHR Extract

- `GET {base}/message/export/{ehr_id}`: export one whole EHR. **200** with a
  JSON array holding one `EXTRACT` that carries every versioned object of the
  EHR, latest versions only. **400** if `ehr_id` is not a well-formed
  identifier, which is refused before any lookup; **404** if the EHR does not
  exist; **406** if `Accept` cannot be satisfied (the extract list is JSON
  only).
- `POST {base}/message/export` with an `EXTRACT_SPEC` body — export by
  specification. **200** with one `EXTRACT` per manifest entity, in manifest
  order. The manifest must name at least one entity, and each entity must
  name its record by `ehr_id` or `subject_id`, otherwise **400**; an
  identifier that names nothing is **404**. `extract_type` must be one of the
  extract-content-type codes the openEHR Reference Model names
  (`openehr-ehr`, `openehr-demographic`, `openehr-synchronisation`,
  `openehr-generic`, `generic-emr`) or the catch-all `other`; anything else
  is **400**, as is a selection this service does not support (search
  criteria, an unsupported commit-time interval). This route is classified as
  a **read**: it selects over held versions and commits nothing, exactly like
  the ad-hoc AQL `POST`.
- `POST {base}/message/import` with an `EXTRACT` body — clone a whole EHR. Add
  `?ehr_id=<uuid>` to fix the identifier the clone lands under; leave it off
  and the source identifier the extract carries is re-used. **201** with
  `{"uid": "<ehr_id>"}`, so a caller that supplied no id still learns what was
  created. The extract must carry an `EHR_STATUS` (**400**), and the target
  must not already exist: **409**, which also covers an imported `EHR_STATUS`
  naming a subject another EHR already holds.
- `POST {base}/message/import/{ehr_id}` with an `EXTRACT` body — add the
  extract's content to an **existing** EHR as new versions. **204**. **404**
  if the EHR does not exist; **409** if the EHR already holds an `EHR_STATUS`
  or `EHR_ACCESS` under a different object id, or the imported status names
  another EHR's subject; **422** if a version in the extract is semantically
  invalid (template, RM-invariant or terminology validation).

All four accept either `application/json` or `application/xml` bodies; any
other `Content-Type` is **415**.

### TDD import

- `POST {base}/message/tdd/{ehr_id}` with an `application/xml` body — import
  one Template Data Document. It is converted against the operational template
  its root names and committed through the ordinary validated composition
  path, so **201** with `{"uid": "<version_uid>"}`. The template must already
  be uploaded through the definition API (**404** otherwise, as for an unknown
  EHR); a root that is not in the template-data namespace, carries no template
  id, or does not conform to the template is **400**; a document that is not
  well-formed XML (or a produced composition that fails validation at
  commit) is **422**. A body sent as anything but `application/xml` is
  **415**.
- `POST {base}/message/tdd/{ehr_id}/batch` with a JSON array of TDD documents
  — import several at once. **201** with the created version ids in input
  order. The batch is **all-or-nothing**: every document is converted before
  any is committed, so one bad document rejects the whole batch and commits
  nothing. An **empty** array is a fulfilled no-op (**200** with `[]`, since
  nothing was created) but the target EHR is checked for every batch, the
  empty one included, so an unknown one is **404** whatever the batch holds.
  The batch has no limit on how many documents it may carry; the only bound is
  the server-wide request-body limit, which answers **413** when exceeded.

> [!NOTE]
> The whole `{base}/message` group is a FerroEHR extension: the openEHR service
> model defines a Message component, but the released REST API publishes no
> message, extract, or TDD endpoint at all. These URLs are our own and gate no
> openEHR conformance claim; see [Conformance](conformance.md). For the
> workflow-level view (what an extract is good for and how to drive an
> import) see [EHR Extract & messaging](beyond-core/messaging.md).

## Other routes under `{base}/admin`

Three further families share the `/admin` path prefix but **not** the
`admin.enabled` switch: each has its own, and each answers **404** (not
`405`) while its own switch is off, because the group is simply not serving:

| Routes | Own switch | Documented in |
|---|---|---|
| `{base}/admin/event_subscription…` | `events.admin_api` | [Change events (AMQP)](beyond-core/amqp.md) |
| `{base}/admin/fhir_mapping…` | `fhir.api_enabled` | [FHIR connectors](beyond-core/fhir.md) |

They are still admin-class routes for authorization, so the same **401** /
**403** split applies.

For everything else an operator needs (migrations, health probes, the
management surface, observability and upgrades) see
[Operations](operations.md).
