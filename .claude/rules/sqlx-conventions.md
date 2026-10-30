---
paths: ["app/ferroehr/**"]
---

# sqlx + sea-query conventions (persistence + the AQL engine — both shipped)

`ferroehr` is the only crate that talks to PostgreSQL, using `sqlx` 0.9 (driver,
pool, migrations) + `sea-query` 1.0 + `sea-query-sqlx` (the dynamic SQL builder
+ binder; `sea-query-binder` is the obsolete sea-query-0.32 pairing — do not
use it). **Not sea-orm.** Target PostgreSQL 18.6+.

## Migrations

- The schema is **our own PG18-native design**: the append-only `version`
  table with its mutable `vo_head` row, the unified `node` table, the
  tier partitions that hold the archival tier, supporting tables, and our
  `ext` helper functions. It is live and CNF-pipeline-verified. The clinical
  and party domains' change-control and node relations are RENDERED from one
  DDL template (`app/ferroehr/migrations/templates/`), so a change to either
  is a change to the template plus a regeneration, never a hand edit of the
  rendered file.
- **A migration that has SHIPPED is never edited (owner rulings 2026-09-09
  and 2026-09-15).** SHIPPED means present at a published release tag:
  the latest `vX.Y.Z`, or a pre-release `vX.Y.Z-rcN` of a version above it,
  which is published and installed like a release (#4425). sqlx records a
  checksum of each applied migration and refuses a database whose recorded
  checksum no longer matches the file
  (https://docs.rs/sqlx/latest/sqlx/migrate/struct.Migrator.html), so editing
  a file an installation has applied does not change history, it locks that
  installation out of its own database at boot. **Never edit, rename or delete
  a migration file that is in a release** — including a comment or a typo.
- **A file in no release yet is rewrite material and is fixed IN PLACE.** It
  has been applied by no installation, so a wrong name, column, grant or
  comment is corrected in the file that defines it, never papered over by a
  follow-up file, a rename migration, a placeholder role or a compatibility
  path (owner ruling 2026-09-15: the rewrite is breaking changes only). The
  rule re-arms by itself at the next release cut.
- **For a shipped file a schema change is a NEW file, always.** Altering an
  existing table, adding a constraint, backfilling a column, correcting a
  defect in a shipped migration: each is a new `sqlx migrate add` file that
  carries the change forward. A shipped migration that was wrong is
  superseded, never rewritten.
- **A shipped SET is retired whole, or not at all.** The one change to shipped
  files that is not an edit is withdrawing a whole migration set, and it is
  only safe in one shape: every shipped file under
  `app/ferroehr/migrations/<schema>/` goes, so no half-set survives for a database to apply against — a NEW set may
  take its place in the same directory, because a schema name can outlive the
  set that used it — and `<schema>` is named in `FIRST_GENERATION_SETS`
  (`app/ferroehr/src/db/mod.rs`) in the same change, so a database carrying that
  set's bookkeeping is refused at boot by name, with the remedy. An installation
  is then TOLD what happened instead of being locked out by a checksum it cannot
  interpret. A single-file edit, a partial turnover, and a turnover the boot
  refusal does not name all stay refused; a rename is judged as the deletion of
  its old path, which is what an installed database sees.
- **The refusal is a SIGNATURE, not a schema name.** Where this build owns no
  set of that name (`ehr`, `demographic`), any bookkeeping in the schema is the
  signature. Where the name survives the rewrite (`ext`, `linkage`, `audit`),
  bookkeeping exists in both generations, so the signature is the description
  sqlx recorded for VERSION 1 — the file that set was opened with. Without
  that half, an old set reaches its own migrator and fails on a checksum
  mismatch instead of the remedy.
- Enforcement (tier 4): `scripts/checks/migration-immutability.sh`, run by the
  `migration-immutability` CI job over the pull request's diff against its
  merge base, judging only the files present at the highest release tag
  in the repository or at a pre-release tag above it, whether or not the
  branch descends from it (every base file when the repository carries no
  tag).
  It fails on any modification, rename or partial deletion of a shipped file
  under `app/ferroehr/migrations/`; an added file passes, and so does any
  change to a file no release carries and a modification that restores a
  shipped file byte for byte to its released content (#4424). The whole-set
  retirement above is accepted only when the guard can verify BOTH halves itself — every shipped
  file of that directory gone at head,
  and the schema named in that table — so the acceptance cannot be claimed by a
  comment. **There is deliberately no escape-hatch label:** the checksum makes
  the rule absolute, so an exception would only ever be a broken deployment.
- Create migrations with the official CLI only:
  `sqlx migrate add --source app/ferroehr/migrations/<schema> --sequential <desc>`,
  written as modern PG 18 SQL (`uuidv7()`, temporal `WITHOUT OVERLAPS`,
  `RETURNING OLD/NEW` where the design calls for them).
- `ferroehr::db::prepare` bootstraps schemas + extensions and runs the sets in
  order (`ext`, `clinical`, `party`, `linkage`, `audit`); each keeps its own
  `_sqlx_migrations` table, and a database whose bookkeeping matches a
  `FIRST_GENERATION_SETS` signature is refused at boot as predating the storage
  rewrite.
- `sea-query` `Iden` table/column definitions (`db/iden.rs`) + hand-written
  row-mapping structs (over the generated `openehr-rm` types) — no ORM/codegen.

## Queries

- Prefer `sqlx::query!`/`query_as!` (compile-time checked) wherever the SQL
  is static; drop to `sea-query` when the AQL engine needs to build SQL
  dynamically (ASL → SQL is inherently dynamic — see `aql-engine.md`).
- Use native PG 18 features where the plan calls for them: `uuidv7()` for
  generated IDs, `RETURNING OLD/NEW` for audit/history writes, temporal
  constraints where the schema models versioned rows, skip scan/JSON_TABLE
  where they simplify AQL-generated SQL.
- `sqlx` has **no `jiff` feature** — use the official `jiff-sqlx` wrapper
  types (`jiff_sqlx::Timestamp`, `.to_jiff()`) on plain sqlx queries. On
  sea-query-built queries the binder's `with-jiff` is unimplemented upstream —
  bind via SQL (`now()`) or a chrono value at the boundary; do not silently
  switch the crate to `chrono`.
- `rust_decimal` is the `BigDecimal` replacement for `DV_QUANTITY` and other
  fixed-point RM fields; use the `sqlx` `rust_decimal` feature, not `f64`.

## Transactions and service boundaries

- One `sqlx::Transaction` per service-level write (composition create/update,
  contribution commit, EHR-status change), matching the openEHR
  contribution/commit semantics (one CONTRIBUTION per change set — the spec
  is the authority).
- Every write emits an `audit_details` + `contribution` row in the same
  transaction — an openEHR requirement: the versioning / CONTRIBUTION /
  audit semantics are defined in `docs/specs/openehr/RM/docs/common/`
  (Change Control: VERSION, VERSIONED_OBJECT, CONTRIBUTION, AUDIT_DETAILS)
  and `docs/specs/openehr/RM/docs/ehr/`; implement against that text
  (spec-adherence.md), with EHRbase as prior art only.

## Testing

- Integration tests get a real PostgreSQL 18 database from the shared
  harness (`testkit::db()`, `tools/testkit` — one server, template-clone per
  test); the template build verifies the vendored migrations apply cleanly.
  See `testing.md` for the full test discipline.

This file adds persistence-specific rules on top of `rust-style.md` (idiomatic
app code).
