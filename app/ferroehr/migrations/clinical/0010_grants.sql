-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the grants over the finished relations.
--
-- One file per domain carries every grant, so the catalog sweep and the boot
-- gate have exactly one file to read per domain. Grants are explicit and
-- non-overlapping: the clinical roles hold this schema and nothing of the
-- party or linkage domains; the reciprocal revokes are issued by whichever
-- set runs last, because a REVOKE cannot name a schema that does not exist
-- yet, and the clinical set runs first.
--
-- No openEHR spec governs database grants: our own operational design.
--
-- Runs with search_path = clinical, ext, public.
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT USAGE ON SCHEMA clinical
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA clinical
            TO ferroehr_clinical;
        GRANT SELECT ON ALL TABLES IN SCHEMA clinical
            TO ferroehr_clinical_reader;
        -- The subject guard runs as the writer that triggers it.
        GRANT EXECUTE ON FUNCTION clinical.subject_pseudonym_guard()
            TO ferroehr_clinical;
        -- The party and linkage roles hold nothing here. The revoke is not
        -- redundant with never having granted: PUBLIC holds privileges by
        -- default on some object kinds (PostgreSQL 18, GRANT, "Notes",
        -- https://www.postgresql.org/docs/18/sql-grant.html).
        REVOKE ALL ON SCHEMA clinical
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
        REVOKE ALL ON ALL TABLES IN SCHEMA clinical
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping clinical grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
