-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the care projection digests subject references without a key (#6092).
--
-- The projection keyed its digests under the national-identifier protection's
-- key, which is gone with the protection. A subject reference is a pseudonym,
-- which the ehr row already holds in clear, so the projector now takes the
-- SHA-256 of the same reference text and runs on every deployment. The
-- projection is emptied and its state reset here, so the first run rebuilds
-- every row and digest in the new form; until then a query binds the census.
-- No openEHR spec governs the projection: our own design.
--
-- Runs with search_path = clinical, ext, public.

DELETE FROM access_care;
UPDATE ehr SET subject_digest = NULL WHERE subject_digest IS NOT NULL;
UPDATE access_care_state SET revision = NULL, key_check = NULL, updated_at = now();

COMMENT ON COLUMN ehr.subject_digest IS 'The SHA-256 digest of the subject reference (subject_namespace and subject_id) the care projection is matched by; NULL without a subject. Our own design.';
COMMENT ON COLUMN access_care.subject_digest IS 'The SHA-256 digest of one subject reference that names the PERSON: its own reference in this CDR''s namespace, or a record subject it declares. Our own design.';
COMMENT ON COLUMN access_care_state.key_check IS 'The check value of the digest form the rows were taken in; a different value makes the projector rebuild them. Our own design.';
