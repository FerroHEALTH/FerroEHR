# Admin & messaging APIs

Two groups of routes sit beside the clinical API for operators rather than for
clinical clients: the **admin API**, which physically deletes, reports on, and
moves repository content, and the **messaging API**, which exports and imports
whole records as openEHR EHR Extracts. This page is the operator's reference
for both: every path, what gates it, what it accepts, and every status code it
answers. Storage integrity, dump and load and the EHR Extract routes are on
[Storage integrity, dump and load](storage-admin-apis.md).

<!-- toc -->

## Where these routes live, and what gates them

The routes ITS-REST defines (the two EHR deletes) sit under the openEHR base
path, `{base}`: with the default `api.base_path` it reads
`/ferroehr/rest/openehr/v1`. Every other route on this page is FerroEHR's own
extension and sits beside it under `{ext}`, which reads
`/ferroehr/rest/extensions/v1`; so `{ext}/admin/dump` is
`/ferroehr/rest/extensions/v1/admin/dump`. Nothing ITS-REST does not define is
served under the openEHR base path.

The two groups are gated differently, and the difference matters
operationally:

| Group | Switch | Authorization class | While switched off |
|---|---|---|---|
| `{base}/admin/…` and `{ext}/admin/…` | `api.admin` (`FERROEHR__API__ADMIN`), default `false` | admin: the configured `authz.rbac.admin_role` (`ADMIN` by default) | every route answers **405 Method Not Allowed** with an **empty** `Allow` header: the resource exists but currently serves no method |
| `{ext}/message/…` | none (always mounted) | ordinary clinical, exactly like the composition API | n/a |

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
> is no confirmation step and no undo. Keep `api.admin` off unless a
> workflow needs it, and gate the admin role tightly.

## Physical deletion

Normal openEHR deletes are *logical*: the version history is retained. The
admin API is the exception: **physical, irreversible** removal, for legal
erasure requests and test-data cleanup.

| Route | Success | Refusals |
|---|---|---|
| `DELETE {base}/admin/ehr/{ehr_id}` | **204**: the EHR and every resource it owns (compositions, `EHR_STATUS`, item tags, contributions, and all their historical versions) are physically gone | **400** malformed id (rejected before any deletion), **404** no EHR with that id |
| `DELETE {base}/admin/ehr/all` | **204**: bulk delete | **400** any id in the list is not a well-formed UUID; the whole request is refused before anything is deleted |
| `DELETE {ext}/admin/party/{party_id}` | **204**: the demographic party, every `PARTY_RELATIONSHIP` naming it as source or target, and all their versions, contributions and commit audits are physically gone | **400** malformed id, **404** no party with that id (a relationship id is refused the same way) |
| `DELETE {ext}/admin/template/{template_id}` | **204** | **404** unknown template, **409** a committed version still references it |
| `DELETE {ext}/admin/query/{qualified_query_name}/{version}` | **204** | **400** a name outside `[{namespace}::]{query-name}` or a version that is no exact `major.minor.patch`, **404** unknown name, or a known name with no such version |
| `GET {ext}/admin/config` | **200**: the effective configuration as a redacted JSON tree | — |

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
delete it (GDPR Art. 19). The externalized multimedia blobs no
surviving version still references are removed from the object store.

**A party delete announces itself the same way.** `DELETE
{ext}/admin/party/{party_id}` deletes a party physically. The released Admin API defines deletion for EHRs only, so the route
is our own extension, and the served OpenAPI document says so. It writes one
erasure tombstone on the party domain's own outbox before its transaction
commits, naming the party and every `PARTY_RELATIONSHIP` erased with it. A
consumer of the party stream has no other way to learn the identity is gone.
The access log records it the way it records an EHR delete: action `D` under
the operation `admin_party_delete`, on the demographic resource, with the
party id as the resource.

