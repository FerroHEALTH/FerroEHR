-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the binding side table.
--
-- What the party binding reads to bind a caller to a party, derived from the
-- committed party versions in the commit transaction: the identifiers, the
-- memberships, the representations, the clients and capabilities a policy
-- grants, the caller facts, the record subjects, the patient identifiers and
-- the binding-key spaces, with the revisions a replica checks its copy
-- against and the triggers that notify every replica of a change. No openEHR
-- spec governs caller binding: our own design.
--
-- Runs with search_path = party, ext, public.

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

CREATE FUNCTION binding_change_set(items jsonb) RETURNS text
    LANGUAGE sql STABLE
    RETURN CASE
        WHEN current_setting('ferroehr.binding_changes', true) = '["*"]' THEN '["*"]'
        WHEN length(coalesce(current_setting('ferroehr.binding_changes', true), ''))
             + length(items::text) > 7800 THEN '["*"]'
        ELSE (coalesce(nullif(current_setting('ferroehr.binding_changes', true), ''), '[]')::jsonb
              || items)::text
    END;
COMMENT ON FUNCTION binding_change_set(jsonb) IS
    'The transaction''s binding change set with items added, or "*" once it no longer fits a notification (#6403).';

CREATE FUNCTION binding_changed() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = party, pg_catalog
    AS $$
BEGIN
    -- Each statement adds what it changed to the transaction's change set,
    -- which binding_revision_committed notifies at commit: the parties of the
    -- rows, the key of each key row and the issuer of each other identifier
    -- row. A statement trigger fires on a statement that changed no row
    -- (PostgreSQL CREATE TRIGGER), so an empty change set returns at once; a
    -- key space, a TRUNCATE and a row trigger change what no row names, "*".
    IF TG_TABLE_NAME = 'binding_identifier' AND TG_OP = 'UPDATE' THEN
        PERFORM set_config('ferroehr.binding_changes', binding_change_set(s.items), true)
          FROM (SELECT jsonb_agg(DISTINCT x) AS items
                  FROM (SELECT * FROM changed UNION ALL SELECT * FROM gone) b,
                       LATERAL (VALUES (jsonb_build_object('p', b.party_id)),
                           (CASE WHEN b.selector = 'key'
                                 THEN jsonb_build_object('k', jsonb_build_array(
                                          b.issuer, b.identifier_type, b.value))
                                 ELSE jsonb_build_object('g', b.issuer) END)) v (x)) s
         WHERE s.items IS NOT NULL;
    ELSIF TG_TABLE_NAME = 'binding_identifier' AND TG_OP <> 'TRUNCATE' THEN
        PERFORM set_config('ferroehr.binding_changes', binding_change_set(s.items), true)
          FROM (SELECT jsonb_agg(DISTINCT x) AS items
                  FROM changed b,
                       LATERAL (VALUES (jsonb_build_object('p', b.party_id)),
                           (CASE WHEN b.selector = 'key'
                                 THEN jsonb_build_object('k', jsonb_build_array(
                                          b.issuer, b.identifier_type, b.value))
                                 ELSE jsonb_build_object('g', b.issuer) END)) v (x)) s
         WHERE s.items IS NOT NULL;
    ELSIF TG_TABLE_NAME = 'binding_membership' AND TG_OP = 'UPDATE' THEN
        PERFORM set_config('ferroehr.binding_changes', binding_change_set(s.items), true)
          FROM (SELECT jsonb_agg(DISTINCT jsonb_build_object('p', p)) AS items
                  FROM (SELECT * FROM changed UNION ALL SELECT * FROM gone) b,
                       LATERAL (VALUES (b.member_id), (b.group_id)) v (p)) s
         WHERE s.items IS NOT NULL;
    ELSIF TG_TABLE_NAME = 'binding_membership' AND TG_OP <> 'TRUNCATE' THEN
        PERFORM set_config('ferroehr.binding_changes', binding_change_set(s.items), true)
          FROM (SELECT jsonb_agg(DISTINCT jsonb_build_object('p', p)) AS items
                  FROM changed b, LATERAL (VALUES (b.member_id), (b.group_id)) v (p)) s
         WHERE s.items IS NOT NULL;
    ELSIF TG_TABLE_NAME = 'binding_representation' AND TG_OP = 'UPDATE' THEN
        PERFORM set_config('ferroehr.binding_changes', binding_change_set(s.items), true)
          FROM (SELECT jsonb_agg(DISTINCT jsonb_build_object('p', p)) AS items
                  FROM (SELECT * FROM changed UNION ALL SELECT * FROM gone) b,
                       LATERAL (VALUES (b.representative_id), (b.subject_id)) v (p)) s
         WHERE s.items IS NOT NULL;
    ELSIF TG_TABLE_NAME = 'binding_representation' AND TG_OP <> 'TRUNCATE' THEN
        PERFORM set_config('ferroehr.binding_changes', binding_change_set(s.items), true)
          FROM (SELECT jsonb_agg(DISTINCT jsonb_build_object('p', p)) AS items
                  FROM changed b,
                       LATERAL (VALUES (b.representative_id), (b.subject_id)) v (p)) s
         WHERE s.items IS NOT NULL;
    ELSIF TG_LEVEL = 'STATEMENT' AND TG_OP <> 'TRUNCATE' THEN
        PERFORM set_config('ferroehr.binding_changes', '["*"]', true)
          WHERE EXISTS (SELECT FROM changed);
    ELSE
        PERFORM set_config('ferroehr.binding_changes', '["*"]', true);
    END IF;
    IF NOT FOUND THEN
        RETURN NULL;
    END IF;
    IF current_setting('ferroehr.binding_revised', true) IS DISTINCT FROM pg_current_xact_id()::text THEN
        UPDATE binding_grant_revision
           SET binding_revision = binding_revision + 1, revised_at = now()
         WHERE singleton;
        PERFORM set_config('ferroehr.binding_revised', pg_current_xact_id()::text, true);
    END IF;
    RETURN NULL;
