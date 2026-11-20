-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the care projection names the carer and the provider (#6055).
--
-- A care relationship's target is a PERSON, a ROLE, a GROUP or an
-- ORGANISATION, and a caller holds it only while it acts for the
-- relationship's provider (draft access control package, Care Relationships;
-- decision procedure, Terms). Each row now names the carer and the
-- provider's identifier issuer and id, and a query's access condition binds
-- the caller's principals and the providers it acts for. The projection is
-- emptied and its state reset here, so the first run rebuilds every row; until
-- then a query binds the census. No openEHR spec governs the projection: our
-- own design.
--
-- Runs with search_path = clinical, ext, public.

DELETE FROM access_care;
UPDATE access_care_state SET revision = NULL, key_check = NULL, updated_at = now();

DROP INDEX ix_access_care_subject;

ALTER TABLE access_care RENAME COLUMN team_id TO carer_id;
ALTER TABLE access_care
    ADD COLUMN provider_issuer text NOT NULL,
    ADD COLUMN provider_id     text NOT NULL;

CREATE INDEX ix_access_care_subject ON access_care (subject_digest, carer_id);

COMMENT ON COLUMN access_care.carer_id IS 'The carer of the relationship, by its container: a PERSON, a ROLE, a GROUP or an ORGANISATION. Our own design.';
COMMENT ON COLUMN access_care.provider_issuer IS 'The issuer of the provider DV_IDENTIFIER in whose care the relationship is. Our own design.';
COMMENT ON COLUMN access_care.provider_id IS 'The id of the provider DV_IDENTIFIER in whose care the relationship is. Our own design.';
