-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the caller facts hold the type of each ORGANISATION (#5691).
--
-- An ORGANISATION whose name is a DV_CODED_TEXT of a terminology the
-- jurisdiction names is of that type (RM demographic
-- org.openehr.rm.demographic.party.adoc: PARTY.type() is the name, invariant
-- type = name), read as 0042 reads a ROLE's. The rules of the controller's
-- ACCESS_POLICY name the callers that act for an organisation of a type
-- (ACCESS_ACTOR_SCOPE.organisation_type). Each commit of an ORGANISATION
-- writes its own row, and the grant revision returns to the never-filled
-- state, so the next boot refills the rows of the ORGANISATIONs already
-- stored before it serves. Reading an organisation's name as an access fact
-- is governed by no openEHR spec: our own design.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE binding_caller_fact DROP CONSTRAINT ck_binding_caller_fact_kind;

ALTER TABLE binding_caller_fact ADD CONSTRAINT ck_binding_caller_fact_kind
    CHECK (kind IN ('role', 'organisation_type', 'MBR', 'under_care_of'));

COMMENT ON COLUMN binding_caller_fact.kind IS 'What the fact is: role, the coded name of a ROLE the party performs, organisation_type, the coded name of the ORGANISATION itself, or the code of a care relationship, MBR of v3-RoleClass or under_care_of of openehr_access.';

COMMENT ON TABLE binding_caller_fact IS 'The roles each ROLE head that is not deleted names for its performer, the type each ORGANISATION head names for itself, and the care relationships of each PERSON and ORGANISATION head. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as access facts, our own design.';

UPDATE binding_grant_revision SET revision = 0, revised_at = now();