END;
$$;
COMMENT ON FUNCTION binding_changed() IS
    'When the statement changed at least one binding row (its transition table changed is not empty, or it is a TRUNCATE or a row trigger), adds what it changed to the transaction''s change set and moves binding_grant_revision.binding_revision, once in the transaction, so no request takes a lookup read before the change; a statement that changed no row moves nothing (#6284, #6403).';

CREATE FUNCTION person_version_changed() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = party, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM pg_notify('ferroehr_party_binding', OLD.vo_id::text);
    ELSE
        PERFORM pg_notify('ferroehr_party_binding', NEW.vo_id::text);
    END IF;
    RETURN NULL;
END;
$$;
COMMENT ON FUNCTION person_version_changed() IS
    'Notifies ferroehr_party_binding with the container of a PERSON whose version was written, changed or removed, so every replica drops its name.';

CREATE TRIGGER version_person_written
    AFTER INSERT OR UPDATE ON version
    FOR EACH ROW WHEN (NEW.kind = 'PERSON') EXECUTE FUNCTION person_version_changed();

CREATE TRIGGER version_person_removed
    AFTER DELETE ON version
    FOR EACH ROW WHEN (OLD.kind = 'PERSON') EXECUTE FUNCTION person_version_changed();

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
    -- The value the selector is compared by: binding_fold(value) for a
    -- userName or a group displayName, the canonical value otherwise.
    folded           text NOT NULL,
    CONSTRAINT ck_binding_identifier_selector
        CHECK (selector IN ('key', 'user_name', 'external_id',
                            'group_external_id', 'group_display_name')),
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
CREATE TRIGGER binding_identifier_inserted
    AFTER INSERT ON binding_identifier REFERENCING NEW TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_identifier_updated
    AFTER UPDATE ON binding_identifier REFERENCING OLD TABLE AS gone NEW TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_identifier_deleted
    AFTER DELETE ON binding_identifier REFERENCING OLD TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_identifier_truncated
    AFTER TRUNCATE ON binding_identifier
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
COMMENT ON TABLE binding_identifier IS 'The selector identifiers of each party head the login binding reads: a PERSON''s binding key with its lifecycle, professional ROLE and representation state, and a GROUP''s externalId. Derived from the party versions in each commit''s transaction; no openEHR spec governs it, our own design.';
COMMENT ON COLUMN binding_identifier.professional IS 'The dates of the PERSON''s professional ROLEs: the union of the time_validity (DV_INTERVAL<DV_DATE>) of each listed ROLE of the mapped role template whose performer names the PERSON and whose head is complete or incomplete.';
COMMENT ON COLUMN binding_identifier.deprovisioned_at IS 'Set on the retired key rows of a deleted PERSON: a login on the key is refused as deprovisioned until a live row binds it again.';
COMMENT ON COLUMN binding_identifier.folded IS 'The value the selector is compared by: binding_fold(value) for a userName or a group displayName, the canonical value otherwise.';

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
CREATE INDEX ix_binding_membership_group_order
    ON binding_membership (group_id, written_at, member_id) WHERE retired_at IS NULL;
CREATE TRIGGER binding_membership_inserted
    AFTER INSERT ON binding_membership REFERENCING NEW TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_membership_updated
    AFTER UPDATE ON binding_membership REFERENCING OLD TABLE AS gone NEW TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_membership_deleted
    AFTER DELETE ON binding_membership REFERENCING OLD TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_membership_truncated
    AFTER TRUNCATE ON binding_membership
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
COMMENT ON TABLE binding_membership IS 'The live members of each GROUP of the mapped group template, from the inline PARTY_RELATIONSHIPs of the membership archetype each member PERSON holds with the GROUP as target; standalone relationship containers never count. No openEHR spec governs it, our own design.';

