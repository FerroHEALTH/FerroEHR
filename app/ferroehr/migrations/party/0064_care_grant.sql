-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: each care relationship fact names the version it was read from and
-- the carer it covers for (#6075).
--
-- The draft access control decision procedure, Refusals: "An access admitted
-- on a care relationship SHOULD be recorded with the relationship it rested
-- on: its role, its path ..., its uid and version, whether grace admitted it,
-- its covers_for and its provider." The fact already names the relationship's
-- container, its role and its provider; it now names the OBJECT_VERSION_ID
-- of the trunk head it was derived from and the covers_for DV_IDENTIFIER's
-- issuer and id. The grant revision returns to the never-filled state, so the
-- next boot refills every fact from the stored versions before it serves. No
-- openEHR spec governs the side table: our own design.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE binding_caller_fact
    ADD COLUMN holder_version text,
    ADD COLUMN covers_issuer  text,
    ADD COLUMN covers_id      text;

ALTER TABLE binding_caller_fact
    ADD CONSTRAINT ck_binding_caller_fact_covers
        CHECK ((covers_issuer IS NULL) = (covers_id IS NULL)
               AND (covers_issuer IS NULL OR kind = 'under_care_of'));

COMMENT ON COLUMN binding_caller_fact.holder_version IS 'The OBJECT_VERSION_ID of the holder''s trunk head the fact was derived from, which the access log names for a care relationship.';
COMMENT ON COLUMN binding_caller_fact.covers_issuer IS 'The issuer of a care relationship''s covers_for DV_IDENTIFIER; NULL when it names none and for every other fact.';
COMMENT ON COLUMN binding_caller_fact.covers_id IS 'The id of a care relationship''s covers_for DV_IDENTIFIER; NULL when it names none and for every other fact.';

UPDATE binding_grant_revision SET revision = 0, revised_at = now();
