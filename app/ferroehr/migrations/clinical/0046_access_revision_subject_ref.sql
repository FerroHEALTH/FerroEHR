-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the access revision names the EHR's whole subject reference
-- (#5634).
--
-- The access evaluator finds the subject's PERSON from the namespace and the
-- id of EHR_STATUS.subject.external_ref: a reference in the CDR's own
-- demographic namespace names the PERSON directly, any other one through the
-- PERSON that declares it (party/0041). A change of either moves what a
-- decision reads, so the revision a read selects beside its own columns
-- (0044_access_revision.sql) carries both, as the JSON array
-- [<ordinal>, <namespace>, <id>], each null when absent. OBJECT_REF.namespace
-- and an id may both hold a '/', so the text is JSON rather than a separator.
-- The function stays one inlined SQL function: one key lookup on
-- access_settings and one on ehr.
--
-- No openEHR spec governs the projection or its revision: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE OR REPLACE FUNCTION access_revision(ehr uuid) RETURNS text
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
        SELECT json_build_array(
                   (SELECT s.sys_version FROM clinical.access_settings s WHERE s.ehr_id = ehr),
                   (SELECT e.subject_namespace FROM clinical.ehr e WHERE e.id = ehr),
                   (SELECT e.subject_id FROM clinical.ehr e WHERE e.id = ehr))::text
    $$;

COMMENT ON FUNCTION access_revision(uuid) IS 'The storage ordinal of the EHR_ACCESS version the EHR''s settings row was projected from and the EHR''s promoted subject_namespace and subject_id, as the JSON array [<ordinal>, <namespace>, <id>], each null when absent. Our own design.';
