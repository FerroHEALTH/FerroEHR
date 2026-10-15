# `testkit` — the shared test-database harness (tooling, not part of the app)

One PostgreSQL 18 server + one migrated **template database** per migration
fingerprint + one `CREATE DATABASE … TEMPLATE` clone per test. Every
DB-backed test in the workspace gets its database from `testkit::db()`
(migrated clone) or `testkit::empty_db()` (bare clone) — never by starting its
own container or running its own migrations.

- **Server resolution:** `FERROEHR_TEST_PG_URL` (CI, local dev server) →
  else the reusable named container `ferroehr-testkit-pg18` (testcontainers
  `reusable-containers`; deliberately left running across runs — reclaim
  with `docker rm -f ferroehr-testkit-pg18`). The container runs the
  non-durable settings the PostgreSQL docs describe for throwaway data
  (`fsync=off` etc.) — never copy those flags anywhere near production
  config — and an **explicit `/dev/shm` size** (`with_shm_size`, 1 GiB;
  the image default is 64 MB, see the failure mode below). The shm size is
  applied at container CREATION: a container that already exists is adopted
  as-is, so changing the value needs
  `docker rm -f ferroehr-testkit-pg18`.
- **Sweep:** every test process reclaims stale harness databases **once, at
  server acquisition, before it hands out any clone** — clones past a short
  grace window plus templates of other migration fingerprints stamped
  complete longer ago than their own grace window (or not stamped with a
  build time), so concurrent suites from different fingerprints on one server
  stop rebuilding each other's templates. Drops are
  **force-free on purpose**: a clone another process is still connected to is
  refused with SQLSTATE `55006` and skipped as benign (never
  `WITH (FORCE)` — that would tear a live test's database out from under it;
  force is correct only in `TestDb::drop`, which drops the caller's *own*
  clone). `testkit::sweep_stale()` is the manual entry point.
- **Template discipline:** the template is migrated by the platform
  library's own `db::run_migrations` and keyed on
  `db::migration_fingerprint()`; completion is stamped as the database
  comment, with the fingerprint and the build time. Never connect to the template outside the build path — an open
  connection blocks every clone (PostgreSQL docs § CREATE DATABASE).
- **Isolation is per-database, not per-server.** A test that needs
  server-global state (ALTER SYSTEM, cross-database assertions) does not
  belong on the shared server — flag it instead of hacking around the
  harness. The single exception is testkit's OWN gate
  (`tools/testkit/tests/it/harness.rs`), which must assert on cluster-wide state
  (the sweep) — it does so under uniquely named databases in the
  `ferroehr_tk_*` namespace so parallel processes cannot collide.
- **Dependency shape:** `testkit → ferroehr` (normal dep); the app crates'
  tests consume testkit as a dev-dependency. The `ferroehr` ↔ `testkit`
  dev-dependency cycle is deliberate and cargo-legal — do not "fix" it.
- Failures are typed (`TestkitError`) and always mean broken test
  infrastructure, never application behaviour — call sites `.expect()`.
- Gates: `cargo clippy -p testkit --all-targets` +
  `cargo nextest run -p testkit`.

## Failure mode: shared memory exhausted by leaked clones

**Symptom.** Every DB-backed test in the workspace fails — even a single one
run in isolation — with

```
could not resize shared memory segment "/PostgreSQL.<n>" to <n> bytes:
No space left on device
```

and `docker exec ferroehr-testkit-pg18 df -h /dev/shm` shows `/dev/shm` nearly
full at idle.

**Cause.** Leaked clone databases accumulating on the reusable container
(observed 2026-07-25: ~2957 `ferroehr_tk_*` clones). PostgreSQL keeps its
cumulative statistics in shared memory, sized per database/relation, and
allocates them from *dynamic* shared memory — POSIX shared memory under
`/dev/shm` on Linux (`dynamic_shared_memory_type = posix`; PostgreSQL docs
§ Resource Consumption). Thousands of databases fill the container's 64 MB
default `/dev/shm`, after which *every* DSM allocation fails, including the
ones ordinary queries need. The database count is the cause; the error is a
symptom of the shm ceiling.

**Recovery.** Nothing to do by hand in the normal case: the startup sweep
reclaims unused stale clones on the next test process, and the container is
created with a 1 GiB `/dev/shm`. To force it: `testkit::sweep_stale()`, or
drop every `ferroehr_tk_*` database except the live `ferroehr_tk_tpl_*` template
and restart the container (`docker restart ferroehr-testkit-pg18`) so the
statistics area is rebuilt. A pre-existing container still carries the old
64 MB — `docker rm -f ferroehr-testkit-pg18` to recreate it with the explicit
size.

**Prevention (in place).** Explicit `--shm-size` on the container (the
official `postgres` image documents exactly this knob:
<https://hub.docker.com/_/postgres>) plus the once-per-process startup sweep,
so unused clones cannot accumulate across runs. Never "fix" a red DB-backed
suite by weakening a test — check `/dev/shm` and the clone count first.
