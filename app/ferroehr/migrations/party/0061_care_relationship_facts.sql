-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: a care relationship is a PARTY_RELATIONSHIP of its own, with a role
-- and a provider (#6055).
--
-- The draft access control package makes the single relationship the stored
-- unit: an under_care_of PARTY_RELATIONSHIP whose source is the subject's
-- PERSON and whose target is a PERSON, a ROLE, a GROUP or an ORGANISATION,
-- with care_role and provider 1..1 in its details (draft access control
-- package, Care Relationships and The Relationship Store). Its caller fact
-- is held by the relationship's own container and is about its source, and
-- carries the target's kind, the role as a CODE_PHRASE and the provider's
-- DV_IDENTIFIER issuer and id. The facts of the former form, held in a
-- PERSON's relationships, go, and the grant revision returns to the
-- never-filled state, so the next boot refills every fact from the stored
-- versions before it serves. Reading a relationship as an access fact is
-- governed by no openEHR spec: our own design.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE binding_caller_fact DROP CONSTRAINT ck_binding_caller_fact_value;

DELETE FROM binding_caller_fact WHERE kind = 'under_care_of';

ALTER TABLE binding_caller_fact
    ADD COLUMN target_kind     text,
    ADD COLUMN provider_issuer text,
    ADD COLUMN provider_id     text;

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
                    ELSE target_id IS NULL AND target_kind IS NULL
                         AND terminology_id IS NOT NULL AND code_string IS NOT NULL
                         AND provider_issuer IS NULL AND provider_id IS NULL END);

COMMENT ON COLUMN binding_caller_fact.target_kind IS 'The RM type of a care relationship''s target: PERSON, ROLE, GROUP or ORGANISATION; NULL for every other fact.';
COMMENT ON COLUMN binding_caller_fact.provider_issuer IS 'The issuer of a care relationship''s provider DV_IDENTIFIER; NULL for every other fact.';
COMMENT ON COLUMN binding_caller_fact.provider_id IS 'The id of a care relationship''s provider DV_IDENTIFIER; NULL for every other fact.';
COMMENT ON COLUMN binding_caller_fact.terminology_id IS 'The terminology of a role, an organisation type or a care relationship''s care_role.';
COMMENT ON COLUMN binding_caller_fact.code_string IS 'The code of a role, an organisation type or a care relationship''s care_role.';

COMMENT ON TABLE binding_caller_fact IS 'The roles each ROLE head that is not deleted names for its performer, the type each ORGANISATION head names for itself, the memberships of each PERSON and ORGANISATION head, and the care relationship each PARTY_RELATIONSHIP head of its own states about its source. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as access facts, our own design.';

UPDATE binding_grant_revision SET revision = 0, revised_at = now();
