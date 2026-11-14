-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: drop the Cedar policy register (#5515).
--
-- The embedded Cedar engine is gone: access decisions are the fixed access
-- evaluator's, over the `access_directives` of each EHR_ACCESS (RM ehr
-- `ehr_access.adoc` §EHR_ACCESS Class) and the controller's ACCESS_POLICY.
-- Every stored version of the `cedar-policies` register is deleted with no
-- conversion, and the register name leaves the CHECK, so no row can name it
-- again. The audit category map and the access delay stay.
--
-- No openEHR spec governs operational policy or its registers: our own
-- design/extension.
--
-- Runs with search_path = clinical, ext, public.

DELETE FROM policy_register WHERE register = 'cedar-policies';

ALTER TABLE policy_register DROP CONSTRAINT ck_policy_register_name;
ALTER TABLE policy_register ADD CONSTRAINT ck_policy_register_name CHECK (
    register IN ('audit-categories', 'access-delay')
);

COMMENT ON TABLE policy_register IS
    'The operational-policy admin registers: one row per version of the audit category map and the EHDS Art. 3(3) access delay, with its committer. Append-only; our own design/extension.';
