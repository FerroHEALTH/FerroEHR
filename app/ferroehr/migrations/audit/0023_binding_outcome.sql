-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: every record carries the outcome of the party binding and the
-- fields that outcome needs (#5118).
--
-- EHDS (Regulation (EU) 2025/327) Annex II 3.2 asks the log to identify
-- "(a) ... the healthcare provider or other individuals having accessed" and
-- "(b) ... the specific natural person or persons having accessed"
-- (docs/law/eu/ehds/text.html). binding names how the caller was bound, and
-- the columns beside it carry the party, the groups, the provider, the guest
-- mark, the representation, a verified external accessor's fields, its
-- agreement, the actor chain, the facilities a system commit named, what a
-- refusal names, and a summary record's count.
--
-- Stored records keep verifying: digest_version restores the per-row digest
-- version 0017 removed. Every existing row is version 1, hashed over the 0017
-- column list; a new row is version 2, hashed over that list, the new columns
-- and digest_version itself, and a row whose version is lower than its
-- predecessor's is refused.
--
-- No openEHR spec governs access-log content: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event
    ADD COLUMN binding         text,
    ADD COLUMN party           jsonb,
    ADD COLUMN party_groups    jsonb,
    ADD COLUMN provider        jsonb,
    ADD COLUMN guest           boolean NOT NULL DEFAULT false,
    ADD COLUMN representation  jsonb,
    ADD COLUMN external_fields jsonb,
    ADD COLUMN agreement       jsonb,
    ADD COLUMN actor_chain     jsonb,
    ADD COLUMN facilities      jsonb,
    ADD COLUMN binding_detail  jsonb,
    ADD COLUMN summary         jsonb,
    ADD COLUMN digest_version  smallint NOT NULL DEFAULT 1;

ALTER TABLE audit_event ALTER COLUMN digest_version SET DEFAULT 2;

ALTER TABLE audit_event
    DROP CONSTRAINT ck_audit_event_acting_mode,
    ADD CONSTRAINT ck_audit_event_acting_mode
        CHECK (acting_mode IS NULL
               OR acting_mode IN ('person', 'client', 'delegated', 'external', 'system')),
    ADD CONSTRAINT ck_audit_event_binding
        CHECK (binding IS NULL OR binding IN (
            'bound', 'provisional', 'external', 'organisation', 'system', 'self',
            'refused-unbound', 'refused-ambiguous', 'refused-inactive', 'refused-abandoned',
            'refused-deprovisioned', 'refused-wrong-marker',
            'refused-representation-unrecorded', 'refused-key-claim-missing',
            'refused-groups-unavailable', 'refused-guest', 'refused-external-incomplete',
            'refused-trc-patient-mismatch', 'refused-trc-missing')),
    ADD CONSTRAINT ck_audit_event_digest_version CHECK (digest_version IN (1, 2));

COMMENT ON COLUMN audit_event.acting_mode IS
    'How the credential acts: a natural person (person), a client application for a person (client or delegated), a verified external accessor (external), or a client acting for nobody (system); NULL when the credential says nothing.';
COMMENT ON COLUMN audit_event.binding IS
    'The outcome of the party binding: how the caller was bound, or the refusal code; NULL on a record with no authenticated caller or one written before the column.';
COMMENT ON COLUMN audit_event.party IS
    'The demographic party the caller is bound to, as a PARTY_REF triple {namespace, type, id}.';
COMMENT ON COLUMN audit_event.party_groups IS
    'The ids of the GROUP parties the token names that the bound party belongs to.';
COMMENT ON COLUMN audit_event.provider IS
    'The healthcare provider the access is attributed to, {issuer, id} (EHDS Annex II 3.2(a)).';
COMMENT ON COLUMN audit_event.guest IS
    'Whether the bound caller is a guest its issuer admits (GDPR Art. 9(3), Art. 29).';
COMMENT ON COLUMN audit_event.representation IS
    'For a representative: whom they act for, on which asserted ground, and the subject_relationship code of their committer.';
COMMENT ON COLUMN audit_event.external_fields IS
    'The fields a federation token asserts about a verified external accessor, by field name.';
COMMENT ON COLUMN audit_event.agreement IS
    'The federation agreement an external accessor was admitted under: {name, version, legal_ground, accessor, access_class}, and for a chained request {correlation_id, correlation_source}.';
COMMENT ON COLUMN audit_event.actor_chain IS
    'The RFC 8693 actor chain of a delegated token, outermost first; recorded, never decided on.';
COMMENT ON COLUMN audit_event.facilities IS
    'The facility identifiers {issuer, id} of the compositions a system caller committed.';
COMMENT ON COLUMN audit_event.binding_detail IS
    'What a refusal names beside its code: the missing claim, the missing fields, the party, the asserted subject.';
COMMENT ON COLUMN audit_event.summary IS
    'For a summary record: the count of the refusals it stands for and the EHR ids they named.';
COMMENT ON COLUMN audit_event.digest_version IS
    'The column list the chain digest of this row hashes (audit.audit_event_digest_columns).';

-- ── the digest ───────────────────────────────────────────────────────────────

DROP FUNCTION audit.audit_event_digest_columns();

-- Version 1 is 0017's list, which every row written before this migration is
-- hashed over; version 2 adds the binding columns and digest_version. A column
-- added later is outside the digest until a version lists it, and the
-- integration test audit_chain::the_digest_covers_every_column fails until it is.
CREATE FUNCTION audit.audit_event_digest_columns(p_version smallint) RETURNS text[]
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE p_version
        WHEN 1 THEN v1.columns
        WHEN 2 THEN v1.columns || ARRAY[
            'binding', 'party', 'party_groups', 'provider', 'guest', 'representation',
            'external_fields', 'agreement', 'actor_chain', 'facilities', 'binding_detail',
            'summary', 'digest_version'
        ]
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
        ]) AS v1(columns)
