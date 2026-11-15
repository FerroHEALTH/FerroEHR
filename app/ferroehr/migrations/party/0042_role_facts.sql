-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the caller facts hold the roles a caller performs, and no longer
-- the sensitivity clearances and permitted purposes of a capability (#5621).
--
-- A ROLE whose name is a DV_CODED_TEXT of a terminology the jurisdiction
-- names is a role its performer holds while the ROLE is in force (RM
-- demographic org.openehr.rm.demographic.party.adoc: PARTY.type() is the
-- role name, invariant type = name). The rules of the controller's
-- ACCESS_POLICY name callers by such a role and clear sensitivities, so
-- a clearance or a permitted purpose on a CAPABILITY decides nothing and
-- its rows go. Each commit of a ROLE writes its own role rows, and the grant
-- revision returns to the never-filled state, so the next boot refills the
-- rows of the ROLEs already stored before it serves (#5675). Reading a role
-- name as an access fact is governed by no openEHR spec: our own design.
--
-- Runs with search_path = party, ext, public.

DELETE FROM binding_caller_fact WHERE kind IN ('sensitivity_clearance', 'permitted_purpose');

ALTER TABLE binding_caller_fact DROP CONSTRAINT ck_binding_caller_fact_kind;

ALTER TABLE binding_caller_fact ADD CONSTRAINT ck_binding_caller_fact_kind
    CHECK (kind IN ('role', 'member_of', 'under_care_of'));

COMMENT ON COLUMN binding_caller_fact.kind IS 'What the fact is: role, the coded name of a ROLE the party performs, or member_of and under_care_of, the ferroehr-operations codes of a care relationship.';

COMMENT ON TABLE binding_caller_fact IS 'The roles each ROLE head that is not deleted names for its performer, and the care relationships of each PERSON and ORGANISATION head. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as access facts, our own design.';

-- The facts of a store filled before this migration hold no role, so the
-- revision returns to the never-filled state the next boot refills from the
-- versions.
UPDATE binding_grant_revision SET revision = 0, revised_at = now();
