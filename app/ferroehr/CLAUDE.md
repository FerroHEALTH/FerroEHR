# `ferroehr` — the platform library

The application core (five app crates, zero re-exports). Top-level modules
(`src/lib.rs`): `service` (the SM service layer), `storage`, `aql` (the query
engine), `versioning` (change control + VERSION `signature` signing),
`validation`, `templates`, `db` (sqlx pools + migrations), `config` (the full
`ferroehr.toml` tree), `telemetry`, `system_log` (IHE ATNA, including the
scheduled hash-chain check `chain_check`), `support` (the release's support
period, embedded from the changelog by `build.rs`), `privacy`,
`licence`, `ids`, `extensions`, `banner`, `manufacturer` (the one source of
the manufacturer every surface names; the viewer compiles this file through
`#[path]`, so it stays dependency-free but for `serde`), `report` (the
redacted deployment report `ferroehr report` writes), `usage_report` (the outbound
installation report to FerroPULSE: typed payload, cross-replica claims on the
clinical pool, the in-process latency window fed by `ferroehr-rest`'s HTTP
metrics layer and AQL execution). Hand-written idiomatic Rust of our own design on the
generated `openehr-*` crates. The binary lives in `app/ferroehr-server`; the REST
adapter (`ferroehr-rest`) depends on this crate and calls the concrete
`FerroEhrService` directly. **Zero re-exports: every import names its defining
module.**

- **Service layer = one module per SM chapter, concrete methods, no trait
  catalog** (`service::{ehr, definition, demographic, query, validity, admin,
  ehr_index, terminology, message}`, plus `linkage` and the support modules
  `committer`, `version_update`, `status`, `response`, `list`, `error`,
  `platform_service`). SM design authority: `docs/specs/openehr/SM/`.
- **Spec first:** every spec-facing behaviour (versioning/change-control,
  validation, AQL semantics) is implemented from the vendored text under
  `docs/specs/openehr/` (`/spec-lookup`) — never from memory or EHRbase
  behaviour. Cite spec file + section in comments; the only citable references
  are the vendored specs and official external docs — NEVER an internal doc.
- **Storage is greenfield PG18, second generation:** an APPEND-ONLY `version`
  table (no validity interval, no close-out statement — validity is derived
  from `committed_at`), one mutable `vo_head` row per versioned object whose
  updated columns are in NO index so the per-commit UPDATE is heap-only, and
  one `node` table (nested-set interval index, canonical JSON fragments — no
  aliasing, no synthetic fields). `version`, `node` and `vo_attestation` are
  partitioned `BY LIST (tier)` with `hot`/`cold` partitions, so archiving is
  `UPDATE … SET tier = 'cold'` and the foreign keys carry the referencing rows
  across with `ON UPDATE CASCADE`; there are no mirror tables and no `*_all`
  views. `LATEST_VERSION` is `vo_head.trunk_head_sys_version`, `ALL_VERSIONS`
  is `version` unfiltered. Every write emits contribution + commit audit in the
  same transaction. Change-control semantics are implemented against RM common
  master06 (`RM/docs/common/master06-change_control_package.adoc`) — do not
  regress them casually.
- **Two pseudonymisation domains, one set of storage code:** the `clinical`
  and `party` schemas carry the same change-control and node relations, both
  RENDERED FROM ONE DDL TEMPLATE (`migrations/templates/*.sql.in` +
  `storage::ddl_template`, whose tests regenerate the committed files and
  refuse any drift), and the ONLY thing that selects a domain is the pool's
  `search_path` (`db::connect_domain`,
  `FerroEhrService::demographic_pool`). Never schema-qualify a domain relation
  in SQL; `service::demographic/**` and the party-scoped `service::admin` paths
  take `demographic_pool`, everything else takes `pool`. A third domain,
  `linkage`, holds the party-to-EHR map under its own `ferroehr_linkage` role.
  `db::verify_domain_isolation` is the boot gate over every runtime role
  (`ferroehr_clinical`/`_reader`, `ferroehr_party`/`_reader`,
  `ferroehr_linkage`).
- **One pool and one DSN per domain** (`db/domain.rs`): `[storage.<domain>]`
  carries `url`/`url_file` for `clinical`, `party`, `linkage` and `audit`, each
  defaulting to `[db].url`, and `db::connect_domains` opens the four
  (`db::DomainPools`). Preparation is per DATABASE — the `ext` set by whichever
  domain reaches it first, then each resident domain's own set. No statement
  ever names two domains' relations; `tests/it/one_schema_per_statement.rs`
  refuses one that does.
