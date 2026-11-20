-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: a kinship is a caller fact about each of its two persons (#6077).
--
-- The draft access control package: a related_to PARTY_RELATIONSHIP records
-- a kinship between two persons, coded in its kinship element, and "Kinship
-- never grants access. It can refuse it, as a conflict of interest" (step 5
-- of the decision procedure). The relationship is its own versioned object,
-- and its commit writes one fact about each end, held by the relationship's
-- container, naming the other person as its target and the kinship as its
-- code, so the facts of either person name the other. The decision reads
-- them to refuse a professional in conflict with a subject it is kin to; no
-- fact of this kind admits anything. No openEHR spec governs the side table:
-- our own design.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE binding_caller_fact DROP CONSTRAINT ck_binding_caller_fact_kind;
ALTER TABLE binding_caller_fact DROP CONSTRAINT ck_binding_caller_fact_value;

ALTER TABLE binding_caller_fact ADD CONSTRAINT ck_binding_caller_fact_kind
    CHECK (kind IN ('role', 'organisation_type', 'MBR', 'under_care_of', 'related_to'));

ALTER TABLE binding_caller_fact
    ADD CONSTRAINT ck_binding_caller_fact_value
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
                         AND provider_issuer IS NULL AND provider_id IS NULL END);

COMMENT ON COLUMN binding_caller_fact.kind IS 'What the fact is: role, the coded name of a ROLE the party performs, organisation_type, the coded name of the ORGANISATION itself, the code of a care relationship, MBR of HL7_RoleClass or under_care_of of openehr_access, or related_to of openehr_access, a kinship of the party to the person target_id names.';

COMMENT ON TABLE binding_caller_fact IS 'The roles each ROLE head that is not deleted names for its performer, the type each ORGANISATION head names for itself, the memberships of each PERSON and ORGANISATION head, the care relationship each PARTY_RELATIONSHIP head of its own states about its source, and the kinship each related_to PARTY_RELATIONSHIP head states about each of its two persons. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as access facts, our own design.';

UPDATE binding_grant_revision SET revision = 0, revised_at = now();
