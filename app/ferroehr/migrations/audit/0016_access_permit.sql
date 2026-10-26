-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the data permit an access was declared under.
--
-- A secondary-use reader, such as the batch that loads a research database
-- over AQL, acts under a data permit a health data access body issued
-- (Regulation (EU) 2025/327 Art. 68, docs/law/eu/ehds/text.html). The caller
-- declares the permit's identifier in [audit] permit_header beside its purpose
-- of use, and the record keeps it, so the trail says under which permit a
-- population query or an export ran. NULL means nothing was declared, the
-- declared value was not an identifier, or the record predates the column.
--
-- The column joins the chain digest as version 4, so a permit cannot be
-- changed or added after the insert without the chain check seeing it.
--
-- No openEHR spec governs access-log content: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event
    ADD COLUMN permit text;

COMMENT ON COLUMN audit_event.permit IS
    'The identifier of the data permit the caller declared it acts under (EHDS Art. 68), from [audit] permit_header; NULL when none was declared or the value was not an identifier.';

CREATE OR REPLACE FUNCTION audit.audit_chain_digest_version() RETURNS smallint
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$ SELECT 4::smallint $$;

CREATE OR REPLACE FUNCTION audit.audit_event_digest_columns(p_version smallint) RETURNS text[]
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE p_version
        WHEN 2 THEN ARRAY[
            'id', 'recorded_at', 'stored_at', 'action', 'outcome', 'event_code',
            'operation', 'principal', 'patient_id', 'resource_class',
            'resource_id', 'client_ip', 'token_id', 'fhir',
            'chain_seq', 'prev_hash', 'chain_digest_version',
            'domain', 'purpose', 'legal_basis', 'result_count', 'request_id',
            'organisation', 'roles', 'origins', 'origin_count',
            'categories', 'category_basis', 'category_evidence',
            'category_map_digest',
            'acting_mode', 'assurance_level', 'assurance_value',
            'professional_id',
            'emergency_access'
        ]
        WHEN 3 THEN ARRAY[
            'id', 'recorded_at', 'stored_at', 'action', 'outcome', 'event_code',
            'operation', 'principal', 'patient_id', 'resource_class',
            'resource_id', 'client_ip', 'token_id', 'fhir',
            'chain_seq', 'prev_hash', 'chain_digest_version',
            'domain', 'purpose', 'legal_basis', 'result_count', 'request_id',
            'organisation', 'roles', 'origins', 'origin_count',
            'categories', 'category_basis', 'category_evidence',
            'category_map_digest',
            'acting_mode', 'assurance_level', 'assurance_value',
            'professional_id',
            'emergency_access',
            'retention_ends', 'retention_stated'
        ]
        WHEN 4 THEN ARRAY[
            'id', 'recorded_at', 'stored_at', 'action', 'outcome', 'event_code',
            'operation', 'principal', 'patient_id', 'resource_class',
            'resource_id', 'client_ip', 'token_id', 'fhir',
            'chain_seq', 'prev_hash', 'chain_digest_version',
            'domain', 'purpose', 'legal_basis', 'result_count', 'request_id',
            'organisation', 'roles', 'origins', 'origin_count',
            'categories', 'category_basis', 'category_evidence',
            'category_map_digest',
            'acting_mode', 'assurance_level', 'assurance_value',
            'professional_id',
            'emergency_access',
            'retention_ends', 'retention_stated',
            'permit'
        ]
    END
$$;
