-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: protected national identifiers — the value leaves the versioned body
-- and lives here, sealed, with a keyed digest beside it for equality lookup.
--
-- The RM models identifiers generically (PARTY_IDENTITY, DV_IDENTIFIER) and
-- says nothing about protecting the value, so no openEHR spec governs this:
-- our own design. The obligations are external: GDPR Art. 32(1)(a) names
-- encryption as an appropriate measure
-- (https://eur-lex.europa.eu/eli/reg/2016/679/oj), NEN 7510-2 carries the
-- cryptographic controls, and UAVG Art. 46 with the Wabvpz permit BSN
-- processing in care only for identification and under the act's conditions
-- (https://wetten.overheid.nl/BWBR0040940, https://wetten.overheid.nl/BWBR0023864).
--
-- Two columns rather than one, because storage and lookup want opposite
-- properties. `ciphertext` is AES-256-GCM with a fresh nonce per record, so
-- two parties with the same identifier store different bytes and neither
-- reveals the value. `lookup_digest` is HMAC-SHA-256 under a separate subkey,
-- so equality search works without decrypting — and, because it is KEYED, the
-- database, a backup or a read replica cannot reverse it by enumerating a
-- nine-digit space.
--
-- Runs with search_path = party, ext, public.

-- ── the scheme registry ──────────────────────────────────────────────────────
-- A closed set rather than free text: an identifier whose scheme nobody
-- declared is an identifier nobody agreed to hold, and the registry is what a
-- reviewer reads to see which kinds this deployment stores at all.
CREATE TABLE identifier_scheme (
    -- The scheme code, `<iso-3166-1-alpha-2 lowercased>-<local name>`, the
    -- same shape the write-path scanner keys its rules by.
    code         text NOT NULL,
    -- The issuing jurisdiction, ISO 3166-1 alpha-2.
    jurisdiction text NOT NULL,
    -- The identifier's own name in its register.
    label        text NOT NULL,
    -- The register or authority that defines it.
    source       text NOT NULL,
    CONSTRAINT pk_identifier_scheme PRIMARY KEY (code),
    CONSTRAINT ck_identifier_scheme_code CHECK (code ~ '^[a-z]{2}-[a-z0-9-]+$'),
    CONSTRAINT ck_identifier_scheme_jurisdiction CHECK (jurisdiction ~ '^[A-Z]{2}$')
);

COMMENT ON TABLE identifier_scheme IS 'The national personal-identifier kinds this deployment may store, by code. A closed set: an identifier whose scheme is not registered here cannot be written.';

INSERT INTO identifier_scheme (code, jurisdiction, label, source) VALUES
    ('nl-bsn', 'NL', 'burgerservicenummer (BSN)',
     'Rijksdienst voor Identiteitsgegevens, Logisch Ontwerp BSN, https://www.rvig.nl/logisch-ontwerp-bsn');

-- ── the protected values ─────────────────────────────────────────────────────
CREATE TABLE national_identifier (
    id            uuid NOT NULL DEFAULT uuidv7(),
    -- The party this identifier belongs to (the demographic VERSIONED_OBJECT
    -- id). No foreign key: a party's versions come and go under change
    -- control, and an identifier outliving a particular version is correct.
    party_id      uuid NOT NULL,
    -- The registered scheme.
    scheme        text NOT NULL,
    -- AES-256-GCM: the 96-bit nonce and the sealed value.
    nonce         bytea NOT NULL,
    ciphertext    bytea NOT NULL,
    -- HMAC-SHA-256 of the value under the lookup subkey.
    lookup_digest bytea NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_national_identifier PRIMARY KEY (id),
    CONSTRAINT fk_national_identifier_scheme
        FOREIGN KEY (scheme) REFERENCES identifier_scheme (code),
    CONSTRAINT ck_national_identifier_nonce CHECK (octet_length(nonce) = 12),
    CONSTRAINT ck_national_identifier_digest CHECK (octet_length(lookup_digest) = 32)
);

-- One party holds one value per scheme, and one value identifies one party:
-- both directions are the point of a national identifier, and a duplicate in
-- either direction is a data defect the store refuses rather than a resolution
-- that silently picks a row.
CREATE UNIQUE INDEX uq_national_identifier_party
    ON national_identifier (scheme, party_id);
CREATE UNIQUE INDEX uq_national_identifier_value
    ON national_identifier (scheme, lookup_digest);

COMMENT ON TABLE national_identifier IS 'National personal identifiers, sealed under an AES-256-GCM key with an HMAC-SHA-256 lookup digest beside them. The versioned body carries a reference to a row here, never the value.';
COMMENT ON COLUMN national_identifier.ciphertext IS 'AES-256-GCM sealed value; the scheme is the associated data, so a row moved between schemes fails to open.';
COMMENT ON COLUMN national_identifier.lookup_digest IS 'HMAC-SHA-256 of the value under the lookup subkey: deterministic for equality search, keyed so the small value space cannot be enumerated.';

-- ── the resolution path ──────────────────────────────────────────────────────
-- The only way to get from an identifier to a party without holding the
-- ciphertext. It takes the DIGEST rather than the value, so the caller proves
-- it already knows the identifier and the plaintext never crosses this
-- boundary; it returns the party and nothing else.
CREATE FUNCTION resolve_national_identifier(
    p_scheme text,
    p_digest bytea
) RETURNS uuid
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = party, pg_catalog
    AS $$
    SELECT party_id
    FROM party.national_identifier
    WHERE scheme = p_scheme
      AND lookup_digest = p_digest;
$$;

COMMENT ON FUNCTION resolve_national_identifier(text, bytea) IS 'The sole identifier-to-party path for a role without ciphertext access. Takes the keyed lookup digest, never the value, and returns the party id alone. Every call is recorded as a linkage-domain access event by the caller.';

REVOKE ALL ON FUNCTION resolve_national_identifier(text, bytea) FROM PUBLIC;