- **AQL engine** (`src/aql/`): typed IR over the BMM-generated RM model, lowered
  via `sea-query`; every unsupported construct is a typed reject, never a silent
  wrong answer. Rules: `.claude/rules/aql-engine.md`.
- **The instance is single-tenant.** No relation carries a `tenant_id`, no row
  policy scopes a read, and no session GUC is stamped: isolation between
  organisations is a deployment property. openEHR puts multi-tenancy at the
  layer that hosts several logical EHR systems, not inside one (BASE
  `architecture_overview/master06-design_of_the_ehr.adoc` §The EHR System).
- **SQL:** `sqlx` + `sea-query` (never sea-orm); migrations only via
  `sqlx migrate add --sequential`, and **append-only once shipped** (owner
  rulings 2026-09-09 and 2026-09-15): never edit, rename or delete a migration
  that is in a release (the latest `vX.Y.Z` tag), because sqlx checksums each
  applied file and an edit locks every installation out of its database at
  boot; a file in no release yet is fixed in place, never papered over. For a
  shipped file a schema change is a NEW file.
  Rules: `.claude/rules/sqlx-conventions.md`.
- **System log** (`src/system_log/`): the ARR drain batches (`recv_many` → one
  multi-row UNNEST INSERT when syslog is off) with concurrent memoized subject
  resolution and rate-limited drop warnings; default `audit.queue_capacity` 8192.
- **Consume `openehr-*` types directly** — never re-model the RM or re-serialize;
  canonical JSON/XML goes through `openehr-its`.
- DB tests take their database from the shared harness — `testkit::db()`
  (`tools/testkit`; template-clone per test). Never start a per-test PG container
  or run migrations in a test. Cluster-global objects a test must create (login
  roles) are named off the clone db name so the testkit sweep reaps them.
- **Benches** (`benches/aql.rs`, `benches/validation.rs`, `benches/storage.rs`;
  criterion, `harness = false`): every bench
  emits a CPU flamegraph under `--profile-time`
  (`cargo bench -p ferroehr --bench aql -- --profile-time 10` →
  `target/criterion/<bench>/profile/flamegraph.svg`). New benches copy that
  file's `criterion::profiler::Profiler`-over-`pprof` impl — never enable
  pprof's own `criterion` feature (pinned to criterion ^0.5, incompatible
  with our 0.8). Profiling how-to: the `/flamegraph` skill.
  `benches/storage.rs` is the storage harness (#3367): it takes a testkit
  database, seeds a corpus sized by `STORAGE_BENCH_CLASS` (`poc`/`s`) behind one
  `seed()` seam, times commit/supersession, point reads, `version_at_time`,
  revision history, `If-Match`, AQL CONTAINS, archive/restore and one prune, and
  writes a JSON record with the database-side facts to
  `docs/benchmarks/storage/<generation>/`. It drives everything through
  `FerroEhrService` and the public storage API and discovers relations through
  `pg_stat_user_tables`, never a table name, so it measures a rewritten schema
  unchanged. A benchmark, never a conformance record.
- **One integration-test binary:** `tests/it/main.rs` + one `mod` per topic
  file. The OPT/archetype fixtures the suites upload live in the shared corpus
  at `corpus/fixtures/service`, reached as `../../corpus/fixtures/service`
  from `CARGO_MANIFEST_DIR`; a test in this crate never reaches into another
  crate's tree either. A new suite is a module registered in `main.rs`, never
  a new top-level `tests/*.rs`. The three container suites (`events_amqp`,
  `fhir_outbound_amqp`, `multimedia_s3`) are serialized by the nextest
  `containers` group, which matches them by module prefix — renaming one of
  those modules means updating `.config/nextest.toml`.
- Gates: `cargo clippy -p ferroehr --all-targets` +
  `cargo nextest run -p ferroehr` green before commit; the CNF pipeline
  (`bash scripts/conformance.sh`) must show zero drift vs the committed baseline
  at phase close. A red row is attributed spec-first
  (`.claude/rules/cnf-triage.md`): this server is a suspect, never assumed
  correct — never bend the catalogue/runner to match it.
