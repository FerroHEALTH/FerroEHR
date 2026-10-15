-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: one sealed row per party version, never rewritten.
--
-- A versioned repository "is by definition indelible", and every modification
-- is "achieved by physically committing new Versions" (RM common
-- master06-change_control_package.adoc §Contributions). A party version's body
-- references its identifier rows, so a row rewritten in place would change
-- what an older version expands to. Each version therefore seals rows of its
-- own, and the rows of the party's latest version are LIVE: `retired_at` is
-- set when a later version (or the logical deletion) supersedes them.
--
-- The one-row-per-party-and-scheme and one-party-per-value rules hold for the
-- live rows only, so they become partial unique indexes, and resolution
-- matches live rows only. Every existing row is the current value of its
-- party, so all of them stay live and both rules hold for them as before.
--
-- No openEHR spec governs identifier storage: our own design.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE national_identifier ADD COLUMN retired_at timestamptz;

COMMENT ON COLUMN national_identifier.retired_at IS 'When a later version of the party superseded the version this row belongs to; NULL while the row is live. A retired row still opens for the version that references it.';

DROP INDEX uq_national_identifier_party;
DROP INDEX uq_national_identifier_value;

-- Led by the party, so retiring a party's live rows on every commit is an
-- index range rather than a scan.
CREATE UNIQUE INDEX uq_national_identifier_party
    ON national_identifier (party_id, scheme) WHERE retired_at IS NULL;
CREATE UNIQUE INDEX uq_national_identifier_value
    ON national_identifier (scheme, lookup_digest) WHERE retired_at IS NULL;

-- CREATE OR REPLACE keeps the function's owner and privileges (PostgreSQL 18,
-- CREATE FUNCTION, https://www.postgresql.org/docs/18/sql-createfunction.html),
-- so the grants of 0004_identifiers.sql and 0008_grants.sql stand.
CREATE OR REPLACE FUNCTION resolve_national_identifier(
    p_scheme text,
    p_digest bytea
) RETURNS uuid
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = party, pg_catalog
    AS $$
    SELECT party_id
    FROM party.national_identifier
    WHERE scheme = p_scheme
      AND lookup_digest = p_digest
      AND retired_at IS NULL;
$$;

COMMENT ON FUNCTION resolve_national_identifier(text, bytea) IS 'The sole identifier-to-party path for a role without ciphertext access. Takes the keyed lookup digest, never the value, and returns the party whose latest version holds it, and nothing else. Every call is recorded as a linkage-domain access event by the caller.';

-- The party reader sees which rows are live, as it sees which party holds one.
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party_reader') THEN
        GRANT SELECT (retired_at) ON party.national_identifier TO ferroehr_party_reader;
    ELSE
        RAISE NOTICE 'skipping the party reader grant (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
