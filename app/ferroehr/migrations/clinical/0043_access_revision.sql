-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the revision of an EHR's access settings row, read beside the
-- data a read leaves the store with (#5589).
--
-- Every replica copies an EHR's settings and drops the copy when the commit
-- that changes them notifies it, which happens after that commit. A read on
-- another replica in between selects access_revision(ehr_id) beside its own
-- columns and compares it with the revision its copy was judged on, so a
-- change committed anywhere is seen or the read refuses. The function is
-- inlined into the statement (PostgreSQL 18, "Inlining of SQL Functions"),
-- one index lookup on access_settings' key. The party schema defines the
-- same function answering NULL (party/0040), so a statement both domains run
-- reads it in either.
--
-- No openEHR spec governs the projection or its revision: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE FUNCTION access_revision(ehr uuid) RETURNS integer
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$ SELECT s.sys_version FROM clinical.access_settings s WHERE s.ehr_id = ehr $$;

COMMENT ON FUNCTION access_revision(uuid) IS 'The storage ordinal of the EHR_ACCESS version the EHR''s settings row was projected from; NULL when it has none. Our own design.';

-- PUBLIC holds EXECUTE on a new function by default (PostgreSQL 18, GRANT,
-- "Notes"); only the clinical roles read it.
REVOKE ALL ON FUNCTION access_revision(uuid) FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT EXECUTE ON FUNCTION access_revision(uuid) TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the access revision grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
