-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the policies an access decision rested on (#5514).
--
-- The access evaluator names the grounds of each decision in the access
-- record (EHDS, Regulation (EU) 2025/327, Annex II 3.2,
-- docs/law/eu/ehds/text.html): each directive with the uid of the EHR_ACCESS
-- version that carries it, and the ACCESS_POLICY version a ground is defined
-- in. The EHDS record writes those versions after the data permit at
-- agent[user].policy, which FHIR R4 defines as "the policy or plan that
-- authorized the activity being recorded". A stored record is rendered from
-- its columns (0017_render_audit_event_from_columns.sql), so the versions are
-- a column: decision_policies holds them as a JSON array of strings, in
-- order and without repeats.
--
-- Stored records keep verifying: a new row is digest version 4, hashed over
-- version 3's list and decision_policies; versions 1 to 3 keep their lists.
--
-- No openEHR spec governs access-log content: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event ADD COLUMN decision_policies jsonb;

ALTER TABLE audit_event ALTER COLUMN digest_version SET DEFAULT 4;

ALTER TABLE audit_event
    DROP CONSTRAINT ck_audit_event_digest_version,
    ADD CONSTRAINT ck_audit_event_digest_version CHECK (digest_version IN (1, 2, 3, 4));

COMMENT ON COLUMN audit_event.decision_policies IS
    'The versions of the policies the access decision''s grounds are defined in (EHR_ACCESS and ACCESS_POLICY version uids), a JSON array of strings in order; NULL when the decision named none.';

CREATE OR REPLACE FUNCTION audit.audit_event_digest_columns(p_version smallint) RETURNS text[]
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE p_version
        WHEN 1 THEN v1.columns
        WHEN 2 THEN v1.columns || v2.columns
        WHEN 3 THEN v1.columns || v2.columns || ARRAY['token_attributes']
        WHEN 4 THEN v1.columns || v2.columns || ARRAY['token_attributes', 'decision_policies']
    END
      FROM (SELECT ARRAY[
            'id', 'recorded_at', 'stored_at', 'action', 'outcome', 'event_code',
            'operation', 'principal', 'patient_id', 'resource_class',
            'resource_id', 'client_ip', 'token_id',
            'chain_seq', 'prev_hash',
            'domain', 'purpose', 'legal_basis', 'result_count', 'request_id',
            'organisation', 'roles', 'origins', 'origin_count',
            'categories', 'category_basis', 'category_evidence',
            'category_map_digest',
            'acting_mode', 'assurance_level', 'assurance_value',
            'professional_id',
            'emergency_access',
            'retention_ends', 'retention_stated',
            'permit',
            'issuer', 'client_id', 'ehr_id', 'subject_namespace',
            'subject_lookup_failed', 'category_content', 'render_policy',
            'search_tokens'
        ]) AS v1(columns),
           (SELECT ARRAY[
            'binding', 'party', 'party_groups', 'provider', 'guest', 'representation',
            'external_fields', 'agreement', 'actor_chain', 'facilities', 'binding_detail',
            'summary', 'digest_version'
        ]) AS v2(columns)
$$;
