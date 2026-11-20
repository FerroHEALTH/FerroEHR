-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: each caller fact names the party that wrote it, and a care
-- relationship whether it is self-declared (#6073).
--
-- The draft access control decision procedure, Writing Relationships: "A
-- caller MUST NOT be admitted on a relationship it wrote naming itself,
-- unless the relationship's basis is self_declared and
-- ACCESS_POLICY.self_declared_care allows it". The writer is the container
-- the commit audit's committer of the holder's head names in external_ref:
-- the bound PERSON of a professional, or the client ACTOR of a system
-- caller. The grant revision returns to the never-filled state, so the next
-- boot refills every fact with its writer from the stored versions before it
-- serves. No openEHR spec governs the side table: our own design.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE binding_caller_fact
    ADD COLUMN writer_id     uuid,
    ADD COLUMN self_declared boolean NOT NULL DEFAULT false;

ALTER TABLE binding_caller_fact
    ADD CONSTRAINT ck_binding_caller_fact_self_declared
        CHECK (NOT self_declared OR kind = 'under_care_of');

COMMENT ON COLUMN binding_caller_fact.writer_id IS 'The container the committer of the holder''s head names in external_ref: the PERSON or client ACTOR that wrote the fact; NULL when the committer names none.';
COMMENT ON COLUMN binding_caller_fact.self_declared IS 'Whether a care relationship''s basis is self_declared; false for every other fact.';

UPDATE binding_grant_revision SET revision = 0, revised_at = now();
