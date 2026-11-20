-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the care projection names each relationship's role (#6075).
--
-- An actor scope of a rule may name care roles and a care path (draft access
-- control decision procedure, Holding a Care Relationship), and a query's
-- access condition decides it inside the one statement, so each row now
-- carries the care_role's terminology and code. The path follows from which
-- of the caller's principals the carer is, which the probe binds. The
-- projection is emptied and its state reset here, so the first run rebuilds
-- every row; until then a query binds the census. No openEHR spec governs the
-- projection: our own design.
--
-- Runs with search_path = clinical, ext, public.

DELETE FROM access_care;
UPDATE access_care_state SET revision = NULL, key_check = NULL, updated_at = now();

ALTER TABLE access_care
    ADD COLUMN role_terminology text NOT NULL,
    ADD COLUMN role_code        text NOT NULL;

COMMENT ON COLUMN access_care.role_terminology IS 'The terminology of the relationship''s care_role. Our own design.';
COMMENT ON COLUMN access_care.role_code IS 'The code of the relationship''s care_role. Our own design.';
