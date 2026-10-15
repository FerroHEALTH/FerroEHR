-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the migration bookkeeping is read-only to the runtime roles.
--
-- `0008_grants.sql` granted SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA party, and that
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
-- Runs with search_path = party, ext, public.

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
        REVOKE ALL ON party._sqlx_migrations FROM ferroehr_party, ferroehr_party_reader;
        GRANT SELECT ON party._sqlx_migrations TO ferroehr_party, ferroehr_party_reader;
    ELSE
        RAISE NOTICE 'skipping party bookkeeping grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
