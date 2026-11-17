-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the access revision of an EHR is read from a view inside the
-- statement that selects it, so no read runs a statement of its own per row
-- (#5888).
--
-- access_revision(ehr) (0044, 0046) was a SQL function whose body holds
-- subqueries, which PostgreSQL never inlines, so each call ran as a nested
-- statement: twice per single-EHR AQL request from its settings reads, once
-- from its stale-copy check, and once per row of every head, version and EHR
-- read that selects the revision. The view access_revision_of gives the same
-- text, the JSON array [<ordinal>, <namespace>, <id>] each null when absent,
-- by EHR, and a statement reads it in a scalar subquery the planner merges
-- with the view: two key lookups in the statement's own plan. The party
-- schema keeps a view of the same shape with no rows (party/0053), so a
-- statement both domains run reads NULL there, as the party function
-- answered.
--
-- The stale-copy check of a query pinned to one EHR (0049) compares the
-- revision the same way inside the query, and calls access_stale_copy(),
-- which raises ZA001 as access_in_force did, only when the revision moved.
-- A missing EHR reads as [null, null, null], the text the function wrote.
--
-- No openEHR spec governs the projection or its revision: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE VIEW access_revision_of WITH (security_invoker = true) AS
SELECT e.id AS ehr_id,
       json_build_array(s.sys_version, e.subject_namespace, e.subject_id)::text AS revision
  FROM clinical.ehr e
  LEFT JOIN clinical.access_settings s ON s.ehr_id = e.id;

COMMENT ON VIEW access_revision_of IS 'The storage ordinal of the EHR_ACCESS version each EHR''s settings row was projected from and the EHR''s promoted subject_namespace and subject_id, as the JSON array [<ordinal>, <namespace>, <id>], each null when absent; read in a scalar subquery by the statement that selects it. Our own design.';

CREATE FUNCTION access_stale_copy() RETURNS boolean
    LANGUAGE plpgsql VOLATILE PARALLEL SAFE
    AS $$
BEGIN
    RAISE EXCEPTION 'the access copy a query was judged on is older than the store'
        USING ERRCODE = 'ZA001';
END
$$;

COMMENT ON FUNCTION access_stale_copy() IS 'Raises SQLSTATE ZA001: the settings or subject of a pinned EHR are no longer the revision its verdict was judged on. Our own design.';

DROP FUNCTION access_in_force(uuid, text);
DROP FUNCTION access_revision(uuid);

-- PUBLIC holds EXECUTE on a new function by default (PostgreSQL 18, GRANT,
-- "Notes"); only the clinical roles run it.
REVOKE ALL ON FUNCTION access_stale_copy() FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT ON access_revision_of TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION access_stale_copy() TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the access revision grants (roles absent: see the ext role block NOTICE)';
    END IF;
END $$;
