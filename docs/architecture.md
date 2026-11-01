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
  `committed_at` serves time-range listing on the append-only heap.
- **`vo_head`** — the one mutable row per versioned object, and the only row a
  commit updates. It answers every "what is current" question in one
  primary-key probe: `trunk_head_sys_version` IS `LATEST_VERSION`,
  `head_sys_version` the RM's `latest_version`, plus the lifecycle state, the
  template, the tier, the archive marker and the restriction and retention
  marks. None of the columns a commit changes appears in an index, which is the
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
  its basic shape only). Below the root (#4525), an archetype root's
  `archetype_details` and `archetype_node_id` leave together when the details
  are exactly its own archetype id and an `rm_version` and the id is the
  recased `archetype` (`openEHR-` and the class part in capitals), with
  `rm_version` in its column, and a `language` in `ISO_639-1` and an
  `encoding` in `IANA_character-sets` leave for the `language` and `encoding`
  columns; the root row keeps its fragment, so the root reads that go around
  `ext.node_fragment` are unchanged. A key is left out only when the columns
  rebuild it exactly; everything else stays in `data` verbatim, and `data` is NULL when
  nothing remains (two thirds of the rows). `ext.node_name`, `ext.node_value`,
  `ext.node_archetype_node_id`, `ext.node_archetype_details`,
  `ext.node_code_phrase` and `ext.node_fragment`, inlined `LANGUAGE sql`
  functions, rebuild the omitted keys, so every reader sees the canonical
  fragment (`crate::storage::slim` is the write half; its Rust rebuild puts a
  restored header key where jsonb key order placed it, so a served fragment
  keeps its bytes). The AQL emitter reads a fragment
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
  `event_outbox_reader` cursors and `event_subscription` filters; and
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
- **Which EHR belongs to which subject** is the `ehr` table's subject columns,
  promoted from `EHR_STATUS.subject.external_ref` on every EHR create and
  `EHR_STATUS` update. A cohort, the AQL demographic join and the EHR index
  resolve a party or a subject through them; `service::linkage` holds those
  crossings and the access record each one owes. The linkage schema that held a
  cross-reference before is gone (`audit/0021_drop_linkage_domain.sql`).
- **One pool and one DSN per storage domain.** `[storage.clinical]`,
  `[storage.party]` and `[storage.audit]` each carry a
  `url`/`url_file`, defaulting to the shared `[db].url`, and
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
  under `deployment_profile = "production"` — when a domain role is missing.
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

A query whose outermost `FROM` class is `PERSON`, `ORGANISATION` or `ROLE` is
the experimental demographic extension (no openEHR spec governs it; QUERY
master03 §FROM gives semantics only to the clinical repository). The planner
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
   with the policies and rules in this object").
3. **RBAC** — the coarse operation class (public / clinical / admin /
   management) against the caller's roles, plus the read-only restriction, which
   overrides any grant. Roles come from the RFC 9068 §2.2.3.1 claim carriers; an
   OAuth2 scope is NOT a role (RFC 6749 §3.3).
4. **ABAC** — off by default. A policy engine (embedded Cedar, or an external
   PDP) decides over subject attributes (subject, roles, scopes, organization,
   patient) and resource attributes (patient, template), fanned out over
   multi-valued attributes as a cartesian product with **all-must-permit** and
   short-circuit deny.
5. **SMART scopes** — off by default, AND-composed onto the ABAC decision, so it
   can only narrow. Disabled SMART produces zero wire drift.

**Deny-by-default and fail-closed, in both senses.** No stage permits by
omission: a gate reached without a principal refuses; an unconfigured resource
kind on the external PDP denies (and the missing rule is a boot error); Cedar is
deny-by-default with `forbid` overriding `permit`. And a stage that cannot
DECIDE is never read as a decision — an unreachable token issuer is `503`, a
policy server answering `5xx` or a Cedar policy that errors during evaluation is
a fail-closed `500`. Silence is never consent, and a broken control never looks
like a policy outcome.

Configuration is boot-validated rather than degraded at the first request: a
mandatory audience list, an `https` issuer, an algorithm set bound to its key
source, an HMAC entropy floor, the OWASP Argon2id parameter floor, and a policy
rule per resource kind the PEP consults.

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
| System log | `ferroehr::system_log` (dual DICOM PS3.15 + FHIR `AuditEvent`/BALP rendering, written and sent by the shared FerroAUDIT crates `ihe-atna` and `ehds-logging`; local Audit Record Repository in the `audit` schema, on by default; syslog + ITI-20 ATX:FHIR Feed forwarding sinks; the ITI-81 retrieval as the read side; ITI-19 mTLS via `[server.tls]`) | IHE ATNA, BALP |
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
| `ferroehr-ext` | Optional integrations behind additive features (`events`, `multimedia`): the AMQP events transport, the content-addressed multimedia store | application |
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
| European logging software component (Annex II 3) | FerroEHR | the shared crates `ehds-logging` 0.1.1 (the Annex II 3.2 access record, the Art 14(1) categories and their classification, the Art 9(2) retention floor, the Art 11(5) emergency mark) and `ihe-atna` 0.1.1 (the BALP `AuditEvent`, the DICOM audit message, RFC 5424 syslog over RFC 5425 TLS, the ATX: FHIR Feed), from FerroAUDIT and pinned exactly in the root `Cargo.toml`; `ferroehr::system_log` (the access-event model, the audit store, what each rendering says, the sinks, and `system_log::chain_check`, the scheduled verification of the store's hash chain), the audit schema migrations in `app/ferroehr/migrations/audit/`, `ferroehr-rest::system_log` (the middleware that records every API access and classifies it), the ITI-81 `GET /ferroehr/rest/fhir/r4/AuditEvent` retrieval in `ferroehr-rest::extensions::fhir`, and the retention register in `ferroehr::storage::marks` |
| European interoperability software component (Annex II 2.1 to 2.3) | FerroBRIDGE | the openEHR-to-exchange-format mappings (#3206), and FHIR R4 resource mapping (<https://github.com/FerroHEALTH/FerroBRIDGE>, BUSL-1.1), reaching FerroEHR over ITS-REST; FerroEHR carries no FHIR mapping of its own, and keeps the AuditEvent route above and the FHIR terminology client |

The two components are independent of each other (Art 2(2)(n) and (o), Art
30(1)(b)) because they are separate products that meet only over ITS-REST. The
technical documentation and the declaration of conformity name the pair;
FerroBRIDGE declares the interoperability component's requirements and
FerroEHR the logging component's and the system-level ones (Annex II 1). A
change to a module in the logging row above is a change to a harmonised
component under Art 30(2) and updates the technical documentation in the same
release.

**When the central log moves to FerroSYS, the EHR system becomes three
products.** The owner direction of 2026-10-06 (#3661, milestone v6.0.0) makes
the access log a shared FerroHEALTH service in FerroSYS. From the release that
ships it, the logging component is split between two products, and the
declared EHR system is FerroEHR, FerroBRIDGE and FerroSYS. Until then FerroEHR
ships the whole component as the table above says.

| Part of the logging component | Shipped by | Annex II |
|---|---|---|
| Recording at the moment of access: the access-event model, the classification from FerroEHR's own data, the local durable spool that refuses an access whose record cannot be written (fail-closed), and the forwarding to FerroSYS with at-least-once delivery | FerroEHR | 3.2 (a) to (e) |
| The central store, the review and analysis tools, retention per origin and category, the natural person's access-log route (EHDS Art 9), ITI-81 retrieval and export | FerroSYS | 3.3, 3.4 |

The boundary is the forwarding: a record stays in FerroEHR's spool until
FerroSYS has ingested it, and a FerroSYS outage queues records without
blocking clinical reads. The split leaves the independence of the two
harmonised components as it is (Art 2(2)(n) and (o), Art 30(1)(b)): both
parts of the logging component sit outside FerroBRIDGE, which still meets
FerroEHR only over ITS-REST and holds no part of the log. The technical
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