-- ── binding_representation ──────────────────────────────────────────────────
-- One row per representation a PERSON is the source of: a PARTY_RELATIONSHIP
-- of its own named openehr_access::represents, from the representative's
-- PERSON to the subject's, with its basis, its provider, the
-- subject_relationship code it names, its validity and the party that wrote
-- it (draft access control package, "A representation is" and The
-- Relationship Store, #6079). A representative is admitted only on a live row
-- for that subject whose provider the caller acts for and that it did not
-- write itself (decision procedure, step 0 Representation; Writing
-- Relationships).
CREATE TABLE binding_representation (
    representative_id uuid NOT NULL,
    subject_id        uuid NOT NULL,
    -- The openEHR subject_relationship code the relationship names; NULL when
    -- it names none.
    relationship      text,
    validity          datemultirange NOT NULL,
    written_at        timestamptz NOT NULL DEFAULT now(),
    retired_at        timestamptz,
    relationship_id   uuid NOT NULL,
    basis_terminology text NOT NULL,
    basis_code        text NOT NULL,
    provider_issuer   text NOT NULL,
    provider_id       text NOT NULL,
    writer_id         uuid
);
-- A container's live row, which its commit retires.
CREATE UNIQUE INDEX uq_binding_representation_container
    ON binding_representation (relationship_id) WHERE retired_at IS NULL;
-- Who ever represented whom.
--
-- A patient-portal login whose PERSON has ever represented the EHR's subject
-- is judged only as a representative, so ending or deleting the
-- representation never leaves the former representative admitted as the
-- patient. binding_representation keeps its retired rows, and a refill
-- rebuilds them from every version that carried the representation code; this
-- index answers whether a representative has ever represented a subject, live
-- or retired, and is the one read a representative's admission takes. No
-- openEHR spec governs the side table: our own design.
CREATE INDEX ix_binding_representation_history
    ON binding_representation (representative_id, subject_id)
    INCLUDE (retired_at, validity, relationship);
CREATE TRIGGER binding_representation_inserted
    AFTER INSERT ON binding_representation REFERENCING NEW TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_representation_updated
    AFTER UPDATE ON binding_representation REFERENCING OLD TABLE AS gone NEW TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_representation_deleted
    AFTER DELETE ON binding_representation REFERENCING OLD TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_representation_truncated
    AFTER TRUNCATE ON binding_representation
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
COMMENT ON COLUMN binding_representation.relationship IS 'The openEHR subject_relationship code the relationship names; NULL when it names none.';
COMMENT ON COLUMN binding_representation.relationship_id IS 'The PARTY_RELATIONSHIP container the row is derived from.';
COMMENT ON COLUMN binding_representation.basis_terminology IS 'The terminology of the representation''s basis.';
COMMENT ON COLUMN binding_representation.basis_code IS 'The code of the representation''s basis.';
COMMENT ON COLUMN binding_representation.provider_issuer IS 'The issuer of the representation''s provider DV_IDENTIFIER.';
COMMENT ON COLUMN binding_representation.provider_id IS 'The id of the representation''s provider DV_IDENTIFIER.';
COMMENT ON COLUMN binding_representation.writer_id IS 'The container the committer of the relationship''s head names in external_ref: the PERSON or client ACTOR that wrote it; NULL when the committer names none.';
COMMENT ON INDEX ix_binding_representation_history IS 'Whether a representative ever represented a subject, live or retired: the patient-portal check that a former representative is never the patient (#5532), and the read a representative is admitted on (#6079).';
COMMENT ON TABLE binding_representation IS 'The representations each PERSON is the source of: the PARTY_RELATIONSHIP containers of their own named openehr_access::represents, with their basis, provider and writer, live at the head and retired for every earlier one. No openEHR spec governs it, our own design.';

-- ── binding_lease ───────────────────────────────────────────────────────────
-- One row per running replica, renewed while it runs: a refill refuses while
-- any lease is live, so no replica derives rows under an old mapping while
-- the table is rebuilt under a new one.
CREATE TABLE binding_lease (
    replica    uuid PRIMARY KEY,
    digest     text NOT NULL,
    renewed_at timestamptz NOT NULL DEFAULT now(),
    -- The binding-key issuers the replica derives key rows for, and those of
    -- them whose key is a portal login.
    issuers    text[] NOT NULL DEFAULT '{}',
    portal     text[] NOT NULL DEFAULT '{}'
);
COMMENT ON TABLE binding_lease IS 'The replicas running against the binding side table, each with the mapping digest it runs and when it last renewed its lease. Our own design.';
COMMENT ON COLUMN binding_lease.digest IS 'The digest of the mapping the replica runs, without its key issuers.';
COMMENT ON COLUMN binding_lease.issuers IS 'The binding-key issuers the replica derives key rows for.';
COMMENT ON COLUMN binding_lease.portal IS 'The issuers among issuers that are patients issuers, whose key is a portal login.';

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
COMMENT ON TABLE binding_state IS 'The digest of the party mapping, without its key issuers, the binding side table was filled under. Our own design.';

-- A membership is held by its member.
--
-- A membership PARTY_RELATIONSHIP has the member PERSON as its source and the
-- GROUP as its target, so it is stored in the PERSON's version (RM demographic
-- master02 §Party Relationships: relationships "are stored as part of the data
-- of the PARTY designated as the source"), and a change of one membership
-- versions one PERSON, not the whole GROUP. binding_membership keeps one live
-- row per (GROUP, member), now derived from the PERSON heads, and serves a
-- Group's members.
--
-- The GROUP gets no version when its members change, so its SCIM version is
-- its head plus the revision below, which moves in the same transaction as
-- each change to its binding_membership rows (RFC 7644 §3.14: the version
-- changes when the resource does). No openEHR spec governs this table: our own
-- design.
CREATE TABLE binding_group_revision (
    -- The GROUP's versioned-object container.
    group_id   uuid PRIMARY KEY,
    -- How many changes its live memberships have had.
    revision   bigint NOT NULL,
    -- When the last of them was committed.
    revised_at timestamptz NOT NULL DEFAULT now()
);
COMMENT ON TABLE binding_group_revision IS 'The membership revision of each GROUP, which moves with every change of its live binding_membership rows; a SCIM Group''s version is its GROUP head and this revision. Our own design.';

-- ── binding_capability ──────────────────────────────────────────────────────
-- One row per operation a CAPABILITY of a ROLE head that is not deleted grants.
CREATE TABLE binding_capability (
    -- The ROLE's version container.
    role_id         uuid NOT NULL,
    -- The CAPABILITY's position in ROLE.capabilities, from 0.
    ordinal         integer NOT NULL,
    -- The ferroehr-operations code the credentials hold, such as import_ehr;
    -- a code the terminology does not define is kept as written and is never
    -- honoured.
    operation       text NOT NULL,
    -- The container ROLE.performer names.
    performer_id    uuid NOT NULL,
    -- The ROLE head's version_lifecycle_state.
    lifecycle_state text NOT NULL,
    -- The dates ROLE.time_validity holds: every date when absent, none when
    -- a bound cannot be read.
    role_validity   daterange NOT NULL,
    -- The dates CAPABILITY.time_validity holds, under the same rule, and none
    -- when a Not before or Not after bound in the credentials cannot be read.
    validity        daterange NOT NULL,
    -- The Not before and Not after instants of the credentials, the elements
    -- the terminology names not_before and not_after, for a bound finer than a
    -- day; NULL when the credentials carry none.
    not_before      timestamptz,
    not_after       timestamptz,
    -- CAPABILITY.credentials as written.
    credentials     jsonb,
    written_at      timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (role_id, ordinal, operation),
    CONSTRAINT ck_binding_capability_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);
CREATE INDEX ix_binding_capability_performer ON binding_capability (performer_id);
COMMENT ON TABLE binding_capability IS 'The operations the capabilities of each ROLE head that is not deleted grant, with the dates and instants each holds. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading a capability as a grant, our own design.';

-- ── binding_provider ────────────────────────────────────────────────────────
-- One row per ORGANISATION head that is not deleted and carries an identifier.
CREATE TABLE binding_provider (
    -- The ORGANISATION's version container.
    party_id   uuid PRIMARY KEY,
    -- DV_IDENTIFIER.issuer.
    issuer     text NOT NULL,
    -- DV_IDENTIFIER.id.
    id         text NOT NULL,
    written_at timestamptz NOT NULL DEFAULT now()
);
COMMENT ON TABLE binding_provider IS 'The identifier each ORGANISATION head that is not deleted carries, by which a system access names the provider its client acts for. Derived from the party versions in each commit''s transaction; no openEHR spec governs it, our own design.';

-- ── binding_client ──────────────────────────────────────────────────────────
-- One row per client identifier of an ACTOR head that is not deleted.
CREATE TABLE binding_client (
    -- The ACTOR's version container.
    party_id        uuid NOT NULL,
    -- DV_IDENTIFIER.issuer: the token issuer.
    issuer          text NOT NULL,
    -- DV_IDENTIFIER.id: the OAuth 2.0 client_id.
    client_id       text NOT NULL,
    -- The ACTOR head's version_lifecycle_state (RM common master06 §Version
    -- Lifecycle); an inactive or abandoned ACTOR holds no grant.
    lifecycle_state text NOT NULL,
    written_at      timestamptz NOT NULL DEFAULT now(),
    -- The container the client ACTOR's acts-for relationship targets; NULL
    -- when it holds none.
    provider_id     uuid,
    CONSTRAINT ck_binding_client_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);
