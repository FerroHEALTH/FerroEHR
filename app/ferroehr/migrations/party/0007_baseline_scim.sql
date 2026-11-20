-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the state the SCIM provisioning endpoint keeps beside the parties it
-- writes: deactivations, the erasure of contact details and the groups.
--
-- Runs with search_path = party, ext, public.

-- The per-client counter that bounds how many Users a SCIM client
-- deactivates or deletes in one window (#5014).
--
-- One compromised provisioning client, or a scoping mistake at the identity
-- provider, could otherwise deactivate every professional at once. Every
-- SCIM deactivation and deletion reads and advances its client's row under a
-- FOR UPDATE lock inside its own write transaction, so the replicas sharing
-- the database share one bound. No openEHR spec governs SCIM or this table:
-- our own design.
CREATE TABLE scim_deactivation (
    -- The client's binding_issuer, which no two clients share.
    binding_issuer text PRIMARY KEY,
    -- When the current window began.
    window_start   timestamptz NOT NULL,
    -- The deactivations and deletions counted in the current window.
    counted        integer NOT NULL CHECK (counted >= 0)
);
COMMENT ON TABLE scim_deactivation IS 'The deactivations and deletions each SCIM client made in its current window, which [scim] max_deactivations bounds. Our own design.';

-- The PERSONs of deleted SCIM Users whose contact values are erased
-- (#5178).
--
-- A SCIM DELETE keeps every version of the PERSON for the record-keeping
-- duties that need the professional's identification, but the contacts
-- serve no attribution, so `[scim] contact_erasure_minutes` after the
-- deletion every version's `contacts` is removed (GDPR Art. 5(1)(e)). A row
-- here says the erasure ran, so the sweep that finds the deletions by their
-- deprovisioned key rows reaches each PERSON once. A physical erasure of the
-- party removes its row with the rest. No openEHR spec governs which values
-- of a deleted party are erased: our own design.
CREATE TABLE scim_contact_erasure (
    -- The PERSON's versioned object.
    party_id  uuid PRIMARY KEY,
    -- When its contacts were erased.
    erased_at timestamptz NOT NULL
);
COMMENT ON TABLE scim_contact_erasure IS 'The PERSONs of deleted SCIM Users whose contact values every version has had erased, which [scim] contact_erasure_minutes schedules. Our own design.';

-- The SCIM client that owns each GROUP it created.
--
-- A SCIM Group belongs to the client that created it, and every other client
-- answers 404 for it (RFC 7644 §3: a client manages its own resources). A
-- Group need not carry an externalId (RFC 7643 §3.1, the §8.4 example has
-- none), so its owner cannot be read from its identifiers; the creating
-- transaction writes the row before it commits the GROUP, and the party
-- protection reads it in every later commit of that GROUP. A row outlives the
-- GROUP's logical deletion, after which its id answers 404. No openEHR spec
-- governs SCIM or this table: our own design.
CREATE TABLE scim_group (
    -- The GROUP's versioned-object container.
    group_id       uuid PRIMARY KEY,
    -- The binding_issuer of the client that created it.
    binding_issuer text NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ix_scim_group_client ON scim_group (binding_issuer, group_id);
COMMENT ON TABLE scim_group IS 'The SCIM client that created each GROUP, which alone writes and reads it through /scim/v2/Groups. Our own design.';
