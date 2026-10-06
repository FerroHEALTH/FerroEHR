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
service layer follows the openEHR SM Platform Service Model (one module per SM
chapter, concrete methods).

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
  reassembly key and never a predicate) and a **canonical openEHR JSON
  fragment** in `data jsonb` (verbatim `openehr-its` encoding: zero translation
  between storage and API, no synthetic fields). Point reads do not reassemble
  the fragments: the whole canonical body is also stored verbatim as
  `version.body text` and served from there in one statement.
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
  time, date-time and duration readers, `openehr_numeric`), the `posture`
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
- **`linkage`** — the third pseudonymisation domain: `subject_ehr`, the EHR id /
  demographic subject cross-reference (the SM's EHR Index, `I_EHR_INDEX`) —
  which party and which subject identifier name which EHR, temporal under a
  PG18 `UNIQUE … WITHOUT OVERLAPS` so a merge or split closes a row rather than
  deleting it. Identifiers and the SM's own association metadata, no attributes;
  `linkage.erase_ehr` is the `SECURITY DEFINER` function the admin delete calls
  so erasure reaches the map without the role holding `DELETE`. The service
  reaches it through its own pool, never through the clinical or party one.
- **One pool and one DSN per storage domain.** `[storage.clinical]`,
  `[storage.party]`, `[storage.linkage]` and `[storage.audit]` each carry a
  `url`/`url_file`, defaulting to the shared `[db].url`, and
  `db::connect_domains` opens the four pools each with only its own schema on
  its `search_path`. A domain whose DSN names another host lives in a database
  or cluster of its own, which is the separation a schema cannot make (a base
  backup, WAL archiving and physical replication carry every schema of a
  database together). Schema preparation runs per database: the `ext` set is
  applied by whichever domain reaches that database first, so two co-located
  domains share one `ext` and a relocated domain carries its own copy.
- **Five `NOINHERIT` runtime roles** hold the domains: `ferroehr_clinical` and
  its read-only twin hold `clinical`, `ferroehr_party` and its twin hold
  `party`, and `ferroehr_linkage` holds `linkage` alone. Each is revoked from
  the domains it does not own, in both directions, and the boot gate
  (`db::verify_domain_isolation`) refuses to serve when any of them can read
  across, when two separately-configured domains authenticate as one role, or —
  under `deployment_profile = "production"` — when a domain role is missing.
- Migrations via `sqlx migrate add` (official CLI): five sets, run per database
  in order — `ext`, then `clinical`, `party`, `linkage`, `audit` — each with
  its own `_sqlx_migrations` table, each a sequence of natural files with one
  concern apiece and its grants in the last file of the set, so a relation file
  stays a declaration of shape. The five schemas and `btree_gist`, the one
  extension the DDL needs, are created by the server before the first migrator
  runs. A database carrying the first generation's `ehr` or
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
emits no GIN-servable operator; the baseline removed the index). The feature
envelope is documented per construct; rejections are explicit typed errors.

## REST surface + auth (`ferroehr-rest`)

Base path `/ferroehr/rest/openehr/v1`, implementing the generated ITS-REST
1.1.0 server traits over `axum` with a `tower-http` middleware stack and
content negotiation (canonical JSON/XML via `openehr-its`). Extensions: admin
API (plus the own-design activity-report, archive/restore and dump/load routes under
the same gate), the `/message` group realizing the SM Message component
(EHR-Extract export/import + TDD import — the release publishes no such API),
the always-on public health family (`/health`, `/health/liveness`,
`/health/readiness`), `/rest/status`, the ops-introspection `/management/*`
surface, item tags (there is no EhrScape surface). **Auth:** Basic +
OAuth2/OIDC via `argon2`/`jsonwebtoken`/`oauth2`/`openidconnect`; authorization is the
shipped RBAC/ABAC `access` module in `ferroehr-rest`.

### The access layer (`ferroehr-rest::extensions::access`) — our own design

ITS-REST makes authentication a `SHOULD` and mandates no scheme, and the SM
places authorization out of band, so **only the 401-vs-403 split is
spec-grounded**: a missing or rejected credential is `401` with a
`WWW-Authenticate` challenge, an authenticated-but-refused caller is `403`.
Everything finer is our own design, built to the IETF OAuth2/JOSE RFCs and
expressed in the vocabulary of NIST SP 800-162 (ABAC) and ANSI/INCITS 359
(Core RBAC).

**Evaluation order** — each stage can only narrow what the previous one allowed,
which is what makes an optional stage safe to disable:

1. **Authentication** — Basic (Argon2id PHC, verified-credential cache) or
   OAuth2/OIDC bearer. Produces a `Principal` (subject, roles, scopes, the
   validated claim set) or a typed refusal. A malformed `Authorization` header is
   a `400`, not a `401`: the server never read a credential.
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
(`openehr_sdt::rm_instance`) and the SMART scope grammar
(`openehr_sdt::smart_scopes`), over the `openehr-its` wire layer.

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
RM 1.2.0 the server emits — register AMB-185), v1 when a request selects it
with the `version` media-type parameter on `application/xml` (our own
extension; no openEHR spec governs namespace selection on the REST wire). The two bundles are not interchangeable as
VALIDATORS, though — the v1 one is frozen at an older RM generation, so a
correct RM 1.2.0 document can fail against it (`docs/VERSIONS.md` §Spec
version policy; register AMB-185). Upstream spec changes and releases are detected
automatically by the scheduled watcher workflows and filed as `spec-update`
issues for triage.

