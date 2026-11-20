-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the definition stores have a revision a request reads (#5948).
--
-- A replica holds what it builds from the definition stores in memory: the
-- Web Template of each operational template and the archetype
-- specialisation lineage AQL widens a predicate with. A replica that did not
-- make a change used to answer from its copy until an eviction or a
-- time-to-live dropped it. definition_revision moves in the transaction of
-- every change of template_store or adl2_artefact, and a request reads it in
-- the one read of the policy heads it already makes, so a copy built before
-- the revision it read is never used for that request.
--
-- The revision moves once per transaction: the first statement takes the
-- row's lock, which the transaction then holds to its end, and the later
-- statements of the transaction find the transaction-local setting and write
-- nothing. No openEHR spec governs the caches: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE definition_revision (
    singleton  boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    revision   bigint NOT NULL,
    revised_at timestamptz NOT NULL DEFAULT now()
);

INSERT INTO definition_revision (singleton, revision) VALUES (true, 0);

COMMENT ON TABLE definition_revision IS 'The revision of template_store and adl2_artefact, which moves once in each transaction that changes either. A request reads it to confirm the definition copies a replica holds. Our own design.';

CREATE OR REPLACE FUNCTION definition_changed() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = clinical, pg_catalog
    AS $$
BEGIN
    IF current_setting('ferroehr.definition_revised', true) IS DISTINCT FROM pg_current_xact_id()::text THEN
        UPDATE definition_revision
           SET revision = revision + 1, revised_at = now()
         WHERE singleton;
        PERFORM set_config('ferroehr.definition_revised', pg_current_xact_id()::text, true);
    END IF;
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION definition_changed() IS
    'Moves definition_revision.revision once in the transaction, so every replica drops the definition copies it built before the change at its next request.';

CREATE TRIGGER template_store_definition_changed
    AFTER INSERT OR UPDATE OR DELETE OR TRUNCATE ON template_store
    FOR EACH STATEMENT EXECUTE FUNCTION definition_changed();

CREATE TRIGGER adl2_artefact_definition_changed
    AFTER INSERT OR UPDATE OR DELETE OR TRUNCATE ON adl2_artefact
    FOR EACH STATEMENT EXECUTE FUNCTION definition_changed();

-- PUBLIC holds EXECUTE on a new function by default (PostgreSQL 18, GRANT,
-- "Notes"); the trigger fires without it.
REVOKE ALL ON FUNCTION definition_changed() FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, UPDATE ON definition_revision TO ferroehr_clinical;
        REVOKE ALL ON definition_revision
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping the definition revision grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