**What it keeps:** the audit records naming the `ehr_id` or the party. Art. 17(3)(b)
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
  password hashes, signing-key secrets and S3 secret keys render as
  `***`; every URL that can carry credentials (database, the FHIR audit
  feed, the syslog audit repository, the terminology server and its token
  endpoint, the OTLP collector, the object store,
  the SMART platform and public base URLs, the SCIM public base URL) keeps
  its host and path and masks the embedded credentials and query
  (`postgres://***@host:5432/db`). The usage-report endpoint refuses
  credentials, and the OIDC issuer and the SMART URLs are refused at boot
  when they carry them. Non-secret identifiers (usernames, roles, an OIDC
  issuer) stay visible. Redaction is a property of the configuration's
  secret types alone, with no key-name scan beside them, so no secret value
  can reach this response.

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
| `GET {ext}/admin/report/contribution` | the matching CONTRIBUTION ids as a JSON array, ordered by commit time then id |
| `GET {ext}/admin/report/contribution/count` | how many there are, as a bare JSON number |
| `GET {ext}/admin/report/versioned_composition/count` | how many distinct COMPOSITION version containers had a version committed in the interval |
| `GET {ext}/admin/report/composition_version/count` | how many individual COMPOSITION versions were committed in the interval |

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
| `POST {ext}/admin/archive/ehrs` | `{"ehr_ids": ["…"]}` | **204**: every named EHR and all its versioned content is archived |
| `POST {ext}/admin/archive/parties` | `{"party_ids": ["…"]}` | **204**: every named demographic party is archived |
| `POST {ext}/admin/archive/ehrs/restore` | `{"ehr_ids": ["…"]}` | **204**: every named EHR's archived content is back in the primary tier |
| `POST {ext}/admin/archive/parties/restore` | `{"party_ids": ["…"]}` | **204**: every named party's archived content is back in the primary tier |

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
tiers; a physical delete clears both tiers; and `{ext}/admin/dump` still
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

## Operational-policy registers

