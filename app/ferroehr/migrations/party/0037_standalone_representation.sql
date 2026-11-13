-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: representations recognised by their code, inline or of their own
-- (#5446).
--
-- A PARTY_RELATIONSHIP is a representation when its name or its details carry
-- the representation code of FerroEHR's operation terminology
-- (ferroehr-operations::represents), in any archetype. Its source is the
-- representative and its target the subject (RM demographic
-- master02-demographic_package.adoc §Party Relationships). Besides the inline
-- relationships of the representative's PERSON, a standalone PARTY_RELATIONSHIP
-- container is one, which is how a patient authorises a proxy; its row names
-- the container, and the representative's key row counts it. The
-- relationship column holds the openEHR subject_relationship code the
-- relationship names itself, so no mapping maps one. No openEHR spec governs
-- the side table: our own design.
--
-- Runs with search_path = party, ext, public.

-- The standalone PARTY_RELATIONSHIP container a row is derived from; NULL for
-- an inline relationship of the representative's PERSON.
ALTER TABLE binding_representation ADD COLUMN relationship_id uuid;

DROP INDEX uq_binding_representation_pair;
CREATE UNIQUE INDEX uq_binding_representation_pair
    ON binding_representation (representative_id, subject_id, relationship, relationship_id)
    NULLS NOT DISTINCT WHERE retired_at IS NULL;

-- A standalone container's live row, which its commit retires.
CREATE INDEX ix_binding_representation_container
    ON binding_representation (relationship_id)
    WHERE retired_at IS NULL AND relationship_id IS NOT NULL;

COMMENT ON TABLE binding_representation IS 'The representation relationships each PERSON is the source of: its inline PARTY_RELATIONSHIPs and the standalone PARTY_RELATIONSHIP containers that carry the ferroehr-operations::represents code. No openEHR spec governs it, our own design.';
COMMENT ON COLUMN binding_representation.relationship IS 'The openEHR subject_relationship code the relationship names in its name or details; NULL when it names none.';
COMMENT ON COLUMN binding_representation.relationship_id IS 'The standalone PARTY_RELATIONSHIP container the row is derived from; NULL for an inline relationship.';
