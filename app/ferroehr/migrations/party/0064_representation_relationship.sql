-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: a representation is a PARTY_RELATIONSHIP of its own with a basis and
-- a provider, and its row names who wrote it (#6079).
--
-- The draft access control package makes the single relationship the stored
-- unit: a PARTY_RELATIONSHIP named represents whose source is the
-- representative and whose target the subject, with basis and provider 1..1
-- in its details (Care Relationships, "A representation is"; The
-- Relationship Store). A representative is admitted only on a live one for
-- that subject whose provider is the one the caller acts for, and never on
-- one it wrote itself (Decision Procedure, step 0 Representation; Writing
-- Relationships). Every row is therefore of a container of its own, carries
-- the basis, the provider and the writer, and the rows of the former inline
-- form go. The stored digest is cleared, so the next boot fills the side
-- table again from the stored versions before it serves. No openEHR spec
-- governs the side table: our own design.
--
-- Runs with search_path = party, ext, public.

DELETE FROM binding_representation;
DELETE FROM binding_state;

DROP INDEX uq_binding_representation_pair;

ALTER TABLE binding_representation
    ALTER COLUMN relationship_id SET NOT NULL,
    ADD COLUMN basis_terminology text NOT NULL,
    ADD COLUMN basis_code        text NOT NULL,
    ADD COLUMN provider_issuer   text NOT NULL,
    ADD COLUMN provider_id       text NOT NULL,
    ADD COLUMN writer_id         uuid;

CREATE UNIQUE INDEX uq_binding_representation_container
    ON binding_representation (relationship_id) WHERE retired_at IS NULL;

COMMENT ON TABLE binding_representation IS 'The representations each PERSON is the source of: the PARTY_RELATIONSHIP containers of their own named openehr_access::represents, with their basis, provider and writer, live at the head and retired for every earlier one. No openEHR spec governs it, our own design.';
COMMENT ON COLUMN binding_representation.relationship_id IS 'The PARTY_RELATIONSHIP container the row is derived from.';
COMMENT ON COLUMN binding_representation.basis_terminology IS 'The terminology of the representation''s basis.';
COMMENT ON COLUMN binding_representation.basis_code IS 'The code of the representation''s basis.';
COMMENT ON COLUMN binding_representation.provider_issuer IS 'The issuer of the representation''s provider DV_IDENTIFIER.';
COMMENT ON COLUMN binding_representation.provider_id IS 'The id of the representation''s provider DV_IDENTIFIER.';
COMMENT ON COLUMN binding_representation.writer_id IS 'The container the committer of the relationship''s head names in external_ref: the PERSON or client ACTOR that wrote it; NULL when the committer names none.';
