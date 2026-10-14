-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the migration bookkeeping is read-only to the runtime roles.
--
-- `0010_grants.sql` granted SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA clinical, and that
-- includes `_sqlx_migrations`, which sqlx creates before the set's first file
-- runs. A runtime credential could then rewrite the record of which migrations
-- ran and from which source, which is what the boot check compares. The runtime
-- roles keep SELECT, so a pool can still read the migration level, and lose
-- every other privilege on it (PostgreSQL 18, REVOKE,
-- https://www.postgresql.org/docs/18/sql-revoke.html). The boot gate refuses a
-- runtime role that can write any bookkeeping table again.
--
-- No openEHR spec governs migration bookkeeping or database roles: our own
-- operational design.
--
-- Runs with search_path = clinical, ext, public.

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON clinical._sqlx_migrations FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON clinical._sqlx_migrations TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping clinical bookkeeping grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
