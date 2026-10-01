---
name: deployment-schema-name-surfaces
description: A database schema rename must be swept through FOUR independent deployment copies (Helm CronJob table, docker-compose backup services, the baked initdb script, the book) — plus the pg_dump fact that makes a missed one silent
metadata:
  type: reference
---

A PostgreSQL schema name reaches the deployment surface in four unrelated
places. They share no generator, so a rename sweep that touches one and not
the others renders and lints clean:

1. `deploy/helm/ferroehr/templates/backup-cronjob.yaml` — the `$domains`
   `list`/`dict` table (`"schemas" (list ...)`), which drives the CronJob args,
   the `kubernetes.io/description` annotation and the render-refusal messages.
2. `docker-compose.yml` — the three `ferroehr-backup-*` services, each with its
   own hand-written `pg_dump --schema=` command. **Not** generated from the
   chart.
3. `docker/postgres/initdb/10-ferroehr-init.sh` — baked into the
   `ghcr.io/ferrohealth/ferroehr-postgres` image (`docker/postgres/Dockerfile`
   `COPY initdb/ /docker-entrypoint-initdb.d/`), so it serves BOTH the compose
   stack and the hosted sandbox's database box
   (`deploy/hosted/cloud-init-postgres.yaml`). It `CREATE SCHEMA ... AUTHORIZATION`s
   a subset of schemas for ownership and installs the extensions `WITH SCHEMA ext`.
4. `website/book/src/operations.md` — the operator's copy of the same
   `pg_dump --schema=` lines.

Plus `deploy/hosted/cloud-init.yaml`, whose `wipe` verb `DROP SCHEMA`s the
current set AND the superseded set.

**Why a miss is silent, not loud** (PostgreSQL 18, pg_dump, `--strict-names`,
<https://www.postgresql.org/docs/18/app-pgdump.html>): "Note that if none of
the extension/schema/table patterns find matches, pg_dump will generate an
error even without `--strict-names`." So a dump listing a mix of live and dead
schema names **succeeds with exit 0** and quietly omits the dead ones; only a
dump where EVERY name is dead fails. A stale backup command therefore produces
a plausible-looking dump file with no data in it.

**How to apply:** on any PR that renames or removes a schema, grep all four
places, and check the pg_dump pattern list against the schemas
`app/ferroehr/migrations/*/` actually creates plus `BOOTSTRAP` in
`app/ferroehr/src/db/mod.rs`. Also check `--extension=` coverage: a `--schema`
dump carries no extension, and only `linkage` needs one (`btree_gist`, for the
`WITHOUT OVERLAPS` primary key).

Related: [[chart-review-read-only-procedure]]
