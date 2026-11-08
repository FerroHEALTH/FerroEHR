-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the SCIM client that owns each GROUP it created (#5015).
--
-- A SCIM Group belongs to the client that created it, and every other client
-- answers 404 for it (RFC 7644 §3: a client manages its own resources). A
-- Group need not carry an externalId (RFC 7643 §3.1, the §8.4 example has
-- none), so its owner cannot be read from its identifiers; the creating
-- transaction writes the row before it commits the GROUP, and the party
-- protection reads it in every later commit of that GROUP. A row outlives the
-- GROUP's logical deletion, after which its id answers 404. No openEHR spec
-- governs SCIM or this table: our own design.
--
-- Runs with search_path = party, ext, public.

CREATE TABLE scim_group (
    -- The GROUP's versioned-object container.
    group_id       uuid PRIMARY KEY,
    -- The binding_issuer of the client that created it.
    binding_issuer text NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX ix_scim_group_client ON scim_group (binding_issuer, group_id);

COMMENT ON TABLE scim_group IS 'The SCIM client that created each GROUP, which alone writes and reads it through /scim/v2/Groups. Our own design.';
