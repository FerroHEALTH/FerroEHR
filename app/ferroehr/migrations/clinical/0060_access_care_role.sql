-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the care projection names each relationship's role (#6056).
--
-- The draft access control ITS-REST page, Queries and the Caseload: a query
-- may be narrowed to the caller's caseload in the roles `caseload_roles`
-- names, applied inside the query's one statement. A caseload probe reads the
-- projection, so each row now carries the role of its relationship, as the
-- terminology id and code of its `care_role`. The projection is emptied and
-- its state reset here, so the first run rebuilds every row; until then a
-- query binds the census. No openEHR spec governs the projection: our own
-- design.
--
-- Runs with search_path = clinical, ext, public.

DELETE FROM access_care;
UPDATE access_care_state SET revision = NULL, key_check = NULL, updated_at = now();

ALTER TABLE access_care
    ADD COLUMN role_terminology text NOT NULL,
    ADD COLUMN role_code        text NOT NULL;

COMMENT ON COLUMN access_care.role_terminology IS 'The terminology id of the relationship''s care_role. Our own design.';
COMMENT ON COLUMN access_care.role_code IS 'The code of the relationship''s care_role. Our own design.';
