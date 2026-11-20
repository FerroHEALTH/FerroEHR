-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the care projection names each relationship's writer and whether
-- it is self-declared (#6073).
--
-- The draft access control decision procedure, Writing Relationships: a
-- caller is never admitted on a relationship it wrote naming itself, unless
-- its basis is self_declared and the policy turns that path on. A query's
-- access condition reads the projection, so each row now carries the party
-- that wrote the relationship and its basis, and the probe excludes a row a
-- principal of the caller wrote. The projection is emptied and its state
-- reset here, so the first run rebuilds every row; until then a query binds
-- the census. No openEHR spec governs the projection: our own design.
--
-- Runs with search_path = clinical, ext, public.

DELETE FROM access_care;
UPDATE access_care_state SET revision = NULL, key_check = NULL, updated_at = now();

ALTER TABLE access_care
    ADD COLUMN writer_id     uuid,
    ADD COLUMN self_declared boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN access_care.writer_id IS 'The PERSON or client ACTOR that wrote the relationship, by its container; NULL when its committer names none. Our own design.';
COMMENT ON COLUMN access_care.self_declared IS 'Whether the relationship''s basis is self_declared. Our own design.';