-- One ACTOR per client: a second ACTOR naming a client another one names is
-- refused at its commit.
CREATE UNIQUE INDEX uq_binding_client ON binding_client (issuer, client_id);
CREATE INDEX ix_binding_client_party ON binding_client (party_id);
COMMENT ON TABLE binding_client IS 'The client identifiers of each ACTOR head that is not deleted, by token issuer and client_id. Derived from the party versions in each commit''s transaction; no openEHR spec governs it, our own design.';

-- ── binding_grant_revision ──────────────────────────────────────────────────
-- Moves with every change of binding_client or binding_capability, in the same
-- transaction, so a replica whose copy is current answers from it with no
-- further read.
CREATE TABLE binding_grant_revision (
    singleton  boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    revision   bigint NOT NULL,
    revised_at timestamptz NOT NULL DEFAULT now(),
    -- The revision of the binding side table, which moves once in each
    -- transaction that changes it (binding_changed).
    binding_revision bigint NOT NULL DEFAULT 0
);
INSERT INTO binding_grant_revision (singleton, revision) VALUES (true, 0);
COMMENT ON TABLE binding_grant_revision IS 'The revision of binding_client and binding_capability, which moves with each change of either. Our own design.';
COMMENT ON COLUMN binding_grant_revision.binding_revision IS 'The revision of the binding side table (binding_identifier, binding_membership, binding_representation and the state of binding_key_space), which moves once in each transaction that changes one of them.';

