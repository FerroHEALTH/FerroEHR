-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the selectors of the one PERSON mapping SCIM, just-in-time creation
-- and the login binding share (#5013).
--
-- binding_identifier gains the SCIM selectors of a PERSON (userName and
-- externalId) and of a GROUP (displayName), the folded value userName and
-- displayName are compared by, and the partial unique indexes that hold each
-- selector to one live party, so a concurrent SCIM create and just-in-time
-- creation cannot both commit. binding_state records the digest of the
-- mapping the rows were derived under, and binding_lease the replicas
-- running against them, so a replica with another mapping refuses to start
-- and a refill waits for every replica to stop. No openEHR spec governs a
-- mapping from SCIM or OpenID Connect to a demographic template: our own
-- design.
--
-- Runs with search_path = party, ext, public.

-- ── the fold ────────────────────────────────────────────────────────────────
-- The RFC 8265 §3.3 UsernameCaseMapped preparation of a userName or a group
-- displayName: fullwidth and halfwidth code points mapped to their
-- decomposition (§3.3.2 rule 1, here the Halfwidth and Fullwidth Forms block
-- and the ideographic space), then lower case (rule 2, the Unicode full case
-- mapping of the PostgreSQL 18 pg_unicode_fast collation), then NFC (rule 3).
-- RFC 7643 §4.1.1 makes userName "case insensitive". The one function serves
-- the derivation, the unique index and a SCIM filter, so the three compare
-- alike.
CREATE FUNCTION binding_fold(p_value text) RETURNS text
    LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE
    SET search_path = pg_catalog
    RETURN (
        SELECT normalize(
                   lower(coalesce(string_agg(
                       CASE WHEN c ~ '[　！-￮]' THEN normalize(c, NFKC) ELSE c END,
                       '' ORDER BY n), '') COLLATE pg_unicode_fast),
                   NFC)
        FROM regexp_split_to_table(p_value, '') WITH ORDINALITY AS t(c, n)
    );

COMMENT ON FUNCTION binding_fold(text) IS 'The RFC 8265 UsernameCaseMapped preparation (width mapping, case mapping, NFC) a userName and a group displayName are compared by. Our own design.';

REVOKE ALL ON FUNCTION binding_fold(text) FROM PUBLIC;

-- ── binding_identifier ──────────────────────────────────────────────────────
ALTER TABLE binding_identifier
    DROP CONSTRAINT ck_binding_identifier_selector,
    ADD CONSTRAINT ck_binding_identifier_selector
        CHECK (selector IN ('key', 'user_name', 'external_id',
                            'group_external_id', 'group_display_name')),
    -- The value a selector is compared by: the fold of a userName or a group
    -- displayName, and the canonical value of every other selector.
    ADD COLUMN folded text;

UPDATE binding_identifier SET folded = value;
ALTER TABLE binding_identifier ALTER COLUMN folded SET NOT NULL;

-- One live binding key per issuer and value: a second PERSON binding a key
-- another one holds is refused at its commit.
CREATE UNIQUE INDEX uq_binding_identifier_key
    ON binding_identifier (issuer, value)
    WHERE selector = 'key' AND retired_at IS NULL;

-- One live userName across the service provider: RFC 7643 §4.1.1, userName
-- "MUST be unique across the service provider's entire set of Users".
CREATE UNIQUE INDEX uq_binding_identifier_user_name
    ON binding_identifier (folded)
    WHERE selector = 'user_name' AND retired_at IS NULL;

-- One live externalId per provisioning client, whose issuer names it
-- (RFC 7643 §3.1: externalId is "scoped to the provisioning domain").
CREATE UNIQUE INDEX uq_binding_identifier_external_id
    ON binding_identifier (selector, issuer, value)
    WHERE selector IN ('external_id', 'group_external_id') AND retired_at IS NULL;

-- One live group displayName.
CREATE UNIQUE INDEX uq_binding_identifier_display_name
    ON binding_identifier (folded)
    WHERE selector = 'group_display_name' AND retired_at IS NULL;

-- A selector looked up by its folded value (a SCIM filter on userName or
-- displayName).
CREATE INDEX ix_binding_identifier_folded
    ON binding_identifier (selector, folded) WHERE retired_at IS NULL;

COMMENT ON COLUMN binding_identifier.folded IS 'The value the selector is compared by: binding_fold(value) for a userName or a group displayName, the canonical value otherwise.';

-- ── binding_state ───────────────────────────────────────────────────────────
-- The digest of the mapping the rows were derived under. A SQL migration
-- cannot read the configured selectors, so the first boot fills the rows and
-- stores the digest; a replica whose mapping digest differs refuses to start,
-- and only `ferroehr party-binding refill` changes it.
CREATE TABLE binding_state (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    digest    text NOT NULL,
    filled_at timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE binding_state IS 'The digest of the party mapping the binding side table was filled under. Our own design.';

-- ── binding_lease ───────────────────────────────────────────────────────────
-- One row per running replica, renewed while it runs: a refill refuses while
-- any lease is live, so no replica derives rows under an old mapping while
-- the table is rebuilt under a new one.
CREATE TABLE binding_lease (
    replica    uuid PRIMARY KEY,
    digest     text NOT NULL,
    renewed_at timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE binding_lease IS 'The replicas running against the binding side table, each with the mapping digest it runs and when it last renewed its lease. Our own design.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
        GRANT EXECUTE ON FUNCTION party.binding_fold(text) TO ferroehr_party;
    ELSE
        RAISE NOTICE 'skipping the binding fold grant (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
