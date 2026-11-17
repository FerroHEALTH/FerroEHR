-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the access revision a statement both domains run reads, which no
-- party object has (#5888).
--
-- The clinical schema reads an EHR's access revision from the view
-- access_revision_of (clinical/0052). A demographic object belongs to no
-- EHR, so the party schema keeps a view of the same shape with no rows, and
-- a statement's scalar subquery over it reads NULL, as access_revision(ehr)
-- (0040) answered, which this replaces.
--
-- No openEHR spec governs the projection or its revision: our own design.
--
-- Runs with search_path = party, ext, public.

CREATE VIEW access_revision_of AS
SELECT NULL::uuid AS ehr_id, NULL::text AS revision WHERE false;

COMMENT ON VIEW access_revision_of IS 'No rows: no party object belongs to an EHR, so none carries EHR access settings. Our own design.';

DROP FUNCTION access_revision(uuid);

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
        GRANT SELECT ON access_revision_of TO ferroehr_party, ferroehr_party_reader;
    ELSE
        RAISE NOTICE 'skipping the access revision grants (roles absent: see the ext role block NOTICE)';
    END IF;
END $$;