CREATE FUNCTION binding_revision_committed() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = party, pg_catalog
    AS $$
BEGIN
    -- NOTE: PostgreSQL NOTIFY, a payload is shorter than 8000 bytes; a longer change set is "*".
    PERFORM pg_notify('ferroehr_party_binding', CASE WHEN length(n.payload) < 7900
                      THEN n.payload
                      ELSE jsonb_build_object('r', NEW.binding_revision, 'c', '["*"]'::jsonb)::text END)
      FROM (SELECT jsonb_build_object('r', NEW.binding_revision, 'c', coalesce(nullif(
                       current_setting('ferroehr.binding_changes', true), ''), '["*"]')::jsonb)::text
                   AS payload) n;
    RETURN NULL;
END;
$$;
COMMENT ON FUNCTION binding_revision_committed() IS
    'Notifies ferroehr_party_binding at commit with the binding revision the transaction moved to and its change set (binding_changed), so each replica drops the lookups the change set names and holds the others at that revision (#6403).';
CREATE CONSTRAINT TRIGGER binding_revision_moved
    AFTER UPDATE ON binding_grant_revision DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW WHEN (OLD.binding_revision IS DISTINCT FROM NEW.binding_revision)
    EXECUTE FUNCTION binding_revision_committed();

-- ── binding_caller_fact ─────────────────────────────────────────────────────
-- One row per fact a party head that is not deleted holds: the roles a ROLE
-- names for its performer, the type an ORGANISATION names for itself, the
-- memberships of a PERSON or ORGANISATION, and the care relationships a
-- PARTY_RELATIONSHIP states.
CREATE TABLE binding_caller_fact (
    -- The container whose head holds the fact.
    holder_id       uuid NOT NULL,
    -- The CAPABILITY's position in ROLE.capabilities, or the relationship's
    -- position in PARTY.relationships, from 0.
    ordinal         integer NOT NULL,
    -- What the fact is: role, organisation_type, or the code of a
    -- membership (MBR of v3-RoleClass) or a care relationship (under_care_of
    -- of openehr_access).
    kind            text NOT NULL,
    -- The party the fact is about: the container ROLE.performer names, or the
    -- PERSON a relationship is of, its source.
    party_id        uuid NOT NULL,
    -- The CODE_PHRASE of a role, an organisation type or a care
    -- relationship's care_role.
    terminology_id  text,
    code_string     text,
    -- The container a care relationship or a membership targets.
    target_id       uuid,
    -- The holder head's version_lifecycle_state.
    lifecycle_state text NOT NULL,
    -- The dates ROLE.time_validity holds for a role, and every date for any
    -- other fact.
    role_validity   daterange NOT NULL,
    -- The dates CAPABILITY.time_validity or PARTY_RELATIONSHIP.time_validity
    -- holds: every date when absent, none when a bound cannot be read.
    validity        daterange NOT NULL,
    -- The Not before and Not after instants of a capability's credentials.
    not_before      timestamptz,
    not_after       timestamptz,
    written_at      timestamptz NOT NULL DEFAULT now(),
    -- The RM type of a care relationship's target; NULL for every other fact.
    target_kind     text,
    -- The provider DV_IDENTIFIER of a care relationship; NULL for every other
    -- fact.
    provider_issuer text,
    provider_id     text,
    -- The PERSON or client ACTOR the committer of the holder's head names:
    -- the writer of the fact; NULL when the committer names none.
    writer_id       uuid,
    -- Whether a care relationship's basis is self_declared.
    self_declared   boolean NOT NULL DEFAULT false,
    -- The OBJECT_VERSION_ID of the holder's trunk head the fact was derived
    -- from, which the access log names for a care relationship.
    holder_version  text,
    -- The covers_for DV_IDENTIFIER of a care relationship, its issuer, id
    -- and the carer's kind its type names; NULL when it names none and for
    -- every other fact.
    covers_issuer   text,
    covers_id       text,
    covers_kind     text,
    CONSTRAINT ck_binding_caller_fact_kind
        CHECK (kind IN ('role', 'organisation_type', 'MBR', 'under_care_of', 'related_to')),
    CONSTRAINT ck_binding_caller_fact_value
        CHECK (CASE WHEN kind = 'under_care_of'
                    THEN target_id IS NOT NULL
                         AND target_kind IN ('PERSON', 'ROLE', 'GROUP', 'ORGANISATION')
                         AND terminology_id IS NOT NULL AND code_string IS NOT NULL
                         AND provider_issuer IS NOT NULL AND provider_id IS NOT NULL
                    WHEN kind = 'MBR'
                    THEN target_id IS NOT NULL AND target_kind IS NULL
                         AND terminology_id IS NULL AND code_string IS NULL
                         AND provider_issuer IS NULL AND provider_id IS NULL
                    WHEN kind = 'related_to'
                    THEN target_id IS NOT NULL AND target_kind IS NULL
                         AND terminology_id IS NOT NULL AND code_string IS NOT NULL
                         AND provider_issuer IS NULL AND provider_id IS NULL
                    ELSE target_id IS NULL AND target_kind IS NULL
                         AND terminology_id IS NOT NULL AND code_string IS NOT NULL
                         AND provider_issuer IS NULL AND provider_id IS NULL END),
    CONSTRAINT ck_binding_caller_fact_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801')),
    CONSTRAINT ck_binding_caller_fact_self_declared
        CHECK (NOT self_declared OR kind = 'under_care_of'),
    CONSTRAINT ck_binding_caller_fact_covers
        CHECK ((covers_issuer IS NULL) = (covers_id IS NULL)
               AND (covers_issuer IS NULL) = (covers_kind IS NULL)
               AND (covers_kind IS NULL
                    OR covers_kind IN ('PERSON', 'ROLE', 'GROUP', 'ORGANISATION'))
               AND (covers_issuer IS NULL OR kind = 'under_care_of'))
);
CREATE UNIQUE INDEX uq_binding_caller_fact
    ON binding_caller_fact (holder_id, ordinal, kind, terminology_id, code_string, target_id)
    NULLS NOT DISTINCT;
