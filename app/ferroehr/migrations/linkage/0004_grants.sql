-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- linkage: the grants, in both directions.
--
-- Neither direction: linkage out of the two domains it joins, and both of them
-- out of linkage. A role that could read this schema and one of the others
-- would hold the join this schema exists to withhold. The revoke is not
-- redundant with never having granted — an earlier blanket grant may exist and
-- PUBLIC holds privileges by default on some object kinds (PostgreSQL 18,
-- GRANT, "Notes", https://www.postgresql.org/docs/18/sql-grant.html).
--
-- No openEHR spec governs database grants: our own operational design.
--
-- Runs with search_path = linkage, ext, public.

-- Unconditional, and first: EXECUTE is granted to PUBLIC by default on a new
-- function, which on a SECURITY DEFINER one would hand every role the owner's
-- reach — including the DELETE this schema grants nobody. This runs outside the
-- role block below because it must hold whether or not the runtime roles were
-- provisioned.
REVOKE ALL ON FUNCTION linkage.erase_ehr(uuid) FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_linkage') THEN
        GRANT USAGE ON SCHEMA linkage TO ferroehr_linkage;
        GRANT SELECT, INSERT, UPDATE ON ALL TABLES IN SCHEMA linkage
            TO ferroehr_linkage;
        ALTER DEFAULT PRIVILEGES IN SCHEMA linkage
            GRANT SELECT, INSERT, UPDATE ON TABLES TO ferroehr_linkage;
        GRANT USAGE ON SCHEMA ext TO ferroehr_linkage;

        -- Erasure, and only erasure, reaches this schema's rows
        -- destructively. The role holds no DELETE on subject_ehr; it may
        -- EXECUTE the definer function that performs the one deletion the law
        -- requires (linkage/0003_erase_ehr).
        GRANT EXECUTE ON FUNCTION linkage.erase_ehr(uuid) TO ferroehr_linkage;

        REVOKE ALL ON SCHEMA clinical, party FROM ferroehr_linkage;
        REVOKE ALL ON ALL TABLES IN SCHEMA clinical, party FROM ferroehr_linkage;
        REVOKE ALL ON FUNCTION party.resolve_national_identifier(text, bytea)
            FROM ferroehr_linkage;

        REVOKE ALL ON SCHEMA linkage
            FROM ferroehr_clinical, ferroehr_clinical_reader, ferroehr_party, ferroehr_party_reader;
        REVOKE ALL ON ALL TABLES IN SCHEMA linkage
            FROM ferroehr_clinical, ferroehr_clinical_reader, ferroehr_party, ferroehr_party_reader;
        REVOKE ALL ON FUNCTION linkage.erase_ehr(uuid)
            FROM ferroehr_clinical, ferroehr_clinical_reader, ferroehr_party, ferroehr_party_reader;
    ELSE
        RAISE NOTICE 'skipping linkage grants (role absent — see the ext role block NOTICE)';
    END IF;
END $$;
