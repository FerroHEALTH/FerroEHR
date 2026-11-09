-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: a membership is held by its member (#5220).
--
-- A membership PARTY_RELATIONSHIP has the member PERSON as its source and the
-- GROUP as its target, so it is stored in the PERSON's version (RM demographic
-- master02 §Party Relationships: relationships "are stored as part of the data
-- of the PARTY designated as the source"), and a change of one membership
-- versions one PERSON, not the whole GROUP. binding_membership keeps one live
-- row per (GROUP, member), now derived from the PERSON heads, and serves a
-- Group's members.
--
-- The GROUP gets no version when its members change, so its SCIM version is
-- its head plus the revision below, which moves in the same transaction as
-- each change to its binding_membership rows (RFC 7644 §3.14: the version
-- changes when the resource does). No openEHR spec governs this table: our own
-- design.
--
-- Runs with search_path = party, ext, public.

CREATE TABLE binding_group_revision (
    -- The GROUP's versioned-object container.
    group_id   uuid PRIMARY KEY,
    -- How many changes its live memberships have had.
    revision   bigint NOT NULL,
    -- When the last of them was committed.
    revised_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ix_binding_membership_group_order
    ON binding_membership (group_id, written_at, member_id) WHERE retired_at IS NULL;

COMMENT ON TABLE binding_group_revision IS 'The membership revision of each GROUP, which moves with every change of its live binding_membership rows; a SCIM Group''s version is its GROUP head and this revision. Our own design.';
COMMENT ON TABLE binding_membership IS 'The live members of each GROUP of the mapped group template, from the inline PARTY_RELATIONSHIPs of the membership archetype each member PERSON holds with the GROUP as target; standalone relationship containers never count. No openEHR spec governs it, our own design.';
