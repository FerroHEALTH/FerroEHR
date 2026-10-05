---
paths: ["Cargo.toml", "**/Cargo.toml", "Cargo.lock", "deny.toml", "rust-toolchain.toml"]
---

# Dependencies: the pinned set and the vetted menu

Toolchain: Rust stable **1.98** (1.98.1), MSRV 1.97, **edition 2024**,
resolver v3. Pin via `rust-toolchain.toml`.
Database: **PostgreSQL 18** (target 18.6+): `uuidv7()`, the temporal `WITHOUT
OVERLAPS` key on `linkage.subject_ehr`, the SQL/JSON path functions, and the
planner-side gains (skip scan, AIO); `docs/postgres-features.md` says which
features the code uses and which are available and deliberately not.
Extension: `btree_gist` alone, which `db::prepare` installs itself.

**The authoritative, fully-pinned dependency set lives in the root
`Cargo.toml` `[workspace.dependencies]`, and every entry there is CONSUMED by
a member** (#2815: 44 orphaned pins were deleted — an unconsumed pin is
dependabot churn nobody exercises; a deliberately staged entry needs a `#
TODO(#NNNN)`). The narrative below is therefore a MENU, not a manifest
mirror: it names both the pinned set and the vetted candidates to reach for
(do not hand-roll anything a crate here provides — auth, HTTP status codes,
OpenAPI/Swagger, etc.). A listed crate missing from the workspace table is
re-added on first consumption, version verified on crates.io/docs.rs at that
moment (versions below are as of 2026-07 and items marked *(verify)* were
never confirmed). Add a crate to a member with `dep.workspace = true`.

Web & HTTP core: `axum` 0.8, `axum-extra` 0.12, `axum-server` 0.8 (graceful
shutdown + TLS), `tower` 0.5, `tower-http` 0.7 (trace, cors, compression,
timeout, limit, request-id, sensitive-headers, catch-panic, normalize-path),
`hyper` 1, `hyper-util` 0.1, `http` 1 (status codes/headers), `http-body` 1,
`http-body-util` 0.1, `mime` 0.3, `mime_guess` 2, `headers` 0.4, `bytes` 1.

Async runtime: `tokio` 1, `tokio-util` 0.7, `tokio-stream` 0.1, `futures`
0.3, `async-trait` 0.1, `pin-project-lite` 0.2.

Auth & authz (never hand-roll): `jsonwebtoken` 10 (JWT), `oauth2` 5
*(verify)*, `openidconnect` 4 (OIDC/Keycloak) *(verify)*, `argon2` 0.5 +
`password-hash` 0.5 (password hashing), `tower-sessions` 0.15 *(verify)*,
`axum-login` 0.18 *(verify)*, `secrecy` 0.10, `zeroize` 1. RBAC/ABAC is
shipped (`ferroehr-rest::access`, incl. a `cedar-policy` path).

TLS/crypto: `rustls` 0.23, `tokio-rustls` 0.26, `rustls-pemfile` 2,
`webpki-roots` 0.26, `rand` 0.9, `getrandom` 0.3, `sha2` 0.10, `hmac` 0.12,
`blake3` 1.

OpenAPI/Swagger: `utoipa` 5, `utoipa-axum` 5, `utoipa-swagger-ui` 9 (serves
Swagger UI). Note (owner rule 2026-07-17): the vendored ITS-REST OpenAPI is
the **codegen input** for the generated `openehr-its` contract (`emit-rest`)
ONLY — and a **subordinate wire source** (owner rulings 2026-07-24 +
2026-07-28): the ITS-REST **docs text**, `docs/specs/openehr/ITS-REST/`, is
the conformance oracle and WINS wherever the two disagree; where the docs
text is SILENT, the released OAS grounds the expectation (the release's own
overview `Specifications.md` presents the OAS files as its computable
artifacts); only both-silent goes to the ambiguity register. It is NEVER
imported into or served by `ferroehr-rest`. The server serves ONLY its own
natively generated OpenAPI (`#[utoipa::path]` on every handler, composed in
`ferroehr-rest::extensions::openapi`; owner hard rule: serve only what we
generate). Any surface change updates our `#[utoipa::path]` declarations in
the same PR.

