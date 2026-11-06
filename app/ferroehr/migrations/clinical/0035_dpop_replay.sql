-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the DPoP proof replay store (#5072).
--
-- RFC 9449 §11.1 asks a resource server to accept a DPoP proof once within the
-- window it accepts proofs in, and notes that single use "may not always be
-- feasible in practice, e.g., when multiple servers behind a single endpoint
-- have no shared state". Every replica of an instance shares the clinical
-- pool, so the record lives here: one row per accepted proof, keyed by the
-- base64url SHA-256 of its `jti` (43 characters, never the `jti` itself),
-- kept until the end of its window. A record past its end may be replaced
-- and is swept.
--
-- No openEHR spec governs authentication: our own design/extension.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE dpop_proof (
    proof      text        NOT NULL,
    expires_at timestamptz NOT NULL,
    CONSTRAINT pk_dpop_proof PRIMARY KEY (proof),
    CONSTRAINT ck_dpop_proof_digest CHECK (char_length(proof) = 43)
);

-- The sweep reads the records whose window ended, oldest first.
CREATE INDEX idx_dpop_proof_expires ON dpop_proof (expires_at);

COMMENT ON TABLE dpop_proof IS
    'The DPoP proofs every replica accepted (RFC 9449 §11.1): the base64url SHA-256 of each proof''s jti and the end of the window it is accepted in. No request content.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON dpop_proof TO ferroehr_clinical;
        REVOKE ALL ON dpop_proof FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping DPoP replay store grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
