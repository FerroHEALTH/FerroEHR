-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the caller facts the access evaluator reads beside the grants
-- (#5511): sensitivity clearances, permitted purposes and care relationships.
--
-- A clearance and a permitted purpose are elements anywhere in the
-- credentials of a CAPABILITY of a ROLE (RM demographic
-- org.openehr.rm.demographic.capability.adoc: "the qualifications of the
-- performer of the role for this capability"), named by the code
-- sensitivity_clearance or permitted_purpose of FerroEHR's terminology
-- (ferroehr-operations), whose value is a DV_CODED_TEXT of the terminology the
-- jurisdiction names, such as an HL7 ActCode sensitivity or an HL7
-- PurposeOfUse code. A care relationship is a PARTY_RELATIONSHIP of a PERSON
-- whose name or details carry member_of (the professional is a member of a
-- care team) or under_care_of (the patient is in the care of a team or unit)
-- of that terminology, in any archetype (RM demographic
-- org.openehr.rm.demographic.party_relationship.adoc: source, target,
-- time_validity). The rows are derived from the party versions in each
-- commit's own transaction and rebuilt by a refill, after the grant rows
-- (0035_policy_grants.sql). Reading a capability or a relationship as an
-- access fact is governed by no openEHR spec: our own design.
--
-- Runs with search_path = party, ext, public.

-- ── binding_caller_fact ─────────────────────────────────────────────────────
-- One row per fact a ROLE or PERSON head that is not deleted holds.
CREATE TABLE binding_caller_fact (
    -- The container whose head holds the fact: the ROLE of a clearance or a
    -- purpose, the PERSON of a care relationship.
    holder_id       uuid NOT NULL,
    -- The CAPABILITY's position in ROLE.capabilities, or the relationship's
    -- position in PARTY.relationships, from 0.
    ordinal         integer NOT NULL,
    -- The ferroehr-operations code the fact is read by.
    kind            text NOT NULL,
    -- The party the fact is about: the container ROLE.performer names, or the
    -- PERSON a relationship is of, its source.
    party_id        uuid NOT NULL,
    -- The CODE_PHRASE of a clearance or a purpose: the terminology the
    -- jurisdiction names and the code.
    terminology_id  text,
    code_string     text,
    -- The container a care relationship targets: the care team GROUP or the
    -- organisational unit.
    target_id       uuid,
    -- The holder head's version_lifecycle_state.
    lifecycle_state text NOT NULL,
    -- The dates ROLE.time_validity holds for a clearance or a purpose, and
    -- every date for a care relationship.
    role_validity   daterange NOT NULL,
    -- The dates CAPABILITY.time_validity or PARTY_RELATIONSHIP.time_validity
    -- holds: every date when absent, none when a bound cannot be read.
    validity        daterange NOT NULL,
    -- The Not before and Not after instants of a capability's credentials.
    not_before      timestamptz,
    not_after       timestamptz,
    written_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT ck_binding_caller_fact_kind
        CHECK (kind IN ('sensitivity_clearance', 'permitted_purpose', 'member_of',
                        'under_care_of')),
    CONSTRAINT ck_binding_caller_fact_value
        CHECK (CASE WHEN kind IN ('member_of', 'under_care_of')
                    THEN target_id IS NOT NULL AND terminology_id IS NULL
                         AND code_string IS NULL
                    ELSE target_id IS NULL AND terminology_id IS NOT NULL
                         AND code_string IS NOT NULL END),
    CONSTRAINT ck_binding_caller_fact_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);

CREATE UNIQUE INDEX uq_binding_caller_fact
    ON binding_caller_fact (holder_id, ordinal, kind, terminology_id, code_string, target_id)
    NULLS NOT DISTINCT;

-- Every fact about one party, the read a replica makes for the parties whose
-- facts changed.
CREATE INDEX ix_binding_caller_fact_party ON binding_caller_fact (party_id);

COMMENT ON TABLE binding_caller_fact IS 'The sensitivity clearances, permitted purposes and care relationships each ROLE or PERSON head that is not deleted holds, read by their ferroehr-operations codes. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as access facts, our own design.';

-- ── binding_caller_fact_party ───────────────────────────────────────────────
-- The facts revision at which each party's facts last changed, so a replica
-- reads again only the facts of the parties that changed since its copy.
CREATE TABLE binding_caller_fact_party (
    party_id uuid PRIMARY KEY,
    revision bigint NOT NULL
);

CREATE INDEX ix_binding_caller_fact_party_revision ON binding_caller_fact_party (revision);

COMMENT ON TABLE binding_caller_fact_party IS 'The facts revision at which the caller facts of each party last changed. Our own design.';

-- ── binding_grant_revision ──────────────────────────────────────────────────
-- The facts revision moves with each change of binding_caller_fact, in the
-- same transaction; the grant revision moves with the grant rows as before.
ALTER TABLE binding_grant_revision ADD COLUMN facts_revision bigint NOT NULL DEFAULT 0;

COMMENT ON COLUMN binding_grant_revision.facts_revision IS 'The revision of binding_caller_fact, which moves with each change of a party''s caller facts and stamps that party in binding_caller_fact_party.';
