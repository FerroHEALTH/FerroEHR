-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the schema itself, its search-path contract and the default
-- privileges every later relation inherits.
--
-- The party pseudonymisation domain holds the demographic versioned objects —
-- PERSON, ORGANISATION, GROUP, AGENT, ROLE and PARTY_RELATIONSHIP. They do not
-- live beside the clinical record, and no runtime role reads both: one role,
-- one backup and one compromised connection must not hold a clinical record
-- and the identity of its subject together.
--
-- GDPR Art. 4(5) defines pseudonymisation as processing where attribution to a
-- person needs additional information "kept separately and subject to
-- technical and organisational measures"; Art. 32(1)(a) names it a security
-- measure for Art. 9 health data
-- (https://eur-lex.europa.eu/eli/reg/2016/679/oj). EDPB Guidelines 01/2025
-- require that separation to hold against internal actors, operators with
-- database access included
-- (https://www.edpb.europa.eu/system/files/2025-01/edpb_guidelines_202501_pseudonymisation_en.pdf).
--
-- No openEHR spec governs storage layout or database roles: the wire contract
-- (ITS-REST Demographic API) and the versioning semantics (RM common
-- master06-change_control_package.adoc) are unchanged, and where each side
-- physically lives is our own design.
--
-- SEARCH PATH. Every relation in this set is created and referenced
-- unqualified, and the pool serving this domain opens each connection with
-- `search_path = party, ext, public`. The clinical schema is deliberately
-- absent from that path, so a statement issued on this pool against a clinical
-- relation fails to resolve rather than quietly crossing the boundary. Nothing
-- in this schema references the clinical one, in either direction.
--
-- THE INSTANCE IS SINGLE-TENANT. No relation in this schema carries a tenant
-- column, a tenancy row policy or a session-GUC dependency: isolation between
-- organisations is a deployment property. Our own design/extension.

CREATE SCHEMA IF NOT EXISTS party;

COMMENT ON SCHEMA party IS 'The party pseudonymisation domain: the demographic versioned objects and their change control, physically separated from the clinical schema so no single runtime role reads both (GDPR Art. 4(5), Art. 32(1)(a); EDPB 01/2025). No openEHR spec governs storage layout — our own design/extension.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
        GRANT USAGE ON SCHEMA party
            TO ferroehr_party, ferroehr_party_reader;
        -- Every relation the later files in this set create is reachable
        -- without a manual grant (PostgreSQL 18, ALTER DEFAULT PRIVILEGES).
        -- The explicit grants over the finished relations are 0007.
        ALTER DEFAULT PRIVILEGES IN SCHEMA party
            GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO ferroehr_party;
        ALTER DEFAULT PRIVILEGES IN SCHEMA party
            GRANT SELECT ON TABLES TO ferroehr_party_reader;
    ELSE
        RAISE NOTICE 'skipping party schema grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