-- Every fact about one party, the read a replica makes for the parties whose
-- facts changed.
CREATE INDEX ix_binding_caller_fact_party ON binding_caller_fact (party_id);
-- The patients a care team cares for, read by the team.
--
-- A replica's copy of the caller facts no longer holds every patient's care
-- relationships. When a query's access condition needs the patients a
-- professional's teams care for, the replica reads each team's care
-- relationships once and holds them in a bounded cache, so the read is
-- answered by the team rather than by a scan of every fact.
--
-- No openEHR spec governs the side table or its index: our own design.
CREATE INDEX ix_binding_caller_fact_care_team ON binding_caller_fact (target_id)
    WHERE kind = 'under_care_of';
COMMENT ON COLUMN binding_caller_fact.kind IS 'What the fact is: role, the coded name of a ROLE the party performs, organisation_type, the coded name of the ORGANISATION itself, the code of a care relationship, MBR of v3-RoleClass or under_care_of of openehr_access, or related_to of openehr_access, a kinship of the party to the person target_id names (#6077).';
COMMENT ON COLUMN binding_caller_fact.target_kind IS 'The RM type of a care relationship''s target: PERSON, ROLE, GROUP or ORGANISATION; NULL for every other fact.';
COMMENT ON COLUMN binding_caller_fact.provider_issuer IS 'The issuer of a care relationship''s provider DV_IDENTIFIER; NULL for every other fact.';
COMMENT ON COLUMN binding_caller_fact.provider_id IS 'The id of a care relationship''s provider DV_IDENTIFIER; NULL for every other fact.';
COMMENT ON COLUMN binding_caller_fact.holder_version IS 'The OBJECT_VERSION_ID of the holder''s trunk head the fact was derived from, which the access log names for a care relationship.';
COMMENT ON COLUMN binding_caller_fact.covers_issuer IS 'The issuer of a care relationship''s covers_for DV_IDENTIFIER; NULL when it names none and for every other fact.';
COMMENT ON COLUMN binding_caller_fact.covers_id IS 'The id of a care relationship''s covers_for DV_IDENTIFIER; NULL when it names none and for every other fact.';
COMMENT ON COLUMN binding_caller_fact.covers_kind IS 'The type of a care relationship''s covers_for DV_IDENTIFIER: the RM type of the carer it covers for, PERSON, ROLE, GROUP or ORGANISATION; NULL when it names none and for every other fact.';
COMMENT ON COLUMN binding_caller_fact.terminology_id IS 'The terminology of a role, an organisation type or a care relationship''s care_role.';
COMMENT ON COLUMN binding_caller_fact.code_string IS 'The code of a role, an organisation type or a care relationship''s care_role.';
COMMENT ON TABLE binding_caller_fact IS 'The roles each ROLE head that is not deleted names for its performer, the type each ORGANISATION head names for itself, the memberships of each PERSON and ORGANISATION head, the care relationship each PARTY_RELATIONSHIP head of its own states about its source, and the kinship each related_to PARTY_RELATIONSHIP head states about each of its two persons. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as access facts, our own design.';
COMMENT ON COLUMN binding_caller_fact.writer_id IS 'The container the committer of the holder''s head names in external_ref: the PERSON or client ACTOR that wrote the fact; NULL when the committer names none.';
COMMENT ON COLUMN binding_caller_fact.self_declared IS 'Whether a care relationship''s basis is self_declared; false for every other fact.';

