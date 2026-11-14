-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the revision of an EHR's access settings row, which no party object
-- has (#5589).
--
-- A version read selects access_revision(ehr_id) beside its own columns, and
-- the party schema shares the statement with the clinical one
-- (clinical/0043). A demographic object belongs to no EHR, so the party
-- domain answers NULL.
--
-- No openEHR spec governs the projection or its revision: our own design.
--
-- Runs with search_path = party, ext, public.

CREATE FUNCTION access_revision(ehr uuid) RETURNS integer
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    AS $$ SELECT NULL::integer $$;

COMMENT ON FUNCTION access_revision(uuid) IS 'NULL: no party object belongs to an EHR, so none carries EHR access settings. Our own design.';

-- PUBLIC holds EXECUTE on a new function by default (PostgreSQL 18, GRANT,
-- "Notes"); only the party roles read it.
REVOKE ALL ON FUNCTION access_revision(uuid) FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
        GRANT EXECUTE ON FUNCTION access_revision(uuid) TO ferroehr_party, ferroehr_party_reader;
    ELSE
        RAISE NOTICE 'skipping the access revision grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
