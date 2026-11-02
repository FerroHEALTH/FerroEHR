-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: what party_relationship_target.source_vo_id names (#4775).
--
-- The index is filled from the relationships a party carries inline (RM
-- demographic master02-demographic_package.adoc §Party Relationships: they are
-- "stored as part of the data of the PARTY designated as the source"), so the
-- source column names that party's version container. 0005 described it as a
-- PARTY_RELATIONSHIP container and has shipped, so the comment moves here.
-- No openEHR spec governs the index relation: our own storage design.
COMMENT ON COLUMN party_relationship_target.source_vo_id IS 'The version container of the source PARTY whose current trunk version carries the PARTY_RELATIONSHIP inline; FK-less because it names the object, not one of its versions.';
