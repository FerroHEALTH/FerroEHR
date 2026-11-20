-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- ext: the runtime roles, and the storage generation label.
--
-- This is the v6.0.0 baseline of the ext set: every object of the schema is
-- created once, in its final shape, by the files 0001 to 0005. A later change
-- is a new file after them.
--
-- No openEHR spec governs database schemas, roles or grants: our own
-- design/extension. What the roles realize is the pseudonymisation-domain
-- separation GDPR Art. 4(5) and Art. 32(1)(a) ask for
-- (https://eur-lex.europa.eu/eli/reg/2016/679/oj) and the EDPB Guidelines
-- 01/2025 require to hold against internal actors with database access
-- (https://www.edpb.europa.eu/system/files/2025-01/edpb_guidelines_202501_pseudonymisation_en.pdf).
--
-- Runs with search_path = ext. The schema itself is created by the server's
-- bootstrap (crate::db), before any set runs.

-- Every role is NOLOGIN and NOINHERIT: a domain role that could inherit
-- another domain's grants would make the boundary a naming convention
-- (PostgreSQL 18, CREATE ROLE, "INHERIT / NOINHERIT",
-- https://www.postgresql.org/docs/18/sql-createrole.html).
--
-- Graceful degradation: when the migrator holds no CREATEROLE (development,
-- compose, the test harness, a managed cluster without role rights) the block
-- is skipped with a NOTICE and role provisioning becomes a deployment step.
DO $$
BEGIN
    BEGIN
        IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_migrator') THEN
            CREATE ROLE ferroehr_migrator NOLOGIN;
        END IF;
        IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
            CREATE ROLE ferroehr_clinical NOLOGIN NOINHERIT;
        END IF;
        IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical_reader') THEN
            CREATE ROLE ferroehr_clinical_reader NOLOGIN NOINHERIT;
        END IF;
        IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
            CREATE ROLE ferroehr_party NOLOGIN NOINHERIT;
        END IF;
        IF NOT EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party_reader') THEN
            CREATE ROLE ferroehr_party_reader NOLOGIN NOINHERIT;
        END IF;
    EXCEPTION WHEN insufficient_privilege THEN
        RAISE NOTICE 'skipping role creation (no CREATEROLE privilege): create ferroehr_migrator/ferroehr_clinical/ferroehr_clinical_reader/ferroehr_party/ferroehr_party_reader at deployment';
    END;
END $$;

-- The storage generation this database carries. A LABEL for instruments (the
-- storage benchmark harness files its records under it); nothing in the
-- serving path reads it, and no data is stamped with it.
CREATE FUNCTION storage_generation() RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$ SELECT 'generation-6'::text $$;
COMMENT ON FUNCTION ext.storage_generation() IS
    'The storage generation of this schema, for measurement records only; nothing in the serving path reads it.';
