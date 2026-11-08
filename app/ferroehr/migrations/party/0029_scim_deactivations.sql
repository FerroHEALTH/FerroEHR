-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the per-client counter that bounds how many Users a SCIM client
-- deactivates or deletes in one window (#5014).
--
-- One compromised provisioning client, or a scoping mistake at the identity
-- provider, could otherwise deactivate every professional at once. Every
-- SCIM deactivation and deletion reads and advances its client's row under a
-- FOR UPDATE lock inside its own write transaction, so the replicas sharing
-- the database share one bound. No openEHR spec governs SCIM or this table:
-- our own design.
--
-- Runs with search_path = party, ext, public.

CREATE TABLE scim_deactivation (
    -- The client's binding_issuer, which no two clients share.
    binding_issuer text PRIMARY KEY,
    -- When the current window began.
    window_start   timestamptz NOT NULL,
    -- The deactivations and deletions counted in the current window.
    counted        integer NOT NULL CHECK (counted >= 0)
);

COMMENT ON TABLE scim_deactivation IS 'The deactivations and deletions each SCIM client made in its current window, which [scim] max_deactivations bounds. Our own design.';
