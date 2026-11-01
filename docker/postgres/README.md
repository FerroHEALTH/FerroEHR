# ferroehr-postgres

A preconfigured PostgreSQL 18 image for FerroEHR, mirroring the official
`ehrbase/ehrbase-v2-postgres` two-image model built fresh for this stack
(the greenfield PG18 storage design, `docs/architecture.md` §Storage). It is `postgres:18.6` plus Debian
security updates applied at image build (the pinned upstream base rebuilds on its own cadence, so
trixie-security fixes are pulled in at OUR build time) plus one-time init scripts.

## What the image removes

- **GnuPG** (gnupg, gpg, gpg-agent, gpgconf, gpgsm, dirmngr, pinentry-curses,
  gnupg-l10n) and the libraries only it pulled in (libassuan9, libgnutls30t64,
  libksba8, libnpth0t64, libp11-kit0, libtasn1-6). Upstream installs it to
  import the PGDG signing key while building its image; nothing at run time
  calls it, and apt verifies packages with sqv (Sequoia), which stays. The
  build fails if any of the main packages survives the purge.
- **Upstream's Go `gosu`.** `/usr/local/bin/gosu` is `gosu` in this directory, a
  POSIX-sh wrapper that resolves the user the way gosu does and execs
  util-linux `setpriv --reuid --regid --init-groups`. The upstream
  `docker-entrypoint.sh` calls it unmodified. Running the container needs the
  same capabilities as before (`SETUID`, `SETGID` for the drop; `CHOWN`,
  `DAC_OVERRIDE`, `FOWNER` for the data directory) and works under
  `no-new-privileges`.

What remains is upstream Debian packages whose open findings have no Debian
fix yet; `security/vex/postgres-os.openvex.json` carries one OpenVEX
statement per finding.

## What the init scripts create

Run once, on an empty data directory, as the bootstrap superuser
(`POSTGRES_USER`) — see `initdb/10-ferroehr-init.sh`:

- a **non-superuser** login role (default `ferroehr`) and a **database it owns**
  (default `ferroehr`);
- the NOLOGIN group roles **`ferroehr_migrator`** and the four domain roles
  **`ferroehr_clinical`**, **`ferroehr_clinical_reader`**, **`ferroehr_party`**
  and **`ferroehr_party_reader`**, each `NOINHERIT`, with the login role
  granted the migrator and the two writer roles, so dev/compose has the same grant topology as a hardened
  deployment. It is one credential playing every part: this stack shows the
  schema separation, not the credential separation, which a deployment reaches
  by giving each domain its own DSN;
- **`ferroehr_linkage`**, which the released grant files still name, so a fresh
  database needs it while they apply. The app role holds no `CREATEROLE`, so
  the migration that retires the linkage domain cannot drop it here; it stays,
  granted nothing;
- schemas **`clinical`**, **`ext`** and **`audit`** (the local IHE ATNA Audit
  Record Repository), owned by the app role. The fourth storage schema,
  **`party`**, is left to the server: `ferroehr::db::prepare` creates all four
  itself before the first migrator runs, and the app role owns the database, so
  it needs no help with the one this script omits.

The server's own bootstrap then finds this in place: its
`CREATE SCHEMA IF NOT EXISTS {ext,clinical,party,audit}` becomes a no-op for
what the script already made, and it migrates the schema **content** into the
four sets.

## Init-scripts only — NO baked migration state (policy)

The image ships **roles, schemas, and extensions only**. It never bakes the
migrated schema (tables, functions, `_sqlx_migrations`) into the image.

Why: the app's sqlx migrators own the schema content and run idempotently at
every boot (a per-schema `_sqlx_migrations` ledger makes re-runs no-ops).
Baking migration state into the image would couple the two images' release
cycles for zero gain and risk a checksum mismatch between a stale baked schema
and the running binary's embedded migrations. The single source of truth for
schema content is `app/ferroehr/migrations/{ext,clinical,party,audit}/`,
applied at boot. This
is also the official EHRbase precedent (its postgres image is init-scripts
only).

## Configuration

| Env var | Default | Meaning |
|---|---|---|
| `POSTGRES_PASSWORD` | *(required by the base image)* | bootstrap **superuser** (`postgres`) password |
| `PG_INIT_USER` | `ferroehr` | app login role created by the init script |
| `PG_INIT_PASSWORD` | `ferroehr` | app role password (**dev default — override in production**) |
| `PG_INIT_DB` | `ferroehr` | app database created by the init script |

These `PG_INIT_*` vars configure this DB container; they are intentionally
outside the server's reserved `FERROEHR_` namespace (the server rejects unknown
`FERROEHR_*` vars at boot). The app then connects with, e.g.,
`FERROEHR__DB__URL=postgres://ferroehr:ferroehr@<host>:5432/ferroehr`.

Init scripts only run on first initialisation (empty volume). To re-provision,
remove the data volume (`docker compose down -v`).
