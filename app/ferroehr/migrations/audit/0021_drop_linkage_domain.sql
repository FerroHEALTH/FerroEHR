-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- Drop the linkage domain: its schema, its runtime role and the extension only
-- its table needed (#4684).
--
-- linkage/0006_drop_subject_ehr.sql left the schema holding no table and no
-- function, and the server no longer opens a linkage pool or applies a linkage
-- set. The released ext, clinical and party sets still name ferroehr_linkage in
-- their grants, and a fresh install replays them, so the role can only go once
-- they have applied: this file sits in the audit set, the last set a database
-- applies. ext/0001_schema_and_roles.sql creates the role again in every
-- database it prepares (behind IF NOT EXISTS), so this file runs once per
-- database, and every statement is guarded to run safely more than once.
--
-- The role is per cluster and its grants are per database: a grant another
-- database still holds keeps the role, and so does a migrator without the
-- right to drop it. Either case raises a NOTICE with the remedy and the
-- migration carries on, so no boot fails on it.
--
-- No specification governs this: our own design.

DO $$
BEGIN
    DROP SCHEMA IF EXISTS linkage CASCADE;
EXCEPTION WHEN insufficient_privilege THEN
    RAISE NOTICE 'keeping schema linkage (%): drop it as its owner with DROP SCHEMA linkage CASCADE', SQLERRM;
END $$;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_linkage') THEN
        -- What the released sets granted the role outside its own schema:
        -- ext/0001 and linkage/0004 on ext, through the migrator's default
        -- privileges on ext functions as well.
        REVOKE ALL ON SCHEMA ext FROM ferroehr_linkage;
        REVOKE ALL ON ALL FUNCTIONS IN SCHEMA ext FROM ferroehr_linkage;
        ALTER DEFAULT PRIVILEGES IN SCHEMA ext
            REVOKE EXECUTE ON FUNCTIONS FROM ferroehr_linkage;

        IF EXISTS (
            SELECT FROM pg_shdepend
            WHERE refclassid = 'pg_authid'::regclass
              AND refobjid = (SELECT oid FROM pg_roles WHERE rolname = 'ferroehr_linkage')
        ) THEN
            RAISE NOTICE 'keeping role ferroehr_linkage: an object or a grant in this or another database of the cluster still names it; run REVOKE ALL ON SCHEMA ext FROM ferroehr_linkage, DROP SCHEMA IF EXISTS linkage CASCADE and DROP OWNED BY ferroehr_linkage in each such database, then DROP ROLE ferroehr_linkage';
        ELSE
            BEGIN
                DROP ROLE ferroehr_linkage;
            EXCEPTION WHEN insufficient_privilege OR dependent_objects_still_exist THEN
                RAISE NOTICE 'keeping role ferroehr_linkage (%): drop it at deployment with DROP ROLE ferroehr_linkage', SQLERRM;
            END;
        END IF;
    END IF;
END $$;

-- btree_gist backed the temporal key of linkage.subject_ehr and nothing else.
DO $$
BEGIN
    DROP EXTENSION IF EXISTS btree_gist;
EXCEPTION WHEN insufficient_privilege OR dependent_objects_still_exist THEN
    RAISE NOTICE 'keeping extension btree_gist (%): nothing in the schema needs it any more', SQLERRM;
END $$;