-- ── binding_caller_fact_party ───────────────────────────────────────────────
-- The transaction that last changed each party's facts, so a replica reads
-- again only the facts of the parties its copy's snapshot did not see. A
-- commit writes only the rows of the parties it changed: no row is shared by
-- commits about different parties.
CREATE TABLE binding_caller_fact_party (
    party_id    uuid PRIMARY KEY,
    stamped_xid xid8 NOT NULL
);
CREATE INDEX ix_binding_caller_fact_party_stamped ON binding_caller_fact_party (stamped_xid);
COMMENT ON TABLE binding_caller_fact_party IS 'The transaction that last changed the caller facts of each party. Our own design.';
COMMENT ON COLUMN binding_caller_fact_party.stamped_xid IS 'pg_current_xact_id() of the transaction that last changed the party''s caller facts.';

-- Whether a stamp exists that the snapshot held did not see: a copy read
-- under held is behind the store. The xmin bound keeps it a range scan of
-- ix_binding_caller_fact_party_stamped (PostgreSQL 19, "Transaction ID and
-- Snapshot Information Functions"). A copy never read (NULL) is behind.
CREATE FUNCTION facts_behind(held pg_snapshot) RETURNS boolean
    LANGUAGE sql STABLE PARALLEL SAFE
BEGIN ATOMIC
    SELECT held IS NULL OR EXISTS (
        SELECT 1 FROM binding_caller_fact_party p
         WHERE p.stamped_xid >= pg_snapshot_xmin(held)
           AND NOT pg_visible_in_snapshot(p.stamped_xid, held));
END;
COMMENT ON FUNCTION facts_behind(pg_snapshot) IS 'Whether binding_caller_fact_party holds a stamp the snapshot held did not see, so a copy read under it is behind. Our own design.';

-- ── binding_record_subject ──────────────────────────────────────────────────
-- One row per reference a PERSON head that is not deleted declares.
CREATE TABLE binding_record_subject (
    -- The PERSON container whose head declares the reference.
    holder_id       uuid NOT NULL,
    -- The element's position among the record_subject elements of the
    -- PERSON's details, depth first, from 0.
    ordinal         integer NOT NULL,
    -- The DV_IDENTIFIER issuer: the reference's namespace.
    namespace       text NOT NULL,
    -- The DV_IDENTIFIER id: the reference's id, as written.
    subject_id      text NOT NULL,
    -- The holder head's version_lifecycle_state.
    lifecycle_state text NOT NULL,
    written_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_binding_record_subject PRIMARY KEY (holder_id, ordinal),
    -- One reference names one PERSON. Deferred to the end of the transaction,
    -- so a contribution that moves a reference from one PERSON to another,
    -- such as after an identity merge, commits its versions in any order; the
    -- commit sets it IMMEDIATE before it ends, which reports a collision.
    CONSTRAINT uq_binding_record_subject UNIQUE (namespace, subject_id)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT ck_binding_record_subject_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);
COMMENT ON TABLE binding_record_subject IS 'The EHR subject references each PERSON head that is not deleted declares by a record_subject element of its details, a code of FerroEHR''s operation terminology. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as the link from an EHR to its subject''s PERSON, our own design.';

-- ── binding_patient_identifier ──────────────────────────────────────────────
-- One row per identifier a patient PERSON head that is not deleted carries.
CREATE TABLE binding_patient_identifier (
    -- The PERSON container whose head carries the identifier.
    holder_id       uuid NOT NULL,
    -- The identifier's position among the PERSON's patient identifiers, in
    -- document order, from 0.
    ordinal         integer NOT NULL,
    -- The DV_IDENTIFIER issuer: the identifier's domain.
    issuer          text NOT NULL,
    -- The DV_IDENTIFIER type, empty when the identifier names none.
    identifier_type text NOT NULL,
    -- The DV_IDENTIFIER id as the stored body carries it: the value, or the
    -- reference to its sealed row for a protected scheme.
    value           text NOT NULL,
    -- The holder head's version_lifecycle_state.
    lifecycle_state text NOT NULL,
    written_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_binding_patient_identifier PRIMARY KEY (holder_id, ordinal),
    CONSTRAINT ck_binding_patient_identifier_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);
-- The lookup by issuer and value; a protected identifier is reached from
-- national_identifier's digest index and the primary key instead.
CREATE INDEX ix_binding_patient_identifier_value
    ON binding_patient_identifier (issuer, value) INCLUDE (holder_id, lifecycle_state);
COMMENT ON TABLE binding_patient_identifier IS 'The identifiers each patient PERSON head that is not deleted carries at the [authz.party_binding.patient] identifier path, by issuer, each id as the body carries it. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as the way to a patient''s EHRs, our own design.';

-- ── binding_key_space ───────────────────────────────────────────────────────
CREATE TABLE binding_key_space (
    issuer     text PRIMARY KEY,
    portal     boolean NOT NULL,
    state      text NOT NULL,
    fill_after uuid,
    changed_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT ck_binding_key_space_state
        CHECK (state IN ('pending', 'filling', 'active', 'retiring')),
    CONSTRAINT ck_binding_key_space_fill_after
        CHECK (state = 'filling' OR fill_after IS NULL)
);
-- A key space's fill progress decides no lookup; its state does.
CREATE TRIGGER binding_key_space_inserted
    AFTER INSERT ON binding_key_space REFERENCING NEW TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_key_space_deleted
    AFTER DELETE ON binding_key_space REFERENCING OLD TABLE AS changed
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_key_space_truncated
    AFTER TRUNCATE ON binding_key_space
    FOR EACH STATEMENT EXECUTE FUNCTION binding_changed();
CREATE TRIGGER binding_key_space_state_changed
    AFTER UPDATE OF state ON binding_key_space
    FOR EACH ROW WHEN (OLD.state IS DISTINCT FROM NEW.state)
    EXECUTE FUNCTION binding_changed();
COMMENT ON TABLE binding_key_space IS 'The key space of each binding-key issuer: its portal flag and its state, pending until every live replica names it, filling while its key rows are filled online, active while they bind, retiring once a live replica no longer names it. Our own design.';
COMMENT ON COLUMN binding_key_space.fill_after IS 'The last PERSON container the online fill of a filling key space has written, so the next page resumes after it.';

-- The end a care relationship written without one takes from the
-- access policy (#6073).
--
-- The draft access control decision procedure, Writing Relationships: "A care
-- relationship written without an end takes the default_period of the first
-- ACCESS_POLICY.relationship_validity entry that covers its role and provider
-- type". The committed version stays exactly as it was sent, so its body,
-- digest and signature are untouched; the end it takes is computed once, in
-- the commit's transaction, from the policy version the request confirmed,
-- and kept here with that version. A refill of the caller facts reads it and
-- never recomputes it, so a later policy version applies only to the
-- relationships written after it. The caller fact of the relationship takes
-- it as the upper bound of its validity. No openEHR spec governs the side
-- table: our own design.
CREATE TABLE binding_care_validity (
    -- The care relationship's PARTY_RELATIONSHIP version container.
    vo_id          uuid PRIMARY KEY,
    -- The OBJECT_VERSION_ID of the version the end was computed for.
    version_uid    text NOT NULL,
    -- The first date the relationship no longer holds.
    effective_until date NOT NULL,
    -- The OBJECT_VERSION_ID of the ACCESS_POLICY version that set it.
    policy_version text NOT NULL,
    written_at     timestamptz NOT NULL DEFAULT now()
);
COMMENT ON TABLE binding_care_validity IS 'The end each care relationship head written without one takes from the access policy version its commit confirmed, with that version. Written in the commit''s transaction and kept by a refill, which never recomputes it; no openEHR spec governs it, our own design.';
COMMENT ON COLUMN binding_care_validity.effective_until IS 'The first date the relationship no longer holds: its start, or the commit''s date when it names none, plus the default period.';
COMMENT ON COLUMN binding_care_validity.policy_version IS 'The OBJECT_VERSION_ID of the ACCESS_POLICY version whose relationship_validity entry gave the period.';
