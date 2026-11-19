-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the binding side table has a revision a request reads (#6048).
--
-- A replica holds the lookups of the binding side table in memory and drops
-- them when ferroehr_party_binding notifies a change (0051). A notification
-- arrives after the commit it reports, so a decision taken on a held lookup
-- rested on that delay. binding_grant_revision gains binding_revision, which
-- moves in the transaction of every change the notification reports: a key
-- row, a membership, a representation or a key space's state. A request reads
-- the row's three revisions in one indexed read and takes a held lookup only
-- when it was read at that revision or a later one.
--
-- The revision moves once per transaction: the first change takes the row's
-- lock, which the transaction then holds to its end, as a change of the
-- caller facts does (0039), and the later changes of the transaction find
-- the transaction-local setting and write nothing. No openEHR spec governs
-- resolving a login to a party: our own design.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE binding_grant_revision ADD COLUMN binding_revision bigint NOT NULL DEFAULT 0;

COMMENT ON COLUMN binding_grant_revision.binding_revision IS 'The revision of the binding side table (binding_identifier, binding_membership, binding_representation and the state of binding_key_space), which moves once in each transaction that changes one of them.';

CREATE OR REPLACE FUNCTION binding_changed() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = party, pg_catalog
    AS $$
BEGIN
    IF current_setting('ferroehr.binding_revised', true) IS DISTINCT FROM pg_current_xact_id()::text THEN
        UPDATE binding_grant_revision
           SET binding_revision = binding_revision + 1, revised_at = now()
         WHERE singleton;
        PERFORM set_config('ferroehr.binding_revised', pg_current_xact_id()::text, true);
    END IF;
    PERFORM pg_notify('ferroehr_party_binding', '');
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION binding_changed() IS
    'Moves binding_grant_revision.binding_revision once in the transaction and notifies ferroehr_party_binding that the binding side table changed, so every replica drops the lookups it holds and no request takes one read before the change.';
