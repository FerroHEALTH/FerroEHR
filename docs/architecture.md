# Architecture

A pure-Rust, **openEHR-spec-conformant** Clinical Data Repository (CDR):
ITS-REST 1.1.0 at the API, AQL 1.1 as the query language, greenfield
PostgreSQL-18-native internals. Two layers:

1. **The openEHR foundation (`openehr-*`) — generated from the official
   machine-readable specs, and done.** The spec types, canonical serialization,
   the ITS-REST contract, and the AQL front end are produced deterministically
   by `openehr-codegen`, not hand-written.
2. **The application (`ferroehr-*`) — modern idiomatic Rust of our own design
   on that foundation.** The server, storage, service layer, AQL
   execution engine, validation, and auth — proper crates, our own algorithms,
   the openEHR specifications as the authority. EHRbase and other CDRs are
   prior art, not an oracle. Shipped.

Authoritative roadmap: the public **FerroEHR Roadmap board** (a GitHub
Project view over the tracker; its readme carries the direction themes —
`.claude/rules/project-board.md`); the open-items tracker is
GitHub Issues (root `CLAUDE.md` §Issue workflow), with `docs/plans/` (deep
working plans) under it; the build record is the closed issues + PR
descriptions + `CHANGELOG.md` + git history. Per-endpoint call chains are
read from the code (router → handler → service → SQL) — there is no standing
endpoint-map document (a standing map goes stale).

Key architectural decisions (described in full in the sections below): the
spec + ITS layer is generated from the vendored machine-readable specs
(BMM/XSD/OAS) by `openehr-codegen`; the application is idiomatic Rust of our
own design on those generated crates, with its own PG18-native storage (one
`node` table, an append-only `version` table with one mutable `vo_head` row
per object, and tier partitions instead of mirror tables) and its own typed AQL
engine,
and acceptance measured by the openEHR conformance suite (EHRbase is prior art,
not an oracle); the application is five crates with zero re-exports, and its
service layer is its own design (one module per domain area, concrete
methods), judged by what goes in and comes out over the wire.

## The generated openEHR foundation (done)

- **Spec types** (BMM → Rust, `openehr-codegen -- emit`): `openehr-base` (BASE
  1.3.0), `openehr-rm` (RM 1.2.0 — the domain model everything consumes),
  `openehr-am` (AM 1.4 + 2.4, as `v1_4`/`v2_4`), `openehr-term` (TERM data
  classes + hand-written bundle/assets), `openehr-lang` (BMM/P_BMM model).
- **Canonical JSON** — emitted **manual serde impls** (`emit-json` → each
  defining crate's `src/json_serde.rs`; no derives, no serde attributes on
  spec types) put `_type` self-tagging on every type, over the small shared
  `openehr_base::serde_support` runtime; `openehr-its::json` is the entry
  points (`to_canonical_json`/`from_canonical_json`/`from_canonical_value`,
  refusal paths via `serde_path_to_error`) + ITS-JSON schema validation.
- **Canonical XML** (`emit-xml`) — generated `ToXml`/`FromXml` over a
  hand-written `quick-xml` runtime, from the vendored XSDs + BMM field model
  (`openehr-its`).
- **ITS-REST contract** (`emit-rest`) — generated DTOs + `#[async_trait]`
  server traits + route tables per API group, from the vendored OpenAPI
  (`openehr-its`); the application implements the traits.
- **AQL front end** — hand-written `logos`+`chumsky` lexer/parser/AST
  (`openehr-query`), corpus-validated.
- **RM invariants** (`emit-validate`) — the BMM's invariant expressions are
  machine-classified (a pinned total tripartition) and the mechanical ones
  emitted as generated cores in `openehr-rm/src/validate/generated.rs` —
  the single source both the typed and fast validation paths call;
  terminology-backed invariants enforce at the `openehr-its` dispatcher
  against the `openehr-term` bundle; the aggregate exclusions carry
  citation-pinned adjudications in the generated register.

The generator (`tools/openehr-codegen`) is a four-stage pipeline — load
(vendored inputs verbatim) → analyze (merged include-closures, polymorphic
seams, ownership graph + back-reference cycle breaking, constructibility
proof, enumerations/constants/invariants) → plan (per-class emission
decisions + the declarative, spec-cited decision maps) → render (the only
text-producing stage). Emitter invariants (completeness — nothing a loaded
schema declares is silently dropped —, constructibility, byte-determinism,
source-package mirroring, closure correctness) are themselves a test suite.
**The spec types carry no serde derives or attributes**: canonical JSON is
emitted MANUAL `serde::Serialize`/`Deserialize` impls (explicit generated
code in each defining crate's `json_serde.rs`, over the small shared
`openehr_base::serde_support` runtime); the wire contract (`_type`-first,
BMM field order, RM number typing, the STRICT reader — undeclared and
duplicate keys refused) is pinned by the canonical-output contract gate.

Fidelity is proven by gates (`openehr-its/tests/`); a `codegen-drift` CI job
regenerates everything and fails on any diff.

## Storage (ours, PG18-native)

Grounded in docs-verified PostgreSQL physics (JSONB has no partial detoast —
big single documents pay whole-document decompression per leaf access; GIN
serves no range/ordering), the storage is a **decomposed node model designed
fresh** (the diagrammed deep-dive is the book's Storage architecture page,
`website/book/src/concepts/storage.md`):

