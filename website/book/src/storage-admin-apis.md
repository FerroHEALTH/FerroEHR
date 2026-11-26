# Storage integrity, dump and load

The admin routes that check and repair stored content, dump and load a repository, and
export and import EHR Extracts. The other admin routes are on [Admin & messaging
APIs](operations-admin-apis.md).

<!-- toc -->

## Storage integrity

Two routes that check the stored data against itself and repair what they
find, behind the same switch and role. FerroEHR stores every version's content twice: once as the
materialized document a point read serves, and once as the decomposed rows the
AQL engine queries. A commit writes both in the same transaction, so they
always agree; anything that changes one of them behind the server's back
breaks that agreement.

| Route | **200** body |
|---|---|
| `POST {ext}/admin/integrity/verify` | the sweep report |
| `POST {ext}/admin/integrity/rebuild-nodes` | the rebuild report |

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
- `orphan_nodes`: decomposed rows exist that no stored version reads, because
  their version is gone or lives in the other storage tier. `kind` names the
  class of the rows' root. The sweep reports them once it has read every
  version of a domain, and a sweep scoped by `committed_since` leaves them out,
  since rows without a version have no commit time.

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

The sweep is the diagnosis. `POST {ext}/admin/integrity/rebuild-nodes` is the
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
which is the repair for `unexpected_nodes`. Rows reported `orphan_nodes` have no
document to rebuild from, so the rebuild deletes them.

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

### `POST {ext}/admin/dump`

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

### `POST {ext}/admin/load`

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
| **400** | a missing or blank `file_sys_loc`; a format value outside the lists above; a non-positive `segment_split_size`; an `encoding` field (the openEHR service model declares that enumeration with no members, so no value a client could send names one); on load, an archive carrying externalized multimedia this server has no store for, and a malformed archive (a mangled or truncated container, manifest, segment or blob entry, or a manifest naming no known format), the [`archive-invalid`](using-the-api/problems.md#archive-invalid) problem, which names the entry. Nothing is loaded |
| **415** | the request `Content-Type` is not `application/json` |
| **500** | a location that holds **no** archive, or one the server's filesystem refuses to read: the service model's `file_not_writable` error for these operations. Nothing is loaded. On dump, the same status covers a location, segment, payload entry or manifest that could not be created or written |

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

## EHR Extract export and import

Four routes under `{ext}/message` that move whole records between systems.
Unlike the admin extensions above, these are **not** admin-gated; see
[the gate table](operations-admin-apis.md#where-these-routes-live-and-what-gates-them).

- `GET {ext}/message/export/{ehr_id}`: export one whole EHR. **200** with a
  JSON array holding one `EXTRACT` that carries every versioned object of the
  EHR, latest versions only. **400** if `ehr_id` is not a well-formed
  identifier, which is refused before any lookup; **404** if the EHR does not
  exist; **406** if `Accept` cannot be satisfied (the extract list is JSON
  only).
- `POST {ext}/message/export` with an `EXTRACT_SPEC` body: export by
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
- `POST {ext}/message/import` with an `EXTRACT` body: clone a whole EHR. Add
  `?ehr_id=<uuid>` to fix the identifier the clone lands under; leave it off
  and the source identifier the extract carries is re-used. **201** with
  `{"uid": "<ehr_id>"}`, so a caller that supplied no id still learns what was
  created. The extract must carry an `EHR_STATUS` (**400**), and the target
  must not already exist: **409**, which also covers an imported `EHR_STATUS`
  naming a subject another EHR already holds.
- `POST {ext}/message/import/{ehr_id}` with an `EXTRACT` body: add the
  extract's content to an **existing** EHR as new versions. **204**. **404**
  if the EHR does not exist; **409** if the EHR already holds an `EHR_STATUS`
  or `EHR_ACCESS` under a different object id, or the imported status names
  another EHR's subject; **422** if a version in the extract is semantically
  invalid (template, RM-invariant or terminology validation).

All four accept either `application/json` or `application/xml` bodies; any
other `Content-Type` is **415**.

> [!NOTE]
> The whole `{ext}/message` group is a FerroEHR extension: the openEHR service
> model defines a Message component, but the released REST API publishes no
> message or extract endpoint at all. These URLs are our own and gate no
> openEHR conformance claim; see [Conformance](conformance.md). For the
> workflow-level view (what an extract is good for and how to drive an
> import) see [EHR Extract & messaging](beyond-core/messaging.md).
