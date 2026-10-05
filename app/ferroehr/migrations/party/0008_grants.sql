-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the grants over the finished relations, and the reciprocal revokes
-- that make the domain boundary hold in both directions.
--
-- This set runs after the clinical one, so it is the first place a REVOKE can
-- name both schemas. The revoke is not redundant with never having granted: an
-- earlier blanket grant may exist and PUBLIC holds privileges by default on
-- some object kinds (PostgreSQL 18, GRANT, "Notes",
-- https://www.postgresql.org/docs/18/sql-grant.html).
--
-- The sealed identifier store is narrower still: only the party writer reaches
-- the ciphertext, and the party READER holds every column but that one, so a
-- reporting role cannot read sealed values it has no business decrypting.
--
-- No openEHR spec governs database grants: our own operational design.
--
-- Runs with search_path = party, ext, public.
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
        GRANT USAGE ON SCHEMA party
            TO ferroehr_party, ferroehr_party_reader;
        GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA party
            TO ferroehr_party;
        GRANT SELECT ON ALL TABLES IN SCHEMA party
            TO ferroehr_party_reader;

        -- The sealed value is the party writer's alone, and so is the lookup
        -- digest: a holder of the lookup subkey could otherwise ask whether a
        -- known identifier is present without ever decrypting anything.
        REVOKE SELECT ON party.national_identifier FROM ferroehr_party_reader;
        GRANT SELECT (id, party_id, scheme, created_at)
            ON party.national_identifier TO ferroehr_party_reader;
        GRANT EXECUTE ON FUNCTION party.resolve_national_identifier(text, bytea)
            TO ferroehr_party;

        -- And the explicit denial in both directions.
        REVOKE ALL ON SCHEMA party
            FROM ferroehr_clinical, ferroehr_clinical_reader, ferroehr_linkage;
        REVOKE ALL ON ALL TABLES IN SCHEMA party
            FROM ferroehr_clinical, ferroehr_clinical_reader, ferroehr_linkage;
        REVOKE ALL ON FUNCTION party.resolve_national_identifier(text, bytea)
            FROM ferroehr_clinical, ferroehr_clinical_reader, ferroehr_linkage;
        REVOKE ALL ON SCHEMA clinical
            FROM ferroehr_party, ferroehr_party_reader;
        REVOKE ALL ON ALL TABLES IN SCHEMA clinical
            FROM ferroehr_party, ferroehr_party_reader;
    ELSE
        RAISE NOTICE 'skipping party grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
