#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# Preconfigure the ferroehr database. Runs once, as the bootstrap superuser
# (POSTGRES_USER), on first initialisation of an empty data directory.
#
# Creates exactly what the app's sqlx migrator (`ferroehr::db::run_migrations`)
# expects to already exist or be a no-op when it connects as the NON-superuser
# app role:
#   * a LOGIN role (non-superuser) and a database it owns;
#   * schemas `clinical`, `ext` and `audit` owned by that role (the app's
#     `CREATE SCHEMA IF NOT EXISTS` then no-ops; it also bootstraps `party` and
#     `linkage`, which it owns and this script leaves to it);
#   * btree_gist, the one extension the schema needs, installed here by the
#     superuser (`CREATE EXTENSION` on non-trusted extensions requires
#     superuser — the whole reason this image exists). The app's bootstrap
#     `CREATE EXTENSION IF NOT EXISTS btree_gist WITH SCHEMA ext` then no-ops.
set -Eeuo pipefail

# PG_INIT_* configure the DB container's app role/database. They are
# deliberately NOT in the server's reserved FERROEHR_ namespace (which the
# server rejects unknown vars from at boot) — these belong to this init script.
APP_USER="${PG_INIT_USER:-ferroehr}"
APP_PASSWORD="${PG_INIT_PASSWORD:-ferroehr}"   # dev default; override in prod
APP_DB="${PG_INIT_DB:-ferroehr}"

# psql as the bootstrap superuser, ON_ERROR_STOP so a failure aborts init.
psql_super() { psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" "$@"; }
# shellcheck disable=SC2120 # forwards flags like its psql_super twin; today's one caller passes none
psql_app() { psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$APP_DB" "$@"; }

echo "ferroehr init: creating role '${APP_USER}' and database '${APP_DB}'"

# 1) Login role (idempotent) — non-superuser by design — plus the six NOLOGIN
#    group roles: the DDL-only ferroehr_migrator and the five domain roles. The
#    app role has no CREATEROLE, so the migration sets can only grant to these
#    roles if they already exist; creating them here gives dev/compose the
#    same grant topology as a hardened deployment (no "roles absent" NOTICEs).
psql_super <<SQL
DO \$do\$
BEGIN
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = '${APP_USER}') THEN
    CREATE ROLE "${APP_USER}" LOGIN PASSWORD '${APP_PASSWORD}';
  ELSE
    ALTER ROLE "${APP_USER}" LOGIN PASSWORD '${APP_PASSWORD}';
  END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_migrator') THEN
    CREATE ROLE ferroehr_migrator NOLOGIN;
  END IF;
  -- The five domain roles, created here because the migration sets grant to
  -- them only if they already exist, and the app role holds no CREATEROLE:
  -- without them compose would run with no domain grants at all and a "roles
  -- absent" NOTICE nobody reads. NOINHERIT and no membership in one another —
  -- a role that could inherit the other domain's grants would make the
  -- boundary a naming convention (PostgreSQL 18, CREATE ROLE
  -- https://www.postgresql.org/docs/18/sql-createrole.html).
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
    CREATE ROLE ferroehr_clinical NOLOGIN NOINHERIT;
  END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
    CREATE ROLE ferroehr_party NOLOGIN NOINHERIT;
  END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical_reader') THEN
    CREATE ROLE ferroehr_clinical_reader NOLOGIN NOINHERIT;
  END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party_reader') THEN
    CREATE ROLE ferroehr_party_reader NOLOGIN NOINHERIT;
  END IF;
  IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_linkage') THEN
    CREATE ROLE ferroehr_linkage NOLOGIN NOINHERIT;
  END IF;
  -- In dev the single app user plays both migrator and writer.
  GRANT ferroehr_migrator TO "${APP_USER}";
  -- ONE login credential, deliberately. This stack demonstrates the SCHEMA
  -- separation — which is unconditional, and which the server's boot
  -- self-check verifies — not the CREDENTIAL separation. A deployment reaches
  -- that by giving the party pool its own DSN (the [storage.party] url config
  -- key, or the chart value database.party.existingSecret) under a
  -- login role that is a member of ferroehr_party and nothing else, and the
  -- linkage pool the same through [storage.linkage] url. Such a deployment also
  -- names db.migrate_url: preparing the schema reads every migration set of a
  -- database, which no domain-scoped credential can do. A demo stack that looked split
  -- while running one credential would be worse than one that says which half
  -- it shows.
  GRANT ferroehr_clinical TO "${APP_USER}";
  GRANT ferroehr_party TO "${APP_USER}";
  GRANT ferroehr_linkage TO "${APP_USER}";
END
\$do\$;
SQL

# 2) Database owned by the app role (CREATE DATABASE cannot run in a DO block).
if ! psql_super -tAc "SELECT 1 FROM pg_database WHERE datname = '${APP_DB}'" | grep -q 1; then
  psql_super -c "CREATE DATABASE \"${APP_DB}\" OWNER \"${APP_USER}\";"
fi

# 3) Schemas owned by the app role + superuser-installed extensions, in the
#    app database. `ext` holds the openEHR helper functions and, by convention,
#    the extensions; both schemas are on the app's search_path (clinical, ext,
#    public).
# shellcheck disable=SC2119 # the SQL arrives on stdin; the script's own $@ is not psql's
psql_app <<SQL
CREATE SCHEMA IF NOT EXISTS clinical AUTHORIZATION "${APP_USER}";
CREATE SCHEMA IF NOT EXISTS ext AUTHORIZATION "${APP_USER}";
-- The local IHE ATNA Audit Record Repository, deliberately its own schema.
CREATE SCHEMA IF NOT EXISTS audit AUTHORIZATION "${APP_USER}";

-- btree_gist is the one extension the schema needs: it backs
-- linkage.subject_ehr's temporal UNIQUE (... WITHOUT OVERLAPS), which
-- PostgreSQL enforces as a GiST index over btree_gist operator classes.
-- Installed here by the superuser so the app role never needs it; the server's
-- own bootstrap runs the same statement and then no-ops.
CREATE EXTENSION IF NOT EXISTS btree_gist WITH SCHEMA ext;

-- The app role owns the schemas already; make the intent explicit.
GRANT ALL ON SCHEMA clinical TO "${APP_USER}";
GRANT ALL ON SCHEMA ext      TO "${APP_USER}";
GRANT ALL ON SCHEMA audit    TO "${APP_USER}";
SQL

echo "ferroehr init: done"