$$;

COMMENT ON FUNCTION audit.audit_event_digest_columns(smallint) IS
    'The columns the chain digest of a version-p_version row hashes: every column but row_hash and the delivery stamps that version knows.';

CREATE OR REPLACE FUNCTION audit.audit_event_row_digest(p_row audit.audit_event) RETURNS bytea
    LANGUAGE sql STABLE PARALLEL SAFE
    SET search_path = pg_catalog
    SET TimeZone = 'UTC'
    SET bytea_output = 'hex'
    AS $$
    SELECT sha256(convert_to(jsonb_build_array(
               'ferroehr.audit_chain'::text,
               jsonb_object_agg(listed.name, whole.doc -> listed.name)
           )::text, 'UTF8'))
      FROM to_jsonb(p_row) AS whole(doc),
           unnest(audit.audit_event_digest_columns(p_row.digest_version)) AS listed(name)
$$;

COMMENT ON FUNCTION audit.audit_event_row_digest(audit.audit_event) IS
    'The chain digest of one stored record over the column list of its own digest_version: the single definition the insert trigger and the verifier both call.';

-- ── the chain writer ─────────────────────────────────────────────────────────

CREATE OR REPLACE FUNCTION audit.audit_event_chain_link() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cursor_value text := current_setting('ferroehr.audit_chain_head', true);
    preceding    smallint;
BEGIN
    IF cursor_value IS NULL OR cursor_value = '' THEN
        RAISE EXCEPTION 'the audit chain cursor is unset: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW.chain_seq := split_part(cursor_value, ':', 1)::bigint + 1;
    NEW.prev_hash := decode(split_part(cursor_value, ':', 2), 'hex');
    SELECT e.digest_version INTO preceding
      FROM audit_event e
     WHERE e.chain_seq = NEW.chain_seq - 1;
    IF preceding IS NOT NULL AND NEW.digest_version < preceding THEN
        RAISE EXCEPTION 'audit record digest version % follows version %: refusing a downgrade',
            NEW.digest_version, preceding
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW.row_hash := audit.audit_event_row_digest(NEW);
    PERFORM set_config('ferroehr.audit_chain_head',
                       NEW.chain_seq || ':' || encode(NEW.row_hash, 'hex'), true);
    RETURN NEW;
END;
$$;
