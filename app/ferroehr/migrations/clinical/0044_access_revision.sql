-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the revision of an EHR's access settings row and its subject,
-- read beside the data a read leaves the store with (#5589).
--
-- Every replica copies an EHR's settings and drops the copy when the commit
-- that changes them notifies it, which happens after that commit. A read on
-- another replica in between selects access_revision(ehr_id) beside its own
-- columns and compares it with the revision its copy was judged on, so a
-- change committed anywhere is seen or the read refuses. The revision is the
-- storage ordinal of the EHR_ACCESS version the settings row was projected
-- from and the promoted ehr.subject_id, whose care relationships the
-- decision reads, as "<ordinal>/<subject>" with either side empty when
-- absent. The function is inlined into the statement (PostgreSQL 18,
-- "Inlining of SQL Functions"): one key lookup on access_settings and one on
-- ehr. The party schema defines the
-- same function answering NULL (party/0040), so a statement both domains run
-- reads it in either.
--
-- No openEHR spec governs the projection or its revision: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE FUNCTION access_revision(ehr uuid) RETURNS text
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
        SELECT coalesce((SELECT s.sys_version::text FROM clinical.access_settings s
                         WHERE s.ehr_id = ehr), '')
            || '/'
            || coalesce((SELECT e.subject_id FROM clinical.ehr e WHERE e.id = ehr), '')
    $$;

COMMENT ON FUNCTION access_revision(uuid) IS 'The storage ordinal of the EHR_ACCESS version the EHR''s settings row was projected from and the EHR''s promoted subject_id, as <ordinal>/<subject>, either side empty when absent. Our own design.';

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