## PostgreSQL 18

We target **PG 18** (18.6+). In use: `uuidv7()` for database-minted ids,
the temporal `UNIQUE … WITHOUT OVERLAPS` on `linkage.subject_ehr` (enforced
as a GiST exclusion over `btree_gist`, the one extension the DDL needs),
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
optional-integration crate — FHIR conversion core, events transport,
multimedia store — one additive cargo feature per integration, default all-on,
slim builds compile them out with loud boot refusals for enabled-but-unbuilt
integrations), and `ferroehr-viewer` (the Leptos SSR viewer — its own
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
`app/* → crates/openehr-*`. The SM Platform Service Model is realized as the
*structure* of `ferroehr::service` — one module per SM chapter, concrete
methods (no trait catalog), SM call semantics as the design authority — with
zero re-exports anywhere (every import names its defining module).

### SM platform component map

The service layer realizes the openEHR **SM Platform Service Model**
(vendored SM spec `docs/specs/openehr/SM/docs/openehr_platform/`). One
`ferroehr::service` module per SM component; the SM interfaces map to concrete
`FerroEhrService` methods (there is no trait catalog):

| SM component | SM interface(s) | Realization (`ferroehr::service`) | Status |
|---|---|---|---|
| EHR | `I_EHR_SERVICE`, `I_EHR_STATUS`, `I_EHR_COMPOSITION`, `I_EHR_DIRECTORY`, `I_EHR_CONTRIBUTION` | `service::ehr` (status/composition/directory/contributions/tags/access modules) | implemented |
| Definitions | `I_DEFINITION_ADL14`/`ADL2`/`QUERY` | `service::definition` (adl14/adl2/query_store/wire modules) | implemented |
| Demographic | `I_DEMOGRAPHIC_SERVICE`, `I_PARTY`, `I_PARTY_RELATIONSHIP` | `service::demographic` | implemented |
| Query | `I_QUERY_SERVICE` | `service::query` | implemented |
| Validity checking | `I_VALIDITY_CHECKER` | `service::validity` | implemented |
| System Log | `I_SYSTEM_LOG` (stub; "IHE ATNA-compliant") | `ferroehr::system_log` (dual DICOM PS3.15 + FHIR `AuditEvent`/BALP rendering; local Audit Record Repository in the `audit` schema, on by default; syslog + ITI-20 ATX:FHIR Feed forwarding sinks; the ITI-81 retrieval as the read side; ITI-19 mTLS via `[server.tls]`) | implemented |
| Admin | `I_ADMIN_SERVICE` (+archive/dump-load) | `service::admin` | implemented |
| EHR Index | `I_EHR_INDEX` | `service::ehr_index` over `linkage.subject_ehr` | implemented |
| Terminology | `I_TERMINOLOGY_SERVICE` | `service::terminology` (in-process `openehr-term` bundle + N simultaneously-materialised FHIR R4B servers, selected per call by an explicit terminology→provider route map with a `default` fallback; optional OAuth2 client-credentials per server; commit-time ac-code constraint-binding resolution) | implemented |
| Message | `I_MESSAGE_SERVICE`, `I_EHR_EXTRACT_SERVICE`, `I_TDD_SERVICE` | `service::message` | implemented |
| Subject Proxy | `I_SUBJECT_PROXY_SERVICE`, `I_DATA_BINDING` | not realised ([#3431](https://github.com/FerroHEALTH/FerroEHR/issues/3431)) | not realised |

| Crate | Role | Kind |
|---|---|---|
| `openehr-base` | BASE 1.3.0 | generated |
| `openehr-rm` | RM 1.2.0 — the domain model | generated |
| `openehr-am` | AM 1.4 + 2.4 (`v1_4`/`v2_4`) | generated |
| `openehr-term` | TERM classes + terminology bundle | generated + hand-written |
| `openehr-lang` | BMM/P_BMM object model | generated |
| `openehr-its` | Canonical JSON/XML + ITS-REST contract + OPT 1.4/AOM2 archetype XML + runtimes + wire validation + gates | generated + hand-written |
| `openehr-sdt` | Simplified Formats (`flat`: FLAT / STRUCTURED / Web Template / TDD) + RM-instance validation (`rm_instance`) + the SMART scope grammar (`smart_scopes`), over `openehr-its` | hand-written |
| `openehr-query` | AQL 1.1 lexer + parser + AST | hand-written |
| `openehr-adl` | ADL 2.4 engine: ADL2/cADL/ODIN parser, AOM2 validation, flattener, OPT2, ADL 1.4→2 conversion | hand-written |
| `openehr-codegen` | BMM/XSD/OAS → Rust generator (+ `emit-rm-model`) | tooling |
| `ferroehr-rest` | ITS-REST protocol adapter (axum) + auth + ATNA audit middleware; `access` module = RBAC/ABAC authz; calls the concrete `FerroEhrService` | application |
| `ferroehr` | The platform library: storage, service layer (one module per SM chapter), AQL engine, `versioning` (change control plus the VERSION `signature` signing under `versioning::signature`), validation, templates, the full config tree, telemetry, `privacy`, `system_log`, `licence` (the boot-verified grant in force and the identifier stamp it keys), `usage_report` (the `start` and `daily` installation reports to FerroPULSE, on by default, contract in FerroHEALTH/FerroPULSE `docs/report-v1.md`, vendored schemas in `corpus/ferropulse/`) | application |
| `ferroehr-server` | The wiring-only binary (config → pool → migrations → service → serve); bin name `ferroehr` | application |
| `ferroehr-ext` | Optional integrations behind additive features (`fhir`, `events`, `multimedia`): FHIR mapping/reverse/feeder-audit cores, the AMQP events transport, the content-addressed multimedia store | application |
| `ferroehr-viewer` | The Leptos SSR viewer: its own binary and OCI image, consuming the CDR strictly over ITS-REST | application |
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

**The EHR system is FerroEHR and FerroBRIDGE together.** EHDS Art 25(1) asks an
EHR system to include both harmonised software components:

| Component | Shipped by | Where |
|---|---|---|
| European logging software component (Annex II 3) | FerroEHR | `ferroehr::system_log` (the access-event model, the audit store, the DICOM and FHIR renderings, the sinks), `ferroehr-rest::system_log` (the middleware that records every API access and classifies it), the ITI-81 `GET /fhir/r4/AuditEvent` retrieval in `ferroehr-rest::extensions::fhir`, and the retention register in `ferroehr::storage::marks` |
| European interoperability software component (Annex II 2.1 to 2.3) | FerroBRIDGE | the openEHR-to-exchange-format mappings (#3206); FerroEHR's in-tree FHIR connector is retired (#3080), and the AuditEvent route above is not part of that retirement |

The two components are independent of each other (Art 2(2)(n) and (o), Art
30(1)(b)) because they are separate products that meet only over ITS-REST. The
technical documentation and the declaration of conformity name the pair;
FerroBRIDGE declares the interoperability component's requirements and
FerroEHR the logging component's and the system-level ones (Annex II 1). A
change to a module in the logging row above is a change to a harmonised
component under Art 30(2) and updates the technical documentation in the same
release.

**Points for counsel.** These are questions, and nothing in this repository
answers them:

1. Whether CRA Art 13(10), which lets a manufacturer patch only the latest
   version when earlier versions' users get it free of charge, holds for
   commercial licensees under BUSL-1.1 (the support period, #3610).
2. The scope of the EHDS Art 35 register: whether a download under the public
   licence supplies an economic operator.
3. Whether a deployment that modifies the source and puts the result into
   service becomes a manufacturer itself (CRA Art 21 and Art 22, EHDS Art 34).
4. Whether FerroEHR is an important product with digital elements under CRA
   Annex III and Implementing Regulation (EU) 2025/2392, which decides the
   conformity-assessment route (#3611).

## Build state

The openEHR-conformant CDR is shipped: the generated spec/ITS foundation plus
the full application (persistence, greenfield storage, REST + auth, the SM
service layer, templates, WebTemplate/FLAT/STRUCTURED, validation, the AQL
engine, conformance). Everything not yet built — the plugin system, further
enterprise capabilities, refinement, performance — is ordinary tracker work:
an issue with a milestone, built as compiling, tested increments of our own
design. The build record is the closed issues + PR descriptions.

## Verification

- **Fidelity gates** (spec/serialization): canonical JSON read + lossless
  round-trip + ITS-JSON schema validation; XML round-trips.
- **Conformance pipeline** (`scripts/conformance.sh`):
  [Veredictum](https://github.com/rubentalstra/Veredictum), the independent
  CNF 2.0 reference runner, over its committed machine-readable catalogue
  (Docker-composed SUT on fresh volumes) — the acceptance instrument, pinned
  in `scripts/lib/veredictum.sh`;
  results → pure-function verdicts → report/statement/certificate + badges,
  all under `docs/conformance/<sut>/` (the baseline lives ONLY in those
  committed artifacts).
- **Measured performance** (the same instrument): `veredictum perf` earns the
  volumetric deployment classes (POC/S/L/R) by open-loop,
  coordinated-omission-free sustained runs (normative hour, extendable
  2–12 h — never shorter) whose re-checkable HDR-V2 records land in
  `results.json` `measurements`, environment-bound; `veredictum stress` is
  the separate step-load exploration instrument (maximum sustainable
  throughput, `stress.json`, never a conformance record). Published SVGs +
  summaries regenerate FROM the committed artifacts
  (`scripts/render/perf-assets.sh`, CI diff-guarded).
- **Drift check** (`scripts/checks/codegen-drift.sh` + CI): the generated layer
  is always in sync with the vendored specs.