Three routes behind the same switch, for `admin_role` only, carry the
operational policy that is data rather than configuration: the audit category
map and the EHDS Art. 3(3) access delay. What each
register holds is on [Operational policy](access-policy.md#the-admin-registers).

| Route | Body or header | Success |
|---|---|---|
| `GET {ext}/admin/registers/{register}` | — | **200** `{"register", "version", "committer", "committed_at", "document"}`: the newest version, or version 0, the default; the `ETag` is the version |
| `PUT {ext}/admin/registers/{register}` | `If-Match: "<version>"` and the whole register document | **200**: the new version, as the read answers it, with its `ETag`; the next request on every replica runs under it |
| `GET {ext}/admin/registers/{register}/history` | — | **200** `{"register", "versions": [...]}`: every version, newest first, ending with version 0 |

`{register}` is `audit-categories` or `access-delay`, and any
other name is **404**. A write without `If-Match` is **400**, and one whose
`If-Match` is no longer the newest version is **412**: read the register again
and write over the version it answers. A document that does not hold is
**400** and leaves the version in force. Every read and write is an access
record of its own.

## The access policy

Four routes behind the same switch, for `admin_role` only, carry the
controller's access policy. What it holds is on
[Operational policy](access-policy.md#the-access-policy).

| Route | Body or header | Success |
|---|---|---|
| `GET {ext}/admin/access_policy` | — | **200**: the newest `ACCESS_POLICY`; the weak `ETag` is its version uid and `Last-Modified` its commit time. **404** while none is committed |
| `PUT {ext}/admin/access_policy` | `If-Match: <version uid>`, or `If-None-Match: *` for the first version, and the whole `ACCESS_POLICY` | **200** (**201** for the first): the new version with its `ETag`; the next request on every replica runs under it. Needs a policy grant of `write_access_policy` |
| `GET {ext}/admin/access_policy/history` | — | **200**: every `ORIGINAL_VERSION`, newest first, with its contribution and commit audit |
| `GET {ext}/admin/access_policy/versioned` | — | **200**: the `VERSIONED_ACCESS_POLICY` |

A write with neither condition is **400**, and one whose condition no longer
holds is **412**: read the policy again and write over the version it answers.
A body that is no canonical-JSON `ACCESS_POLICY` is **400**, a policy that reads but breaks a rule of the draft model or of its template is **422**, and both leave the version in force.

## Legal marks: restriction, objection and retention

These routes sit behind the same switch and role and carry the marks the law
asks a repository to hold beside its clinical content. None of them deletes
anything.

| Route | Body or parameter | Success |
|---|---|---|
| `POST {ext}/admin/restriction` | `{"ehr_id": "…", "vo_id": "…"?, "ground": "…", "note": "…"?}` | **204**: the restriction is recorded and in force |
| `GET {ext}/admin/restriction` | `?ehr_id=…` | **200**: the register, newest request first, lifts included |
| `POST {ext}/admin/restriction/lift` | `{"ehr_id": "…", "vo_id": "…"?}` | **204**: every in-force restriction at that grain is lifted |
| `POST {ext}/admin/research-objection` | `{"ehr_id": "…", "objected": true, "ground": "…"?}` | **204**: the objection, its override, or its withdrawal is recorded |
| `GET {ext}/admin/research-objection` | `?ehr_id=…` | **200**: `{"objected": …, "objected_at": …, "ground": …}`, whether an objection stands unoverridden, when the subject objected and any override ground; `false` and nulls for an EHR with none |
| `PUT {ext}/admin/retention/policy` | `{"kind": "…", "jurisdiction": "…", "category": "…"?, "origin": "…"?, "period": "…", "anchor": "…", "source": "…"}` | **204**: the period is declared, keyed on one EHDS priority category when `category` is set and on one origin a `data_origin` grant in force names when `origin` is set |
| `GET {ext}/admin/retention/policy` | — | **200**: the whole retention register, each period with its `category` and `origin` (`null` for every object of the kind, or every origin) |
| `PUT {ext}/admin/retention/anchor` | `{"ehr_id": "…", "jurisdiction": "…", "anchored_at": "…"?, "hold_at": "…"?, "hold_ground": "…"?}` | **204**: the EHR's anchor and any whole-record hold |
| `GET {ext}/admin/retention/anchor` | `?ehr_id=…` | **200**: `{"anchor": {"jurisdiction": …, "anchored_at": …, "hold_at": …, "hold_ground": …}}`, or `{"anchor": null}` while none is recorded |
| `POST {ext}/admin/retention/hold` | `{"vo_id": "…", "held": true}` | **204**: the per-object exemption is placed or released |
| `GET {ext}/admin/retention/hold` | `?ehr_id=…` | **200**: `{"holds": [{"vo_id": …, "held_at": …}]}`, the EHR's held objects, newest hold first |
| `GET {ext}/admin/retention/due` | `?limit=100` | **200**: what has run out, oldest first, per period and category, with the due and held object counts |
| `POST {ext}/admin/access-delay` | `{"ehr_id": "…", "vo_id": "…", "until": "…"?, "note": "…"?}` | **204**: the composition is withheld from the person under EHDS Art. 3(3) until `until` (an RFC 3339 instant within the access-delay register's `max_delay` of now) or for the access-delay register's `max_delay`; **400** while the [access-delay register](installation/config-auth.md#the-access-delay-register) is off |
| `GET {ext}/admin/access-delay` | `?ehr_id=…` | **200**: the access-delay register, newest hold first, each with its `basis` (`category` or `professional`), its end and its release |
| `POST {ext}/admin/access-delay/release` | `{"ehr_id": "…", "vo_id": "…"}` | **204**: every hold of the composition still in force is released, and the person reads it |
| `POST {ext}/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/access-delay/release` | none | **204**: the treating professional's release from the clinical surface, with the checks of a write to that EHR; **403** to a request that may be the person's |
| `POST {ext}/admin/access-log/erase` | `{"uid": "…", "grounds": [DV_CODED_TEXT, …]}` | **200**: the `ACCESS_LOG_TOMBSTONE` that replaced the access-log entry, for an erasure a law orders before the entry's retention ends; it keeps the entry's uid, `system_id`, `time_recorded` and chain link and names these grounds ([Retention, and who chooses it](ehds-logging.md#retention-and-who-chooses-it)); **409** for a tombstone or while the local audit store is off |
| `GET {ext}/admin/access-log` | `?ehr_id=…&from=…&to=…&outcome=…&offset=…&fetch=…` and the other filters | **200**: the access log's records across EHRs, newest first, in the full view ([Reading the access log over ITS-REST](access-log.md#reading-the-access-log-over-its-rest)); the read writes its own access-log entry |
| `GET {ext}/admin/access-log/seals` | `?from_id=…&to_id=…` | **200**: `seals`, each window that covers an entry id of the span, less than 10000 apart, with its `window_no`, `first_id`, `last_id`, `entries`, `previous_digest`, `digest` and `folded`; **400** for a span of 10000 or longer |
| `GET {ext}/admin/access-log/checkpoint` | `?sequence=…`? | **200**: the newest stored `ACCESS_LOG_CHECKPOINT`, or the one at `sequence`; **404** when none is stored there |
| `POST {ext}/admin/ehr-access` | `{"ehr_id": "…", "preceding_version_uid": "…", "settings": {…} or null}` | **200** `{"ehr_id", "version_uid"}`: the `EHR_ACCESS` settings are committed on the person's instruction as a new version over `preceding_version_uid`; they hold the EHDS Art. 8 restriction and the Art. 10 opt-out as `restriction` and `opt_out` directives ([Security](access-control.md#restriction-of-access-by-health-professionals-ehds-art-8)); **400** for settings that do not parse; **422** for settings that break an invariant of the draft model, a restricted object id that names no versioned object, or an `opt_out` directive while `[authz.grants] allowed` does not allow `access_service`; **412** when `preceding_version_uid` is not the current version |
| `GET {ext}/admin/ehr-access` | `?ehr_id=…` | **200** `{"ehr_id", "versions": [{"version_uid", "settings"}]}`: every `EHR_ACCESS` version, newest first, so the person's lifted restrictions and reversed opt-outs stay readable |
| `POST {ext}/admin/ehr-access/repair` | `{"ehr_id": "…", "settings": {…}}` | **200** `{"ehr_id", "version_uid"}`: the `EHR_ACCESS` of an EHR held closed by settings that do not parse is committed again with these settings, which must parse; **409** for an EHR that is not held |

A body of the wrong shape or a malformed id is **400**, an id that names nothing
is **404**, and a body without `Content-Type: application/json` is **415**, in
each case before anything is recorded. Setting or lifting a mark is an access
record of its own, and so is each read of one.

A `category` is named as the [audit category register](installation/config-audit.md#the-audit-category-register) names it (a priority category by
its code, `Patient-Summaries`, `Electronic-Prescriptions`,
`Electronic-Dispensations`, `Medical-Imaging`, `Laboratory-Reports` or
`Discharge-Reports`, or as `<system>|<code>`; a declared national category as
its `<system>|<code>`; or `none`; an unknown one is **400**) and stored as its
`<system>|<code>` token, which serves EHDS
Annex II 3.4: retention periods that take the categories of the data into
account. The server mirrors the map into the register at every boot and with
every write of the audit category register. A period
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

An access restriction is not a restriction of processing: it limits what a
health professional reaches and nothing else, and it is invisible to them. What
it changes is on [restriction of access by health
professionals](access-control.md#restriction-of-access-by-health-professionals-ehds-art-8).

An access delay is the opposite direction: it withholds a composition from
the person, never from a professional. What it changes is on [delayed access
by the person](access-control.md#delayed-access-by-the-person-ehds-art-33).

## Re-keying a professional's binding key

A `PERSON`'s binding key, the login identifier it is bound to, is immutable
on every ordinary write (RFC 7643 §2.2). `POST {ext}/admin/party/binding-key`
is the one change, made by an administrator. No openEHR specification
governs it; it is our own extension.

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"party_id": "0d6c…", "issuer": "https://login.example/tenant", "id": "5b0f…"}' \
  "$BASE/admin/party/binding-key"
```

The `PERSON` gets a new version carrying the new key, the old key stops
binding in the same commit, and one access record carries both keys. The
route answers **403** when the new key is the requesting administrator's own
login, **409** when the `PERSON` carries no key of that issuer or the new
key is already bound to another party, and **404** for an unknown or
deleted `PERSON`. The [party mapping](installation/config-authz.md#the-party-mapping)
says which parties are protected.

## Other routes under `{base}/admin`

The outbox feed shares the `/admin` path prefix but **not** the
`api.admin` switch: it answers for the readers the `read_change_feed` grants in
force name, and **404** for any other name, whatever `api.admin` says:

| Routes | Gate | Documented in |
|---|---|---|
| `{base}/admin/outbox/{reader}`, `{base}/admin/outbox/{reader}/ack` | the `__ferroehr.outbox` scope and the caller's own `read_change_feed` grant naming the reader | [Change events (outbox feed)](beyond-core/outbox-feed.md) |

An unauthenticated caller is **401**, and a caller without the scope or the
grant **403**.

For everything else an operator needs (migrations, health probes, the
management surface, observability and upgrades) see
[Operations](operations.md).
