-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: one signed checkpoint per append replaces the signature of each
-- link (#5704).
--
-- The access log signed every link (0029), which cost one signature per
-- entry while the head of the chain was locked. Each append now ends, in its
-- own transaction, with one ACCESS_LOG_CHECKPOINT over the last link it
-- wrote: every committed entry is covered by a signature from the moment it
-- commits, and the chain of links, kept per entry, carries the signature down
-- to every record below it. ACCESS_LOG_LINK.signature is optional in the
-- draft and this system writes none, so the column goes from the record and
-- from the gap, and audit.access_log_chain_reserve(), the lock a writer took
-- to sign links before it inserted them, goes with it.
--
-- A checkpoint is stored by the columns its canonical form is rebuilt from:
-- the signer is the organisation the system_id names and the algorithm the
-- chain's one, so neither is stored. A checkpoint must name a link the log
-- holds at its position. The checkpoints and the link signatures written
-- before this migration are dropped: they signed links that keep no
-- signature now, and the first append after it seals the whole chain below.
--
-- The fold (0033) cut a run of expired tombstones after each checkpoint's
-- position, so every checkpoint kept a link to verify against. With one
-- checkpoint per append that would leave one gap per append. A run now folds
-- whole, and the checkpoints inside it, before its last position, leave with
-- it through the same sanctioned path: the gap keeps the run's last link, and
-- the checkpoints after the run, which seal every link below them through the
-- chain, still cover it. A checkpoint at a run's last position stays and
-- verifies against the gap's link.
--
-- No openEHR spec governs log signing or audit storage: our own design.
--
-- Runs with search_path = audit, ext, public.

-- ── the record keeps no signature ────────────────────────────────────────────

DROP VIEW audit_record;

CREATE OR REPLACE FUNCTION audit.access_log_record_tombstone_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    NEW := audit.access_log_record_derive(NEW);
    IF OLD.record_type <> 'ACCESS_LOG_ENTRY'
       OR NEW.record_type IS DISTINCT FROM 'ACCESS_LOG_TOMBSTONE'
       OR NEW.uid IS DISTINCT FROM OLD.uid
       OR NEW.system_id IS DISTINCT FROM OLD.system_id
       OR NEW.time_recorded IS DISTINCT FROM OLD.time_recorded
       OR (NEW.chain_seq, NEW.algorithm, NEW.previous_digest, NEW.content_digest,
           NEW.digest, NEW.stored_at)
          IS DISTINCT FROM
          (OLD.chain_seq, OLD.algorithm, OLD.previous_digest, OLD.content_digest,
           OLD.digest, OLD.stored_at)
    THEN
        RAISE EXCEPTION
            'audit.access_log_record is append-only: an entry is only replaced by its own tombstone (record %)',
            OLD.chain_seq
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;

ALTER TABLE access_log_record DROP COLUMN signature;
ALTER TABLE access_log_chain_gap DROP COLUMN signature;

DROP FUNCTION audit.access_log_chain_reserve();

COMMENT ON TABLE access_log_chain_state IS
    'The head of this system''s access log: one row, written only by the SECURITY DEFINER chain triggers; its row lock, taken by the first insert of a transaction and held to its end, serialises the writers of the log.';

CREATE VIEW audit_record AS
    SELECT e.chain_seq, e.recorded_at, e.stored_at, e.action, e.outcome, e.operation,
           e.principal, e.patient_id, e.resource_class, e.resource_id,
           e.client_ip, e.token_id, e.domain, e.purpose, e.legal_basis, e.result_count,
           e.request_id, e.roles, e.origins, e.origin_count, e.category_map_digest,
           e.acting_mode, e.assurance_level, e.assurance_value, e.professional_id,
           e.emergency_access, e.permit, e.issuer, e.client_id, e.ehr_id,
           e.subject_namespace, e.subject_lookup_failed, e.category_content,
           e.render_policy, e.token_attributes, e.search_tokens,
           NULL::text AS content, NULL::text AS algorithm, NULL::text AS previous_digest,
           NULL::text AS content_digest, NULL::text AS digest,
           'system'::text AS chain
      FROM audit_event e
    UNION ALL
    SELECT s.chain_seq, s.recorded_at, r.stored_at, s.action, s.outcome, NULL::text,
           s.principal, s.patient_id, NULL::text, s.resource_id,
           s.client_ip, NULL::text, NULL::text, NULL::text, NULL::text, NULL::bigint,
           s.request_id, NULL::jsonb, NULL::jsonb, NULL::bigint, NULL::text,
           NULL::text, NULL::text, NULL::text, NULL::text,
           false, NULL::text, NULL::text, NULL::text, s.ehr_id,
           NULL::text, false, NULL::jsonb,
           s.render_policy, NULL::jsonb, s.search_tokens,
           r.content, r.algorithm, r.previous_digest, r.content_digest, r.digest,
           'access_log'::text
      FROM access_log_search s
      JOIN access_log_record r ON r.chain_seq = s.chain_seq;

COMMENT ON VIEW audit_record IS
    'Every record the ITI-81 search, the person''s own trail and the FHIR Feed read: an audit_event record of the system chain by its columns, an access-log entry by its content and link, which its projection is rendered from. A record is named by its chain and position.';

-- ── the checkpoint, stored by its columns ───────────────────────────────────

DROP TABLE access_log_checkpoint;

CREATE TABLE access_log_checkpoint (
    sequence    bigint      NOT NULL,
    system_id   text        NOT NULL,
    digest      text        NOT NULL,
    time_sealed timestamptz NOT NULL,
    proof       text        NOT NULL,
    CONSTRAINT pk_access_log_checkpoint PRIMARY KEY (sequence),
    CONSTRAINT ck_access_log_checkpoint_digest CHECK (digest ~ '^[0-9a-f]{64}$'),
    CONSTRAINT ck_access_log_checkpoint_proof CHECK (proof <> '')
);

COMMENT ON TABLE access_log_checkpoint IS
    'The signed ACCESS_LOG_CHECKPOINTs of this system''s chain, one per append that wrote a link while the keyring held a key, append-only.';
COMMENT ON COLUMN access_log_checkpoint.time_sealed IS
    'ACCESS_LOG_CHECKPOINT.time_sealed, to the microsecond the canonical form names.';
COMMENT ON COLUMN access_log_checkpoint.proof IS
    'ACCESS_LOG_CHECKPOINT.proof: the armored OpenPGP signature over the checkpoint''s canonical form without its proof.';

CREATE FUNCTION audit.access_log_checkpoint_reject_removal() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND coalesce(current_setting('ferroehr.access_log_folding', true), 'off') = 'on'
    THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'audit.access_log_checkpoint is append-only: a checkpoint leaves only with the run of tombstones it lies inside'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER access_log_checkpoint_append_only
    BEFORE UPDATE OR DELETE ON access_log_checkpoint
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_checkpoint_reject_removal();
CREATE TRIGGER access_log_checkpoint_no_truncate
    BEFORE TRUNCATE ON access_log_checkpoint
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_checkpoint_append_only();

CREATE FUNCTION audit.access_log_checkpoint_names_a_link() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF audit.access_log_link_at(NEW.sequence) IS DISTINCT FROM NEW.digest THEN
        RAISE EXCEPTION 'an access-log checkpoint seals the link the log holds at its position (sequence %)',
            NEW.sequence
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER access_log_checkpoint_names_a_link
    BEFORE INSERT ON access_log_checkpoint
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_checkpoint_names_a_link();

-- ── the fold keeps no signature and takes the checkpoints inside a run ─────

CREATE OR REPLACE FUNCTION audit.fold_access_log_tombstones(p_now timestamptz, p_limit integer)
    RETURNS TABLE (folded bigint, gaps bigint)
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF p_now IS NULL OR coalesce(p_limit, 0) < 1 THEN
        RETURN QUERY SELECT 0::bigint, 0::bigint;
        RETURN;
    END IF;
    PERFORM pg_advisory_xact_lock(hashtext('audit.access_log_chain_gap'), 0);
    PERFORM set_config('ferroehr.access_log_folding', 'on', true);
    RETURN QUERY
    WITH due AS (
        SELECT r.chain_seq, r.digest, r.content
          FROM audit.access_log_record r
         WHERE r.fold_after <= p_now
         ORDER BY r.fold_after, r.chain_seq
         LIMIT p_limit
    ),
    islanded AS (
        SELECT d.*, d.chain_seq - row_number() OVER (ORDER BY d.chain_seq) AS island
          FROM due d
    ),
    grounded AS (
        SELECT i.island, jsonb_agg(DISTINCT g.ground ORDER BY g.ground) AS grounds
          FROM islanded i
          CROSS JOIN LATERAL jsonb_array_elements(i.content::jsonb -> 'grounds') AS g(ground)
         GROUP BY i.island
    ),
    spans AS (
        SELECT min(i.chain_seq) AS first_seq, max(i.chain_seq) AS last_seq,
               (array_agg(i.digest ORDER BY i.chain_seq DESC))[1] AS link,
               i.island
          FROM islanded i
         GROUP BY i.island
    ),
    written AS (
        INSERT INTO audit.access_log_chain_gap
               (first_seq, last_seq, link, grounds, time_folded)
        SELECT s.first_seq, s.last_seq, s.link, g.grounds, p_now
          FROM spans s
          JOIN grounded g ON g.island = s.island
         ORDER BY s.first_seq
        RETURNING 1
    ),
    unsealed AS (
        DELETE FROM audit.access_log_checkpoint c
         USING spans s
         WHERE c.sequence >= s.first_seq AND c.sequence < s.last_seq
        RETURNING 1
    ),
    removed AS (
        DELETE FROM audit.access_log_record r
         USING due d
         WHERE r.chain_seq = d.chain_seq
        RETURNING 1
    )
    SELECT (SELECT count(*) FROM removed), (SELECT count(*) FROM written)
      FROM (SELECT count(*) FROM unsealed) AS taken;
    PERFORM set_config('ferroehr.access_log_folding', 'off', true);
END;
$$;

COMMENT ON FUNCTION audit.fold_access_log_tombstones(timestamptz, integer) IS
    'The one path a tombstone leaves the access log by: up to p_limit tombstones past their end fold into one access_log_chain_gap per run, and the checkpoints inside a run, before its last position, leave with it.';

-- ── grants ───────────────────────────────────────────────────────────────────

REVOKE ALL ON FUNCTION audit.access_log_checkpoint_names_a_link() FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_checkpoint_reject_removal() FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON access_log_checkpoint, audit_record
            FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON access_log_checkpoint, audit_record
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT INSERT ON access_log_checkpoint TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the access-log checkpoint grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
