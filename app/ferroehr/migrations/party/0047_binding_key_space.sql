-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: one key space per binding-key issuer, added and removed online
-- (#5688).
--
-- binding_state keeps the digest of the mapping without its key issuers;
-- each key issuer is a row of binding_key_space with its portal flag and a
-- state, and each replica's lease lists the key issuers it maintains. A
-- replica that names a new issuer records it pending; once every live lease
-- lists it, its key rows are filled online and it turns active. A replica
-- that drops an issuer turns it retiring, and its rows are deleted once no
-- live lease lists it. A key row binds only while its issuer's key space is
-- active. The stored digest covered the key issuers, so it is cleared: the
-- next boot fills the side table once under the new digest. No openEHR spec
-- governs the side table: our own design.
--
-- Runs with search_path = party, ext, public.

DELETE FROM binding_state;
DELETE FROM binding_lease;

-- ── binding_key_space ───────────────────────────────────────────────────────
CREATE TABLE binding_key_space (
    issuer     text PRIMARY KEY,
    portal     boolean NOT NULL,
    state      text NOT NULL,
    fill_after uuid,
    changed_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT ck_binding_key_space_state
        CHECK (state IN ('pending', 'filling', 'active', 'retiring')),
    CONSTRAINT ck_binding_key_space_fill_after
        CHECK (state = 'filling' OR fill_after IS NULL)
);

COMMENT ON TABLE binding_key_space IS 'The key space of each binding-key issuer: its portal flag and its state, pending until every live replica names it, filling while its key rows are filled online, active while they bind, retiring once a live replica no longer names it. Our own design.';
COMMENT ON COLUMN binding_key_space.fill_after IS 'The last PERSON container the online fill of a filling key space has written, so the next page resumes after it.';

-- ── binding_lease: the key issuers each replica maintains ───────────────────
ALTER TABLE binding_lease
    ADD COLUMN issuers text[] NOT NULL DEFAULT '{}',
    ADD COLUMN portal  text[] NOT NULL DEFAULT '{}';

COMMENT ON COLUMN binding_lease.digest IS 'The digest of the mapping the replica runs, without its key issuers.';
COMMENT ON COLUMN binding_lease.issuers IS 'The binding-key issuers the replica derives key rows for.';
COMMENT ON COLUMN binding_lease.portal IS 'The issuers among issuers that are patients issuers, whose key is a portal login.';
COMMENT ON TABLE binding_state IS 'The digest of the party mapping, without its key issuers, the binding side table was filled under. Our own design.';