- **`version`** — one write-once row per version of a versioned object. A
  version row and its node rows are never updated after commit (BASE
  `architecture_overview/master07-security.adoc` §Integrity), so a supersession
  is one insert: there is no validity interval, no partial index over a mutable
  predicate and no close-out statement. Validity is derived from `committed_at`,
  which the RM already copies into every version from the contribution audit
  (RM common master06 §Committal and Audits): version *i* is valid over
  `[committed_at_i, committed_at_i+1)`, and `version_at_time(t)` is the trunk
  row with the greatest `committed_at` at or before `t`. A BRIN index on
  `committed_at` serves time-range listing on the append-only heap. A version
  whose content equals the preceding tip's (SHA-256 over the canonical body
  with the root `uid` removed, `version.content_digest`) writes its row, its
  audit, its contribution and its uid-stamped body as any other, and no node
  rows: `version.content_sys_version` names the version whose rows hold the
  content, and every read of a version's rows joins `node` on the stored
  `version.content_key`, `coalesce(content_sys_version, sys_version)` kept
  as a generated column so the planner joins on a plain column (#4415), or
  under `LATEST_VERSION` on the head's copy of it (`vo_head` below), the
  root's `uid` and
  header coming back from the reading version's row. Nothing physical is
  ever deleted below the object, so a shared tree is never orphaned; the
  opt-in `[record.versioning] record.versioning.idempotent_resend` answers an identical re-send
  with the existing version and commits nothing, which ITS-REST does not
  govern: our own design.
- **`vo_head`** — the one mutable row per versioned object, and the only row a
  commit updates. It answers every "what is current" question in one
  primary-key probe: `trunk_head_sys_version` IS `LATEST_VERSION`,
  `head_sys_version` the RM's `latest_version`, plus the lifecycle state, the
  template, the tier, the archive marker and the restriction and retention
  marks. `trunk_head_content_key` is the trunk head's `content_key`, and a
  `LATEST_VERSION` read joins the node rows on it, so a node row an
  identical-content chain shares links to the one head and reaches the
  version row through the head's key. The version partitions declare a
  `vo_id` distinct count of half their rows (`n_distinct = -0.5`, PostgreSQL
  18, ALTER TABLE): a long identical-content chain puts most version rows
  on a few objects, and priced from that skew a probe of one object's
  versions by `vo_id` either looked as cheap as the head's exact path or,
  for the at-time read, too dear for its per-root nested loop, which then
  hashed every composition root (489 against 1 719 buffers on the storage
  bench, #4944). No specification governs planner statistics: our own design.
  None of the columns a commit changes appears in an index, which is the
  condition PostgreSQL 18 §"Heap-Only Tuples (HOT)" states for a heap-only
  update; `fillfactor = 90` reserves the same-page room.
- **`node`** — one unified table for all versioned-object content
  (COMPOSITION / EHR_STATUS / FOLDER). One row per RM structure node with a
  **nested-set index** (`num`, `num_cap`, `parent_num`): AQL CONTAINS is an
  integer interval join, never a JSON walk. Promoted predicate columns
  (`rm_type` — full RM type names, no alias compaction —, `archetype`, `name`,
  `name_code`, `name_terminology`, `ehr_id`; `path COLLATE "C"` is the
  reassembly key and never a predicate) and the node's **canonical openEHR
  JSON fragment** (`openehr-its` encoding, no synthetic fields) stored as only
  what no column holds (storage generation 3, #4306): `data jsonb` leaves out
  `_type` (it is `rm_type`), a lowercase `archetype_node_id` (it is
  `archetype`), a plain or simply coded `name` (it is `name`, `name_code`,
  `name_terminology`) and an ELEMENT `value` of a common shape, which the typed
  value columns `value_type`, `value_num`, `value_text`, `value_code` and
  `value_terminology` hold instead (DV_QUANTITY, DV_COUNT, DV_ORDINAL,
  DV_CODED_TEXT, DV_IDENTIFIER, DV_BOOLEAN and the string-valued types, each in
  its basic shape only). An archetype root's
  `archetype_details` and `archetype_node_id` leave together (#4525) when the details
  are exactly its own archetype id and an `rm_version` and the id is the
  recased `archetype` (`openEHR-` and the class part in capitals), with
  `rm_version` in its column, and a `language` in `ISO_639-1` and an
  `encoding` in `IANA_character-sets` leave for the `language` and `encoding`
  columns. The versioned object's root (storage generation 5, #4904) takes
  the same header rule with the version row's `template_id` as the template
  its header may name, and leaves out its `uid` when it is exactly the
  `OBJECT_VERSION_ID` the version row's identity columns render
  (`ext.version_uid`). A `HISTORY.origin`, `EVENT.time` or `ACTION.time` of
  exactly `_type: DV_DATE_TIME` and a `value` is stored as that string, and a
  `subject` of exactly `{"_type": "PARTY_SELF"}` leaves. Four nullable
  boolean markers, `origin_compact`, `time_compact`, `party_self` and
  `uid_derived`, say which of those keys a row left out
  (`crate::storage::compact`): the rebuild reads a marker, never an absent key
  or a JSON shape, so a fragment of any other shape reads back as stored and
  a row written before the markers reads as it did. A key is left out only when the columns
  rebuild it exactly; everything else stays in `data` verbatim, and `data` is NULL when
  nothing remains (two thirds of the rows). `ext.node_name`, `ext.node_value`,
  `ext.node_archetype_node_id`, `ext.node_archetype_details`,
  `ext.node_code_phrase`, `ext.node_date_time` and `ext.node_fragment`, inlined `LANGUAGE sql`
  functions, rebuild the omitted keys, so every reader sees the canonical
  fragment (`crate::storage::slim` is the write half; its Rust rebuild puts a
  restored key where jsonb key order placed it, so a served fragment
  keeps its bytes). A root row's fragment is read with its version row
  beside it, which is where its uid and template come back from. The AQL emitter reads a fragment
  through the first member its path names, and the scalar paths into a typed
  value (`value/magnitude`, `value/value`, `value/units`, the defining code)
  read the typed column directly. No index sits on the typed columns: no
  measured statement would use one. Projections are read from the row and
  never from the version body, whose per-row parse measured 50 to 300 times
  slower than a fragment read. Point reads do not reassemble the fragments: the
  whole canonical body is also stored verbatim as `version.body text` and
  served from there in one statement.
- **Tiers are partitions.** `version`, `node` and `vo_attestation` are each
  `PARTITION BY LIST (tier)` with a `hot` and a `cold` partition and no default
  partition. Archiving an EHR is `UPDATE version SET tier = 'cold' WHERE
  ehr_id = $1`; PostgreSQL moves the rows between partitions and the `node` and
  `vo_attestation` foreign keys carry their rows across with `ON UPDATE
  CASCADE` — verified first-hand on PostgreSQL 18.6, since the documentation
  neither permits nor forbids it. What that buys over the former mirror tables:
  one relation, so foreign keys hold across the tier and every read path and
  every future column reaches cold without a view rebuild; and AQL excludes
  cold by writing `tier = 'hot'` as a literal, which the planner prunes at plan
  time. The cold partitions carry the primary key alone, because archived
  content leaves the queryable store until it is restored.
- **The supporting relations** — `ehr`, `contribution` and `commit_audit`;
  `ehr_folder` and `item_tag`, which stand outside change control; the
  definition stores `template_ref`, `template_store`, `archetype_store`,
  `adl2_artefact` and `stored_query`; the change-event `event_outbox` with its
  `event_outbox_reader` cursors, which the readers the `read_change_feed`
  grants name move over the outbox feed (`extensions::outbox`); and
  `blob_ref`, which records that a version references an externalized
  multimedia blob so collection is an index probe rather than a scan of `node`.
  Every write emits contribution + commit audit in the same transaction
  (openEHR requirement).
- **`restriction`, `retention_policy`, `retention_anchor`** — the registers
  behind the legal marks: the ground a restriction rests on (GDPR Art. 18),
  denormalised onto `vo_head.restricted_at` and `ehr.restricted_at`, and the
  retention period per category and jurisdiction with its citation
  (Art. 5(1)(e), Art. 30(1)(f)), measured against each EHR's anchor and
  suspended by a whole-record or per-object hold. The subject's objection to
  research processing (Art. 21(6)) needs no register of its own: it is
  `ehr.research_objected_at` with the controller's public-interest ground
  beside it. `retention_due` is a view, never a job: the CDR deletes no
  clinical content on a timer.
- **`ext`** — our own helper functions over openEHR values
  (`openehr_magnitude(jsonb)` for DV_ORDERED ordering semantics, the date,
  time, date-time and duration readers, `openehr_numeric`), the node fragment
  readers (`node_name`, `node_value`, `node_fragment`), the `posture`
  table the server stamps at boot and the clinical subject guard reads, and
  `storage_generation()`, a label the measurement harness files its records
  under. Each helper ships in the language that measured faster over a
  50 000-row corpus of the values it sees: five fold into the calling statement
  as `LANGUAGE sql`, and the three that read their argument many times stay
  PL/pgSQL. Only `openehr_date_days` and `openehr_timestamp` keep an
  `EXCEPTION` block, because there the trapped native cast measured cheaper
  than any guard that would let the same cast run untrapped; every other helper
  reaches a cast through an ordered guard that proves it cannot raise. The
  `IMMUTABLE` ones are index-legal, so an expression index can carry a
  measured hot path; none exists today, and `openehr_timestamp` is `STABLE`
  by necessity (its result depends on the session TimeZone) and can never be
  indexed.
- **`party`** — the pseudonymisation domain: the PARTY versioned objects and
  their change control, rendered from the SAME DDL template as the clinical
  relations (`app/ferroehr/migrations/templates/*.sql.in`), so the two domains
  cannot drift. A party body decomposes like clinical content: the party root,
  each `PARTY_IDENTITY`, `CONTACT`, `ADDRESS` and `CAPABILITY` nested in it, and
  the `ITEM_STRUCTURE` under each get their own `node` row. Beside them,
  `national_identifier` holds identifiers sealed under authenticated encryption
  against the schemes `identifier_scheme` declares, `party_relationship_target`
  is the target-side index behind `PARTY.reverse_relationships`, and the domain
  carries its own `item_tag`, `event_outbox` and `blob_ref` because the commit
  path is one code path over two schemas. Nothing selects a domain but the pool's
  `search_path`, so one set of storage code serves both; a `CHECK` on each side
  refuses the other's rows. GDPR Art. 4(5) and Art. 32(1)(a); no openEHR spec
  governs storage layout or database roles.
- **`PARTY.reverse_relationships` at a version or at a time** (#5473). RM
  1.1.0 makes it an attribute of `PARTY` and RM 1.2.0 a function
  (`master00-amendment_record.adoc` SPECRM-124), whose result a repository
  lookup defines (`Post_reverse_relationships_validity`). No openEHR spec says
  how a derived list behaves on an old version, and no REST resource serves the
  1.2.0 function, so both rules here are our own design:
  - A read of the current version by its container id serves the list as it
    stands now. Every read addressed to one version, by its
    `OBJECT_VERSION_ID`, at a `version_at_time`, or as the `data` of a
    `VERSION<PARTY>`, and the representation a create or update returns, serves
    it as it stood when that version was committed. The `ETag` of each of
    those responses names that version, so one version has one document.
  - `party_relationship_target` keeps one row per interval over which a source
    party's trunk named the target (`valid_from`, `valid_to`, open while the
    current trunk version names it), written incrementally in the commit
    transaction and rebuilt from the stored trunk versions by an import or an
    archive load (`storage::party_reverse::rebuild`). A point-in-time read is
    an indexed probe of the intervals plus one indexed lookup of each source's
    trunk version at that instant, never a walk of a source's versions.
  - The stable profile carries the list in the body of every PARTY
    representation, canonical JSON and XML. The development profile does not:
    ITS-JSON publishes no RM 1.2.0 schema, its released schemas are generated
    from the BMM properties with `additionalProperties: false`, and the 1.2.0
    BMM lists `reverse_relationships` among the functions, so a member in the
    body would be refused by a development reader, FerroEHR's own included,
    when the document comes back. The evaluated function is served instead on
    the extension route `GET {extensions}/demographic/party/{uid_based_id}/reverse_relationships`
    (optional `version_at_time`), under the same pinning rule, in both
    profiles: a JSON array of `LOCATABLE_REF`s, `[]` when none.
- **Which EHR belongs to which subject** is the `ehr` table's subject columns,
  promoted from `EHR_STATUS.subject.external_ref` on every EHR create and
  `EHR_STATUS` update. A cohort, the AQL demographic join and the EHR index
  resolve a party or a subject through them; `service::linkage` holds those
  crossings and the access record each one owes. The linkage schema that held a
  cross-reference before is gone (`audit/0021_drop_linkage_domain.sql`).
- **The access log** is the `audit` schema. `audit_event` holds one row per
  access as the request path writes it today, with the FHIR `AuditEvent`
  rendered from its columns. `access_log_record` holds the draft openEHR
  `ACCESS_LOG_ENTRY` (#5121) as its ITS-JSON canonical form without its link
  (RFC 8785), the search columns its insert trigger derives from that form,
  and its `ACCESS_LOG_LINK`: the content digest over the canonical form and
  the digest over the previous digest, the sequence and the content digest.
  Both tables continue one hash chain under one cursor and one head
  (`audit_chain_state`), each `audit_event` row keeps verifying under its
  `digest_version`, and `audit.verify_audit_chain()` and its incremental form
  walk both in position order. An entry changes only by its replacement with
  its own tombstone, which keeps its uid, time and link, so the chain verifies
  after its content is gone (`audit/0027_access_log_record.sql`,
  `access_log::store`). Each entry states its retention when it is written
  (`access_log::retention`: `retain_until` and the grounds `ehds-logging`'s
  policy computes from its categories and origins, the longest when they
  differ), and the audit store's hourly worker replaces every entry past its
  `retain_until` by its tombstone, in any order of expiry
  (`access_log::expiry`); an erasure a law orders takes the same path through
  `POST /admin/access-log/erase`, which writes the access-log entry recording
  the erasure in the tombstone's own transaction, after the audit sender has
  said it would take the record, so the log holds both or neither.
  `access_log_checkpoint` keeps the signed `ACCESS_LOG_CHECKPOINT`s of the head, signed with the version-signing
  keyring's OpenPGP key; a link's `signature` may be set once, after the
  insert that computed its digest. `access_log::checkpoint` seals and verifies
  (the chain through `audit.verify_audit_chain()`, the checkpoint and the link
  signatures beside it), `access_log::chain` verifies a span of links apart
  from the database (an export), and `access_log::transfer` exports one EHR's
  records with the span and checkpoint and imports another system's into
  `access_log_imported_*`, outside this system's chain
  (`audit/0028_access_log_checkpoint.sql`).
  Every access to EHR data writes one entry per EHR it
  reached, refusals and reads of the log included: the drain builds it from
  the access's `AuditEvent` (`access_log::entry`) and `AuditStore::insert_batch`
  appends it in the transaction that stores the batch's records, so an access
  adds no transaction and the store holds a record and its entry or neither.
  No openEHR spec governs audit storage: our own design.
- **One pool and one DSN per storage domain.** `[database.clinical]`,
  `[database.party]` and `[database.audit]` each carry a
  `url`/`url_file`, defaulting to the shared `[database].url`, and
  `db::connect_domains` opens the three pools each with only its own schema on
  its `search_path`. A domain whose DSN names another host lives in a database
  or cluster of its own, which is the separation a schema cannot make (a base
  backup, WAL archiving and physical replication carry every schema of a
  database together). Schema preparation runs per database: the `ext` set is
  applied by whichever domain reaches that database first, so two co-located
  domains share one `ext` and a relocated domain carries its own copy.
- **One checkout per pool per request.** Every REST request runs inside
  `db::acquire::request_scope`: the connection a request takes from a pool is
  kept between its statements and returns to the pool when the request
  answers, so the access checks and the read or commit of one request queue
  for the pool once, and the pool's release round trip is paid once. A background task outside a scope checks out per statement.
- **Four `NOINHERIT` runtime roles** hold the domains: `ferroehr_clinical` and
  its read-only twin hold `clinical`, and `ferroehr_party` and its twin hold
  `party`. Each is revoked from
  the domains it does not own, in both directions, and the boot gate
  (`db::verify_domain_isolation`) refuses to serve when any of them can read
  across, when two separately-configured domains authenticate as one role, or —
  under `deployment.profile = "production"` — when a domain role is missing.
- Migrations via `sqlx migrate add` (official CLI): four sets, run per database
  in order — `ext`, then `clinical`, `party`, `audit` — each with
  its own `_sqlx_migrations` table, each a sequence of natural files with one
  concern apiece and its grants in the last file of the set, so a relation file
  stays a declaration of shape. The four schemas are created by the server
  before the first migrator runs; the DDL needs no extension. A database carrying the first generation's `ehr` or
  `demographic` bookkeeping is refused at boot, by name, with the remedy.

## AQL engine (ours)

Parsed AST (`openehr-query`, done) → **path analysis + typing against a
BMM-generated RM attribute model** (attribute→types, multiplicity,
abstract→concrete descendants — generated, not reflected, not hand-written)
→ **our typed query IR** → SQL via `sea-query` (nested-set interval joins for
CONTAINS chains; `jsonb_path_query_first` for leaf extraction and
`jsonb_path_query` as a lateral set-returning function for array unnesting;
`ext.openehr_magnitude` and `ext.openehr_timestamp` for typed comparison and
ordering) → execute (`sqlx`) → `RESULT_SET` (1.1.0). The emitter uses no
jsonpath item methods (their date/time conversions depend on the session
TimeZone and are not index-legal), no `JSON_TABLE`, and no GIN index (it
emits no GIN-servable operator; the baseline removed the index). The one
value index is `idx_node_hot_value_search` on `ext.node_search_text`, the
text an equality on a text leaf of an ELEMENT value compares against from
the typed value columns or the stored fragment: the WHERE lowering adds, for
such an equality, a FROM item of the distinct versions some ELEMENT row of
which carries the value (`aql::sql::condition`, the value anchor), so a
lookup by a value reads the one object instead of every node of the
archetype (#4437). The feature envelope is documented per construct;
rejections are explicit typed errors.

A query whose outermost `FROM` class is a demographic class is the
experimental demographic extension (no openEHR spec governs it; QUERY
master03 §FROM gives semantics only to the clinical repository). Every class
whose concrete RM descendants are all demographic versioned objects is a root
(#5474): the five concrete `PARTY` classes, the abstract `PARTY` and `ACTOR`
over the concrete classes the RM model lists below them (`aql::lower`, the
`descendants` of the generated RM model, so a root reads `rm_type IN (…)`),
and the standalone `PARTY_RELATIONSHIP` container, whose root row is the only
`PARTY_RELATIONSHIP` row (an inline relationship stays in its source party's
data and is read by path). Every root carries the one policy predicate,
`vo_head.policy_party_at IS NULL` on the head row it joins by key
(`aql::sql::gates`), so a caller who is no administrator reads no policy
party or acts-for relationship container under any root. A join under an abstract root reads
each party's class (`version.kind`) and matches the subject reference against
that class and its demographic RM ancestors. The planner
marks the IR `QueryDomain::Demographic`, and the same SQL runs on the party
pool, whose `search_path` names the `party` schema rendered from the same DDL,
without the EHR gates (population, `ehr_ids`, `EHR_ACCESS`, ABAC subject
scope). An `EHR` operand directly under the root is the join to the EHR the
party is the subject of: `aql::join` cuts the AST into a party half and a
clinical half, the service runs the party half on the party pool, finds the
EHRs whose subject names those parties, runs the clinical half scoped to them
on the clinical pool, and joins, orders (each key by the rank its own
half's `ORDER BY` assigned) and pages the rows in memory, each half bounded
by the result ceiling. Any other mix of a demographic root with `EHR` or
clinical classes is refused.

## REST surface + auth (`ferroehr-rest`)

Base path `/ferroehr/rest/openehr/v1`, implementing the generated ITS-REST
1.1.0 server traits over `axum` with a `tower-http` middleware stack and
content negotiation (canonical JSON/XML via `openehr-its`). Extensions: admin
API (plus the own-design activity-report, archive/restore and dump/load routes under
the same gate), the `/message` group (EHR-Extract export/import over the RM EHR
Extract model; ITS-REST publishes no such API, so the routes are our own design),
the always-on public health family (`/health`, `/health/liveness`,
`/health/readiness`), `/rest/status`, the ops-introspection `/management/*`
surface, item tags (there is no EhrScape surface). **Auth:** Basic +
OAuth 2.0 bearer tokens via `ferrohealth-smart`; authorization is the
shipped RBAC/ABAC `access` module in `ferroehr-rest`.

### The access layer (`ferroehr-rest::extensions::access`) — our own design

ITS-REST makes authentication a `SHOULD` and mandates no scheme, and no openEHR
specification governs authorization, so **only the 401-vs-403 split is
spec-grounded**: a missing or rejected credential is `401` with a
`WWW-Authenticate` challenge, an authenticated-but-refused caller is `403`.
Everything finer is our own design, built to the IETF OAuth2/JOSE RFCs and
expressed in the vocabulary of NIST SP 800-162 (ABAC) and ANSI/INCITS 359
(Core RBAC).

**Evaluation order** — each stage can only narrow what the previous one allowed,
which is what makes an optional stage safe to disable:

1. **Authentication** — Basic (Argon2id PHC, the `ferrohealth-smart` store's
   verified-credential cache) or OAuth2/OIDC bearer, both checked by the
   crate's gate. Produces a `Principal` (subject, roles, scopes, the validated
   claim set) or a typed refusal. A header that is no scheme and credential is
   a `400`, not a `401`: the server never read a credential. A bearer token
   outside the RFC 6750 §2.1 `b64token` alphabet is a malformed token, `401`.
2. **`EHR_ACCESS`** — always on and unconditional, from the RM's own gateway
   clause ("All access decisions to data in the EHR must be made in accordance
   with the policies and rules in this object"). The settings are the
   `access_directives` scheme, an `ACCESS_DIRECTIVE_SETTINGS` of the draft
   access control package, refused whole with `400` at every write unless the
   draft BMM's invariants and the terminology groups hold (RM EHR IM
   §`ACCESS_CONTROL_SETTINGS` leaves the class "Currently implementation
   dependent": the scheme is our own design). The default archetype
   `access_directives.v1.0.0` constrains the settings to that class, which the
   ADL 2 validator reads from the draft class tables beside RM 1.2.0
   (`draft_model`, AOM2 master11 RM adaptation). A new version is committed by a
   gate-keeper the current settings name or by `admin_role` on the subject's
   instruction (BASE Architecture Overview §Security Policy, "Access control of
   access settings"); the settings are read only by those callers and the
   subject, and are absent to anyone else in AQL, an EHR Extract and the
   CONTRIBUTION read, since a restriction stays invisible to a provider (EHDS
   Art. 8). An EHR Extract also carries them for the holder of an
   `export_ehr_extract` grant covering the EHR, since the extract transfers
   the whole EHR (RM EHR Extract IM, `X_VERSIONED_EHR_ACCESS`). Who writes and
   reads them is our own design (`service::ehr::access_rule`). The EHDS
   Art. 8 restriction and Art. 10 opt-out are `restriction` and `opt_out` directives of these settings,
   committed by the controller over `POST /admin/ehr-access` (which names the
   EHR in its body, so it reaches an EHR a restriction hides) or by a
   gate-keeper. Every `EHR_ACCESS` commit, import, archive load and repair
   projects the directives in force, in its own transaction, onto the marks
   `ehr.access_restricted_at`, `vo_head.access_restricted_at` and
   `ehr.opted_out_at`, which the version reads, AQL, the exports and the write
   guard filter on, and each boot projects every stored `EHR_ACCESS` again
   (`service::ehr::access_marks`). The projection never reads a directive more
   permissively than it is written: a data scope beyond objects restricts the
   whole EHR, a directive holds until its validity ends, and one that no
   emergency may lift is refused `400` until the evaluator reads
   `emergency_override`. The projection and its rules are our own design;
   no specification governs them.
3. **RBAC** — the coarse operation class (public / clinical / admin /
   management) against the caller's roles, plus the read-only restriction, which
   overrides any grant. Roles come from the RFC 9068 §2.2.3.1 claim carriers; an
   OAuth2 scope is NOT a role (RFC 6749 §3.3).
4. **ABAC** — off by default. The embedded Cedar engine decides over subject attributes (subject, roles, scopes, organization,
   patient) and resource attributes (patient, template), fanned out over
   multi-valued attributes as a cartesian product with **all-must-permit** and
   short-circuit deny.
5. **SMART scopes** — off by default, AND-composed onto the ABAC decision, so it
   can only narrow. Disabled SMART produces zero wire drift.

**Deny-by-default and fail-closed, in both senses.** No stage permits by
omission: a gate reached without a principal refuses; Cedar is
deny-by-default with `forbid` overriding `permit`. And a stage that cannot
DECIDE is never read as a decision — an unreachable token issuer is `503`, a
Cedar policy that errors during evaluation is a fail-closed `500`. FerroEHR
calls no remote policy decision point: the owner removed it on 2026-11-13
(#5487, #5516), since it is a second policy world over HTTP that receives none
of the openEHR access facts, and ITS-REST 1.1.0 mandates no authorization
scheme (`Requests_and_responses.md` §Authentication and authorization). Silence is never consent, and a broken control never looks
like a policy outcome.

Configuration is boot-validated rather than degraded at the first request: a
mandatory audience list, an `https` issuer, an algorithm set bound to its key
source, an HMAC entropy floor, the OWASP Argon2id parameter floor, and a policy
rule per resource kind the PEP consults.

### Operational policy as demographic data

Who may do what is held in the party store, not in `ferroehr.toml`, so a
change takes effect without a restart and every change is a version with its
committer (#5433). The classes are the openEHR RM demographic package
(`docs/specs/openehr/RM/docs/demographic/master02-demographic_package.adoc`
and the class pages under `RM/docs/UML/classes/org.openehr.rm.demographic.*`);
reading them as policy is our own design, and each own-design point below
is labelled.

**What stays in the configuration.** What the server needs before it can
reach its store or trust anything: listeners, database, TLS, telemetry,
audit sinks, signing, the issuers with their keys and trust bounds,
`[auth] product_scope`, `[authz.rbac] admin_role` with `role_issuers` (the
authority that writes policy parties), the party mapping (digest-locked),
the `provider`, the security floors (assurance, `ehr_access_default`, the
cohort thresholds), break-glass (`emergency_access_role`,
`emergency_purpose_codes`) and the Basic users' password hashes. A key that
moves into the party store is removed and refused as an unknown key, like
any other. The policy parties add no configuration key, required or
optional. No openEHR spec governs configuration: our own design.

**The model.**

- *The actor* is an `AGENT` for a system ("any kind of agent, including
  devices, software systems", `agent.adoc`), an `ORGANISATION` for a
  provider or a federation partner, or a `PERSON`. It is identified by a
  `DV_IDENTIFIER` in `PARTY.details` (§Party Identification: identifiers
  given by organisations "are treated as any other attribute of a Party");
  a system's `AGENT` carries its OAuth 2.0 `client_id` under the token
  issuer.
- *What it may do* is a `ROLE` it performs (`role.adoc`: `performer`,
  `time_validity`, "Roles should have credentials qualifying the performer
  to perform the role") whose `CAPABILITY.credentials` hold the grant, with
  the capability's own `time_validity` (`capability.adoc`). **Our own
  design:** the RM defines `credentials` as "the qualifications of the
  performer of the role for this capability", and CKM's
  `openEHR-DEMOGRAPHIC-CAPABILITY.individual_credentials.v0` records a
  person's registrations and qualifications, which is how the default
  professional `ROLE` uses it. Reading a capability as an authorization
  grant is ours, and so is the capability archetype of each policy `ROLE`.
- *Bounds finer than a day* go in the credentials as `DV_DATE_TIME`
  elements, because `time_validity` is `DV_INTERVAL<DV_DATE>` on `ROLE`,
  `CAPABILITY` and `PARTY_RELATIONSHIP` (the RM 1.1.0 BMM and the 1.2.0
  text agree). **Our own design.**
- *On whose behalf* is an acts-for `PARTY_RELATIONSHIP` from the system's
  `AGENT` to the provider's `ORGANISATION`, stored in the `AGENT`, its
  source (§Party Relationships: "stored as part of the data of the PARTY
  designated as the source"), so it changes through the `AGENT`'s update.
- *Management* is the standard ITS-REST 1.1.0 Demographic API
  (`docs/specs/openehr/ITS-REST/specifications/demographic.openapi.yaml`,
  `x-status: DEVELOPMENT`): the party routes, `POST
  /demographic/contribution`, and `versioned_party` with its
  `revision_history` as the grant history. No endpoint of our own. The
  container ids are minted at commit, so a `ROLE`'s `performer` names an
  `AGENT` committed before it.
- *Audit:* every change carries `AUDIT_DETAILS.committer`
  (`org.openehr.rm.common.audit_details.adoc`) and is an access record of the
  Demographic API, which serves the EHDS access record of Regulation (EU)
  2025/327 Annex II 3.2 (`docs/law/eu/ehds/text.html`), and Annex II 3.4's
  access rights that "take into account the origins and categories" are what
  the grants express.

**Recognised by coded content, seeded as examples.** Definitions are
versioned and a deployment models its own parties from any archetype, so
FerroEHR never recognises a policy party by archetype id (owner decision on
#5433, 2026-11-12). A version is a policy party when it holds a `CAPABILITY`
whose `credentials` hold a `DV_CODED_TEXT` of the operation terminology
`ferroehr-operations`, a `PARTY_RELATIONSHIP` whose `name` or `details`
hold `ferroehr-operations::acts_for`, or a `DV_IDENTIFIER` of type
`client_id` (`party_binding::policy_mark::policy_content`, the RM's
`ITEM_STRUCTURE` with a terminology binding). The shipped archetypes
`openEHR-DEMOGRAPHIC-AGENT.system.v1`,
`openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.acts_for.v1` and
`openEHR-DEMOGRAPHIC-ROLE.{system_client,federation_partner,privileged}.v1`
are examples that carry those codes; `PolicyKind::of` names them only so a
party of one is validated against its own template. The binary carries
a default template of each (`party_binding::policy::DEFINITIONS`, from `app/ferroehr/assets/definitions/policy/`); each boot stores one
through the normal definition path only when no definition of its id is
stored, never overwriting, never refusing a deployment's change and never
re-seeding over it (`seed_default_definitions`). After that they are ordinary
definitions, versioned and replaced through the definition API, and every
write of a policy party is validated against its own template. The
recognition runs with or without a party mapping and enters no mapping
digest: the grant rows it derives do not depend on the mapping. **Our own
design.**

**The access control terminology**
(`openehr_rm::draft::access_control::terminology`, #5507, beside the draft
it codes since #5540)
adds the groups the ABAC design of #5487 reads to `ferroehr-operations`:
`access default`, `access directive kind`, `access effect`, `access
relationship`, `access caller kind`, `access operation`, `access
sensitivity clearance`, `access permitted purpose`, `care relationship` and
`access refusal`, each a closed set of rubrics read into a typed term
(`Term::read`), with a value of another terminology or a code outside its
group refused. The operations gain the grants `write_care_relationship` and
`write_access_policy`, and the clearance, purpose and care relationship codes
are qualifiers, never operations. Sensitivity, category, purpose and legal
ground stay in the terminology the jurisdiction names and are compared by
terminology id and code (`ForeignCode`). The draft invariants of
`access_control_invariants` read their groups through it, and the evaluator
and the `EHR_ACCESS` and `ACCESS_POLICY` write checks read no other code. RM
EHR IM `ACCESS_CONTROL_SETTINGS` is "Currently implementation dependent" and
openEHR names no access control terminology: **our own design.**

The default demographic definitions of `deploy/definitions/` (the
professional `PERSON` and `ROLE`, the care-team `GROUP`, the membership and
representation `PARTY_RELATIONSHIP`s, the patient `PERSON` and the provider
`ORGANISATION`; the representation is an example, recognised by its code) are
carried and seeded the same way
(`party_binding::shipped::DEFINITIONS`), and each `[authz.party_binding]`
mapping section defaults to them (the `Default` of `PartyBindingConfig`), so
an empty configuration boots with SCIM Users, Groups and representation. A
section a deployment sets replaces that section's default whole, and nothing
is seeded under a deployment's own ids. A SCIM client whose `[[auth.party]]`
table names no `group_identifier_issuer` writes its Groups' `externalId`s
under its `binding_issuer` with `/groups` appended
(`config::scim::group_issuer`). **Our own design.**

**Who writes and reads a policy party.** A party is a policy party once a
version of it holds the coded content above, inline or on our own
relationship routes. Only a caller holding `admin_role` creates, updates or
deletes one, on its own route or inside a demographic `CONTRIBUTION`; any
other caller, a SCIM client included, is `403` naming what makes it one.
`party_binding::protect` judges the new version and the party's mark in the
commit's transaction, so a policy party is never planted, changed, deleted
or turned into an ordinary one by anyone else. Reads are an administrator's too: the Demographic API reads of a
party or relationship any of whose versions is a policy party are `403` to
any other caller, and demographic AQL by such a caller leaves out every
version of it (#5450, #5485). Both read one mark: `vo_head.policy_party_at`
(party migration `0034_policy_party_mark.sql`), which the commit of a policy
version sets in its own transaction (`protect::check`, from the body the
commit holds), which nothing clears, and which an import, an archive load
and `party-binding refill` set from the stored bodies
(`policy_mark::mark_stored`, by the SQL/JSON path `POLICY_PATH`, which a test
runs against `policy_content` over one set of bodies). `protect::check_read` reads it by the head's primary key,
and AQL adds it to the head join every versioned-object source makes
(`aql::sql::gates`), so counts and paging see only what the caller may read
and no extra statement runs.
**Our own design:** the RM's
`ACCESS_CONTROL_SETTINGS` is "Currently implementation dependent" and the
Demographic API defines no access rule for its operations.

**The grant rows and the read path** (`party_binding::grants`, #5435). By
the owner's correction on #5433, FerroEHR reads permission from coded
content and never recognises a grant by archetype or template id, so a
deployment models its parties from any archetypes. The contract is a
terminology of our own, `ferroehr-operations`
(`party_binding::grants::operation`): a `CAPABILITY` grants an operation when
its `credentials` hold, at any depth, an element whose value is a
`DV_CODED_TEXT` of that terminology with an operation code
(`commit_composition`, `import_ehr`, `import_ehr_extract`,
`export_ehr_extract`, `federated_person_access`,
`federated_organisation_access`, `read_subject_audit`, `access_service`,
`read_identifiers`, `data_origin`, `read_change_feed`, `scim_provision`,
`hold_roles`, `hold_scopes`, `create_at_login`, `query_system_wide`,
`access_demographic`); elements named by the
qualifier codes `not_before` and `not_after` bound it finer than a day,
`template`, `facility` (an element of a `DV_IDENTIFIER`, or a cluster of
`issuer` with `identifier` or `identifier_prefix`), `subject_namespace`,
`creating_system`, `origin`, `reader`, `binding_issuer` and `token_scope`
scope it, `origin` (`relay` or `device`) and `device_identifier_issuer`
qualify a `commit_composition` grant, and `role` and `scope` name what a
`hold_roles` or `hold_scopes` grant gives; `acts_for` codes the acts-for
relationship.
That is the RM's `ITEM_STRUCTURE` with a terminology binding, which any
modeller produces (RM `data_types` `DV_CODED_TEXT`, `CODE_PHRASE`). A client
is any `ACTOR` carrying a `DV_IDENTIFIER` of type `client_id` in its
`details` or `identities`, whose issuer is the token issuer (§Party
Identification). **Our own design.**

The binding side table gains two relations (migration `party/0035`):
`binding_client`, one row per client identifier of an `ACTOR` head that is
not deleted, unique per `(issuer, client_id)`, so a second party naming a
client is `409` at its commit; and `binding_capability`, one row per
operation a `CAPABILITY` of a `ROLE` head grants, with its performer, the
`ROLE`'s and the `CAPABILITY`'s `time_validity` as date ranges (absent is
every date, a bound that does not read is none), the `not_before` and
`not_after` instants (a `DV_DATE_TIME` without an offset reads in UTC, one
that does not read makes the capability hold on no date), and the
credentials as written. They are written in each party commit's own
transaction from the body the commit holds, replaced per party, and rebuilt
from the versions by `ferroehr party-binding refill`; a boot fills them once
if no change has moved their revision yet. A grant is in force for a request
when the client and the `ROLE` heads are `complete` or `incomplete`, both
date ranges hold the request's date in UTC (the zone every `DV_DATE` is read
in), the instants hold the request's instant, and the ceiling admits the
operation. The `ROLE`'s `performer` alone ties a grant to its party: a
demographic `CONTRIBUTION` cannot carry an `AGENT` and the `ROLE` that names
it (the ids are minted at commit), so the performer's `roles` list is not
required. **Our own design.**

The request path adds no transaction: each replica keeps the rows in memory
(`LiveGrants`) after the pattern of the admin registers below, listening on
`ferroehr_policy_grants`. A commit that changes a grant row moves the
revision row `binding_grant_revision` and notifies with it in its own
transaction; a notification marks the copy stale, and the next request reads
the revision, one indexed read, and the rows only when it moved. The request
whose own commit changed a grant reads the copy again before it answers, so
the caller's next request on that replica runs under the change; another
replica does from its first request after the notification. While the
listener is not connected every request that resolves a caller reads the
revision. A request resolves its caller after authentication: a system
client (`CallerKind::System`) by its token's issuer and `client_id`, a
caller the login binding bound by its party, anyone else to no grants and no
read; a copy that must be read and cannot be is `503`. The caller is
resolved before any gate runs, and a check reads `Capabilities::current()`,
which holds the caller's grants for the duration of the request, each an
`Operation` with its `ROLE` and credentials read by the codes of their
elements' names. A per-request read without a cache,
which the program first named, would add a transaction to every request,
which the performance gate's transactions-per-request measure refuses
(#5459).

**Caller facts** (#5511). Beside the grants, the side table holds what the
access evaluator reads about a caller (migration `party/0039`):
`binding_caller_fact`, one row per sensitivity clearance and permitted
purpose a `CAPABILITY` carries (an element named `sensitivity_clearance` or
`permitted_purpose` of `ferroehr-operations` whose value is a
`DV_CODED_TEXT` of the jurisdiction's terminology, read with
`ForeignCode::read`), and per care relationship a `PERSON` holds (an inline
`PARTY_RELATIONSHIP` whose `name` or `details` carry `member_of` or
`under_care_of`, RM demographic `party_relationship.adoc`: `source`,
`target`, `time_validity`), each with the term of its holder. The patient's
`under_care_of` team ids are projected at the commit of the patient's
`PERSON`. A commit whose facts changed moves `facts_revision` beside the
grant revision and stamps every party whose facts changed in
`binding_caller_fact_party`; a replica whose grant revision is current reads
only the facts of the parties stamped since its copy, one indexed read
(`grants::caller::store::FACTS`), so an admission costs each replica one
small read and the request path still adds no transaction. A refill and an
erasure move the grant revision, which reads every fact again.
`Capabilities::facts()` gives a request's caller its clearances, purposes
and care teams in force; `LiveGrants::under_care_of` gives a subject's care
relationships with their dates, so the evaluator applies the grace after a
discharge. A `PERSON` version that changes its care relationships commits
only for a caller holding `write_care_relationship` or an administrator
(BASE Architecture Overview `master07-security.adoc` §The Problem of Roles:
the care delivery environment evaluates "care deliverer"). A care
relationship of another party kind or of its own container is refused
`422`. **Our own design:** no openEHR spec gives a capability or a
relationship an access meaning.

**The client lists are grants** (#5437). A backend client, whose
`system/aql-*` scope "would grant access to all registered and ad-hoc AQL
queries system-wide" (SMART on openEHR `master08-scopes.adoc` §Resource
Scopes), is a client whose `ACTOR` holds `query_system_wide`; a client that
reaches the DEMOGRAPHIC API, which no SMART on openEHR resource family
covers, holds `access_demographic`. The middleware resolves the caller's
grants before the RBAC and SMART gates and reads both from them
(`authn::grants::client_holds`), so `[auth.smart] backend_clients` and
`demographic_clients` are gone. A grant counts only for a caller resolved as
a client, the app-only token of a `machines` issuer; a person signed in
through that client holds the person's grants. `[auth.smart]
patient_issuers` stays in the file: it is trust, the issuers whose `ehrId`
claim names an EHR of this platform (master07 §Context Selection), which the
server needs before it believes a `patient/` token. The person-token client
lists are answered the same way inside `ferrohealth-smart`'s token
classification, through the crate's `kind::standing::Standing` seam:
`party_binding::grants::standing::GrantStanding` reads the grants of the
`ACTOR` carrying the client identifier under the token's issuer from the
replica's `LiveGrants` copy (no transaction), so a client granted
`sign_in_persons` is first party, one granted `act_for_persons` a delegate
(and an RFC 8693 actor by its `sub` the same way), and a system caller granted
`access_without_scope` meets every SMART on openEHR requirement without a
resource scope. `[auth]` is read as the crate's `ProductStandingSection`, so
`first_party_clients`, `delegates` and `scopeless_subjects` are unknown keys.
A Basic user is a credential alone: its roles and scopes are the `hold_roles`
and `hold_scopes` grants of its `PERSON`, granted onto the caller with
`Caller::with_granted_scope` and `with_granted_roles`, and `[authz.rbac]
first_admin` names the one Basic user holding `admin_role` and the product
scope from the file, the trust anchor that writes the first policy parties.
**Our own design.**

**The grant ceiling.** `[authz.grants] allowed` bounds the operations the
party store may grant, so a compromised administrator account cannot grant
more than the operator allows. A `ROLE` commit with a capability coding an
operation the ceiling does not admit, or a code of the terminology that
names no operation, is `403` naming the code and the key; a stored one, such
as one committed before the ceiling was narrowed, is never honoured and is
logged when the rows are read. The default admits `commit_composition`
alone, which reaches only the templates the grant names: an import, an
export, a federation agreement and every privilege reach beyond one system's
own writes, so the operator admits each by name. It is the one key this
program adds. **Our own design.**

**Trusted import systems, origins, change-feed readers and SCIM clients**
(#5439) are grants read per request through `Capabilities::current` or, for a
store-wide reading, `LiveGrants::all_in_force`; their TOML keys
(`[authz.import] trusted_systems`, `[[authz.origin]]`, `[record.change_feed]
readers`, `[[auth.scim.client]]`) are gone. A wrapped committer of an import
may name a local party only when the version's `creating_system_id` is a
`creating_system` element of the importer's grant of that import operation,
never this node's own (`service::message::import_policy`); the RM keeps an
imported audit as received (`IMPORTED_VERSION`), so the trust is the
importer's. An origin is the `origin` element of a `data_origin` grant: the
registered origins a retention period may name are those in force
(`origins::registered`), and a direct commit is stamped with its committer's
one origin, a committer with two refused for a body with no `FEEDER_AUDIT`
(`origins::DirectOrigin`), which serves EHDS Annex II 3.4. A change-feed
reader is the `reader` element of a `read_change_feed` grant: the binary's
`OutboxGate::WhileGranted` writes the outbox row while one is in force, and
the registry is reconciled with the grants at boot, on every feed call and
before every prune; only a caller whose own grant names a reader reads it. A
SCIM client is a client `ACTOR` holding `scim_provision`, whose
`binding_issuer` is an `[[auth.party]]` identifier issuer no other client's
grant names and whose `token_scope` its token holds; the contact erasure
reaches the binding issuers of every stored grant, ended ones included, so
revoking a client stops no GDPR Art. 17 erasure. The `ROLE` commit refuses a
capability that names `origin`, `reader` or `binding_issuer` other than once
(`grants::check_role`). Each replica's refresher reads the grant copy on
every notification (`LiveGrants::spawn_refresher`), so a commit of a request
that resolves no caller still sees the change-feed grants in force, with no
read on its path. **Our own design:** no openEHR spec defines a change feed,
SCIM, or an authorization meaning for these.

**The first grant is the operator's.** An administrator is a person bound to
a `PERSON`, and a new deployment's `PERSON`s arrive through a SCIM client
that needs a grant, so `ferroehr party-binding grant`
(`party_binding::grants::bootstrap`) commits the client's `AGENT` and a
`ROLE` granting one operation offline, through `party_create` with the
server's own committer, under the ceiling and the element rules. It is
idempotent per client and operation. **Our own design.**

**Basic users' roles and scopes, and the privileged grants** (#5438). A
Basic user bound to a `PERSON` holds, beside its `[[auth.basic.users]]`
table, the roles and SMART scopes its `hold_roles` and `hold_scopes` grants
name (`ferroehr-rest` `authn::grants::basic`), which every gate then reads.
A token's roles and scopes stay the token's: its issuer is their authority,
and a SMART scope is what the user consented to (SMART App Launch 2.2.0,
Scopes and Launch Context). The privileges beyond clinical access are
grants and nothing else (#5521): `import_ehr` and `import_ehr_extract` admit
the matching import, `read_identifiers` the protected-identifier read,
`read_subject_audit` the ITI-81 read of one subject's access log and
`access_service` an EHDS Art. 10 access service, each read from the
request's `Capabilities` with no transaction added; `[authz.rbac]` names no
role for any of them, and the four keys that did are unknown keys. The
privileged roles of `[authz.rbac]` (`admin_role`, `emergency_access_role`)
are reserved: a `hold_roles` grant naming one is
`403` at commit (`GrantCeiling::with_reserved_roles`) and never honoured, so
the administrator authority and break-glass never come from the party
store. The keys `[[auth.basic.users]] roles` and `scopes` belong to
`ferrohealth-smart`, so they leave the schema with that crate. **Our own
design.**

**Just-in-time admission** (`party_binding::grants::admission`, #5441). Which
first logins of a `persons` issuer create a provisional `PERSON` is policy,
so it is CDR data: a `CAPABILITY` coding `create_at_login` admits the
logins of each issuer its `login_issuer` elements name that meet every
condition it declares (`admit_idp`, `admit_group`, `admit_role`,
`admit_assurance`, and `admit_guests` for a guest), and any grant in force
admits a login. The grant is read by its credentials, whoever performs its
`ROLE`, from the same `LiveGrants` copy (`LiveGrants::admissions`), which
the login binding consults only for an unbound person or delegated caller,
so the request path adds no transaction. A grant that names no issuer or
declares no condition is refused at commit with `422`
(`admission::check`) and a stored one is never honoured, so no grant admits
every login of an issuer; `create_at_login` is above the default ceiling.
The `[[authz.party_binding.issuer]]` table keeps only how the tokens are
read (`user_name_claim`, `idp_claim`), and the six admission keys it held
are gone. **Our own design:** the RM (Incomplete Content) allows an
incomplete demographic version, and no specification governs when a login
creates one.

**Federation agreements are grants on the partner** (#5440). An agreement
is a capability coding `federated_person_access` or
`federated_organisation_access` in a `ROLE` the partner's `ACTOR` performs;
that `ACTOR` carries a `client_id` identifier per client of its `federation`
issuer, so a verified external caller resolves to its partner as a system
client does, and the agreement is one of its grants, read per request from
the in-memory copy. Its terms are credentials elements named by codes of
the same terminology (`agreement_name` to `requester_log_retention`), whose
values are never operations, and its validity is the grant's. The rules the
configuration loader once applied at boot run at the `ROLE`'s commit
(`party_binding::grants::check_agreements`, `422` naming each rule), judged
against `ferroehr::config::federation::trust`, what `[auth]` trusts of each
`federation` issuer and `[audit.store]`'s horizon: the partner's issuers are
`federation` ones, one agreement per partner and accessor, a caller rate and
`patient/` reads on every agreement, a contact point's `ehdsi_service`, and
the EHDS Art. 13(4) and Annex II 3.2(b) rules of an organisation and of a
`chained` access. The admission judges the agreement again for the caller's
own issuer, so one that no longer keeps them, after its partner changed,
admits nobody. No rule is a country's: the jurisdiction is the agreement's
element and the consent source is `registry` or `local`. `/management/info`
reads the agreements in force from the copy on each request. **Our own
design:** no specification gives a capability an authorization meaning or
governs a data-sharing agreement's form.

**System clients from the CDR** (#5436, #5432). The
`[[authz.system_client]]` table is gone: a system caller
(`CallerKind::System`) is what the policy parties say. Its commit into an EHR
(`versioning::system`) needs a `commit_composition` grant, or it is `403`, and a
`COMPOSITION` is admitted when one grant lists its template, has the origin
the act needs (a device's registration under `device`, recognised by a
composer identifier of a `device_identifier_issuer` equal to
`feeder_audit.originating_system_audit.system_id`; any other act under
`relay`, naming the professional) and covers the facility it names (EHDS
Art. 13(4)). A `ROLE` whose grant lacks the terms its operation is checked by
is `422` at its commit (`party_binding::grants::check_role`), the boot
refusals the table had. The import grant of #5438 is held to its scope:
`message_import_ehr` needs an `import_ehr` grant whose `subject_namespace` names the subject namespace of the
`EHR_STATUS` the extract carries, and `message_import_ehr_extract` one of
`import_ehr_extract` naming that of the target EHR and of any `EHR_STATUS`
the extract carries; a system caller no longer imports without a grant; a system caller's
export needs an `export_ehr_extract` grant covering the EHR the same way
(`service::message::import_policy`). A grant names its namespaces, so there
is no grant of every EHR. An EHR whose `EHR_STATUS.subject` has no
`external_ref`, which RM EHR IM allows for an anonymous record, is reached
only by a `subject_namespace` element holding the coded
`ferroehr-operations::no_subject_namespace`, which covers those EHRs and no
other (`Capabilities::covers`, #5530). FerroEHR's own extracts rewrite every reference
namespace to `local` (RM `ehr_extract` `master09-semantics.adoc` §Creation
Semantics), so a grant over them names `local`. The commit of a system
caller is attributed to its `ACTOR`: the committer's `external_ref` is a
`PARTY_REF` of type `ACTOR` (BASE `base_types` `PARTY_REF`
`Type_validity`), and its access record names the `ACTOR` as its party and
the provider by the first `DV_IDENTIFIER` in the `details` of the
`ORGANISATION` its acts-for relationship targets, or, with none, the
deployment's `[authz.party_binding] provider` (EHDS Annex II 3.2(a)). The
target and that identifier are side-table rows too (`binding_client.provider_id`,
`binding_provider`, migration `party/0036`), written in the commit of the
`AGENT` and of the `ORGANISATION`, so the request path still adds no
transaction. There is no data migration role: a migration is an `AGENT` whose
`ROLE` holds the import and export operations. **Our own design.**

**Representatives in the patient portal (#5446).** A representation is a
`PARTY_RELATIONSHIP` from the representative's `PERSON` (its source) to the
subject's (its target) whose `name` or `details` carry
`ferroehr-operations::represents`, in any archetype, recognised by the code
as the acts-for relationship is (§Party Relationships: a relationship lives in
its source party). It names its openEHR `subject_relationship` code itself
and makes no policy party. Besides the inline relationships of the
representative's `PERSON`, a `PARTY_RELATIONSHIP` of its own is one: the
patient writes it to authorise a proxy, and the party protection admits a
patient's write only of a representation whose target is the subject of
their launch EHR, before and after, changing only one they inserted
themself (the EHDS Art. 5 rule, its committer naming their `PERSON`). The
side table `binding_representation` holds both, the standalone rows keyed by
their container (party migration `0037`). A `patients` token's representative,
the RFC 8693 `act` of the patient's token or a login of their own naming the
patient's EHR, is bound by FerroEHR itself after authentication
(`service::demographic::portal`): the login under the token's issuer is the
binding key of a representative's `PERSON`, which must hold a live
representation of the EHR's subject; the request is then confined to that
EHR, recorded bound to the representative naming the subject, and committed
as a `PARTY_RELATED`. `ferrohealth-smart` classifies the `act` path as
`CallerKind::PatientRepresentative` and binds no party to a `patients` caller,
so the binding is FerroEHR's. Who may represent whom and until when, the age
and legal-capacity rules of national law (EHDS Art. 4(2): "in accordance
with national law"), reach FerroEHR as the representations and the
`time_validity` the deployment records; FerroEHR holds no rule of its own.
**Our own design**, as no openEHR spec governs resolving a login to a party
or who may write a relationship.

**Break-glass stays in the configuration.** Emergency access (EHDS Art.
11(5)) must work when the party store does not answer, so
`emergency_access_role` and `emergency_purpose_codes` stay TOML. **Our own
design:** no specification governs break-glass when the policy store is
down.

**The default definitions** sit under `app/ferroehr/assets/definitions/policy/`
with a `PROVENANCE.md`: the `AGENT` and the acts-for relationship adapt CKM's
`ORGANISATION.organisation.v0` (CKM publishes no `AGENT` or
`PARTY_RELATIONSHIP` archetype), the system-client and privileged
`ROLE`s adapt `ROLE.individual_provider.v0`, the federation-partner `ROLE`
adapts `ROLE.healthcare_provider_organisation.v0` (its performer is an
organisation), and each grant `CAPABILITY` is ours. The provider and the
partner are `ORGANISATION`s of `deploy/definitions/provider/` (CKM
`ORGANISATION.organisation.v0` with its name, provider identifier and
address archetypes), identified by a mandatory provider identifier. The
professional `ROLE` keeps CKM's meaning of a capability: its `CAPABILITY` is
`CAPABILITY.individual_credentials.v0`, a clinician's registrations and
qualifications, carries no grant kind, and is never read as a grant, since
only the policy archetypes are. Every CKM source is a v0 draft, so each
`derived_from` names its CKM citeable id and modification time. The
deployment-modelled definitions (professional, care team, representation,
patient, provider) stay under `deploy/definitions/`, load wherever the
default set loads and ride every release as assets.

### The operational-policy admin registers (`ferroehr::policy_register`) — our own design

No openEHR specification governs operational policy or where a CDR keeps it:
our own design (#5442). Three kinds of policy change while the CDR runs and have
no home in the RM demographic model (no `PARTY`, `ROLE` or `CAPABILITY` carries
a category map, a Member State's access delay or a Cedar policy set), so each
is an **admin register**: `audit-categories` (the EHDS Annex II 3.2(c) category
map), `access-delay` (the EHDS Art. 3(3) delay) and `cedar-policies` (the
embedded Cedar engine's policy set). The TOML keys they replace
(`[audit.categories]`, `[privacy.access_delay]`, `[authz.abac.cedar]`) are
removed outright and refused as unknown keys.

- **Storage.** One append-only clinical table, `policy_register (register,
  version, document jsonb, committer, committed_at)`. A register with no row
  holds its default (version 0): the shipped category map, the delay off, an
  empty Cedar set, which permits nothing.
- **Write.** `PUT {ext}/admin/registers/{register}` replaces the whole document
  over the version `If-Match` names; under a per-register advisory lock the
  write appends the next version only while that version is still the newest,
  else `412`. The document is checked before it is stored and refused whole
  with `400`: a category map must compile (and must classify something under
  the production `no_category_map` floor, which stays TOML because it must
  hold before the register is readable); the access delay's limit and
  categories must hold under the category map in force; a Cedar set must parse
  and validate strictly against the schema built from the ABAC request model.
  A category write mirrors its map into the retention register's
  `category_map` in the same transaction. The committer is the request's
  authenticated subject; the access record of the request is the audit entry.
- **Read path, with no database work per request.** Each replica keeps the
  compiled registers in memory and listens on the PostgreSQL channel
  `ferroehr_policy_register`; a register write runs `pg_notify` in its own
  transaction, which PostgreSQL delivers to every listener when it commits
  (PostgreSQL 18 `NOTIFY`). A notification marks the registers stale, and the
  next operation that consults a register (all but the admin group's
  maintenance routes) reads the newest version of each, one `GROUP BY` over
  the primary key, and installs what changed: the classifier goes to the audit
  sender, which stamps every record with its map digest; the access-delay
  policy is recompiled against the map in force; the Cedar engine rebuilds its
  set on its next decision. With nothing stale a request adds no transaction,
  which the performance gate's transactions-per-request measure holds. The
  writing replica reads the registers before it answers, so its next request
  runs under the change; another replica runs under it from the first request
  after the notification reaches it. While a listener is not connected (at
  boot, or after it lost its connection, until it connects again and marks
  the registers stale) that replica reads the registers on every request, so a
  lost notification never serves an outdated policy; a store that cannot
  answer the read is `503`. Two alternatives were weighed: reading the
  versions inside a transaction each request already opens would thread the
  read through every handler's own transaction, and a per-request read of its
  own costs one transaction per request (measured: 6.5 to 7.5). A stored
  version that stops compiling under a later release leaves the document in
  force and is logged, so a code change never empties a policy.
- **Audit records under a changing map.** The audit store keeps one render
  policy per category map and writes each record under the policy of the map
  its digest names, so a record emitted under one map and written after a
  change still renders with the categories it was classified into.

### The controller's access policy (`ferroehr::access_policy`) — our own design

`ACCESS_POLICY` is a class of FerroEHR's draft access control package
(`tools/openehr-codegen/draft/bmm/openehr_access_control_0.1.0.bmm.json`, no
openEHR release), and ITS-REST defines no resource for it: the store, the
resource and the in-memory copy are our own design (#5512). It holds the facts
that hold for every EHR the controller keeps: the sensitivity of kinds of data
(`classifications`), whether clinical access needs a care relationship
(`care_relationship_required`, `relationship_grace`) and a declared purpose
(`purpose_required`), and the `catalogue` of named directives a subject accepts
by reference (IHE BPPC). BASE Architecture Overview §Security Policy leaves
privacy levels to "standards or agreements within jurisdictions of use", which
the classifications carry. Break-glass and the default floor stay in TOML; the
policy can only tighten them.

- **One versioned object.** The instance is single-tenant, so it keeps one
  `VERSIONED_ACCESS_POLICY`, outside every EHR and outside the demographic
  service. Each write commits one `ORIGINAL_VERSION` on its trunk in its own
  `CONTRIBUTION` with the request committer's `AUDIT_DETAILS` (RM Common IM
  master06 §Contributions, §Committal and Audits), as one row of the
  append-only clinical table `access_policy_version` (migration
  `clinical/0038`). The `ORIGINAL_VERSION` is rendered by the same builder as
  every other version. Its `owner_id` is the controller of the first version,
  and a later version naming another controller is refused. Our own choice:
  the policy rows stay out of the `version`/`node` spine, so no AQL, EHR
  extract or EHR-scoped contribution read reaches them.
- **Strict on write.** A body is read by the strict canonical-JSON reader of
  the generated draft type, then checked against the invariants of the draft
  BMM (`openehr_rm::draft::access_control::invariants::check_policy`), the root `LOCATABLE` rules
  (the root `archetype_node_id` is the archetype id `archetype_details` names,
  of an `ACCESS_POLICY` archetype) and the controller's type (`ORGANISATION`).
  A body that does not hold is refused whole with `400`.
- **Resource.** `GET {ext}/admin/access_policy` (weak `ETag` the version uid,
  `Last-Modified` its commit time), `PUT` over the version `If-Match` names or
  creating the first with `If-None-Match: *` (RFC 9110 §13.1), `412` when the
  precondition fails, `GET …/history` (every `ORIGINAL_VERSION`, newest first)
  and `GET …/versioned`. Admin role; the write also needs the
  `write_access_policy` grant.
- **Default archetype.** `openEHR-EHR-ACCESS_POLICY.controller.v1` is seeded
  at boot like the policy definitions. Archetypes are validated against the RM
  extended by the draft classes (`ferroehr::draft_model`, over the class
  tables `emit-drafts` writes into `openehr_rm::draft` and `openehr_adl::validate::rm_extended::ExtendedRmModel`,
  AOM2 master11 RM adaptation), so an archetype over a draft class passes
  VCORM. With no version committed nothing is tightened.
- **In memory, invalidated by `LISTEN`/`NOTIFY`.** As the registers above:
  each replica holds the newest version and listens on
  `ferroehr_access_policy`; a write notifies in its transaction, a
  notification marks the copy stale and the next operation outside the admin
  group reads the head (one indexed read) and installs a new version. With
  nothing stale a request touches no database; while the listener is down
  every request reads; a store that cannot answer is `503`.

## Templates, validation, FLAT

OPT 1.4 XML ingestion → `openehr-am`; WebTemplate builder (`moka`-cached);
composition validation (walker over WebTemplate + RM invariants + terminology
binding via `openehr-term`); FLAT/STRUCTURED/Web-Template JSON in
`openehr_sdt::flat`, whose only authority is the vendored Simplified Formats
spec text — there is no vendor-quirk mode and no feature flag; vendor
implementations are prior art only. Simplified Formats is a STABLE ITS-REST
1.1.0 sub-specification with no machine-readable model, so it is hand-written
in `openehr-sdt`, beside the template-independent RM-instance validation
(`openehr_sdt::rm_instance`), over the `openehr-its` wire layer.

**A node under a template overlay carries its parent archetype's id** in the
ADL 2 Web Template: no specification governs this, it is our own design. ADL2
and AOM2 §Templates make a template overlay a template-local specialisation
that fills a slot, and its id never leaves the template. So the OPT generator
(`openehr_adl::opt::create_opt`) records each overlay's specialisation parent
in the OPT's `other_meta_data` under `template_overlay_parents`, and
`openehr-sdt`'s ADL 2 builder writes the parent's interface id on the node,
its paths and its slot admissions. A FLAT path, an AQL path and a mapping
written against the published archetype then match the template's data. The
OPT itself stays literal: it still names the overlay.

## Spec version policy

One pin per openEHR component, always the newest generation we have
vendored (`docs/VERSIONS.md` is the ladder). openEHR's own release strategy
guarantees within-major compatibility (minor releases are additive), so the
newer-generation pin accepts every valid older-minor instance — no version
negotiation and no parallel generations, with exactly one exception: **AM
ships both extant majors** (`v1_4` + `v2_4`), because the spec itself keeps
ADL 1.4 and ADL 2 side by side. A future major release triggers a
per-component decision (dual generation via the version-module codegen
pattern only if the ecosystem runs both; otherwise cutover). ITS-REST is
single-version by owner ruling: the CDR implements the latest released REST
API, nothing else. **ITS-XML is not a second generation either**: its 2.0.0
restructure changed the schemas' target namespace while leaving every element
name and `xsi:type` spelled identically, so one generated codec serves both
published lineages and the wire namespace is a serialize-time choice — v2 by default
(owner ruling 2026-08-03, issue #1666: only the v2 bundle's schemas model the
RM 1.2.0 the server emits; the conformance instrument leaves the lineage open
as its `xml-namespace` option), v1 when a request selects it
with the `version` media-type parameter on `application/xml` (our own
extension; no openEHR spec governs namespace selection on the REST wire). The two bundles are not interchangeable as
VALIDATORS, though — the v1 one is frozen at an older RM generation, so a
correct RM 1.2.0 document can fail against it (`docs/VERSIONS.md` §Spec
version policy). Upstream spec changes and releases are detected
automatically by the scheduled watcher workflows and filed as `spec-update`
issues for triage.

## PostgreSQL 18

We target **PG 18** (18.6+). In use: `uuidv7()` for database-minted ids,
`PARTITION BY LIST (tier)` on `version`, `node` and `vo_attestation` with the
cold partition pruned at plan time, the SQL/JSON path functions
(`jsonb_path_query*`, PG 17), and the planner-side gains that need no code
(skip scan, self-join elimination, async I/O). Available
and deliberately not used, with the reason for each: `RETURNING OLD/NEW`,
`JSON_TABLE`, jsonpath item methods, `MERGE`, generated columns. The
used/available table is `docs/postgres-features.md`.

## Workspace layout

Three physical directories (consolidated 2026-07-16):
**`app/*`** holds the application — `ferroehr` (the platform **library**),
`ferroehr-rest` (the ITS-REST protocol adapter, which calls the concrete
`FerroEhrService` directly), `ferroehr-server` (the wiring-only binary; the
bin is still named `ferroehr`), `ferroehr-ext` (the feature-gated
optional-integration crate — events transport, multimedia store — one
additive cargo feature per integration, default all-on,
slim builds compile them out with loud boot refusals for enabled-but-unbuilt
integrations), and `ferroehr-viewer` (the server-rendered viewer on FerroUI, its own
binary/OCI image, consuming the CDR strictly over ITS-REST);
**`tools/*`** holds the dev/verification
tooling that is *not* part of the shipped application
(`testkit` — the shared test-database harness, and
`openehr-codegen` — the BMM/XSD/OAS → Rust generator); **`crates/*`** holds the
generated openEHR spec layer + its tooling (`openehr-*`; the one internal
arrow beyond the generated crates' own is `openehr-sdt → openehr-its`). Root
workspace `members = ["crates/*", "app/*", "tools/*"]`. Arrows:
`ferroehr-server → {ferroehr-rest, ferroehr}`, `ferroehr-rest → ferroehr`,
`ferroehr → ferroehr-ext` (optional, feature-forwarded),
`app/* → crates/openehr-*`. `ferroehr::service` is our own design: one module
per domain area, concrete methods (no trait catalog), with zero re-exports
anywhere (every import names its defining module).

### Repository scope: what lives here, and where each boundary runs

Settled on #2807 (2026-10-20, `main` at `7d8eeb828`). No openEHR spec governs
repository scope; this is our own decision. Each component that could stand on
its own has one verdict and one boundary.

| Component | Verdict | Boundary |
|---|---|---|
| The viewer (`app/ferroehr-viewer`) | stays here, as the one in-tree consumer of the CDR | the wire: ITS-REST only, never a crate dependency on `app/ferroehr` or `app/ferroehr-rest`; it draws in `ferrohealth-ui`, the component library every FerroHEALTH console shares |
| The FHIR and OMOP connectors | FerroBRIDGE, a sibling product; the in-tree FHIR connector was retired in its favour (#3702) | the wire: FerroBRIDGE reaches the CDR over ITS-REST only |
| The spec layer (`crates/openehr-*`) | published on crates.io on one lockstep line | crates.io: `openehr-its` splits into the `json`, `xml`, `opt14`, `schema-validation`, `rest`, `rest-server` and `rest-client` features, so a consumer with no HTTP server takes `default-features = false, features = ["json", "opt14"]` and no HTTP engine |
| SMART on openEHR scope handling | the grammar (`ferrohealth_smart::openehr::smart_scopes`) and checking tokens and scopes are both `ferrohealth-smart`, the family crate every product shares | crates.io: the CDR consumes both as dependencies and keeps only its route-to-requirement table (`ferroehr-rest::smart`) |

The other products of the family (FerroTERM, FerroFORMS, FerroFED and the
platform services) and the conformance instrument (Veredictum) live in their
own repositories by the same rule: a sibling reaches the CDR over ITS-REST, and
shared code travels as a published crate. The structure of the application
layer itself is #3086.

### The service layer — our own design

No openEHR specification governs how a CDR is built inside. The oracles are
what goes in and comes out: BASE, RM, AM, TERM, QUERY/AQL, ITS-REST, ITS-JSON,
ITS-XML and the serialisation formats. The openEHR Service Model (SM) text is
vendored under `docs/specs/openehr/SM/` as reference only: it describes
internal service interfaces, openEHR holds no implementation to them, and no
feature is added, kept or reported missing because SM names it.

`ferroehr::service` holds one module per domain area, each a set of concrete
`FerroEhrService` methods (there is no trait catalog) that the protocol
adapter (`ferroehr-rest`) calls directly:

| Area | Module | Wire surface it serves |
|---|---|---|
| EHR | `service::ehr` (status/composition/directory/contributions/tags/access modules) | ITS-REST EHR API |
| Definitions | `service::definition` (adl14/adl2/query_store/wire modules) | ITS-REST Definition API |
| Demographic | `service::demographic` | ITS-REST Demographic API |
| Query | `service::query` | ITS-REST Query API, AQL |
| Validity checking | `service::validity` | RM and AM invariants at commit |
| System log | `ferroehr::system_log` and `ferroehr::access_log` (the draft openEHR access log's store; dual DICOM PS3.15 + FHIR `AuditEvent`/BALP rendering, written and sent by the shared FerroAUDIT crates `ihe-atna` and `ehds-logging`; local Audit Record Repository in the `audit` schema, on by default; syslog + ITI-20 ATX:FHIR Feed forwarding sinks; the ITI-81 retrieval as the read side; ITI-19 mTLS via `[server.tls]`) | IHE ATNA, BALP |
| Admin | `service::admin` (+archive/dump-load) | ITS-REST Admin API plus our own extension routes |
| EHR index | `service::ehr_index` over the `ehr` subject columns or an IHE PIX Manager | our own extension |
| Terminology | `service::terminology` (in-process `openehr-term` bundle + one FHIR R4B terminology server, FerroTERM in every shipped deployment, reached through the published `ferrobridge-term` client with an optional OAuth2 client-credentials grant; commit-time ac-code constraint-binding resolution) | TERM, AQL terminology function |
| Message | `service::message` | `/message` extension (RM EHR Extract) |

| Crate | Role | Kind |
|---|---|---|
| `openehr-base` | BASE 1.3.0 | generated |
| `openehr-rm` | RM 1.2.0 — the domain model | generated |
| `openehr-am` | AM 1.4 + 2.4 (`v1_4`/`v2_4`) | generated |
| `openehr-term` | TERM classes + terminology bundle | generated + hand-written |
| `openehr-lang` | BMM/P_BMM object model | generated |
| `openehr-its` | Canonical JSON/XML + ITS-REST contract + OPT 1.4/AOM2 archetype XML + runtimes + wire validation + gates | generated + hand-written |
| `openehr-sdt` | Simplified Formats (`flat`: FLAT / STRUCTURED / Web Template) + RM-instance validation (`rm_instance`), over `openehr-its` | hand-written |
| `openehr-query` | AQL 1.1 lexer + parser + AST | hand-written |
| `openehr-adl` | ADL 2.4 engine: ADL2/cADL/ODIN parser, AOM2 validation, flattener, OPT2, ADL 1.4→2 conversion | hand-written |
| `openehr-codegen` | BMM/XSD/OAS → Rust generator (+ `emit-rm-model`) | tooling |
| `ferroehr-rest` | ITS-REST protocol adapter (axum) + auth + ATNA audit middleware; `access` module = RBAC/ABAC authz; calls the concrete `FerroEhrService` | application |
| `ferroehr` | The platform library: storage, service layer (one module per domain area), AQL engine, `versioning` (change control plus the VERSION `signature` signing under `versioning::signature`), validation, templates, the full config tree, telemetry, `privacy`, `system_log`, `licence` (the boot-verified grant in force and the identifier stamp it keys), `usage_report` (FerroEHR's status, route groups and PostgreSQL state store for the `start` and `daily` installation reports that `ferrohealth-usage-report` sends to FerroPULSE, on by default, envelope v2 in FerroHEALTH/FerroPULSE `docs/report-v2.md`, vendored schema in `corpus/ferropulse/`) | application |
| `ferroehr-server` | The wiring-only binary (config → pool → migrations → service → serve); bin name `ferroehr` | application |
| `ferroehr-ext` | Optional integrations behind additive features (`multimedia`): the content-addressed multimedia store | application |
| `ferroehr-viewer` | The server-rendered viewer on FerroUI: its own binary and OCI image, consuming the CDR strictly over ITS-REST | application |
| `testkit` | Shared test-database harness: one PG18 server + template-database cloning (`tools/*`) | tooling |

## Regulatory position (EHDS and CRA)

Two owner decisions of 2026-10-06 (#3606) settle who carries the
manufacturer's duties and what the EHR system consists of. Every compliance
page, release procedure and EHDS or CRA issue follows them. The texts are the
vendored acts in `docs/law/eu/` (`ehds/` is Regulation (EU) 2025/327, `cra/` is
Regulation (EU) 2024/2847).

**Cadasto B.V. is the manufacturer of each tagged release.** Each tagged
release (the binaries, the OCI images, the Helm chart and the source archive)
is one product Cadasto B.V. places on the market. This is the reading FerroFED
recorded as its decision A73 (FerroHEALTH/FerroFED#654), adopted here for the
same licence and the same holder:

- BUSL-1.1 is not a free and open-source licence under CRA Art 3(48): it does
  not grant every right to use, modify and redistribute. Neither the CRA's
  free and open-source software provisions nor the open-source software steward
  regime (Art 24) applies.
- CRA Art 14 reporting of actively exploited vulnerabilities and severe
  incidents applies now: Art 71(2) applies Art 14 from 11 September 2026, and
  Art 69(3) extends it to products placed on the market before 11 December
  2027. The procedure is #3609.
- From 11 December 2027 one technical documentation set, one EU declaration of
  conformity and one CE marking cover both acts, through CRA Art 32(5a) as EHDS
  Art 104 inserts it. The documentation is #3616.
- EHDS Art 35 (identifying the economic operators supplied, for 10 years) is met
  by a register of every operator supplied under a contract. The public
  release is not gated behind it.
- A deployment of an unmodified release puts that product into service; it does
  not make the deploying organisation the manufacturer. The readiness pages no
  longer say otherwise.
- Cadasto B.V. may also host FerroEHR as a service for other organisations
  (owner, 2026-10-06). A hosted deployment is put into service by Cadasto B.V.
  itself (EHDS Art 26(2)), and Cadasto B.V. is then also the customer's
  processor (GDPR Art 28). Every page states both cases, self-hosted and
  hosted, and never assumes only one.
- Cadasto B.V. holds ISO 9001, ISO/IEC 27001 and NEN 7510 certification (the
  badges on <https://www.cadasto.com/>, read 2026-10-06). A certificate covers
  Cadasto B.V.'s management system within its certified scope, never the
  product: no page calls FerroEHR certified, and a page that relies on the
  certification for the development process or a hosted service says so only
  once the owner has confirmed that the scope covers it.
- Only the newest release is supported (owner, 2026-10-19, #3853). When a new
  release is published, every earlier release stops receiving fixes: there
  are no maintenance branches, no long-term-support line and no backports,
  and the remedy is to upgrade, which is free. The CRA support period is a
  commitment for the product: Cadasto B.V. keeps publishing security fixes for
  at least five years after each release is placed on the market (CRA Art
  13(8)) and delivers them only in the version last placed on the market (Art
  13(10)). The end month of that commitment is printed per release (Art
  13(19), Annex II point 7); it is the date until which fixed releases keep
  coming, and a release counts as supported only while it is the newest.

**The EHR system is FerroEHR and FerroBRIDGE together.** EHDS Art 25(1) asks an
EHR system to include both harmonised software components:

| Component | Shipped by | Where |
|---|---|---|
| European logging software component (Annex II 3) | FerroEHR, and FerroBRIDGE for the accesses of its FHIR facade (below) | the shared crates `ehds-logging` 0.18.0 (the Annex II 3.2 access record, the Art 14(1) categories and their classification, the Art 9(2) retention floor, the Art 11(5) emergency mark) and `ihe-atna` 0.18.0 (the BALP `AuditEvent`, the DICOM audit message, RFC 5424 syslog over RFC 5425 TLS, the ATX: FHIR Feed), from FerroAUDIT and pinned exactly in the root `Cargo.toml`; `ferroehr::system_log` (the access-event model, the audit store, what each rendering says, the sinks, and `system_log::chain_check`, the scheduled verification of the store's hash chain), the audit schema migrations in `app/ferroehr/migrations/audit/`, `ferroehr-rest::system_log` (the middleware that records every API access and classifies it), the ITI-81 `GET /ferroehr/rest/fhir/r4/AuditEvent` retrieval in `ferroehr-rest::extensions::fhir`, and the retention register in `ferroehr::storage::marks` |
| European interoperability software component (Annex II 2.1 to 2.3) | FerroBRIDGE | the openEHR-to-exchange-format mappings (#3206), and FHIR R4 resource mapping (<https://github.com/FerroHEALTH/FerroBRIDGE>, BUSL-1.1), reaching FerroEHR over ITS-REST; FerroEHR carries no FHIR mapping of its own, and keeps the AuditEvent route above and the FHIR terminology client |

**FerroBRIDGE records the accesses of its FHIR facade** (owner decision on
FerroHEALTH/FerroBRIDGE#1301, built in FerroHEALTH/FerroBRIDGE#1377, from
FerroBRIDGE 0.0.16). ITS-REST 1.1.0 carries no end-user identity on a read, so
FerroEHR's record of a facade read names the bridge's credential and cannot
name the natural person Annex II 3.2(b) asks for. The bridge knows that person,
so for each facade interaction it writes the Annex II 3.2 record through
`ehds-logging` and sends it as an IHE BALP `AuditEvent` through `ihe-atna`
over the ITI-20 FHIR feed to an Audit Record Repository the operator
configures, and it refuses an access whose record it cannot take. The
recording part of the logging component therefore sits in each product that
sees the access: FerroEHR for its own API, FerroBRIDGE for the facade. The
store, the review tools and the ITI-81 retrieval stay with FerroEHR.

The two components stay independent of each other (Art 2(2)(n) and (o), Art
30(1)(b)). FerroEHR carries no interoperability component and meets
FerroBRIDGE only over ITS-REST. Inside FerroBRIDGE only its server crate links
`ehds-logging` and `ihe-atna`, and an architecture test fails when `eehrxf`,
the interoperability component, or the mapping crates under it reach either.
The technical documentation and the declaration of conformity name the pair;
FerroBRIDGE declares the interoperability component's requirements and its
facade recorder, and FerroEHR the rest of the logging component's
requirements and the system-level ones (Annex II 1). A change to a module in
the logging row above is a change to a harmonised component under Art 30(2)
and updates the technical documentation in the same release.

**When the central log moves to FerroSYS, the EHR system becomes three
products.** The owner direction of 2026-10-06 (#3661, milestone v6.0.0) makes
the access log a shared FerroHEALTH service in FerroSYS. From the release that
ships it, the logging component is split between two products, and the
declared EHR system is FerroEHR, FerroBRIDGE and FerroSYS. Until then the
component ships as the table above says.

| Part of the logging component | Shipped by | Annex II |
|---|---|---|
| Recording at the moment of access: the access-event model, the classification from FerroEHR's own data, the local durable spool that refuses an access whose record cannot be written (fail-closed), and the forwarding to FerroSYS with at-least-once delivery | FerroEHR | 3.2 (a) to (e) |
| Recording at the moment of a FHIR facade access, with the natural person behind it, refused when its record cannot be taken | FerroBRIDGE | 3.2 (a) to (e) |
| The central store, the review and analysis tools, retention per origin and category, the natural person's access-log route (EHDS Art 9), ITI-81 retrieval and export | FerroSYS | 3.3, 3.4 |

The boundary is the forwarding: a record stays in FerroEHR's spool until
FerroSYS has ingested it, and a FerroSYS outage queues records without
blocking clinical reads. The split leaves the independence of the two
harmonised components as it is (Art 2(2)(n) and (o), Art 30(1)(b)): the
central store and the review sit outside FerroBRIDGE, whose facade recorder
stays out of reach of its interoperability crates as above. The technical
documentation then covers FerroSYS's part as well, and the declaration of
conformity names the three products with the version of each.

**Points for counsel.** These are questions, and nothing in this repository
answers them:

1. Whether CRA Art 13(10), which lets a manufacturer patch only the latest
   version when earlier versions' users get it free of charge, holds for
   commercial licensees under BUSL-1.1 (the support period, #3610, #3647).
2. The scope of the EHDS Art 35 register: whether a download under the public
   licence supplies an economic operator.
3. Whether a deployment that modifies the source and puts the result into
   service becomes a manufacturer itself (CRA Art 21 and Art 22, EHDS Art 34).
4. Whether a deployment Cadasto B.V. hosts as a service is remote data
   processing outside the CRA's product scope, and whether Cadasto B.V. is then
   an essential or important entity under NIS2 (for example as a managed
   service provider).
5. Whether FerroEHR is an important product with digital elements under CRA
   Annex III and Implementing Regulation (EU) 2025/2392, which decides the
   conformity-assessment route.

## Build state

The openEHR-conformant CDR is shipped: the generated spec/ITS foundation plus
the full application (persistence, greenfield storage, REST + auth, the
service layer, templates, WebTemplate/FLAT/STRUCTURED, validation, the AQL
engine, conformance). Everything not yet built — the plugin system, further
enterprise capabilities, refinement, performance — is ordinary tracker work:
an issue with a milestone, built as compiling, tested increments of our own
design. The build record is the closed issues + PR descriptions.

## Verification

- **Fidelity gates** (spec/serialization): canonical JSON read + lossless
  round-trip + ITS-JSON schema validation; XML round-trips.
- **Conformance pipeline** (`scripts/conformance.sh`):
  [Veredictum](https://github.com/Vernum-Projecten/Veredictum), the independent
  CNF 2.0 reference runner, over its committed machine-readable catalogue
  (Docker-composed SUT on fresh volumes) — the acceptance instrument, pinned
  in `scripts/lib/veredictum.sh`;
  results → pure-function verdicts → report/statement/certificate + badges,
  all under `docs/conformance/<sut>/` (the baseline lives ONLY in those
  committed artifacts).
- **Measured performance** (the same instrument): `scripts/conformance.sh perf`
  runs `veredictum perf` over a pinned workload (open-loop Poisson arrivals, a
  scout ramp and a ladder of plateaus, at most ten minutes after seeding) and
  writes `docs/conformance/ferroehr/perf-result.json`: the knee bracket, the
  peak, per-operation percentiles from recorded histograms, server-side
  readings, environment-bound and earning no class. `perf-judge` re-derives
  it; `perf-compare` runs A/A and A/B on one host. Published SVGs and the
  summary regenerate FROM the committed record
  (`scripts/render/perf-assets.sh`, CI diff-guarded).
- **Drift check** (`scripts/checks/codegen-drift.sh` + CI): the generated layer
  is always in sync with the vendored specs.