Database & persistence: `sqlx` 0.9 (postgres, macros, migrate, uuid, json,
rust_decimal, chrono; TLS via `tls-rustls-aws-lc-rs`); `sea-query` 1.0 +
`sea-query-sqlx` (the sea-query 1.0↔sqlx 0.9 binder — `sea-query-binder` is
stuck on sea-query 0.32; dynamic SQL for the AQL→SQL engine — **not**
sea-orm); `jiff-sqlx` for jiff↔Postgres on plain sqlx queries (sqlx has no
`jiff` feature; the binder's with-jiff is unimplemented upstream);
`deadpool-postgres` + `tokio-postgres` for an optional pipelined hot-read
path. Migrations: five sets (`ext`, `clinical`, `party`, `linkage`, `audit`),
each a sequence of natural files with one concern apiece and its grants in
the last file; new ones via `sqlx migrate add --sequential`, and a file the
latest release tag carries is never edited
(`.claude/rules/sqlx-conventions.md`).

Serialization & formats: `serde` 1, `serde_json` 1 (`preserve_order`),
`serde_with` 3, `serde_path_to_error` 0.1, `quick-xml` 0.41 (with
`serialize`), `base64` 0.22, `rust_decimal` 1 (+`rust_decimal_macros`) as the
BigDecimal replacement for DV_QUANTITY, `ordered-float` 4, canonical JSON via
`serde_jcs` 0.2 or a ~150-LoC hand-roll. C14N (canonical XML): `xmllint
--c14n` fallback for now.

Parsers (native ADL/cADL/ODIN/AQL): `logos` 0.16 (lexer), `chumsky` 0.13
(stable; 1.0 still alpha, repo now on Codeberg) or `winnow` 0.7, `regex` 1,
`fancy-regex` 0.18 *(verify)* for cADL backreferences; diagnostics via
`miette` 7 and/or `ariadne` 0.6 *(verify)*.

IDs / time / validation: `uuid` 1 (v4+v7, serde, fast-rng), `jiff` 0.2 (1.0
not yet released as of 2026-07), `garde` 0.23 *(verify)* + a custom
RM-invariant framework, `url` 2, `urlencoding` 2.1.3 (ALL URL/percent
encoding+decoding — never hand-roll a percent codec; owner rule 2026-07-11).

Observability (opentelemetry set is lockstep — keep equal): `tracing` 0.1,
`tracing-subscriber` 0.3, `tracing-opentelemetry` 0.33, `opentelemetry` 0.31,
`opentelemetry_sdk` 0.31, `opentelemetry-otlp` 0.31,
`opentelemetry-semantic-conventions` 0.31, `metrics` 0.24,
`metrics-exporter-prometheus` 0.18 *(verify)*, `axum-prometheus` 0.10
*(verify)*.

Caching / rate limiting / resilience: `moka` 0.12 (Caffeine equivalent for
the template/WebTemplate cache), `quick_cache` 0.6, `tower_governor` 0.8
*(verify)* + `governor` 0.10 *(verify)*, `backon` 1 (retry; the `backoff`
crate is deprecated).

Errors & utilities: `thiserror` 2 (libs), `anyhow` 1 (bins only), `config`
0.14 or `figment` 0.10, `dotenvy` 0.15, `clap` 4, `parking_lot` 0.12,
`dashmap` 6, `arc-swap` 1, `indexmap` 2, `smallvec` 1, `itertools` 0.14,
`bitflags` 2. Use `std::sync::LazyLock` (edition 2024) instead of `once_cell`
for statics.

HTTP client & external integration: `reqwest` 0.13 (rustls, json) for the
terminology/FHIR client and conformance runner; `jsonschema` 0.46 *(verify)*
to validate against the openEHR ITS-JSON schemas.

Testing & benches (dev-deps): `cargo-nextest`, `insta` 1 (snapshots — the key
tool for canonical JSON/XML parity), `proptest` 1, `rstest` 0.26 *(verify)*,
`wiremock` 0.6, `mockall` 0.15 *(verify)*, `fake` 5 *(verify)*, `assert_cmd`
2, `assert_fs` 1, `testcontainers` 0.27 *(verify)* + `testcontainers-modules`
0.12 *(verify)* (real PG 18), `criterion` 0.5 + `divan` 0.1.

Dev tooling (CI, not deps): `cargo-nextest`, `cargo-audit`, `cargo-deny`,
`cargo-machete`, `cargo-hakari`, `cargo-llvm-cov`, `sccache`; `mold` linker
on Linux.

openEHR spec versions are pinned in root `CLAUDE.md` §Code generation (RM
1.2.0, BASE 1.3.0, TERM 3.1.0, AM 1.4.0 + 2.4.0) and in `docs/VERSIONS.md`
(the single source of truth) — including the ITS-XML/REST/JSON pins.
