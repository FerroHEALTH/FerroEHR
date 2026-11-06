-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the binding side table the login-to-party lookup reads on every
-- request (#5004).
--
-- A login is bound to the PERSON whose binding-key identifier it carries. No
-- index over party node rows can express "the trunk head of a non-deleted
-- PERSON of a mapped template, at the key selector's path, issuer and type",
-- so the rows the lookup needs are derived from the party versions into
-- these relations, in each party commit's own transaction, after the pattern
-- of national_identifier (0010_identifier_versions.sql): the rows of a party's
-- current head are LIVE, and `retired_at` marks the rows of a superseded or
-- deleted head. Every row is derivable from the versions, so a refill
-- rebuilds the relations from the versions alone.
--
-- RM demographic master02-demographic_package.adoc: identifiers "given by
-- organisations or the state" are "recorded as part of the data in the
-- PARTY._details_ structure" (§Party Identification); a ROLE's `performer` is
-- the "Reference to Version container of Actor playing the role" with its
-- `time_validity` (org.openehr.rm.demographic.role.adoc); a PARTY_RELATIONSHIP
-- is stored "as part of the data of the PARTY designated as the source"
-- (§Party Relationships), with its own `time_validity`
-- (org.openehr.rm.demographic.party_relationship.adoc). Resolving a login to
-- a party is governed by no openEHR spec: the relations are our own design.
--
-- Runs with search_path = party, ext, public.

-- ── binding_identifier ──────────────────────────────────────────────────────
-- One row per selector identifier of a party head: the binding key of a
-- PERSON that performs a professional ROLE or is the source of a
-- representation relationship, and the externalId of a GROUP of the mapped
-- group template.
CREATE TABLE binding_identifier (
    -- The party's version container (HIER_OBJECT_ID root).
    party_id         uuid NOT NULL,
    -- Which selector of the mapping the identifier sits at.
    selector         text NOT NULL,
    -- DV_IDENTIFIER.issuer.
    issuer           text NOT NULL,
    -- DV_IDENTIFIER.type.
    identifier_type  text NOT NULL,
    -- DV_IDENTIFIER.id, in the canonical form its type declares (a GUID or
    -- UUID in lower case).
    value            text NOT NULL,
    -- The party head's version_lifecycle_state (RM common master06 §Version
    -- Lifecycle); a deleted head (523) has no live row.
    lifecycle_state  text NOT NULL,
    -- The dates on which the PERSON performs a professional ROLE: the union of
    -- the time_validity of every listed ROLE of the mapped role template
    -- whose performer names it and whose head is complete or incomplete. An
    -- empty multirange performs none; a ROLE without time_validity is valid
    -- on every date.
    professional     datemultirange NOT NULL DEFAULT '{}',
    -- Whether the PERSON is the source of a representation relationship,
    -- live or ended.
    representative   boolean NOT NULL DEFAULT false,
    written_at       timestamptz NOT NULL DEFAULT now(),
    -- When a later head (or the logical deletion) superseded the head this
    -- row was derived from; NULL while the row is live.
    retired_at       timestamptz,
    -- Set on the retired key rows of a deleted PERSON: the key is a tombstone
    -- the lookup answers `deprovisioned`, until a live row binds it again.
    deprovisioned_at timestamptz,
    CONSTRAINT ck_binding_identifier_selector
        CHECK (selector IN ('key', 'group_external_id')),
    CONSTRAINT ck_binding_identifier_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801')),
    CONSTRAINT ck_binding_identifier_tombstone
        CHECK (deprovisioned_at IS NULL OR (retired_at IS NOT NULL AND selector = 'key'))
);

