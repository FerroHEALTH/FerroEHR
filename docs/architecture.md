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
  ehr_id = $1`; PostgreSQL moves the rows between partitions and the
  `vo_attestation` foreign key carries its rows across with `ON UPDATE
  CASCADE` — verified first-hand on PostgreSQL 18.6, since the documentation
  neither permits nor forbids it. `node` has no foreign key into `version`
  (#5969): its referential trigger ran once per row, 402 nested statements on
  an IPS commit. Every node row is written in its version's transaction, the
  tier moves and the deletes name the node rows in the version's statement,
  the storage-parity sweep reports a row set no version reads
  (`orphan_nodes`), and `node_version_integrity` holds the invariant across
  commit, delete, archive, erasure and decommissioning. The per-EHR class index
  `idx_node_hot_ehr_type` leaves out the ELEMENT rows, which are reached
  through the nested-set range of the primary key, and the subsumption index
  `idx_node_hot_arch_subsume` keeps only the rows with a parsed archetype id;
  the AQL emitter writes RM types as literals so a generic plan proves the
  partial predicates. What partitions buy over the former mirror tables:
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
  Each replica holds the addresses `template_ref` registers in memory
  (`service::definition::registered`), refreshed through `LISTEN` on
  `ferroehr_template_ref`, which a registration or a release notifies, so a
  COMPOSITION commit asks the store only of an address the copy does not hold
  (#5775, our own design); the `version.template_id` foreign key and the
  template delete guard still refuse a template released at the same moment.
  A COMPOSITION commit reads its EHR's existence, `is_modifiable`, restriction
  mark and settings revision in one statement (`ehr_repo::ehr_writability`).
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
  the `ITEM_STRUCTURE` under each get their own `node` row. A party is stored
  as committed: a national identifier belongs in the master patient index,
  and the CDR seals none (#6092; `party/0059` drops the former sealed-value
  tables). Beside them, `party_relationship_target` is the target-side index behind `PARTY.reverse_relationships`, and the domain
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
  crossings and the access record each one owes. The v6.0.0 baseline creates no
  linkage schema and no `ferroehr_linkage` role.
- **The access log** is the `audit` schema. An access to EHR data is stored
  as its `ACCESS_LOG_ENTRY` alone; `audit_event` keeps the records of
  accesses outside the access log (an authentication, a template or party
  operation, an application activity), with the FHIR `AuditEvent` rendered
  from its columns. `access_log_entry` holds the draft openEHR
  `ACCESS_LOG_ENTRY` (#5121) in two parts of its ITS-JSON canonical form
  without its link (RFC 8785, #5703, our own design): the part its caller
  shares (accessor, credential, roles, authorisations, purposes, retention
  and the caller's access details) once in `access_log_shared`, keyed by its
  SHA-256, and the rest on its row, compressed in place, with its digests as
  32-byte `bytea`, its uid a `uuid` and no `system_id` or `request_id` column
  (the content holds both), and the repeating part of its `outcome` and
  `query` in the shared part, the row keeping only the delivered count and
  the rows (#5714); the search columns
  its insert trigger derives from both; and its `ACCESS_LOG_LINK`: the
  content digest over the whole canonical form, which the writer computes
  (`access_log::stored_form`), and the digest over the previous digest, the
  sequence and the content digest. The server recomputes each content digest
  from the joined parts and the database checks the links, so an
  incremental run advances no further than the server checked. A run walks
  each chain from its cursor to the head it started with in windows of at
  most `VERIFY_STEP_RECORDS` records, one statement each, and every intact
  window moves the cursor, so no statement grows with the backlog and a
  backlog reads as a lag, never as unverifiable (#5779, our own design,
  `audit/0004_baseline_access_log.sql`). An access detail the core states as the record holds it
  (the principal and issuer the credential names, the count the outcome
  delivered, the action and outcome the core codes) is not written
  (`access_log::recorded::Core`). A tombstone keeps no shared part, and a
  shared part leaves with the last row that named it.
  The two tables are two hash chains (#5573, our own design): the access log
  holds only the accesses to EHR data, one chain per system from 1 with no
  gaps, under its own head (`access_log_chain_state`), verified by
  `audit.verify_access_log_chain()` and its stepped form; `audit_event`,
  the system chain, keeps its head (`audit_chain_state`), its retention gaps
  and `audit.verify_audit_chain()`. Writers of a chain serialise on its head
  row, which holds across replicas, and the scheduled chain check verifies
  both. Each chain links from its own genesis
  (`audit/0002_baseline_audit_chain.sql`, `audit/0004_baseline_access_log.sql`). A record of the ITI-81 search, the person's trail and the FHIR
  Feed's outbox is named by its chain and position (`audit_record.chain`,
  `fhir_feed_outbox`). An entry changes only by its replacement with
  its own tombstone, which keeps its uid, time and link, so the chain verifies
  after its content is gone (`audit/0004_baseline_access_log.sql`,
  `access_log::store`). Each entry states its retention when it is written
  (`access_log::retention`: `retain_until` from the period of the access
  policy's `log_retention`, the grounds it declares beside EHDS Art. 9(2)
  coded by the regulation's ELI, and the policy version, #5660), and the audit store's hourly worker replaces every entry past its
  `retain_until` by its tombstone, in any order of expiry
  (`access_log::expiry`). A tombstone is kept one more retention horizon,
  its entry's own: its `fold_after` is `time_removed` plus the time its entry
  was kept. After that the same worker folds each run of consecutive
  tombstones past their end into one `access_log_chain_gap` row through
  `audit.fold_access_log_tombstones()`, the one path a record leaves the log
  by (`audit/0004_baseline_access_log.sql`, `access_log::gap`): the
  positions the run held, the link of its last record and the grounds of its
  tombstones. The gap row is the record of the fold and
  is append-only; replicas serialise the fold on a transaction-scoped
  advisory lock, and the request path takes no part in it. A run folds whole.
  An unpublished checkpoint inside it, before its last position, leaves with
  it; a published one (`published_at`, set by `access_log::publish`) stays,
  and the gap's `seals` record its position, the link it sealed and the
  SHA-256 of its proof, which `audit.access_log_sealed_link` answers for that
  checkpoint, so a checkpoint published before a fold verifies after it
  (#5727). One at the run's last position names the gap's link
  (`audit.access_log_link_at`). The verifier, its incremental form,
  the checkpoint check, the export (`LogExport.gaps`), the
  chain read and an import (`access_log_imported_gap`) walk a gap as one
  link whose digest is its link; a missing position no gap accounts for is
  still a deletion (our own design, #5619). An erasure a law orders takes
  the tombstone path through
  `POST /admin/access-log/erase`, which first chains every intake row, then
  writes the tombstone and the access-log entry recording the erasure, with
  its index row, into the intake in one transaction through the audit store
  (`AuditStore::erase_entry`), so the log holds both or neither and the act
  is stored once, as its entry; the act names its record
  `EntryScope::Written`, and the middleware writes no operation record of
  its own for such a response. Every mounted operation has a
  row in `ferroehr-rest::api::operation` naming its audit action and resource
  class; there is no fallback class, the dispatcher refuses an operation
  without a row before its handler runs, and the route-coverage test fails on
  a mounted route without one. Every entry
  is built by `access_log::entry` from an `AuditEvent`: the request's
  per-access entries, the erasure's, and the import's (`access_log::transfer`). An AQL
  statement attributes each served row to the EHR of every bound `EHR` source
  and versioned-object root (`aql::sql` `served_ehr_ids`, the list the patient-scope
  collection reads too), so an EHR-only statement writes its per-EHR
  entries; a management route that touches EHRs records one access per EHR
  from the service (`emit_mark_access`, `emit_extract_audit`).
  `access_log_checkpoint` keeps the signed `ACCESS_LOG_CHECKPOINT`s, by the
  columns their canonical form is rebuilt from, signed with the
  version-signing keyring's OpenPGP key. Whenever the keyring holds one,
  every append ends, in its own transaction, with one checkpoint of the last
  link it wrote: a chainer round (`access_log::chainer`) begins the
  checkpoint's signature on the blocking pool as soon as it has computed the
  links, so the signature runs beside the insert of the records, and the
  head is held for one signature at most. No link carries a signature; a checkpoint
  insert must name the link the log holds at its position
  (`audit/0004_baseline_access_log.sql`, our own design, #5704).
  Without a key nothing is sealed, and no key of `[audit.access_log]`
  switches it (#5562). `access_log::publish` writes the newest checkpoint
  every 30 seconds as its own write-once file under `[audit.access_log]
  publish_directory`, the copy outside the database that shows a removal of
  the newest entries with their checkpoints; the `access_log_checkpoints`
  indicator reads `DEGRADED` without a key, without the directory, or while
  publication fails. `access_log::checkpoint` seals and verifies (the chain
  through `audit.verify_access_log_chain()` and the checkpoint beside it).
  The system chain is sealed every five minutes with the same key
  (`access_log::schedule`, `system_log::seal`, `audit_chain_seal`, `ferroehr audit-chain seal`
  and `verify`). `access_log::chain` verifies a span of links apart from the database
  (an export), and `access_log::transfer` exports one EHR's records with the
  span and checkpoint and imports another system's into
  `access_log_imported_*`, outside this system's chain
  (`audit/0005_baseline_access_log_imported.sql`). An imported entry ends at the
  `retain_until` it states and its tombstone is kept as long again; the
  hourly run then folds, per exporting system and within each span between
  two imported checkpoints, only the positions before the first imported
  record still kept (tombstones past their end and the other EHRs' links)
  into `access_log_imported_gap` through `audit.fold_access_log_imported()`,
  so every kept imported record stays hash-chained to its checkpoint with or
  without link signatures (`audit/0005_baseline_access_log_imported.sql`,
  `access_log::imported`, our own design, #5664). `Checkpoints::verify`
  verifies each imported checkpoint's stored span through `chain::verify`.
  Every access to EHR data writes one entry per EHR it
  reached, refusals and reads of the log included, and every record is
  durable before the response that reports its access (#5747, #5745):
  `AuditSender::record` builds the entries from the request's `AuditEvent`s
  (`access_log::entry`) and `AuditStore::write` puts every record of the
  request, with each entry's index row (projection, search tokens, render
  policy) and every column the log derives from its content, into the
  intake in one short transaction of the audit pool: each entry is written
  once, into `access_log_entry`, with one `access_log_intake` row naming the
  entries of the statement until the chainer links them, and the other
  records into `audit_event_intake` (`audit/0006_baseline_intake.sql`,
  `audit/0004_baseline_access_log.sql`, #5968). The intake statement finds each entry's
  search tokens by the digests the server computes
  (`system_log::token_digest`, the bytes of `audit.search_token_digest`)
  and groups them by entry in one pass, so its cost grows with the batch's
  tokens and not with its entries times its tokens
  (`audit/0006_baseline_intake.sql`, #5889). A write that fails refuses the request
  with `503`, so back-pressure replaces loss; there is no queue in memory and
  no fail mode. The intake is append-only: the runtime role may insert and
  read, the tables refuse every update, delete and truncate, and only the
  two `SECURITY DEFINER` moves remove a row, each by chaining it. The
  chainer (`system_log::sender::chainer`) runs on every replica on the audit
  runtime: a request's write wakes it and it runs one round 200 ms later for
  what that window wrote, touching only the chains written since the last
  round, and it polls both chains every 2 seconds for the rows a stopped
  replica left. A round takes the access log's head
  with `audit.access_log_chain_take()` (`FOR UPDATE SKIP LOCKED`, so one
  replica chains at a time and the others skip), reads the entries of the
  oldest intake rows, whole rows up to about 512 entries, with their content
  digests, computes the links in Rust, and `audit.access_log_chain_move()`
  inserts one narrow `access_log_link` row per entry (position, previous
  digest, digest) and the outbox rows of the FHIR Feed and syslog, and
  deletes the intake rows; the checkpoint and the commit end the round. The
  link table's statement trigger checks the new links together against the
  head and the entries' content digests and moves the head; the verifier
  derives every column of the entry again and compares (0039, 0048). The statements PostgreSQL runs
  inside these writes are held to a budget (#5885): the digest functions are
  inlined into the statements that call them, the records and their index
  rows carry no foreign keys, since the chain move is their one writer and the
  verifier derives each record from its shared part again, and the link check
  moves the head cursor by an assignment
  (`audit/0006_baseline_intake.sql`). The intake rows carry no
  foreign keys either (#5887): the intake statement refuses a render policy or
  shared part the store does not hold in the statement that writes the rows,
  and locks each distinct stored part its batch names once, so the
  retention sweep's collection of shared parts skips a part a write in
  flight names (`audit/0006_baseline_intake.sql`). A shared part
  keeps the latest `retain_until` of its entries, never earlier than the day
  the intake last named it, and the sweep removes the parts past it that no
  entry names, in batches read through the entries' `retain_until` index
  (`audit.access_log_shared_collect`, #5968). The ferroehr-rest test
  `statement_budget` holds an AQL request to its round trips and nested
  statements, read from `pg_stat_statements`. `audit.audit_event_chain_move()`
  moves the system intake into the system chain the same way. A
  request-scoped write commits through one choke point,
  `FerroEhrService::commit_recorded` (`system_log::reservation`, #5762): its
  records go into the intake held under a reservation id (`reserved_by`,
  which the moves skip), the id into the write's own domain
  (`clinical/0050`, `party/0052` `audit_reservation`) in the write's
  transaction, and the chainer's next round releases them in its own
  transaction once the response is built;
  `system_log::sender::reconciler` releases or discards what a stopped
  replica left and removes spent reservations, and
  `scripts/checks/request-commits.sh` guards the choke point. Our own
  design; one chain carries about 15 000 entries per second on the
  reference host (#5745), and partitioned chains (one per EHR hash, a
  draft-model change) are the recorded next step past that. The entry keeps in its `other_details` the facts its
  projections carry that the draft's core does not hold, FerroEHR's access
  details (`access_log::details`, `access_log::recorded`): the audit forms of
  the action, the outcome and the resource class, the subject of care of the
  EHR, the authentication facts, the category evidence and the binding. The
  IHE BALP `AuditEvent`, the DICOM audit message and the EHDS access record
  are projections of the stored entry (`access_log::projection`): the ITI-81
  search, the person's own trail and the FHIR Feed read it through
  `audit.audit_record`, the view of the `audit_event` records and the
  indexed entries. An entry is one row of `access_log_entry`, and
  `access_log_record` is the view of the entries with their link: its index
  columns (the search columns, the search tokens of the projection and the
  render policy) sit on the entry's own row and are emptied with its content
  when a tombstone replaces it, and `access_log_search` is the view of them
  the ITI-81 search reads (#5784). An ITI-81 page reads each chain's first
  `offset + count` records through its time index, the access log's
  candidates from `access_log_search` with the records tied on the last
  instant, and takes the page from their union by the records' keys alone;
  only the records the page serves are then read whole, so an access-log
  entry's document is rendered once per served entry (#5965). The exact
  total counts the
  access log from the time index alone, which carries `render_policy_id`
  and the entry id (`system_log::store::search_page`, #5646); it also counts
  the entries of the last fraction of a second that the chainer has not
  linked, which a page does not serve yet (#5968). A row part keeps no member a
  column holds: `_type` goes always, and `uid`, `ehr_id` and `time_recorded`
  when their column gives back the exact text (`ehr_id` is a `uuid`, the
  render policy is named by its `audit_render_policy` id), and
  `audit.access_log_document` is the one reader that joins the row part, the
  shared part and the columns back into the canonical form (#5784). The
  content is stored raw: PostgreSQL compresses a value only in a row above
  about 2 kB, whatever `toast_tuple_target` says. syslog and the FHIR Feed are
  projections of the stored records, delivered by `ihe-atna`'s forwarders
  from `syslog_outbox` and `fhir_feed_outbox` (`system_log::store::outbox`,
  `system_log::feed`), so a record waits for its repository in the store,
  never in memory.
  The log is read over ITS-REST (#5130): `GET {ext}/ehr/{ehr_id}/access_log`
  and its `/{uid}` serve the subject view (`access_log::view`) to the
  patient token's launch-context EHR and a bound representative's
  represented EHR, and the full view to the caller the admin class admits;
  `GET {ext}/admin/access-log`, `/chain` and `/checkpoint` serve the review.
  `access_log::search` pages the records over the trigger's search columns,
  the coded filters (`action`, `outcome`, `accessor_kind`) taken as a FHIR
  token `terminology_id|code` and compared by `defining_code`, never by
  rubric (#5687, our own design),
  `service::admin::access_log_read` holds the EHR check and the EHDS Art.
  3(3) holds the person's view leaves out, the subject view withholds what
  the access policy's Art. 9(3) `log_restrictions` name (#5570), and every
  read writes its own `LogRead` entry naming the log it read. AQL reads the
  log as the top-level class `ACCESS_LOG_ENTRY` (#5131): `aql::access_log`
  plans it apart from the clinical engine and its plan cache (alone in `FROM`,
  never inside or around an `EHR`), its `sql` statement reads the trigger's
  search columns where a path has one (the `(ehr_id, time_recorded)` index
  serves the person's lookup) and the canonical form elsewhere, and
  `service::query::access_log` applies the caller's `LogStanding` (the REST
  resources' rules, decided once in `api::access_log_query::standing`), reads
  each column out of `access_log::view::view_of` under the policy's
  restrictions and reports a `LogRead` the query's record writes; a withheld
  path reads as absent in a condition (`view::subject_withholds`).
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
  Every checkout goes through `db::acquire::acquire`, which answers a
  failure to open a connection `503` whatever the driver reports (a refused
  connect, a resolver failure, a timeout); the `clippy.toml` ban on
  `sqlx::Pool::acquire`, `begin` and `try_begin` holds the server to it, and a
  test or tool that reads on a raw pool connection says so with a scoped
  `#[expect]` (#5109).
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
CONTAINS chains; `jsonb_path_query_first` for a leaf a WHERE or ORDER BY
reads and `jsonb_path_query` as a lateral set-returning function for array
unnesting; `ext.openehr_magnitude` and `ext.openehr_timestamp` for typed
comparison and ordering) → execute (`sqlx`) → `RESULT_SET` (1.1.0).

