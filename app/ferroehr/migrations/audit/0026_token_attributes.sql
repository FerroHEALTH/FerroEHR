-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the purposes of use and the professional roles the access token
-- states (#5323).
--
-- An issuer's [[auth.attributes]] table reads them from the token: the IHE
-- IUA subject_role and purpose_of_use claims (IHE ITI TF-2 §3.71.4.2.2.1.1)
-- and the purpose_of_use of each RFC 9396 authorization_details entry.
-- EHDS (Regulation (EU) 2025/327) Annex II 3.2 asks the log for the purpose
-- of the access (docs/law/eu/ehds/text.html). token_attributes holds them as
-- {"purposes": [{system, code}], "roles": [{system, code}]}.
--
-- Stored records keep verifying: a new row is digest version 3, hashed over
-- version 2's list and token_attributes; versions 1 and 2 keep their lists.
--
-- No openEHR spec governs access-log content: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event ADD COLUMN token_attributes jsonb;

ALTER TABLE audit_event ALTER COLUMN digest_version SET DEFAULT 3;

ALTER TABLE audit_event
    DROP CONSTRAINT ck_audit_event_digest_version,
    ADD CONSTRAINT ck_audit_event_digest_version CHECK (digest_version IN (1, 2, 3));

COMMENT ON COLUMN audit_event.token_attributes IS
    'The purposes of use and the professional roles the access token states under its issuer''s [[auth.attributes]] table, {"purposes": [{system, code}], "roles": [{system, code}]}; NULL when it states none.';

CREATE OR REPLACE FUNCTION audit.audit_event_digest_columns(p_version smallint) RETURNS text[]
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE p_version
        WHEN 1 THEN v1.columns
        WHEN 2 THEN v1.columns || v2.columns
        WHEN 3 THEN v1.columns || v2.columns || ARRAY['token_attributes']
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
