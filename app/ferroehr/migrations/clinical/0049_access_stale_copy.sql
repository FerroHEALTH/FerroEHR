-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: a query judged on an access copy older than the store fails
-- instead of answering with fewer rows (#5794).
--
-- A query's access condition is judged from this replica's copies: the
-- settings classes, and for a query pinned to one EHR that EHR's settings
-- and subject (#5746). A copy older than the store used to make the
-- statement withhold the rows it could not decide, so the caller got an
-- empty or a shorter result that read as a real answer. The two functions
-- below raise instead, each with its own SQLSTATE in a class PostgreSQL
-- leaves to implementations (PostgreSQL 18, Appendix A, "PostgreSQL Error
-- Codes": classes beginning with 5 to 9 or I to Z), so the server waits for
-- its copy and asks again, or refuses with 503:
--
--   ZA001  access_in_force: the settings or subject of the pinned EHR are no
--          longer the revision its verdict was judged on;
--   ZA002  access_class_unknown: a candidate row's EHR has settings of a
--          class the condition does not hold.
--
-- access_in_force takes constant arguments, so the statement evaluates it
-- once; access_class_unknown is reached only for a row whose class the
-- condition does not hold. No openEHR spec governs the copies or the
-- refusal: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE FUNCTION access_in_force(ehr uuid, judged text) RETURNS boolean
    LANGUAGE plpgsql STABLE PARALLEL SAFE
    AS $$
BEGIN
    IF CAST(access_revision(ehr) AS jsonb) IS NOT DISTINCT FROM CAST(judged AS jsonb) THEN
        RETURN true;
    END IF;
    RAISE EXCEPTION 'the access copy a query was judged on is older than the store'
        USING ERRCODE = 'ZA001';
END
$$;

COMMENT ON FUNCTION access_in_force(uuid, text) IS 'True when access_revision(ehr) is the revision a pinned query''s verdict was judged on; raises SQLSTATE ZA001 otherwise. Our own design.';

CREATE FUNCTION access_class_unknown(ehr uuid) RETURNS boolean
    LANGUAGE plpgsql STABLE PARALLEL SAFE
    AS $$
BEGIN
    RAISE EXCEPTION 'a query''s access condition does not hold the settings class of an EHR it reads'
        USING ERRCODE = 'ZA002';
END
$$;

COMMENT ON FUNCTION access_class_unknown(uuid) IS 'Raises SQLSTATE ZA002: a query reached an EHR whose settings class its access condition does not hold. Our own design.';

-- PUBLIC holds EXECUTE on a new function by default (PostgreSQL 18, GRANT,
-- "Notes"); only the clinical roles run them.
REVOKE ALL ON FUNCTION access_in_force(uuid, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION access_class_unknown(uuid) FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT EXECUTE ON FUNCTION access_in_force(uuid, text) TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION access_class_unknown(uuid) TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the stale access copy grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