A projected cell is rendered in Rust, never built as jsonb in SQL (#6004).
For a data leaf the statement returns the stored columns of the node row the
leaf reads (`data`, the fragment columns and the root identity
`storage::node_repo` reads), once per node alias however many cells read it;
`aql::read` rebuilds the fragment with `storage::slim::rebuild`, evaluates
the fragment path with the lax SQL/JSON path semantics PostgreSQL gives the
same text (`aql::fragment`), and writes the cell with its object keys in
jsonb order. A VERSION or EHR field, a function and an aggregate return
their typed value (a `timestamptz` beside its session offset, so it renders
as `to_jsonb` would), and a literal returns nothing. Each row is written
once into its cells (`aql::cell`) and the `RESULT_SET` once into the body,
its weak `ETag` the SHA-256 of the bytes as written without the
`meta._created` stamp (`service::query::result_set`). `SELECT DISTINCT`
and a projected list of attestations keep their cells as jsonb in SQL,
where PostgreSQL compares or aggregates them. A debug build selects, in
the same statement, the jsonb cell PostgreSQL builds beside each rendered
cell and requires the two to be the same bytes. No openEHR spec governs the rendering: our own
design. A path whose candidate
types are mixed (an `ELEMENT`'s whole `DATA_VALUE`) compares and orders by
keys read off each value's own `_type` (`aql::sql::coerce`): a number against
a JSON number or the magnitude of a whole numeric `DV_ORDERED`, an ISO 8601
date/time against the instant of a whole `DV_DATE`, `DV_TIME` or
`DV_DATE_TIME` of its kind, text against a JSON scalar, and never the
serialized JSON; AQL 1.1.0 leaves this open, so the reading is our own
(#5751). A Real binds as its shortest round-trip decimal cast to `numeric`,
never through `float8` (#5753). The emitter uses no
jsonpath item methods (their date/time conversions depend on the session
TimeZone and are not index-legal), no `JSON_TABLE`, and no GIN index (it
emits no GIN-servable operator; the baseline removed the index). Five
value indexes give a filter an entry: `idx_node_hot_value_search` on
`ext.node_search_text`, the text an equality on a text leaf of an ELEMENT
value compares against from the typed value columns or the stored fragment
(#4437); `idx_node_hot_value_code` on `ext.node_code`, the code an equality
on the `defining_code` of a coded text or of an ordinal's symbol reads from
`value_code` or the stored fragment (#6033); `idx_node_hot_event_time` on
`node.event_time`, the
`ext.openehr_timestamp` of an event row's `time` and of a history row's
`origin`, which the node insert stores; and `idx_node_hot_value_magnitude`
on `ext.node_magnitude`, the number an ordering comparison of an ELEMENT
value's `magnitude` reads from `value_num` or the stored fragment (#6022);
and `idx_node_hot_value_number` on `ext.node_number`, the number a
`DV_ORDINAL`'s or `DV_SCALE`'s `value` holds, from `value_num` or the stored
fragment, which an equality of `value/value` with a text in plain decimal
form (`'19500101'`) reads beside the search text, since such a value's
search text is its symbol (#6090). It costs 72 bytes of WAL per ordinal or
scale value a commit writes, and nothing on a commit without one.
The WHERE lowering adds, for a top-level comparison one of them serves, a
FROM item of the distinct versions some row of which carries a value the
comparison keeps (`aql::sql::anchor`), so the statement
reads the objects the filter keeps instead of every node of the archetype.
The comparison itself stays in the statement, and each anchor reads the
value the comparison reads, so an anchor removes work and no row. The
comparisons of one leaf share one anchor, so a window reads the rows inside
it. A `matches` over a list of strings reads as the equalities it lists
(QUERY master03 §matches), so it takes the equality anchor with the members
as an `IN` list, when every member could anchor its equality (#6084). In a
demographic query that reads the trunk head, an equality of a party's
`relationships/target/id/value` with a text that names a party enters
through the open rows of `party_relationship_target`, which hold the party
each trunk head's relationships name, as the commit derives it (root before
`::`, case-folded, a hyphenated UUID); the comparison keeps its exact text,
and under `ALL_VERSIONS` or a version predicate it has no entry (#6090). An
equality's anchor always applies. A range may keep most of the store,
so its anchor depends on the statement (`aql::sql::anchor::RangeEntry`): in
the flat shape the planner places it; a population statement that streams
toward its LIMIT is led by the first row of each version the range keeps,
read from the index row by row; and a statement that names its EHRs takes
none, because their versions are its entry.
`event_time` is a stored copy of a `STABLE` function's value: it equals the
comparison's reading because every session runs under `TimeZone = UTC`,
which the driver sets at connection start. The feature envelope is
documented per construct; rejections are explicit typed errors.

A cached plan renders its SQL once per statement shape (`aql::sql::shape`,
#5783). The text depends on the parameters, the scope, the paging and the
caller's access condition, and only some of their values vary between the
requests of one caller: the EHR a single-EHR query names (a `$parameter`
compared with `ehr_id/value` and nothing else), the `ehr_ids` scope, the
paging bounds, the request's moment and day, the caller's parties and care
teams, the EHRs it cares for or is confined to, and the pinned EHR and its
revision. A shape is keyed on every other input of the build, and its
values are numbered slots: fixed, or read from the request. The second
request of a key derives the shape from two builds over random stand-ins for
those values, keeping it only when both texts are the request's own, every
value is fixed in both or the image of one source in each, and the shape
binds exactly what the request's build bound; from the third request on, the
statement is bound with no tree built and no text rendered. A plan remembers
32 keys, and a key whose shape depends on a value any other way is built on
every request. No openEHR spec governs the SQL build: our own design.

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
without the EHR gates (population, `ehr_ids`, `EHR_ACCESS`, and the access evaluator's per-EHR condition, which decides access to an EHR and has no row to judge on a party, #6087). An `EHR` operand directly under the root is the join to the EHR the
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
`access` module in `ferroehr-rest`: RBAC and the access evaluator.

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
   draft BMM's invariants, the terminology groups and the RM `Value_valid` of
   each `DV_DATE_TIME` and `DV_DURATION` it holds (#5629) hold (RM EHR IM
   §`ACCESS_CONTROL_SETTINGS` leaves the class "Currently implementation
   dependent": the scheme is our own design). The default archetype
   `access_directives.v1.0.0` constrains the settings to that class, which the
   ADL 2 validator reads from the draft class tables beside RM 1.2.0
   (`draft_model`, AOM2 master11 RM adaptation). A new version is committed by the
   subject while the current settings name no gate-keeper, by a gate-keeper
   the current settings name, or by `admin_role` on the subject's instruction
   (BASE Architecture Overview §Access Control: the gate-keeper is "usually
   the patient for mentally competent adults"), so naming gate-keepers without
   `PARTY_SELF` takes the subject's write away (#5636); the settings are read only by those callers and the
   subject, and are absent to anyone else in AQL, an EHR Extract and the
   CONTRIBUTION read, since a restriction stays invisible to a provider (EHDS
   Art. 8). An EHR Extract also carries them for the holder of an
   `export_ehr_extract` grant covering the EHR, since the extract transfers
   the whole EHR (RM EHR Extract IM, `X_VERSIONED_EHR_ACCESS`). Who writes and
   reads them is our own design (`service::ehr::access_rule`). Those callers
   read them over the wire through FerroEHR's own extension,
   `GET {extensions}/ehr/{ehr_id}/ehr_access` and `…/versioned_ehr_access`
   with its version and revision-history reads, since ITS-REST 1.1.0 has no
   `EHR_ACCESS` operation (#5517). The routes are shaped on the `EHR_STATUS`
   and `VERSIONED_EHR_STATUS` operations and run their handlers, generic over
   the RM class (`api::ehr::ehr_access`, `service::ehr::object`), behind the
   same `guarded_dispatch`; a read that carries the settings is canonical
   JSON only, since canonical XML maps no class of the scheme. Every write
   path (CONTRIBUTION, the controller's commit, EHR Extract import, archive
   load) refuses with `422` a gate-keeper whose `external_ref`, or a
   directive whose actor-scope `party` or `instructed_by.external_ref`, in
   the directive or an exception, names no
   party of the demographic store of its type, nor one the import's
   demographics chapter or the archive's demographic records carry
   (`service::ehr::access_parties`); `PARTY_SELF` needs no lookup. An import
   and an archive load check only the head version, the one in force, since
   an earlier version is the immutable record of what was in force then. The EHDS
   Art. 8 restriction and Art. 10 opt-out are `restriction` and `opt_out` directives of these settings,
   committed by the controller over `POST /admin/ehr-access` (which names the
   EHR in its body, so it reaches an EHR a restriction hides) or by a
   gate-keeper. A data scope that names an object by a reference naming no
   versioned object is refused at commit (`400`), since nothing could place
   it. The event outbox feed is judged as the request that reads it: each
   event is judged for the feed's reader like any read of that EHR's data,
   so a directive that denies the reader, an access policy it does not meet
   or the default of an EHR with no settings withholds it, and an event whose
   verdict depends on the data is published only when every object its
   contribution wrote is admitted (`extensions::outbox::judge`, #5587). The
   judgement of a contribution as a whole is our own design; no
   specification governs it.
   The commit of an `EHR_ACCESS` version also
   projects the settings in its own transaction (RM Common IM change control:
   every change is a version in a contribution) into `access_settings`: the
   settings, their **class** and their **facts**
   (`service::ehr::access_projection`, `access_eval::class`, #5588). The
   class is the settings with what the decision does not read left out and
   every value particular to the EHR replaced by a fixed one: the instants of
   a validity or a `recorded` interval, the party of an actor scope or a
   gate-keeper, the objects and origins of a data scope, and the objects each
   sensitivity labels. The facts are those values, in the order of a
   depth-first walk. A value the decision cannot read stays in the class. One
   `access_class` row holds each distinct class, so every opt-out from a date
   shares one. A trigger on `access_settings` notifies `ferroehr_ehr_access`
   with the EHR id, one on `access_class` with `class`, and the first boot and
   `party-binding refill` rebuild the rows. Each replica copies an EHR's
   settings on first read, at most 4096 EHRs, and drops the copy on that
   notification (`policy_register::listen::Listened` with a `Subscriber`);
   beside them it copies the classes, dropped on `class`. Nothing on a
   replica holds every EHR that carries settings, so its memory does not
   grow with them. While the listener is down it reads the projection per
   request, and a projection that cannot be read is `503`. No openEHR spec
   governs the projection, the classes or the copy: our own design (#5509,
   #5588). The copy also holds the EHR's subject, and a commit that changes
   it notifies the same channel.

   **Staleness fails closed** (#5589). Each copied entry and each verdict a
   request judges carries the revision of the EHR's settings row (the
   `sys_version` of the `EHR_ACCESS` version it was projected from) and the
   EHR's subject reference (namespace and id, `clinical/0046`), whose
   `PERSON`'s care relationships the decision reads. Every
   read that leaves the store selects the revision beside its own columns, in
   a scalar subquery over the view `access_revision_of` that the planner merges
   into the statement, so the read runs no statement of its own
   (`access_revision!` in `head_facts!`, `version_select!`, the EHR summary
   read and the settings reads; migration `clinical/0051`, with a `party/0053`
   twin holding no rows so a statement both domains run reads `NULL`; #5888),
   and `condition::admits` refuses a verdict judged
   on another revision and marks the copy stale (`StaleCopies`), which drops
   it before its next read. A commit reads the revision with its restriction
   check and judges the EHR again from its own transaction; a replica's own
   `EHR_ACCESS` commit judges the request's EHR again after it commits. A
   change committed on another replica is therefore never admitted under the
   old settings, with no added statement. Every channel of one database is
   held on one `LISTEN` connection apart from the pools
   (`policy_register::listen::Listeners`, #5845; `[database] listen_url`,
   `[database.<domain>] listen_url`), named `ferroehr policy listener`, so
   the request pools may run through a transaction-mode pooler; the
   connections an instance opens, its pools and those sessions, stay inside
   `[database] connection_budget` (`db::pool`). Each instance writes its signing
   key fingerprint and configuration digest into `clinical.instance`
   (migration `clinical/0045`, `instance::Instances`) and the `instances` indicator
   names an instance that differs.

   **The access evaluator** (`access_eval`, #5513) decides every EHR a
   request reaches by a fixed procedure over three fact sets held in memory:
   the subject's directives and labels (the copy above), the caller's
   capabilities, the roles of the `ROLE`s it performs and its care teams
   (`party_binding::grants::live`, #5511, #5621), and the controller's
   `ACCESS_POLICY` with its rules (`access_policy`, #5512, #5621). The first step that refuses
   decides, deny overrides permit, and one level of exceptions narrows each
   directive: (1) an `opt_out` binding an access service, `404`
   `refused-opt-out` (EHDS Art. 10); (2) a `restriction` binding a
   professional, a system or an access service, its data `404`
   `refused-restricted` (Art. 8); (3) any other deny that applies, `403`
   `refused-consent-denied`, and a representative a `delegation` names reads
   only what it permits (Art. 4(2)); (4) no declared purpose where the
   policy requires one, `403` `refused-purpose-missing`, checked as input
   before any rule is read (#5737); (5) a deny rule of the controller's
   policy that applies, `403` `refused-policy-denied`, which no permit of the
   subject lifts (Art. 11(3)), a rule's purposes matching only a declared
   purpose, and the purposes a caller may declare being such a rule; (6) a
   sensitivity from a label or a classification that no applying permit
   directive or permit rule names, `403` `refused-sensitivity`; (7) no live
   care relationship, plus the policy's grace, where the policy requires
   one, `403` `refused-no-care-relationship` (BASE Architecture Overview
   §Security Policy, "Time-limitation of access"); (8) the default disposition, `403` `refused-consent-required`, an EHR with no
   settings taking `[authz.rbac] ehr_access_default`. The subject skips every
   step on their own EHR (Art. 3). Who the subject is has one source, the
   party binding (#6106): a caller is the subject of an EHR whose
   `EHR_STATUS.subject` names the `PERSON` its authenticated identity is
   bound to, whatever roles it holds and with no purpose declared. The
   evaluator reads that subject reference from its per-EHR copy, whose
   revision every read checks against the store, and resolves its `PERSON`
   as the care step does, so the decision adds no statement; it then
   records the access as the subject's own and reads the caller as the
   subject for that EHR's `EHR_ACCESS` settings. A bound person holding no
   role passes the role gate and is confined to those records: it is the
   subject kind, and every other EHR refuses it `refused-not-subject`. A
   `persons` login whose one `PERSON` carries neither the professional nor
   the representative marker is that patient, through a first-party client or
   a SMART app the CDR grants `act_for_persons` (an openEHR launch, #6146):
   the shared binding answers it
   `wrong_marker`, and FerroEHR admits it as the person themself, with no
   role, `PARTY_SELF` as its committer and the binding `self` on its access
   record, confined in the same way. A `persons` login of the same two kinds
   whose one `PERSON` carries the representative marker alone is admitted as
   a representative read from the CDR's own facts (#6147): it holds no role,
   and reaches the subject of the EHR its launch context names, else of the
   EHR the request names, else the `PERSON` an EHR lookup by subject names,
   only on a live stored representation of that subject (#6079), else
   `refused-representation-unrecorded`. When that subject is its own
   `PERSON`, it is admitted as the person themself, as above, with no
   representation (#6167, EHDS Art. 3). An
   openEHR launch token carries no `fhirUser` and no `patient` claim (SMART
   on openEHR `master07` §Context Selection names the `ehrId`), so no
   reading of the token alone can name the person. A token
   `ferrohealth-smart` 0.31 reads as the patient's own (`Caller::is_the_person`:
   a `patients` issuer's patient-context token, or, under the `ferrosmart`
   preset's `fhir_user_patient`, a `persons` token whose `fhirUser` is the
   `Patient` its `patient` claim names, with `patient/` scopes alone) is a
   check against the binding and never a grant (#6080). On a `persons` issuer
   its login is looked up as a person's through the `[[auth.party]]` table,
   which the shared binding never does for a `Patient` caller, and has to bind
   a `PERSON` with neither marker, else `refused-unbound`,
   `refused-key-claim-missing` or `refused-wrong-marker`. On a `patients`
   issuer, which may carry no table, the patient-portal binding reads the
   login as a key of the `PERSON` the launch EHR's subject names, else
   `refused-unbound`. A caller bound as the person themself whose token names
   an `ehrId` whose subject is another `PERSON` is refused
   `refused-representation-unrecorded`, as a portal login is. The refusals
   reuse the binding codes the access record carries. FerroEHR cannot map a
   FHIR `Patient` id to a `PERSON` without the FHIR pairing, so the `patient`
   claim is not compared. A confined caller is refused `403`
   `refused-not-subject` on every other EHR, before step 1 (#5605): the
   `subject` kind, through a SMART App Launch patient-context credential, and
   any caller whose grants on the EHR's data are `patient/` grants alone. Its
   own EHR is the `ehrId` of the launch context (ITS-REST `master07` §Context
   Selection; `master08` §Resource Scopes, "Access is restricted to data
   within that patient's EHR"), or a contact point's TRC patient's EHR. The
   evaluator alone confines it, on every route that names an EHR in its path,
   its single-EHR query or import scope, its manifest or its result, and on
   every row, count and page of a query; an operation on EHR data that names
   no EHR (an EHR create, an unscoped query or import) is refused at step 0.
   A lookup by subject judges the EHR it finds, and a confined caller finds
   only its own: any other answers `404` as a subject no EHR names does. The SMART layer checks only a grant's family and
   permission (#5586, #5606, our own design: SMART App Launch requires the
   patient context held and names no layer). On its own EHR a confined
   professional takes every step. A representative is confined the same way
   to the EHR whose `EHR_STATUS.subject` names the subject of its recorded
   representation `PARTY_RELATIONSHIP` (resolved once at authentication, no
   added query), refused `403` `refused-not-represented` on every other EHR
   and on an operation that names none, recorded as step 0 (representation)
   (#5609, our own design under EHDS Art. 4(2)). On that EHR it is held to
   steps 1 to 3 and 5, the `delegation` directives of its `EHR_ACCESS` and
   the controller's rules included, and skips steps 4 and 6 to 8. A caller acting for
   someone else is a representative whatever issuer it signed in through, so
   a `patients` portal login bound to a representative's `PERSON` takes
   those steps too; only the person themselves is the subject (#5613). The
   subject and the representative pass the RBAC role gate's Clinical class
   with no role, since their right comes from who they are (EHDS Art. 3 and
   4(2), #5633), and the access log's standing reads their EHR from the same
   evaluator `Caller` (#5614). A read, query or export by a caller admitted
   under a federation agreement whose `consent` condition applies, naming no
   consent decision reference, is refused at step 0 with
   `refused-consent-reference-missing`, a fact of the `Caller` (#5614; the
   SMART layer checks scopes alone). An EHR id the store does not hold
   answers `404` before any step refuses, read only when a verdict refuses,
   so a restriction's `404` is never told apart from it (#5623). A
   COMPOSITION read by its version or its latest is judged on the facts of
   its object that the statement reading the version returns, after the read
   and before anything of the version is served, so the read is one statement
   and a refused read answers and is recorded as before (#5773, our own
   design). A read by version id takes the version's body from the
   replica's version copy (`service::ehr::version_copy`, 64 MiB) once one
   indexed read (`version_repo::judged::read_marks`) finds the object's head
   where it was read and the version's content digest unchanged; the
   restriction and access delay marks and the settings revision that read
   finds are the ones judged, so no decision rests on a notification
   (owner decision on #5891). The read is judged and recorded all the same,
   and an `If-None-Match` naming the version answers `304` (RFC 9110
   §13.1.2) (#5778, our own design). A write is
   judged on the template, root archetype, origins and commit instant of the
   content it is about to store, inside the commit transaction and before
   anything is stored (#5626, `versioning::restriction::WriteGate`), by
   steps 3 to 8: an opt-out and a restriction never judge a write's content,
   since EHDS Art. 8 restricts who sees data and not its recording, a refused
   write would stop care being documented and its `404` would hint at the
   restriction; they decide a write for the EHR alone, and the data written
   stay covered by them for reading (owner ruling on #5621). An
   emergency purpose lifts an opt-out for an access service that may
   override it (Art. 10(2)), and, with `[authz.rbac] emergency_access_role`,
   a restriction, a consent deny or a deny rule whose `emergency_override`
   holds and steps 6 to 8 (Art. 11(5)).

   **Controller rules** (#5621, #5624, #5627; our own design, proposed in
   the draft specification). `ACCESS_POLICY.rules` holds the controller's
   rules for every EHR, each an `ACCESS_DIRECTIVE` of kind
   `controller_rule`, the form EHDS Art. 11(3) asks a Member State's rules
   to take and IHE APPC gives a foundational policy. A deny rule binds every
   caller but the subject (whose own right of access, Art. 3, is not the
   controller's to deny), representatives included; a permit rule acts only
   in step 6, and a rule's permit exception only by narrowing its deny. An
   actor scope's `role` matches the coded `name` of a `ROLE` in force the
   caller performs (RM Demographic IM `PARTY.type()`), held as a caller fact
   (`FactKind::Role`, migration `party/0042`); a token's role claims stay
   the coarse RBAC class gate and drive no rule, except that a caller
   admitted under a federation agreement adds the roles and facility types
   its verified credential attests (`professional.hp_professional_role`,
   `provider.facility_type`), each a code of the system it names, matched by
   an actor scope's `role` and `organisation_type` (#5691, our own design).
   An actor scope's `organisation_type` also matches the coded name of the
   `ORGANISATION` a system client acts for (`FactKind::OrganisationType`,
   `party/0046`). The `ROLE` is the
   assignment and the rule the permission, the split of the XACML
   Hierarchical RBAC Profile. A SCIM User's `roles` (RFC 7643 §4.1.2) are
   such `ROLE`s (#5625, `service::demographic::user_roles`): each value one
   `ROLE` of the professional role template its `PERSON` performs, named by
   the value coded in the `role_terminology` of the client's `scim_provision`
   grant and limited to the `role` codes that grant lists; a removed value
   ends its `ROLE` the day before and leaves the `PERSON`'s `roles`, and the
   caller facts decide the next request. The `ROLE`s carry no capability, so
   the SCIM client writes them and only an administrator grants. The SCIM
   client is the Role Enablement Authority of the XACML profile; the policy
   holds the permissions. Clearances and permitted purposes are rules
   too: the capability codes `sensitivity_clearance` and `permitted_purpose`
   and their caller facts are gone, and a purpose limit is a deny rule with a
   permit exception. Which templates a caller commits is a rule as well, for
   every caller: the `template` element of a `commit_composition` grant is
   gone. A federation agreement's `template` elements stay the contract with
   the partner, and the rules hold for its persons too, deny winning. Where a
   decision depends on the datum, the route guard judges a named object, and
   the commit a created version, before any step about the EHR alone decides
   (#5627). A
   directive applies by `applies_to` (every attribute of an actor scope
   holds), `operations`, `purposes` and `time_validity`; a fact that cannot
   be read (a validity bound with no offset, an undeclared purpose, an
   unstamped origin) is in force for a deny and not for a permit. A verdict
   is an ordered list of rules over the EHR's data (objects, templates, root
   archetypes, categories through the category map, origins, commit time,
   sensitivities), so one verdict answers a named object, the EHR itself,
   and every row of a query: the version reads, contributions, exports, the
   subject lookup and the write guard judge each object by its head row
   (`storage::access_facts`), and a query renders the verdict of every
   settings class, of an EHR held closed and the default for an EHR with no
   settings as a condition on the head row and the EHR id of its one SQL
   statement (`aql::sql::access`), so `COUNT`, `LIMIT` and paging count only
   readable rows. A class verdict holds an atom wherever it reads a fact of
   the class (`scope::Held`, `scope::EhrFact`), which the SQL reads from the
   EHR's `access_settings.facts`, so one evaluator decides a direct read and a
   query alike, and an EHR of a class the condition does not hold fails the
   statement, which the service judges again (#5794). Care is read in one of three forms, chosen per query once the
   service knows which EHRs it reads (`Care`, #5699). A query pinned to the
   EHRs it names (an `ehr_id` scope, or `ehr_id/value` held equal in the
   `EHR` source's predicate or the `WHERE` clause) has each EHR's care
   decided from the evaluator's copies and binds only the cared EHR ids. Any
   other query reads the clinical care projection (`clinical/0009_baseline_access.sql`):
   the care relationships of the party domain, one row per subject reference
   and relationship, written by each replica's care projector
   (`service::ehr::care_projection`), which reads the party domain on the
   party pool in one repeatable-read snapshot and writes the clinical domain
   on the clinical pool, so no role reaches both. A row holds the `PERSON`
   and team container UUIDs, the care dates and the SHA-256 digest of each
   subject reference that names the `PERSON`; the EHR row carries the digest
   of its own reference, written with it, and both digest columns are held to
   32 octets. A subject reference is a pseudonym, so the digest needs no key
   (#6092, `clinical/0056`). The probe binds
   the caller's team ids, the day and the grace, one index probe per EHR row.
   A replica reads the projection only while the facts revision it reflects
   is at least the one of the facts the replica holds, so a discharge, a
   removed membership or an erasure is never granted from a stale row: until
   the projector applies it the query binds the census, the subjects of every
   patient under the caller's teams as a `jsonb` object in a `WITH` clause
   (#5645). Erasing a `PERSON` deletes its rows in the erasing flow. Our own
   design. A query pinned to one EHR is
   decided before its statement (`access_eval::condition::pinned`, #5746): the
   evaluator judges that EHR from its copy as a direct read of it is judged,
   and the statement carries only the part of the verdict that reads each
   object, the EHR id compared with a constant, and one uncorrelated check,
   which PostgreSQL runs once, that the revision of the EHR's settings row
   and subject in force is the one the verdict was judged on, so a copy older
   than the store answers no row: the check fails the statement (SQLSTATE
   `ZA001`, `clinical/0009_baseline_access.sql`), and so does a row of an
   EHR whose settings class the condition does not hold (`ZA002`). The
   service then drops the stale copy, judges the query again from a fresh
   read and runs it once more; when the copy has not caught up within 100 ms
   it refuses with `503` and the problem type `access-refreshing`, so a
   stale copy never answers fewer rows, a smaller count or a shorter page
   (`service::query::stale`, #5794). Whether the caller reads that EHR's
   `EHR_ACCESS` objects is decided from the same copy, so no settings row is
   probed for it either. Every other query keeps the class condition
   and its probe of the settings row per object, once per row (#5782): a root
   linked to an `EHR` source has that source's id as its EHR id, and since an
   admitted object is one its EHR reaches, the head condition of the linked
   root decides the source's EHR too, so neither repeats the probe. The head
   row carries the root archetype of the trunk head (`vo_head.root_archetype`,
   written by every commit and import, `clinical/0003_baseline_change_control.sql`),
   which an archetype or category scope compares instead of probing the root
   node row. A condition whose every known class, closed verdict and default
   admit every item, an administrator's, reduces exactly to "the EHR carries
   no settings row of a class the condition does not know", one uncorrelated
   subquery PostgreSQL hashes once, which fails the statement for a row of
   such an EHR as the full condition does. Before it renders a verdict, the emitter
   folds what the SQL would decide alike. A repeated member counts once. A
   two-valued predicate beside its own negation decides the conjunction or the
   disjunction (a fact of the EHR, which may be unknown, is never folded). A
   scope that names an empty list covers nothing. Each `EXISTS` folded away is
   one subquery fewer to plan (`aql::sql::fold::simplified`, #5667). The request path
   adds no transaction: every fact is a copy, and a named object is read by
   its head row only when the verdict depends on the object. The procedure,
   its order and its codes are our own design; RM EHR IM §EHR Access only
   requires every decision to follow `EHR_ACCESS`. The access record names
   each decision's grounds (`access_eval::record`, #5514): each rule keeps
   the positions of the directives it rests on and each verdict what it read
   (the permits, a care relationship, a gate-keeper, the denies an emergency
   lifted), which `RequestAccess::decision` renders as draft
   `ACCESS_AUTHORISATION`s naming the `EHR_ACCESS` and `ACCESS_POLICY`
   version uids, and a refusal, the `404`s of steps 1 and 2 included, as its
   code with the step and the directive or policy that decided it (EHDS
   Annex II 3.2, Art. 8). The adapter publishes the request's evaluator on
   the response, the audit layer stamps each record with the decision on its
   EHR into its entry's `authorisations` and `outcome`, and every projection
   reads the versions back from the stored entry
   (`access_log::recorded`) into `agent[user].policy` of the EHDS record. A query's
   condition groups verdicts with their grounds stripped (`Verdict::bare`),
   so recording adds no SQL. Each served row says whether its EHR carries
   settings (a hidden column beside the access-EHR column), and the record of
   a served EHR that does is judged from the per-EHR copy, with its subject
   and `EHR_ACCESS` version uid (#5581), so it names the grounds a direct read
   names; the EHRs the copy does not hold are read in one statement. A served
   EHR with no settings needs no read, and its care ground is inferred from
   which of its two verdicts could have served the row (our own design).
   Every EHR a query touches gets an entry (#5639): an EHR its scope names,
   whatever its rows hold, and, for an aggregate projection, each EHR whose
   data the aggregate read, carried per row as a hidden
   `array_agg(DISTINCT …)` column in the same statement; a refused query is
   recorded against the EHR it asked for. An emergency's lifted denies and
   lifted controller deny rules carry the items they cover, and an entry
   naming the objects it admitted names only the lifted denies and rules
   covering one of them (#5630, #5676). A record the service
   writes itself, such as an EHR Extract export's, names the request and the
   caller's binding, which the adapter publishes for the request (#5638).
3. **RBAC** — the coarse operation class (public / clinical / admin /
   management) against the caller's roles, plus the read-only restriction, which
   overrides any grant. Roles come from the RFC 9068 §2.2.3.1 claim carriers; an
   OAuth2 scope is NOT a role (RFC 6749 §3.3).
4. **SMART scopes** — off by default, AND-composed onto the decisions before
   it, so it can only narrow. Disabled SMART produces zero wire drift.

**Deny-by-default and fail-closed, in both senses.** No stage permits by
omission: a gate reached without a principal refuses. And a stage that cannot
DECIDE is never read as a decision — an unreachable token issuer is `503`, a
fact the evaluator cannot read never opens data. FerroEHR evaluates no policy
language and calls no remote policy decision point (#5487), since either
would be a second policy world that receives none of the openEHR access
facts, and ITS-REST 1.1.0 mandates no authorization scheme
(`Requests_and_responses.md` §Authentication and authorization). Silence is never consent, and a broken control never looks
like a policy outcome.

Configuration is boot-validated rather than degraded at the first request: a
mandatory audience list, an `https` issuer, an algorithm set bound to its key
source, an HMAC entropy floor, the OWASP Argon2id parameter floor, and a policy
rule per resource kind the PEP consults.

### ABAC as openEHR content: the design record (#5487)

The question #5487 put was whether the attribute-based rules, which a Cedar
policy register held through 5.7.1, could become openEHR content beside the
coded roles and capabilities of the RBAC model. The owner approved the
design on 2026-11-13 and ruled the same day that the remote PDP goes too.
What ships is described in the access layer above; this record answers the
issue's questions and says where each answer rests on a specification and
where it is FerroEHR's own.

**The vocabulary is NIST SP 800-162** (Guide to Attribute Based Access
Control Definition and Considerations, January 2014, §2.2 and §2.3,
<https://doi.org/10.6028/NIST.SP.800-162>): access is decided from "assigned
attributes of the subject, assigned attributes of the object, environment
conditions, and a set of policies that are specified in terms of those
attributes and conditions". FerroEHR places each term in openEHR content:

| NIST SP 800-162 | FerroEHR | Where it lives |
|---|---|---|
| Subject attributes | the caller facts: the coded `name` of each `ROLE` in force it performs, its care-team memberships, the `ORGANISATION` its client acts for with that organisation's coded type and `GROUP` memberships, its caller kind, its recorded representation | the demographic service (RM Demographic IM), held per replica by `party_binding::grants::live` |
| Object attributes | the data facts: the object, its template, root archetype, category through the category map, origins, commit time, and the sensitivities its labels and the controller's classifications give it | the record, read by its head row (`storage::access_facts`) |
| Environment conditions | the request instant, the declared purpose of use, an emergency purpose | the request |
| Policy | the subject's directives and labels; the controller's rules, classifications and requirements | `EHR_ACCESS.settings` per EHR, `ACCESS_POLICY` per controller |
| PDP | the fixed evaluator | `access_eval` |
| PEP | the route guard, the AQL condition in the query's one statement, the commit's write gate | `extensions::access`, `aql::sql::access`, `versioning::restriction::WriteGate` |
| PIP | the per-replica copies, kept fresh by `LISTEN`/`NOTIFY` | `policy_register::listen::Listened` and its subscribers |
| PAP | a CONTRIBUTION of an `EHR_ACCESS` version, the admin `ACCESS_POLICY` resource, the Demographic API and SCIM | the commit paths that version each change |

**What the RM offers, and what it leaves open.** RM EHR IM §EHR Access
(`docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc`) makes
`EHR_ACCESS` the authority: "All access decisions to data in the EHR must be
made in accordance with the policies and rules in this object", its changes
"versioned via the normal mechanism". Its `settings` is an
`ACCESS_CONTROL_SETTINGS`, abstract and "Currently implementation
dependent", with `scheme` naming the subtype in use. BASE Architecture
Overview §Access Control, Overview
(`docs/specs/openehr/BASE/docs/architecture_overview/master07-security.adoc`)
gives the reason: "there is currently no published formal, proven model of
access control for shared health information", so each model is a subtype
and "the 'scheme' in use at any given time is always indicated in the EHR
Access object". The same chapter supplies the facts the design reads:
consent controls sharing (§Privacy, Confidentiality and Consent), the
gate-keeper is "usually the patient for mentally competent adults",
privacy levels are defined "by standards or agreements within jurisdictions
of use" and access is bounded by the episode of care "plus some further
time" (§Security Policy, Access Control), and roles resolve to identities
"in each care delivery environment, not in the EHR" (§The Problem of
Roles). The RM Entry package records a patient consent as an `ADMIN_ENTRY`
(`master08-entry_package.adoc` §Standard Clinical Types of Data); a
directive points at it through `evidence` and never takes its place. openEHR
gives no concrete settings class, no consent model, no rule language and no
home for a policy that holds for every EHR. CKM archetypes for consent or
privacy were not consulted, and nothing depends on one.

The answer is a concrete subtype, `ACCESS_DIRECTIVE_SETTINGS`, under the
scheme `access_directives`, and a controller root, `ACCESS_POLICY`, both
classes of FerroEHR's draft package `org.openehr.rm.access_control`
(`tools/openehr-codegen/draft/bmm/openehr_access_control_0.1.0.bmm.json`,
generated into `openehr_rm::draft::access_control` by #5506). The settings
hold the default disposition, the gate-keepers, the subject's directives
(consent permit and deny, restriction, opt-out, delegation) and sensitivity
labels on single objects, so a patient's choices are versioned with the EHR
and travel in an EHR Extract (`X_VERSIONED_EHR_ACCESS`, RM EHR Extract IM).
Their codes are those of the draft `openehr_access` terminology and the HL7
code systems it reuses (#5507, #5655). FerroEHR never reads an archetype id,
so a deployment's own archetypes are first-class. The draft is written up as
an openEHR RM change request under `docs/specifications/access-control/antora/`
(#5519); nothing is submitted without the owner's approval. Our own design:
the scheme, every class of the package and the terminology.

**Facts with a fixed evaluator.** The CDR holds the facts the decision
reads, and one Rust evaluator holds the procedure. A rule language inside
archetyped content would make a record's meaning depend on a program stored
beside it. Facts keep the meaning in RM attributes and codes that any
openEHR system reads. They map onto FHIR `Consent` and IHE APPC, project at
commit into the columns an AQL statement filters on (so `COUNT` and paging
stay exact in one statement), and name the directive and policy version
behind each refusal. No rule can error at evaluation or open data through
an unintended permit. The cost: a deployment cannot invent a new kind of
condition. It combines the attributes the model has (scope, exceptions,
effect, time, purpose, sensitivity, care), and a new kind of condition is a
new code or attribute in the model. Our own design.

**The decision procedure** is the one above: step 0 (subject,
representation, consent reference), then opt-out, restriction, consent
deny, purpose, controller rule, sensitivity, care relationship and the
default. Deny overrides permit, one level of exceptions of the opposite
effect narrows each directive, and every refusal has a `refused-*` code
recorded with its step. Deny-overrides across an EHR is the deny-overrides
combining algorithm of XACML 2.0 (`docs/specs/xacml/`, Appendix C.1), the
algorithm the APPC example uses. The order, the codes and the single level
of exceptions are our own design.

**Standards and regulation, one model for exchange.**

- **EHDS** (Regulation (EU) 2025/327, `docs/law/eu/ehds/text.html`): Art. 8
  is the `restriction` directive, invisible to the provider (`404`, and the
  settings absent from every read but the subject's, the gate-keepers' and
  the administrator's); Art. 11(5) is its `emergency_override`, lifted for
  `[authz.rbac] emergency_access_role` with an emergency purpose and logged
  with the emergency mark; Art. 10 is the `opt_out` directive for access
  services, Art. 10(2) its emergency lift and recital 55 an override held
  false; Art. 4(2) is the representation `PARTY_RELATIONSHIP` narrowed by
  `delegation` directives; Art. 11(3) is the controller's rules; Annex II 3.4
  is the categories and origins a data scope names; Art. 9 and Annex II 3.2
  are the access record with its grounds. Art. 3(3), the delay of the
  person's own access, stays the access-delay register: a controller rule on
  categories, not a patient's choice. It holds wherever the caller is the
  record's person: on every EHR for a request that acts for the person, and
  on each EHR whose subject the caller's bound `PERSON` is, which a query
  across the population checks per row against that `PERSON`'s subject
  references (#6108). Art. 3 is why the subject skips every
  step on their own EHR.
- **GDPR** (`docs/law/eu/gdpr/text.html`): care runs on Art. 9(2)(h) with
  Art. 6(1)(c) or (e), so a directive in care is often a choice under
  national law and no GDPR consent. A directive and a controller rule carry
  their ground in `legal_grounds`, and a refusing rule's grounds are
  recorded as a `legal obligation` ground.
- **ISO 22600-1 and -2:2014,** by their public scope only
  (`docs/specs/iso-health-informatics/PROVENANCE.md`, not vendored): the
  three fact sets are a patient's policy per EHR, a controller's policy and
  the role and delegation model of the demographic service. Nothing is
  checked against the standard's text.
- **HL7 FHIR `Consent` (R4, R5), IHE BPPC and IHE APPC** (vendored by #5505
  under `docs/specs/fhir-consent/` and `docs/specs/ihe-consents/`, XACML 2.0
  by #5525): a directive with one level of exceptions is FHIR's "base policy
  ... followed by a listing of exceptions" (§6.2.3 of both releases); a BPPC
  acknowledgement names an `ACCESS_POLICY.catalogue` entry by its policy
  code (ITI TF-3 5.1.2.2.6); an APPC policy maps by its base rule and its
  rule-combining algorithm (APPC TF-3 5.6.2.1). FerroEHR implements none of
  them (owner decision of 2026-11-14 on #5517): FerroBRIDGE translates,
  reaching the directives through FerroEHR's `EHR_ACCESS` reads and the
  CONTRIBUTION write. The element-by-element mapping is the draft
  specification's `mappings.adoc`.

**Cedar's place: removed.** Cedar, its policy register, its configuration
keys, schema, example policies and tests went in #5515, and the remote PDP
in #5516; a remote PDP over the openEHR access model is a later feature
(#5520). Each was a second policy world that received none of the openEHR
access facts. What a Cedar policy could say moved as follows: an
organisation condition to the acts-for relationship and its organisation
type; a patient match to the evaluator's confinement of a subject and a
representative; a template allow-list or a revoked template to a controller
deny rule with a permit exception; a break-glass role to
`emergency_access_role` with a declared emergency purpose; a read-only
client to the SMART scope layer and the operation codes; a category or
origin rule to a classification or a data scope; a rule on one caller to a
directive or rule naming that caller's party. A condition on a raw scope,
claim or `sub` string, and free boolean composition, have no form, by
design. Cedar could not express purpose, care relationships, the subject's
own directives or per-object sensitivity; the model has them. What stays in
`ferroehr.toml` is trust and the floor the policy can only tighten:
`[authz.rbac] admin_role`, `emergency_access_role` and
`ehr_access_default`, and `[audit] emergency_purpose_codes`.

**Operations.**

- **Versioning.** An `EHR_ACCESS` change is a version in a CONTRIBUTION (RM
  Common IM change control); `ACCESS_POLICY` is one `VERSIONED_ACCESS_POLICY`
  committed the same way; a caller fact is a version of a demographic party.
- **Audit.** Every decision, a refusal included, is one access-log entry
  naming its step, code and grounds as draft `ACCESS_AUTHORISATION`s with the
  `EHR_ACCESS` and `ACCESS_POLICY` version uids (#5514, #5581).
- **Emergency.** Lifted only where a directive's or rule's
  `emergency_override` holds, only for a declared emergency purpose, and
  always marked.
- **Performance and freshness.** The request path adds no transaction: the
  settings, their classes, the caller facts and the policy are per-replica
  copies, dropped on the `NOTIFY` of the commit that changes them, and read
  per request while a listener is down. NIST SP 800-162 §3.3.1 (Attribute
  Caching) names the risk of a copy: "a subject's access privileges may have
  changed since the last refresh". FerroEHR closes it. A verdict carries the
  revision of the EHR's settings row and subject, and a read that finds
  another revision refuses and marks the copy stale (#5589). A query pinned
  to one EHR checks the revision once in its statement, `access_in_force`
  raising SQLSTATE `ZA001`, and a query reaching an EHR of a settings class
  its condition does not hold fails through `access_class_unknown`, `ZA002`
  (`clinical/0009_baseline_access.sql`). The service then judges again
  from a fresh read, and answers `503` `access-refreshing` when the copy has
  not caught up within 100 ms (#5794). The care projection a population
  query reads is used only while the party facts revision it reflects
  covers the one the replica holds; otherwise the query binds the census
  (`service::ehr::care_projection`, #5645). A stale copy therefore never
  admits under old facts and never answers fewer rows. Our own design.

**Our own design, in one list:** the `access_directives` scheme and every
class of the draft package; facts with a fixed evaluator; the decision
procedure, its order and its `refused-*` codes; care relationships through
care-team `GROUP`s and `under_care_of` relationships, with the grace after
discharge; roles read from a `ROLE`'s coded name and organisation types from
an `ORGANISATION`'s, with clearances and permitted purposes as controller
rules (the capability codes first proposed for them are gone, #5621);
`ACCESS_POLICY` as a versioned root outside the EHR and the demographic
service, and its extension resource; the `EHR_ACCESS` read and write rule
and its wire reads; the projection with its classes and facts, the copies,
their invalidation and their staleness checks.

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
`openehr_access`, a `PARTY_RELATIONSHIP` whose `name` or `details`
hold `openehr_access::acts_for`, or a `DV_IDENTIFIER` of type
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
holds the groups the ABAC design of #5487 reads, coded in the draft
terminology `openehr_access` that the access control change request proposes
to openEHR (#5655): `access default`, `access directive kind` (with
`controller_rule`, #5621), `access effect`, `access relationship`, `access
caller kind`, `access operation`, `care relationship`, `group kind` (#5617),
`representation`, `access refusal` (with `refused-policy-denied`, #5621) and
`access log withheld` (#5570), and the access log's `access action`, `access
outcome`, `accessor kind`, `access authorisation`, `access refusal
reason` and `credential binding` (#5660), which codes `ACCESSOR.binding`
from the party binding outcome (`BindingCode::term`). Each is a closed set of terms read into a typed term (`Term::read`)
by terminology id and code. A term whose meaning an HL7 code system
publishes is that code under the code system's id: `HL7_ConsentProvisionType`
`permit` and `deny`, `HL7_SecurityRoleType` `datasubject` (the subject
caller kind and the subject-of-care accessor kind), `HL7_RoleClass` `MBR` (a
care-team membership) and `HL7_AuditEventAction` `C`, `R`, `U`, `D`. A value of
a terminology the group holds no term of, or a code outside the group, is
refused. The operations gain the grants `write_care_relationship` and
`write_access_policy`, and the care relationship codes are qualifiers, never
operations. Role, organisation type, sensitivity, category, purpose and
legal ground stay in the terminology the jurisdiction names and are compared by
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
(`party/0002_baseline_change_control.sql`), which the commit of a policy
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
terminology, the draft `openehr_access`, in groups of FerroEHR's own that
the change request does not propose (`party_binding::grants::operation`): a `CAPABILITY` grants an operation when
its `credentials` hold, at any depth, an element whose value is a
`DV_CODED_TEXT` of that terminology with an operation code
(`commit_composition`, `import_ehr`, `import_ehr_extract`,
`export_ehr_extract`, `federated_person_access`,
`federated_organisation_access`, `read_subject_audit`, `access_service`,
`data_origin`, `read_change_feed`, `scim_provision`,
`hold_roles`, `hold_scopes`, `create_at_login`, `query_system_wide`,
`access_demographic`); elements named by the
qualifier codes `not_before` and `not_after` bound it finer than a day,
`template` (a federation agreement's), `facility` (an element of a `DV_IDENTIFIER`, or a cluster of
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

**Key spaces** (#5688). A key row under an issuer depends only on whether
the mapping lists the issuer and whether it is a `patients` issuer, so each
key issuer is a key space of its own (`party_binding::key_space`, migration
`party/0047`). `binding_state` holds the digest of the rest of the mapping,
which a boot compares and `party-binding refill` alone changes; each
`binding_lease` lists the key issuers its replica derives keys for; and
`binding_key_space` records each issuer with its portal flag and state. A
replica naming a new issuer records it `pending`; once every live lease lists
it with one flag, a replica's renewal tick wipes its key rows and fills them
from the party versions in pages under a gate every party commit takes shared,
and it turns `active`. A live lease that does not list an `active` issuer
turns it `retiring`; a `retiring` or `pending` one no live lease lists goes
with its rows, and a `retiring` one every live lease lists again fills anew.
The lookup reads the key space's state in its one statement and binds only
under `active`, so an issuer no key space records binds nothing: a closed issuer's login is `503` while pending or filling,
so just-in-time creation never runs before the keys are read, and
`refused-unbound` while retiring. Every live replica therefore maintains
every key space a login is read from. **Our own design.**

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

**The login's binding** (#5771) is held the same way. `LiveBindings` keeps
each login key's lookup of the side table (its parties, their states,
markers, groups and represented subjects, under the date its states were
judged on) and each bound `PERSON`'s name, read on a miss, listening on
`ferroehr_party_binding`. Triggers of `party/0051` notify it with an empty
payload on any change of a key row, a membership, a representation or a key
space's state, which drops every lookup, and with a `PERSON`'s container on
a write of its version, which drops its name. A request whose own commit
changed the side table drops the copy before it answers, so a deprovisioned
key is refused on that replica's next request and on another's once the
notification arrives; while the listener is down every request reads the
store. A request whose caller is held makes no party-store statement to
resolve it. **Our own design.**

**Caller facts** (#5511). Beside the grants, the side table holds what the
access evaluator reads about a caller (migrations `party/0039`, `party/0042`;
`party/0044` keys a membership by its code, `MBR`):
`binding_caller_fact`, one row per `ROLE` whose `name` is a `DV_CODED_TEXT`
of the jurisdiction's terminology (the role its performer holds while the
`ROLE` is in force, read with `ForeignCode::read`, #5621), per `ORGANISATION`
whose `name` is one (its own type, `organisation_type`, `party/0046`, #5691),
and per care
relationship a `PERSON` holds (an inline
`PARTY_RELATIONSHIP` whose `name` or `details` carry a membership, HL7 v3
`RoleClass#MBR`, or `openehr_access::under_care_of`, RM demographic `party_relationship.adoc`: `source`,
`target`, `time_validity`), each with the term of its holder. The patient's
`under_care_of` team ids are projected at the commit of the patient's
`PERSON`. A commit whose facts changed moves `facts_revision` beside the
grant revision and stamps every party whose facts changed in
`binding_caller_fact_party`; a replica whose grant revision is current reads
only the facts of the parties stamped since its copy, one indexed read
(`grants::caller::store::FACTS`), so an admission costs each replica one
small read and the request path still adds no transaction. A refill and an
erasure move the grant revision, which reads every fact again. Migration
`party/0042` returns the grant revision to the never-filled state, so the
next boot refills the facts of the `ROLE`s already stored before it serves,
with no command run (#5675).
`Capabilities::facts()` gives a request's caller its roles and care teams in
force. The replica's copy holds the facts of callers
alone: a patient's `under_care_of` relationships grow with the record, so
`grants::care::CareCopy` holds them in two bounded caches (#5615), a
subject's relationships (`LiveGrants::under_care_of`, at most
`care::SUBJECTS` subjects) and a care team's patients
(`LiveGrants::cared_for_by`, teams whose censuses take at most
`care::CENSUS_BYTES` together, each weighed by its patients and the record
subjects they declare, #5760, read by `ix_binding_caller_fact_care_team` of migration
`party/0041`). Each is read on a miss, one indexed statement, and dropped
when the facts read of a notification stamps a party it holds or a team the
party is now under the care of; a grant reload and a listener that connects
or loses its connection drop both. The relationships carry their dates, so
the evaluator applies the grace after a discharge. A `PERSON` version that changes its care relationships commits
only for a caller holding `write_care_relationship` or an administrator
(BASE Architecture Overview `master07-security.adoc` §The Problem of Roles:
the care delivery environment evaluates "care deliverer"). A care
relationship of another party kind or of its own container is refused
`422`. An `ORGANISATION`'s `MBR` relationship is its membership of a
`GROUP` (#5575): written as its own facts on the commit of the
`ORGANISATION` under the same grant rule, and resolved for a client by
`LiveGrants::memberships` of the provider its acts-for relationship names,
which `Capabilities::facts().provider_groups()` carries and the request's
caller adds to the parties it acts for, one level deep, from the copy held,
so the regional sharing example adds no transaction. The care check
(`grants::caller::check_care`) compares what each care relationship of the
committed body says (kind, target, `time_validity`), and the record subjects
it declares (namespace, id), with the last earlier version holding a body, never its position in `relationships` (#5640) and
never the derived `binding_caller_fact` rows, whose lifecycle state and
deleted-head rule made a SCIM deactivation or deletion read as a care change
(#5628). A logical deletion holds no body and changes none; a SCIM
deactivation keeps every care relationship and drops only plain Group
memberships, so revocation needs no grant, and the binding refuses the login
on its next request (`refused-inactive`, `refused-deprovisioned`) before the
evaluator runs. A `write_care_relationship` roster whose version changes the
lifecycle state is no roster (`protect::care_writer_roster`), and an
`inactive` version takes no update (RM common master06 §Version Lifecycle),
so the ADT system writes no version of a deactivated professional and cannot
reactivate one. Its memberships decide nothing while the binding refuses the
login; a SCIM deletion (master06 §Logical Deletion) or a write after
reactivation cleans a leaver's roster up. On a SCIM User's protected `PERSON`, a version whose only change
from its predecessor is to its care relationships (the two stored bodies
compared with every care relationship and the `uid` taken out,
`grants::caller::only_care_differs`) passes the party protection for a
holder of `write_care_relationship`, and the care check decides it, so the
ADT system rosters a provisioned professional (#5617). A SCIM Group created
with `careTeam` in FerroEHR's Group extension is a care team: its `GROUP`
is named by the coded `openehr_access::care_team` (group `group kind`) and
each membership by the coded `HL7_RoleClass::MBR`, both bound in the default
definitions' `term_bindings`, since RM demographic `PARTY.type()` and
`PARTY_RELATIONSHIP.type()` are the `name` (`Type_valid`); creating
one needs the client's `write_care_relationship` grant, and its membership
changes meet the same care check. **Our own design:** no openEHR spec gives a
role or a relationship an access meaning, and RFC 7643 §4.2 leaves a
Group's meaning to the provider.

**Record subjects** (#5634). The evaluator finds the `PERSON` of an EHR's
subject with `LiveGrants::person_of`, from the namespace and the id of
`EHR_STATUS.subject.external_ref` (BASE `OBJECT_REF`: both name the service,
so both are compared): a reference under `[authz.party_binding] namespace`
whose id is a UUID names that `PERSON` container; any other names the
`complete` or `incomplete` `PERSON` that declares it by an element of its
`PARTY.details` named by the `record_subject` code of FerroEHR's operation
terminology whose value is
a `DV_IDENTIFIER` (`issuer` the namespace, `id` the id; RM Demographic IM
§2.1.3 puts an identifier another organisation issued in `details`, RM EHR IM
§EHR Status puts the cross-reference outside the EHR); otherwise there is
none and care fails closed. Migration `party/0043` projects the elements into
`binding_record_subject` at the commit of the `PERSON`, beside its care
relationships, and stamps the `PERSON`. The care copy (#5615) holds each
resolution in a bounded cache read on a miss, one indexed statement, and a
team's census carries its patients' record subjects; the facts notification
drops every resolution that names a stamped `PERSON` or a reference a stamped
`PERSON` now declares, so a hit adds no transaction and no identity-source
call; the unique constraint `(namespace, subject_id)` is deferred to the end
of the commit (`caller::subject::settle`), so a merge moves a pseudonym from
one `PERSON` to another in one demographic contribution in either order, and
a reference two `PERSON`s declare is `409`. An element that is no
`DV_IDENTIFIER` with an issuer and an id, sits outside a `PERSON`'s `details`,
names the CDR's own namespace, or names a `[privacy] subject_namespaces`
namespace with an id the subject rule refuses is `422`; writing one takes
`write_care_relationship` or an administrator. `cared_for_by` answers the
subject references of the cared `PERSON`s, which a query's care probe matches
against `ehr (subject_namespace, subject_id)`, and a representative reaches
every EHR one of its `PERSON`'s references names (`ehr_ids_for_person`), so
`Caller::own_ehrs` and the access log's `LogStanding::Person` are sets. The
access revision (`clinical/0046`) carries the namespace with the id. **Our own
design:** the element, the projection, its uniqueness, the resolution order
and the grant.

**A care decision on a lagging replica** (#5806). The caller facts and the
care copy reach a replica through its notification, which arrives after the
commit. A verdict that care decides (the same EHR judged with and without
care differs), and a query whose resolved care binds any EHR, first compare
the replica's grant and facts revisions with `binding_grant_revision`: one
indexed read of one row on the party pool, once per request
(`RequestAccess::care_in_force`). A read of a COMPOSITION runs it alongside
the version read, and a query alongside its care read
(`RequestAccess::care_ahead`), so it adds a transaction and no latency; the
owner accepted that cost on #5891. When the copy is older, the request reads
it again (the care copy drops what changed before the revision moves), keeps
only the care teams the caller is still a member of in the copy read, and
judges care again; a copy that stays older for 100 ms refuses with `503`
`access-refreshing`. A care relationship that ended, a membership that ended
and an erased `PERSON` therefore never admit on another replica; an addition
may wait for the next request, which fails closed. The counter
`access_care_stale_total{outcome}` counts each case. **Our own design:** the
check, its place before the decision, and the wait.

**The definition copies on a replica that did not make the change** (#5948).
A replica keeps the Web Template of each stored template and the archetype
lineage index in one generation, tagged with the `definition_revision`
(`clinical/0002`) a request read. A statement trigger on `template_store` and
`adl2_artefact` moves the revision once per transaction, and the request reads
it in its read of the policy heads, so a generation older than the revision is
replaced by an empty one before the request answers, with no added statement.
A build that read the store before a newer generation was installed lands in
the replaced one. A lineage that cannot be read refuses the query with `503`.
**Our own design:** the revision, the generation and the refusal.

**The patient's EHRs** (#5648, #5649). `FerroEhrService::subject_ehrs`, the
index lookup a subject cohort makes, answers the EHR the clinical subject
names and the EHR of every other record subject of the `PERSON` that declares
it (`ehr_ids_for_person`). After a PIX merge, ITI-83 answers the survivor's
pseudonym for the merged identifier (PIXm 3.1.0 §2:3.83.4.2.2.5, FerroPIX
§2:3.104.4.2.3), and the merge contribution has moved the merged patient's
pseudonym onto the survivor's `PERSON`, so both EHRs answer. That is the
consumer's handling of a merge (PMIR 1.6.0 §2:3.93.4.1.3). A contact point's
TRC patient resolves in the party domain alone (`patient_ehrs`): migration
`party/0045` projects the `DV_IDENTIFIER`s at the `[authz.party_binding.patient]
identifier` path of a `PERSON` of the patient template into
`binding_patient_identifier` at its commit, and one indexed statement finds
the `PERSON` that carries the TRC identifier by issuer and value. Then the
`PERSON`'s references
name its EHRs. Under `local` the identifier also names the EHR whose
`EHR_STATUS` carries it. An identifier no `PERSON`, or more than one,
carries finds nothing, and the request is refused with `trc-patient-mismatch`.
So the TRC confinement makes no ITI-83 call, and an unavailable PIX Manager
refuses no contact point. **Our own design:** the projection, the match on
issuer and value, the refusal of an ambiguous identifier, and reaching a
merged patient's EHR through the survivor's `PERSON`.

**The PMIR Patient Identity Consumer** (#5693,
`service::demographic::identity_feed`). Under `[pix]` FerroEHR subscribes to
the Patient Identity Registry with ITI-94 (PMIR 1.6.0 §2:3.94), criteria
`Patient`: a Registry matches criteria against the `Patient` after a change,
so a narrower criterion would miss the unlink that removes what it matches.
One replica at a time checks the subscription under an advisory lock: it
reads the status at the `Location` the store records (§2:3.94.4.3) and
creates one when none is recorded or the Registry no longer holds it. The Registry posts each
ITI-93 message (§2:3.93) to `POST {public root}/pmir/feed`, which admits only
the client `pix.feed_issuer` and `pix.feed_client_id` name, by its IUA token
(§2:3.93.5). The route reads the message with `ihe_iti::pmir::feed`, which
keeps the resource ids and identifiers alone, and in one party transaction
stores it in `pmir_message` by `MessageHeader.id` and enters what it splits
in `pmir_suppression`: the patient identifiers an update or a delete no longer
announces, and the record subjects a standing merge moved when a message
about the merged identity states anything but that merge. The same
transaction stamps the holders and notifies, so every replica drops its copy,
and the patient identifier lookup and both record subject reads skip a
suppressed row. The response leaves after that commit: a split closes at
receipt, before its contribution commits. One replica then applies the
messages in arrival order under a lease (`pmir_apply_next`): it locks the
message row and, in that transaction, commits one demographic CONTRIBUTION as
the feed's client (its committer and its policy grants, so the commit checks
hold it to what it may write), records the merges (`pmir_merge`, with the
record subjects moved) and the bindings (`pmir_identity`), lifts the splits,
marks the message applied and clears its identifiers, and writes the access
record naming `pmir-message:<id>`. A merge moves the merged `PERSON`'s
record subjects onto the survivor's; an unmerge, read from `active` true with
no `replaced-by` link, moves the recorded ones back; an update makes the
`PERSON`'s patient identifiers the announced ones, moving an identifier from
the `PERSON` whose identity gave it up or writing one no `PERSON` carries
under its domain's `DV_IDENTIFIER` type, which `[[pix.identifier_domain]]`
names or the patient `PERSON`s already carry that domain with. A message about a merged identity
that states neither the merge nor its revert, a linked identifier of a domain
with no `DV_IDENTIFIER` type, an ambiguous binding, a contested identifier or a
refused commit refuses the message: it keeps its splits and is logged, and
the next message applies. An operator then acts on it (#5725,
`identity_feed::operator`, `GET /admin/pmir/messages`, `POST
…/{message_id}/retry` and `…/settle`, `ferroehr pmir`): a retry puts it back
to `received` at its own `sequence`, so it applies next, under the
transaction-level advisory lock every apply holds, and is refused when an
applied message with a higher `sequence` names one of its `identities` (the
Registry `Patient.id`s each row keeps after it applies), since each
announcement states an identity's whole identifier set and the older one
would roll it back. A settlement closes it unapplied with a `decision`, the
operator (`decided_by`) and a reason: `discard` keeps its suppressions,
`lift-splits` deletes them and stamps the holders, restoring the resolution
before the message; either clears its identifiers. No settlement applies a
message by an operator's choice of `PERSON`, identifier or survivor, which
would assert an identity the Registry did not state. Every list, retry and
settlement is an access record of the demographic domain on
`pmir-message:<id>` naming the operator; the CLI's committer carries the
stated `--operator` as its subject and the process's effective user
(`os:<name>/<uid>`, read with `nix`) as its client, which the record names
beside it. The `pmir_messages_refused` gauge
and the `pmir_refused_messages` indicator (`DEGRADED` from `[pix]
refused_degraded_at` on, never withdrawing readiness) count the refused
messages. An identity is bound to the one `PERSON` that
carries one of its announced identifiers. A redelivery is stored once. **Our
own design:** all of it (PMIR §2:3.93.4.1.3 leaves the Consumer's handling to
its capabilities), including acknowledging once stored rather than once
applied, which reads §2:3.93.4.2.1's "fully processed" as durably accepted
with its effect on resolution in force.

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
server needs before it believes a `patient/` token. It never makes a caller
the record's person, which the party binding alone decides (#6080). The person-token client
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
Beside an `[[auth.party]]` table a Basic user is a person's credential
(`PartyBinding::check_basic`), so `first_admin` must map to the key of a
`PERSON`, and the deployment starts with the operator grant
(`ferroehr party-binding grant`, `party_binding::grants::bootstrap`): the
SCIM client's `scim_provision`, then SCIM provisions the staff and the
administrator's `PERSON`. No HTTP route grants an operation before an
administrator exists (#5620). **Our own design.**

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
the matching import, `read_subject_audit` the ITI-81 read of one subject's access log and
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
`federation` issuer, held against the period of the access policy's
`log_retention` (`LivePolicy::attach_grants`, #5660): the partner's issuers are
`federation` ones, one agreement per partner and accessor, a caller rate and
`patient/` reads on every agreement, a contact point's `ehdsi_service`, and
the EHDS Art. 13(4) and Annex II 3.2(b) rules of an organisation and of a
`chained` access. The admission judges the agreement again for the caller's
own issuer, so one that no longer keeps them, after its partner changed,
admits nobody. No rule is a country's: the agreement names no jurisdiction,
the consent source is `registry` or `local`, and a national exclusion such
as Wabvpz Art. 15f is a deny rule of the controller's `ACCESS_POLICY` over
the roles and facility types the caller's credential attests, refused at
step 5 as `refused-policy-denied` with the rule's `legal_grounds` (#5691;
`scripts/checks/access-decides-no-country.sh` refuses a country in the
access decision). `/management/info`
reads the agreements in force from the copy on each request. **Our own
design:** no specification gives a capability an authorization meaning or
governs a data-sharing agreement's form.

**System clients from the CDR** (#5436, #5432). The
`[[authz.system_client]]` table is gone: a system caller
(`CallerKind::System`) is what the policy parties say. Its commit into an EHR
(`versioning::system`) needs a `commit_composition` grant, or it is `403`, and a
`COMPOSITION` is admitted when one grant has the origin
the act needs (a device's registration under `device`, recognised by a
composer identifier of a `device_identifier_issuer` equal to
`feeder_audit.originating_system_audit.system_id`; any other act under
`relay`, naming the professional) and covers the facility it names (EHDS
Art. 13(4)). Which templates it commits is a controller rule (#5624). A `ROLE` whose grant lacks the terms its operation is checked by
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
`openehr_access::no_subject_namespace`, which covers those EHRs and no
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
`openehr_access::represents`, in any archetype, recognised by the code
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
A login a live `PERSON` carries under a `patients` issuer is held to that
`PERSON` (#5622): the patient only when it is the EHR's subject, a
representative only through a live representation, refused
(`refused-representation-unrecorded`) otherwise, so the CDR's facts decide
over the issuer's claim of the EHR; a login no `PERSON` carries is the
patient. The key issuers the server compiles the person mapping from and the
SCIM gate admits a `binding_issuer` from are one set,
`config::binding::key_issuers`: the `[[auth.party]]` identifier issuers and
every `patients` issuer (#5652, `installed_mapping`). A key under a
`patients` issuer is held in `binding_identifier` with no professional
`ROLE`. A SCIM client whose `binding_issuer` is a `patients` issuer
provisions the portal's representatives as `PERSON`s of the patient
template, a non-default person template of the default mapping, with no
`ROLE`, and the party protection refuses any version from a SCIM client that
adds, changes or drops a representation (EHDS Art. 4(2)). **Our own
design**, as no openEHR spec governs resolving a login to a party, SCIM
provisioning or who may write a relationship.

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
our own design (#5442). Two kinds of policy change while the CDR runs and have
no home in the RM demographic model (no `PARTY`, `ROLE` or `CAPABILITY` carries
a category map or a Member State's access delay), so each is an **admin
register**: `audit-categories` (the EHDS Annex II 3.2(c) category map) and
`access-delay` (the EHDS Art. 3(3) delay). The TOML keys they replace
(`[audit.categories]`, `[privacy.access_delay]`) are removed outright and
refused as unknown keys.

- **Storage.** One append-only clinical table, `policy_register (register,
  version, document jsonb, committer, committed_at)`. A register with no row
  holds its default (version 0): the shipped category map and the delay off.
- **Write.** `PUT {ext}/admin/registers/{register}` replaces the whole document
  over the version `If-Match` names; under a per-register advisory lock the
  write appends the next version only while that version is still the newest,
  else `412`. The document is checked before it is stored and refused whole
  with `400`: a category map must compile (and must classify something under
  the production `no_category_map` floor, which stays TOML because it must
  hold before the register is readable); the access delay's limit and
  categories must hold under the category map in force. A category write mirrors its map into the retention register's
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
  policy is recompiled against the map in force. With nothing stale a request adds no transaction,
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
- **What an operator sees.** Each listener keeps counts of its own (lost
  connections, the newest notification, the reads a stale copy or a lost
  listener caused) that `ferroehr::telemetry::access_path` reads at each
  collection, labelled by the name the `policy_listeners` health indicator
  uses. The per-EHR access copy, the evaluator (one decision per EHR a
  request reached, counted when the request's evaluator scope ends), the
  access log's appends, signatures, seals and retention, and the chain
  verification record into the same module. Every label is a closed set
  written in the crate, never an id. No specification governs the metrics:
  our own design (#5591); the book's "Access path metrics and runbooks" page
  is the operator's reference.
- **Audit records under a changing map.** The audit store keeps one render
  policy per category map and writes each record under the policy of the map
  its digest names, so a record emitted under one map and written after a
  change still renders with the categories it was classified into.

### The memory budget (`ferroehr::memory`) — our own design

Every in-memory copy and cache of a replica has a cap in entries or in bytes,
and reports what it holds as a gauge: the access copies as
`access_copy_entries` and `access_copy_size` by `copy`, the caches as
`cache_entries` and `cache_size` by `cache` (#5926). The bytes are estimates:
`memory::Weigh` weighs a key and its value inline and on the heap, a `moka`
entry adds `memory::CACHE_ENTRY` (256 bytes, measured with moka 0.12.16 at 220
to 250), a generated settings object weighs its JSON text, and an AQL plan
its `Debug` rendering. The gauges walk each cache at collection time, so the
request path pays nothing for them. The copies follow the exact-reads rule of
#5891: a copy only speeds a request up, so an entry the cap evicts is read or
built again with the store's answer, and a test per cap fills it past its
limit and shows the answer unchanged. The audit sender renders each act's
records from the subjects that act read, never from its memo a second time,
so an eviction between the two never leaves a record without its subject.
Two copies have no cap of their own: the grants copy and the caller facts
copy hold the whole grant register and the facts of every caller party,
since a row left out would decide a request without it. The book's
configuration page carries the budget table with each cap, the bytes it
allows and the sum per instance.

### The controller's access policy (`ferroehr::access_policy`) — our own design

`ACCESS_POLICY` is a class of FerroEHR's draft access control package
(`tools/openehr-codegen/draft/bmm/openehr_access_control_0.1.0.bmm.json`, no
openEHR release), and ITS-REST defines no resource for it: the store, the
resource and the in-memory copy are our own design (#5512). It holds the facts
that hold for every EHR the controller keeps: the sensitivity of kinds of data
(`classifications`), whether clinical access needs a care relationship
(`care_relationship_required`, `relationship_grace`) and a declared purpose
(`purpose_required`), and the `catalogue` of named directives a subject accepts
by reference (IHE BPPC), and the `log_restrictions` of the access log's
subject view a Member State law lays down under EHDS Art. 9(3) (#5570).
BASE Architecture Overview §Security Policy leaves
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
- **Restrictions of the subject view (#5570).** Each `ACCESS_LOG_RESTRICTION`
  names the `accessor kind` rubrics of the access log it applies to, the
  `ACCESSOR` members it withholds (`access log withheld`) and its legal
  grounds (jurisdiction codes). `access_log::view::Restrictions` reads them
  from the policy in force and `subject_view` drops those members from an
  entry of a named kind for the subject and their representative; the full
  view and the stored entry are unchanged. A write that changes them writes a
  `LogMeaning` access-log entry naming the write (the access log's recording
  rule 7), refused before the commit under a closed fail mode. The default
  archetype constrains them.

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
issues for triage. The spec-update watcher files only actionable changes
(#5364): a change that lands in a master-pinned repository is listed on that
component's one rolling issue, a Jira ticket no commit past our pin names or
whose Jira component is `Documentation` files nothing, an amendment row
counts only by the key that heads it, and a delta of rendered HTML, tooling
or amendment records alone opens no issue. No openEHR specification governs
this filtering: our own design.

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
| `ferroehr-rest` | ITS-REST protocol adapter (axum) + auth + ATNA audit middleware; `access` module = RBAC and the access evaluator; calls the concrete `FerroEhrService` | application |
| `ferroehr` | The platform library: storage, service layer (one module per domain area), AQL engine, `versioning` (change control plus the VERSION `signature` signing under `versioning::signature`), validation, templates, the full config tree, telemetry, `privacy`, `system_log`, `licence` (the boot-verified grant in force and the identifier stamp it keys), `usage_report` (FerroEHR's status, route groups and PostgreSQL state store for the `start` and `daily` installation reports that `ferrohealth-usage-report` sends to FerroPULSE, on by default, envelope v2 in FerroHEALTH/FerroPULSE `docs/report-v2.md`, vendored schema in `corpus/ferropulse/`) | application |
| `ferroehr-server` | The wiring-only binary (config → pool → migrations → service → serve); bin name `ferroehr` | application |
| `ferroehr-ext` | Optional integrations behind additive features (`multimedia`): the content-addressed multimedia store, one OpenDAL operator over S3 | application |
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
| European logging software component (Annex II 3) | FerroEHR, and FerroBRIDGE for the accesses of its FHIR facade (below) | the shared crates `ehds-logging` 0.21.0 (the Annex II 3.2 access record, the Art 14(1) categories and their classification, the Art 9(2) retention floor, the Art 11(5) emergency mark) and `ihe-atna` 0.21.0 (the BALP `AuditEvent`, the DICOM audit message, RFC 5424 syslog over RFC 5425 TLS, the ATX: FHIR Feed), from FerroAUDIT and pinned exactly in the root `Cargo.toml`; `ferroehr::system_log` (the access-event model, the audit store, what each rendering says, the sinks, and `system_log::chain_check`, the scheduled verification of the store's hash chain), the audit schema migrations in `app/ferroehr/migrations/audit/`, `ferroehr-rest::system_log` (the middleware that records every API access and classifies it), the ITI-81 `GET /ferroehr/rest/fhir/r4/AuditEvent` retrieval in `ferroehr-rest::extensions::fhir`, and the retention register in `ferroehr::storage::marks` |
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
