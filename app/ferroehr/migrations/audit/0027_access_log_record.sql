-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the openEHR access-log record is the stored record, chained over its
-- content digest, and a tombstone replaces an entry in place (#5125).
--
-- The draft access log (org.openehr.rm.access_log 0.1.0, its BMM in
-- tools/openehr-codegen/draft/bmm/) defines the chain: the canonical form of
-- a record is its ITS-JSON serialisation without its link, canonicalised by
-- RFC 8785 (docs/specs/rfc-independent/); content_digest is the SHA-256 of
-- its UTF-8 octets; digest is the SHA-256 of previous_digest, a line feed,
-- the decimal sequence, a line feed and content_digest, each digest in
-- lower-case hexadecimal. A tombstone keeps the uid, the system_id, the
-- time_recorded and the link, so the chain verifies after the content is gone.
--
-- access_log_record keeps the canonical form as the record (content), the
-- link in columns, and the search columns, which the row trigger derives from
-- content so they cannot disagree with it. It continues the chain of
-- audit_event: one cursor and one head (audit_chain_state) order both tables,
-- so a record of either form links to the record before it whatever its form.
-- audit_event keeps its records, which verify under their digest_version as
-- before, and the verifier walks both forms in one pass.
--
-- No openEHR spec governs audit storage: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

-- ── the record ───────────────────────────────────────────────────────────────

CREATE TABLE access_log_record (
    chain_seq       bigint      NOT NULL,
    record_type     text        NOT NULL,
    uid             text        NOT NULL,
    system_id       text        NOT NULL,
    time_recorded   timestamptz NOT NULL,
    content         text        NOT NULL,
    algorithm       text        NOT NULL,
    previous_digest text        NOT NULL,
    content_digest  text        NOT NULL,
    digest          text        NOT NULL,
    signature       text,
    stored_at       timestamptz NOT NULL DEFAULT now(),
    ehr_id          text,
    action          text,
    result          text,
    party_id        text,
    request_id      text,
    retain_until    date,
    time_removed    timestamptz,
    CONSTRAINT pk_access_log_record PRIMARY KEY (chain_seq),
    CONSTRAINT uq_access_log_record_uid UNIQUE (uid),
    CONSTRAINT ck_access_log_record_type
        CHECK (record_type IN ('ACCESS_LOG_ENTRY', 'ACCESS_LOG_TOMBSTONE')),
    CONSTRAINT ck_access_log_record_algorithm CHECK (algorithm = 'SHA-256'),
    CONSTRAINT ck_access_log_record_digests
        CHECK (previous_digest ~ '^[0-9a-f]{64}$'
               AND content_digest ~ '^[0-9a-f]{64}$'
               AND digest ~ '^[0-9a-f]{64}$'),
    CONSTRAINT ck_access_log_record_removed
        CHECK ((time_removed IS NOT NULL) = (record_type = 'ACCESS_LOG_TOMBSTONE'))
);

COMMENT ON TABLE access_log_record IS
    'The openEHR access log (draft org.openehr.rm.access_log 0.1.0): one row per ACCESS_LOG_ENTRY or ACCESS_LOG_TOMBSTONE, continuing the hash chain of audit_event. Strictly outside the EHR content (BASE architecture_overview/master07-security.adoc §Access logging); append-only except the replacement of an entry by its tombstone.';
COMMENT ON COLUMN access_log_record.chain_seq IS
    'The position in the chain, ACCESS_LOG_LINK.sequence, set by the insert trigger.';
COMMENT ON COLUMN access_log_record.record_type IS
    'The record''s class, the _type of content.';
COMMENT ON COLUMN access_log_record.content IS
    'The record''s canonical form: its ITS-JSON serialisation without its link, canonicalised by RFC 8785. The record as served is this with the link the columns hold.';
COMMENT ON COLUMN access_log_record.content_digest IS
    'ACCESS_LOG_LINK.content_digest: the SHA-256 of content''s UTF-8 octets when the entry was written, in lower-case hexadecimal; a tombstone keeps it.';
COMMENT ON COLUMN access_log_record.digest IS
    'ACCESS_LOG_LINK.digest: the SHA-256 of previous_digest, a line feed, the decimal chain_seq, a line feed and content_digest, in lower-case hexadecimal.';
COMMENT ON COLUMN access_log_record.signature IS
    'ACCESS_LOG_LINK.signature, the optional signature of the link.';
COMMENT ON COLUMN access_log_record.ehr_id IS
    'The entry''s ehr_id, derived from content; NULL on a tombstone.';
COMMENT ON COLUMN access_log_record.party_id IS
    'The id of the party accessor.party references, derived from content; NULL when it references none.';
COMMENT ON COLUMN access_log_record.retain_until IS
    'The entry''s retention.retain_until, derived from content; NULL on a tombstone and when the entry states none.';
COMMENT ON COLUMN access_log_record.time_removed IS
    'A tombstone''s time_removed, derived from content; NULL on an entry.';

CREATE INDEX ix_access_log_record_ehr ON access_log_record (ehr_id, time_recorded)
    WHERE ehr_id IS NOT NULL;
CREATE INDEX ix_access_log_record_time ON access_log_record (time_recorded);
CREATE INDEX ix_access_log_record_party ON access_log_record (party_id, time_recorded)
    WHERE party_id IS NOT NULL;
CREATE INDEX ix_access_log_record_retain_until ON access_log_record (retain_until)
    WHERE retain_until IS NOT NULL;

-- ── the digests ──────────────────────────────────────────────────────────────

CREATE FUNCTION audit.access_log_content_digest(p_content text) RETURNS text
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$ SELECT encode(sha256(convert_to(p_content, 'UTF8')), 'hex') $$;

COMMENT ON FUNCTION audit.access_log_content_digest(text) IS
    'ACCESS_LOG_LINK.content_digest of a canonical form: the single definition the insert trigger and the verifier both call.';

CREATE FUNCTION audit.access_log_link_digest(p_previous text, p_sequence bigint, p_content_digest text)
    RETURNS text
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT encode(sha256(convert_to(
               p_previous || E'\n' || p_sequence::text || E'\n' || p_content_digest, 'UTF8')), 'hex')
$$;

COMMENT ON FUNCTION audit.access_log_link_digest(text, bigint, text) IS
    'ACCESS_LOG_LINK.digest: the single definition the insert trigger and the verifier both call.';

-- ── the columns content decides ──────────────────────────────────────────────

-- The search columns and the identity of a record, read from its canonical
-- form. A content that is no access-log record, carries a link, or whose
-- times do not read is refused.
CREATE FUNCTION audit.access_log_record_derive(p_row audit.access_log_record)
    RETURNS audit.access_log_record
    LANGUAGE plpgsql STABLE
    SET search_path = pg_catalog
    SET TimeZone = 'UTC'
    AS $$
DECLARE
    doc jsonb := p_row.content::jsonb;
BEGIN
    IF jsonb_typeof(doc) IS DISTINCT FROM 'object' OR doc ? 'link' THEN
        RAISE EXCEPTION 'an access-log record is stored as its canonical form without its link'
            USING ERRCODE = 'check_violation';
    END IF;
    p_row.record_type := doc ->> '_type';
    p_row.uid := doc #>> '{uid,value}';
    p_row.system_id := doc ->> 'system_id';
    p_row.time_recorded := (doc #>> '{time_recorded,value}')::timestamptz;
    p_row.ehr_id := doc #>> '{ehr_id,value}';
    p_row.action := doc #>> '{action,value}';
    p_row.result := doc #>> '{outcome,result,value}';
    p_row.party_id := doc #>> '{accessor,party,external_ref,id,value}';
    p_row.request_id := doc #>> '{channel,request_id}';
    p_row.retain_until := (doc #>> '{retention,retain_until,value}')::date;
    p_row.time_removed := (doc #>> '{time_removed,value}')::timestamptz;
    RETURN p_row;
END;
$$;

COMMENT ON FUNCTION audit.access_log_record_derive(audit.access_log_record) IS
    'The row with every column its content decides read from it: the record type, uid, system_id, time_recorded and the search columns.';

-- ── the chain writer ─────────────────────────────────────────────────────────

-- A record is written as an entry. Every column but content is the trigger's:
-- the identity and search columns from content, the link from the chain
-- cursor that audit.audit_event_chain_begin set for the statement.
CREATE FUNCTION audit.access_log_record_chain_link() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cursor_value text := current_setting('ferroehr.audit_chain_head', true);
BEGIN
    IF cursor_value IS NULL OR cursor_value = '' THEN
        RAISE EXCEPTION 'the audit chain cursor is unset: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW := audit.access_log_record_derive(NEW);
    IF NEW.record_type IS DISTINCT FROM 'ACCESS_LOG_ENTRY' THEN
        RAISE EXCEPTION 'an access-log record is written as an ACCESS_LOG_ENTRY, not %', NEW.record_type
            USING ERRCODE = 'check_violation';
    END IF;
    NEW.chain_seq := split_part(cursor_value, ':', 1)::bigint + 1;
    NEW.previous_digest := split_part(cursor_value, ':', 2);
    NEW.algorithm := 'SHA-256';
    NEW.content_digest := audit.access_log_content_digest(NEW.content);
    NEW.digest := audit.access_log_link_digest(NEW.previous_digest, NEW.chain_seq, NEW.content_digest);
    NEW.signature := NULL;
    NEW.stored_at := now();
    PERFORM set_config('ferroehr.audit_chain_head', NEW.chain_seq || ':' || NEW.digest, true);
    RETURN NEW;
END;
$$;

CREATE TRIGGER access_log_record_chain_begin
    BEFORE INSERT ON access_log_record
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_event_chain_begin();
CREATE TRIGGER access_log_record_chain_link
    BEFORE INSERT ON access_log_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_record_chain_link();
CREATE TRIGGER access_log_record_chain_commit
    AFTER INSERT ON access_log_record
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_event_chain_commit();

-- ── append-only, but for the tombstone ───────────────────────────────────────

-- The one change a stored record admits: an entry's content replaced by its
-- tombstone's, with the same uid, system_id and time_recorded, and the link
-- and position unchanged. The search columns follow the new content.
CREATE FUNCTION audit.access_log_record_tombstone_only() RETURNS trigger
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
           NEW.digest, NEW.signature, NEW.stored_at)
          IS DISTINCT FROM
          (OLD.chain_seq, OLD.algorithm, OLD.previous_digest, OLD.content_digest,
           OLD.digest, OLD.signature, OLD.stored_at)
    THEN
        RAISE EXCEPTION
            'audit.access_log_record is append-only: an entry is only replaced by its own tombstone (record %)',
            OLD.chain_seq
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER access_log_record_tombstone_only
    BEFORE UPDATE ON access_log_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_record_tombstone_only();

CREATE FUNCTION audit.access_log_record_reject_removal() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION 'audit.access_log_record is append-only: a record is replaced by its tombstone, never removed'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER access_log_record_reject_deletion
    BEFORE DELETE ON access_log_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_record_reject_removal();
CREATE TRIGGER access_log_record_reject_truncate
    BEFORE TRUNCATE ON access_log_record
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_record_reject_removal();

-- ── verification ─────────────────────────────────────────────────────────────

-- The newest record of the chain, of either form, with its link.
CREATE FUNCTION audit.audit_chain_newest()
    RETURNS TABLE (chain_seq bigint, link bytea)
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT n.chain_seq, n.link
      FROM (
          (SELECT e.chain_seq, e.row_hash AS link
             FROM audit.audit_event e ORDER BY e.chain_seq DESC LIMIT 1)
          UNION ALL
          (SELECT r.chain_seq, decode(r.digest, 'hex')
             FROM audit.access_log_record r ORDER BY r.chain_seq DESC LIMIT 1)
      ) AS n
     ORDER BY n.chain_seq DESC
     LIMIT 1
$$;

COMMENT ON FUNCTION audit.audit_chain_newest() IS
    'The position and link of the newest record in the chain, an audit_event record or an access-log record.';

-- The chain from the link (p_after_seq, p_after_hash) on, over both forms in
-- position order. An audit_event record is recomputed under its digest_version
-- and its render policy checked; an entry's content_digest is recomputed from
-- its content and a tombstone's taken as recorded, as the draft's verifier
-- does; every record's link is recomputed and matched to the one before it.
CREATE OR REPLACE FUNCTION audit.verify_audit_chain_after(p_after_seq bigint, p_after_hash bytea)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE plpgsql STABLE
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    head_position bigint;
    head_digest   bytea;
    highest       bigint;
    highest_row   bytea;
BEGIN
    SELECT s.head_seq, s.head_hash INTO head_position, head_digest
      FROM audit.audit_chain_state s WHERE s.singleton;
    IF NOT FOUND THEN
        RETURN QUERY SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
            'the audit chain state row is missing: the chain cannot be verified'::text;
        RETURN;
    END IF;

    RETURN QUERY
    SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
           ('the render policy ' || encode(p.digest, 'hex')
            || ' was modified after it was written')::text
      FROM audit.audit_render_policy p
     WHERE p.digest IS DISTINCT FROM sha256(convert_to(p.policy::text, 'UTF8'))
     ORDER BY p.digest;

    RETURN QUERY
    SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
           ('the search token ' || t.id || ' was modified after it was written')::text
      FROM audit.audit_search_token t
     WHERE t.digest IS DISTINCT FROM audit.search_token_digest(t.parameter, t.system, t.code)
     ORDER BY t.id;
    RETURN QUERY
    SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
           ('the search set ' || s.id || ' was modified after it was written')::text
      FROM audit.audit_search_set s
     WHERE s.digest IS DISTINCT FROM audit.search_set_digest(s.tokens)
     ORDER BY s.id;

    RETURN QUERY
    WITH chain AS (
        SELECT e.chain_seq, e.id, e.recorded_at, e.prev_hash, e.row_hash,
               e.row_hash IS DISTINCT FROM audit.audit_event_row_digest(e) AS modified,
               NOT EXISTS (SELECT 1 FROM audit.audit_render_policy p
                            WHERE p.digest = e.render_policy) AS policy_missing
          FROM audit.audit_event e
         WHERE e.chain_seq > p_after_seq
        UNION ALL
        SELECT r.chain_seq,
               CASE WHEN r.uid ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                    THEN r.uid::uuid END,
               r.time_recorded,
               decode(r.previous_digest, 'hex'), decode(r.digest, 'hex'),
               r.digest IS DISTINCT FROM audit.access_log_link_digest(
                   r.previous_digest, r.chain_seq, r.content_digest)
                 OR (r.record_type = 'ACCESS_LOG_ENTRY'
                     AND r.content_digest IS DISTINCT FROM audit.access_log_content_digest(r.content))
                 OR audit.access_log_record_derive(r) IS DISTINCT FROM r,
               false
          FROM audit.access_log_record r
         WHERE r.chain_seq > p_after_seq
    ),
    linked AS (
        SELECT c.*,
               coalesce(lag(c.row_hash) OVER (ORDER BY c.chain_seq), p_after_hash) AS before_hash,
               coalesce(lag(c.chain_seq) OVER (ORDER BY c.chain_seq), p_after_seq) AS before_seq
          FROM chain c
    ),
    judged AS (
        SELECT l.*,
               l.chain_seq = l.before_seq + 1 AS adjacent,
               EXISTS (
                   SELECT 1 FROM audit.audit_chain_gap g
                    WHERE (g.from_seq = l.before_seq + 1
                           OR (l.before_seq = p_after_seq AND p_after_seq > 0
                               AND g.from_seq <= l.before_seq + 1))
                      AND g.to_seq = l.chain_seq - 1
                      AND g.link_hash = l.prev_hash
               ) AS gap_accounted
          FROM linked l
    )
    SELECT j.chain_seq, j.id, j.recorded_at,
           CASE
             WHEN j.modified
               THEN 'record content was modified after it was written'
             WHEN j.policy_missing
               THEN 'the render policy the record names is missing'
             WHEN NOT j.adjacent
               THEN 'record(s) deleted between chain position ' || j.before_seq
                    || ' and this one, with no retention record for the removal'
             ELSE 'the link to the preceding record does not match'
           END::text
      FROM judged j
     WHERE j.modified
        OR j.policy_missing
        OR (NOT j.adjacent AND NOT j.gap_accounted)
        OR (j.adjacent AND j.prev_hash IS DISTINCT FROM j.before_hash);

    SELECT newest.chain_seq, newest.link INTO highest, highest_row
      FROM audit.audit_chain_newest() newest;
    highest := coalesce(highest, 0);

    IF highest = head_position THEN
        IF highest > 0 AND highest_row IS DISTINCT FROM head_digest THEN
            RETURN QUERY SELECT highest, NULL::uuid, NULL::timestamptz,
                'the newest record does not match the recorded chain head'::text;
        END IF;
    ELSIF NOT EXISTS (
        SELECT 1 FROM audit.audit_chain_gap g
         WHERE g.from_seq = highest + 1 AND g.to_seq = head_position
           AND g.link_hash = head_digest
    ) THEN
        RETURN QUERY SELECT highest, NULL::uuid, NULL::timestamptz,
            'record(s) deleted from the end of the chain, with no retention record for the removal'::text;
    END IF;
END;
$$;

COMMENT ON FUNCTION audit.verify_audit_chain_after(bigint, bytea) IS
    'The chain verification from one verified link on, over audit_event and access_log_record in position order: the body of both the full and the incremental verification.';

-- As 0017's, with the newest record read across both forms.
CREATE OR REPLACE FUNCTION audit.verify_audit_chain_incremental(p_full boolean DEFAULT false)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    from_seq  bigint := 0;
    from_hash bytea := audit.audit_chain_genesis();
    damage    audit.audit_chain_finding[];
    top_seq   bigint;
    top_hash  bytea;
BEGIN
    IF NOT coalesce(p_full, false) THEN
        SELECT c.through_seq, c.through_hash INTO from_seq, from_hash
          FROM audit.audit_chain_checkpoint c WHERE c.singleton;
        IF NOT FOUND THEN
            from_seq := 0;
            from_hash := audit.audit_chain_genesis();
        END IF;
    END IF;

    SELECT (SELECT coalesce(array_agg(f ORDER BY f.chain_seq NULLS FIRST), '{}')
              FROM audit.verify_audit_chain_after(from_seq, from_hash) f),
           newest.chain_seq, newest.link
      INTO damage, top_seq, top_hash
      FROM (SELECT 1) AS one
      LEFT JOIN LATERAL audit.audit_chain_newest() AS newest ON true;

    IF cardinality(damage) = 0 AND top_seq IS NOT NULL THEN
        INSERT INTO audit.audit_chain_checkpoint AS c
               (through_seq, through_hash, verified_at, full_verified_at)
        VALUES (top_seq, top_hash, now(), CASE WHEN p_full THEN now() END)
        ON CONFLICT (singleton) DO UPDATE
           SET through_seq = excluded.through_seq,
               through_hash = excluded.through_hash,
               verified_at = excluded.verified_at,
               full_verified_at = coalesce(excluded.full_verified_at, c.full_verified_at)
         WHERE excluded.through_seq >= c.through_seq;
    END IF;

    RETURN QUERY SELECT * FROM unnest(damage);
END;
$$;

-- ── grants ───────────────────────────────────────────────────────────────────

-- The application writes content and nothing else: on insert every other
-- column is the trigger's, and an update is the tombstone replacement.
REVOKE ALL ON FUNCTION audit.audit_chain_newest() FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON access_log_record FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON access_log_record TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT INSERT (content), UPDATE (content) ON access_log_record TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.audit_chain_newest()
            TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping access-log grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
