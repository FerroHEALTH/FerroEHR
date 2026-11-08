-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the PERSONs of deleted SCIM Users whose contact values are erased
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
--
-- Runs with search_path = party, ext, public.

CREATE TABLE scim_contact_erasure (
    -- The PERSON's versioned object.
    party_id  uuid PRIMARY KEY,
    -- When its contacts were erased.
    erased_at timestamptz NOT NULL
);

COMMENT ON TABLE scim_contact_erasure IS 'The PERSONs of deleted SCIM Users whose contact values every version has had erased, which [scim] contact_erasure_minutes schedules. Our own design.';
