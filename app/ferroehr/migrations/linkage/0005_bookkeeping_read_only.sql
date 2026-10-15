-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- linkage: the migration bookkeeping is read-only to the runtime roles.
--
-- `0004_grants.sql` granted SELECT, INSERT, UPDATE ON ALL TABLES IN SCHEMA linkage, and that
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
-- Runs with search_path = linkage, ext, public.

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_linkage') THEN
        REVOKE ALL ON linkage._sqlx_migrations FROM ferroehr_linkage;
        GRANT SELECT ON linkage._sqlx_migrations TO ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping linkage bookkeeping grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