-- The lookup: every live key row, and every tombstone, for one issuer, type
-- and value, answered from the index alone (the INCLUDE columns are every
-- column the lookup reads; PostgreSQL 18, "Index-Only Scans and Covering
-- Indexes", https://www.postgresql.org/docs/18/indexes-index-only-scans.html).
CREATE INDEX ix_binding_identifier_lookup
    ON binding_identifier (selector, issuer, identifier_type, value)
    INCLUDE (party_id, lifecycle_state, professional, representative,
             retired_at, deprovisioned_at)
    WHERE retired_at IS NULL OR deprovisioned_at IS NOT NULL;

-- Led by the party, so retiring a party's live rows on every commit, and
-- reading a group's live externalId, is an index range rather than a scan.
CREATE INDEX ix_binding_identifier_party
    ON binding_identifier (party_id) WHERE retired_at IS NULL;

COMMENT ON TABLE binding_identifier IS 'The selector identifiers of each party head the login binding reads: a PERSON''s binding key with its lifecycle, professional ROLE and representation state, and a GROUP''s externalId. Derived from the party versions in each commit''s transaction; no openEHR spec governs it, our own design.';
COMMENT ON COLUMN binding_identifier.professional IS 'The dates of the PERSON''s professional ROLEs: the union of the time_validity (DV_INTERVAL<DV_DATE>) of each listed ROLE of the mapped role template whose performer names the PERSON and whose head is complete or incomplete.';
COMMENT ON COLUMN binding_identifier.deprovisioned_at IS 'Set on the retired key rows of a deleted PERSON: a login on the key is refused as deprovisioned until a live row binds it again.';

-- ── binding_membership ──────────────────────────────────────────────────────
-- One row per member of a GROUP of the mapped group template: an inline
-- PARTY_RELATIONSHIP of the mapped membership archetype whose source is the
-- GROUP and whose target is the member. A version that keeps a membership
-- leaves its row alone; a removal retires it and a re-add inserts a new row,
-- so a membership change writes one row, not one per member.
CREATE TABLE binding_membership (
    group_id   uuid NOT NULL,
    member_id  uuid NOT NULL,
    written_at timestamptz NOT NULL DEFAULT now(),
    retired_at timestamptz
);

CREATE UNIQUE INDEX uq_binding_membership_member
    ON binding_membership (member_id, group_id) WHERE retired_at IS NULL;
CREATE INDEX ix_binding_membership_group
    ON binding_membership (group_id) WHERE retired_at IS NULL;

COMMENT ON TABLE binding_membership IS 'The live members of each GROUP of the mapped group template, from its inline PARTY_RELATIONSHIPs of the membership archetype; standalone relationship containers never count. No openEHR spec governs it, our own design.';

-- ── binding_representation ──────────────────────────────────────────────────
-- One row per representation relationship a PERSON is the source of: an
-- inline PARTY_RELATIONSHIP of the mapped representation archetype, from the
-- representative's PERSON to the subject's, with the subject_relationship
-- code its type maps to and its validity.
CREATE TABLE binding_representation (
    representative_id uuid NOT NULL,
    subject_id        uuid NOT NULL,
    -- The openEHR subject_relationship code the relationship's type maps to;
    -- NULL when the mapping maps none.
    relationship      text,
    -- The union of the time_validity of the relationships of this coded type
    -- between the two.
    validity          datemultirange NOT NULL,
    written_at        timestamptz NOT NULL DEFAULT now(),
    retired_at        timestamptz
);

CREATE UNIQUE INDEX uq_binding_representation_pair
    ON binding_representation (representative_id, subject_id, relationship)
    NULLS NOT DISTINCT WHERE retired_at IS NULL;

COMMENT ON TABLE binding_representation IS 'The representation relationships each PERSON is the source of, from its inline PARTY_RELATIONSHIPs of the mapped representation archetype. No openEHR spec governs it, our own design.';

-- ── the validity of a DV_INTERVAL<DV_DATE> ──────────────────────────────────
-- An ISO 8601 calendar date in extended or basic form, complete or reduced to
-- the month or the year (RM data_types DV_DATE). A reduced date reads as its
-- first day at a lower bound and its last day at an upper one. NULL when the
-- text is no such date.
CREATE FUNCTION binding_date(p_value text, p_upper boolean) RETURNS date
    LANGUAGE plpgsql IMMUTABLE STRICT PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
DECLARE
    digits text := replace(p_value, '-', '');
    first  date;
BEGIN
    IF digits !~ '^[0-9]{4}([0-9]{2}([0-9]{2})?)?$' THEN
        RETURN NULL;
    END IF;
    first := make_date(substr(digits, 1, 4)::integer,
                       coalesce(nullif(substr(digits, 5, 2), ''), '01')::integer,
                       coalesce(nullif(substr(digits, 7, 2), ''), '01')::integer);
    IF NOT p_upper OR length(digits) = 8 THEN
        RETURN first;
    ELSIF length(digits) = 6 THEN
        RETURN (first + interval '1 month' - interval '1 day')::date;
    END IF;
    RETURN (first + interval '1 year' - interval '1 day')::date;
EXCEPTION WHEN data_exception THEN
    RETURN NULL;
END $$;

-- The dates a time_validity (DV_INTERVAL<DV_DATE>) holds: every date when the
-- attribute is absent, and none when a bound cannot be read, so a validity
-- that cannot be read never makes a ROLE or a relationship live.
CREATE FUNCTION binding_validity(p_interval jsonb) RETURNS daterange
    LANGUAGE plpgsql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
DECLARE
    lower_bound date;
    upper_bound date;
    bounds      text;
BEGIN
    IF p_interval IS NULL OR jsonb_typeof(p_interval) <> 'object' THEN
        RETURN daterange(NULL, NULL);
    END IF;
    IF NOT coalesce((p_interval ->> 'lower_unbounded')::boolean, false)
       AND p_interval -> 'lower' IS NOT NULL THEN
        lower_bound := party.binding_date(p_interval -> 'lower' ->> 'value', false);
        IF lower_bound IS NULL THEN
            RETURN 'empty'::daterange;
        END IF;
    END IF;
    IF NOT coalesce((p_interval ->> 'upper_unbounded')::boolean, false)
       AND p_interval -> 'upper' IS NOT NULL THEN
        upper_bound := party.binding_date(p_interval -> 'upper' ->> 'value', true);
        IF upper_bound IS NULL THEN
            RETURN 'empty'::daterange;
        END IF;
    END IF;
    bounds := CASE WHEN coalesce((p_interval ->> 'lower_included')::boolean, true)
                   THEN '[' ELSE '(' END
           || CASE WHEN coalesce((p_interval ->> 'upper_included')::boolean, true)
                   THEN ']' ELSE ')' END;
    RETURN daterange(lower_bound, upper_bound, bounds);
EXCEPTION WHEN data_exception THEN
    RETURN 'empty'::daterange;
END $$;

COMMENT ON FUNCTION binding_validity(jsonb) IS 'The dates a DV_INTERVAL<DV_DATE> time_validity holds: every date when absent, none when a bound cannot be read. Our own design.';

-- PUBLIC holds EXECUTE on a new function by default (PostgreSQL 18, GRANT,
-- "Notes", https://www.postgresql.org/docs/18/sql-grant.html), so both are
-- revoked from it and granted to the party writer alone, which derives the
-- rows: no role of another domain reaches a party function.
REVOKE ALL ON FUNCTION binding_date(text, boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION binding_validity(jsonb) FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
        GRANT EXECUTE ON FUNCTION party.binding_date(text, boolean),
                                  party.binding_validity(jsonb)
            TO ferroehr_party;
    ELSE
        RAISE NOTICE 'skipping the binding function grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
