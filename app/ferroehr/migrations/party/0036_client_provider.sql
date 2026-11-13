-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the provider each system client acts for (#5436).
--
-- A client ACTOR names its provider with an inline PARTY_RELATIONSHIP that
-- carries the acts-for code of FerroEHR's operation terminology
-- (ferroehr-operations::acts_for) and targets the provider's ORGANISATION
-- (RM demographic master02-demographic_package.adoc §Party Relationships: a
-- relationship is stored in its source party). Every system access records
-- the provider (EHDS Annex II 3.2(a)) by the ORGANISATION's identifier, the
-- first DV_IDENTIFIER of its details (§Party Identification). Both are derived
-- from the party versions in each commit's own transaction, as the other grant
-- rows are, and a refill rebuilds them. No openEHR spec governs the side table:
-- our own design.
--
-- Runs with search_path = party, ext, public.

-- The container the client ACTOR's acts-for relationship targets; NULL when
-- it holds none.
ALTER TABLE binding_client ADD COLUMN provider_id uuid;

-- ── binding_provider ────────────────────────────────────────────────────────
-- One row per ORGANISATION head that is not deleted and carries an identifier.
CREATE TABLE binding_provider (
    -- The ORGANISATION's version container.
    party_id   uuid PRIMARY KEY,
    -- DV_IDENTIFIER.issuer.
    issuer     text NOT NULL,
    -- DV_IDENTIFIER.id.
    id         text NOT NULL,
    written_at timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE binding_provider IS 'The identifier each ORGANISATION head that is not deleted carries, by which a system access names the provider its client acts for. Derived from the party versions in each commit''s transaction; no openEHR spec governs it, our own design.';

-- The grant rows of a store filled before this migration hold no provider, so
-- the revision returns to the never-filled state the next boot refills from
-- the versions.
UPDATE binding_grant_revision SET revision = 0, revised_at = now();
