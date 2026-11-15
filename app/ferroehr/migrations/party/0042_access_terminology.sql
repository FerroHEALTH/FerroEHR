-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the access codes move to the draft openehr_access terminology
-- (#5655).
--
-- The operation grants, their qualifiers, the caller-fact element names and
-- the acts-for and representation codes are codes of the draft
-- openehr_access terminology the access-control change request proposes to
-- openEHR, and a professional's membership of a care team is HL7 v3
-- RoleClass MBR ("a member of a group", terminology id v3-RoleClass). This
-- migration writes the stored code the caller-fact side table keeps for a
-- membership, re-marks the policy parties with the path
-- party_binding::policy_mark::POLICY_PATH reads (a unit test pins it to this
-- file), and restates the comments that named the former terminology. A
-- stored party version is immutable and keeps the body it was committed
-- with (RM common, VERSION). Which terminology codes FerroEHR's grants is
-- governed by no openEHR spec: our own design.
--
-- Runs with search_path = party, ext, public.

-- ── binding_caller_fact: a membership is read by MBR ───────────────────────
ALTER TABLE binding_caller_fact
    DROP CONSTRAINT ck_binding_caller_fact_kind,
    DROP CONSTRAINT ck_binding_caller_fact_value;

UPDATE binding_caller_fact SET kind = 'MBR' WHERE kind = 'member_of';

ALTER TABLE binding_caller_fact
    ADD CONSTRAINT ck_binding_caller_fact_kind
        CHECK (kind IN ('sensitivity_clearance', 'permitted_purpose', 'MBR', 'under_care_of')),
    ADD CONSTRAINT ck_binding_caller_fact_value
        CHECK (CASE WHEN kind IN ('MBR', 'under_care_of')
                    THEN target_id IS NOT NULL AND terminology_id IS NULL
                         AND code_string IS NULL
                    ELSE target_id IS NULL AND terminology_id IS NOT NULL
                         AND code_string IS NOT NULL END);

COMMENT ON TABLE binding_caller_fact IS 'The sensitivity clearances, permitted purposes and care relationships each ROLE or PERSON head that is not deleted holds, read by their codes: sensitivity_clearance, permitted_purpose and under_care_of of the draft openehr_access terminology, and MBR of HL7 v3 RoleClass. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as access facts, our own design.';
COMMENT ON COLUMN binding_caller_fact.kind IS 'The code the fact is read by: sensitivity_clearance, permitted_purpose or under_care_of of openehr_access, or MBR of v3-RoleClass.';

-- ── binding_representation ──────────────────────────────────────────────────
COMMENT ON TABLE binding_representation IS 'The representation relationships each PERSON is the source of: its inline PARTY_RELATIONSHIPs and the standalone PARTY_RELATIONSHIP containers that carry the openehr_access::represents code. No openEHR spec governs it, our own design.';

-- ── vo_head.policy_party_at: marked by the openehr_access codes ────────────
UPDATE vo_head h SET policy_party_at = now()
WHERE h.policy_party_at IS NULL
  AND EXISTS (SELECT FROM version v
              WHERE v.vo_id = h.vo_id AND v.body IS NOT NULL
                AND jsonb_path_exists(v.body::jsonb, '$ ? (exists($.**?(@._type == "CAPABILITY").credentials.**?(@._type == "DV_CODED_TEXT" && @.defining_code.terminology_id.value == "openehr_access")) || exists($.**?(@._type == "PARTY_RELATIONSHIP")?(exists(@.name?(@._type == "DV_CODED_TEXT" && @.defining_code.terminology_id.value == "openehr_access" && @.defining_code.code_string == "acts_for")) || exists(@.details.**?(@._type == "DV_CODED_TEXT" && @.defining_code.terminology_id.value == "openehr_access" && @.defining_code.code_string == "acts_for")))) || exists($.**?(@._type == "DV_IDENTIFIER" && @.type == "client_id")))'));
