-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the schema of the local IHE ATNA Audit Record Repository.
--
-- No openEHR spec governs audit storage mechanics: our own design/extension.
-- openEHR endorses in-system access logging and rules it OUT of the EHR
-- content — "read accesses by application users to EHR data should be logged
-- in the EHR system. ... currently openEHR does not support [logs as part of
-- the EHR proper]" (BASE architecture_overview/master07-security.adoc §Access
-- logging) — so this schema is strictly separate from the clinical one: no
-- foreign key into any EHR-content relation, no AQL visibility, never
-- versioned.
--
-- The record identifies the INSTANCE by the system id it carries in its FHIR
-- payload; there is no tenant column, because the instance is single-tenant
-- and isolation between organisations is a deployment property.
--
-- The roles are created by the ext set, which runs first. Runs with
-- search_path = audit, ext, public.

CREATE SCHEMA IF NOT EXISTS audit;

COMMENT ON SCHEMA audit IS 'The local IHE ATNA Audit Record Repository (IHE ITI TF-1 §9): the node''s security-surveillance record, strictly outside the EHR content (BASE architecture_overview/master07-security.adoc §Access logging).';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT USAGE ON SCHEMA audit
            TO ferroehr_clinical, ferroehr_clinical_reader;
        -- The audit trail is not a pseudonymisation domain: the clinical
        -- runtime role writes it (the store writes through the clinical pool),
        -- and each reader holds what its writer holds minus the writes.
        ALTER DEFAULT PRIVILEGES IN SCHEMA audit
            GRANT SELECT, INSERT ON TABLES TO ferroehr_clinical;
        ALTER DEFAULT PRIVILEGES IN SCHEMA audit
            GRANT SELECT ON TABLES TO ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping audit schema grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
