-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the schema itself, its search-path contract and the default
-- privileges every later relation inherits.
--
-- The clinical pseudonymisation domain holds the EHR-owned versioned objects.
-- The party domain holds the demographic ones, and neither domain's runtime
-- role may read the other's relations: GDPR Art. 4(5) defines pseudonymisation
-- as processing where attribution to a person needs additional information
-- "kept separately and subject to technical and organisational measures", and
-- Art. 32(1)(a) names it a security measure for Art. 9 health data
-- (https://eur-lex.europa.eu/eli/reg/2016/679/oj). EDPB Guidelines 01/2025
-- require that separation to hold against internal actors, operators with
-- database access included
-- (https://www.edpb.europa.eu/system/files/2025-01/edpb_guidelines_202501_pseudonymisation_en.pdf).
--
-- No openEHR spec governs storage layout or database roles: the wire contract
-- and the versioning semantics are unchanged, and where each side physically
-- lives is our own design.
--
-- SEARCH PATH. Every relation in this set is created and referenced
-- unqualified, and the pool serving this domain opens each connection with
-- `search_path = clinical, ext, public`. That one setting is the whole routing
-- mechanism: the same storage code serves either domain, because the SQL it
-- emits names its relations unqualified and the connection decides which
-- schema they resolve in. The party schema is deliberately absent from this
-- path, so a statement issued on this pool against a party relation fails to
-- resolve rather than quietly crossing the boundary.
--
-- THE INSTANCE IS SINGLE-TENANT. No relation in this schema carries a tenant
-- column, a tenancy row policy or a session-GUC dependency: isolation between
-- organisations is a deployment property (one instance, one database, one set
-- of domain roles), which is stronger than a row predicate. Our own
-- design/extension.

CREATE SCHEMA IF NOT EXISTS clinical;

COMMENT ON SCHEMA clinical IS 'The clinical pseudonymisation domain: the EHR-owned versioned objects and their change control, physically separated from the party domain so no single runtime role reads both (GDPR Art. 4(5), Art. 32(1)(a); EDPB 01/2025). No openEHR spec governs storage layout — our own design/extension.';

DO $$
BEGIN
    -- Lock down the public schema: no PUBLIC CREATE (PostgreSQL 18,
    -- "Privileges", https://www.postgresql.org/docs/18/ddl-priv.html).
    BEGIN
        REVOKE CREATE ON SCHEMA public FROM PUBLIC;
    EXCEPTION WHEN insufficient_privilege THEN
        RAISE NOTICE 'skipping public-schema lockdown (not schema owner)';
    END;
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT USAGE ON SCHEMA clinical
            TO ferroehr_clinical, ferroehr_clinical_reader;
        -- Every relation the later files in this set create is reachable
        -- without a manual grant (PostgreSQL 18, ALTER DEFAULT PRIVILEGES,
        -- https://www.postgresql.org/docs/18/sql-alterdefaultprivileges.html).
        -- The explicit grants over the finished relations are 0010.
        ALTER DEFAULT PRIVILEGES IN SCHEMA clinical
            GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES
            TO ferroehr_clinical;
        ALTER DEFAULT PRIVILEGES IN SCHEMA clinical
            GRANT SELECT ON TABLES TO ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping clinical schema grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
