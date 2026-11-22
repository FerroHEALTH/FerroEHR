-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the holds, the reaped positions and the sealed windows of the
-- system records (audit_event).
--
-- Each system record is written once by the request that made the access,
-- with its content digest, computed by the writer (#6242). The sealer closes
-- windows of records by position up to the frontier and signs each window's
-- seal (system_log::seal). Retention removes a record past its end and keeps
-- its position and content digest in audit_event_reaped, so its window still
-- verifies; the fold removes the reaped positions of a sealed window once
-- every record of it is reaped. The verifier recomputes each window from its
-- records and reaped positions.
--
-- No openEHR spec governs audit tamper detection: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

-- The records an import brought never change, and a record changes only by
-- its replacement with its own tombstone.
CREATE FUNCTION audit.access_log_imported_append_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION '% is append-only', TG_TABLE_NAME
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- A seal is marked published once and never changed or removed; a fold row
-- is never changed or removed. Shared by the seals of both logs.
CREATE FUNCTION audit.seal_reject_change() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND TG_TABLE_NAME IN ('access_log_seal', 'audit_event_seal')
       AND OLD.published_at IS NULL AND NEW.published_at IS NOT NULL
       AND (NEW.window_no, NEW.first_id, NEW.last_id, NEW.entries, NEW.previous_digest,
            NEW.digest, NEW.sealed_at, NEW.proof, NEW.system_id)
           IS NOT DISTINCT FROM (OLD.window_no, OLD.first_id, OLD.last_id, OLD.entries,
                                 OLD.previous_digest, OLD.digest, OLD.sealed_at, OLD.proof,
                                 OLD.system_id)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'audit.% is append-only: a seal is marked published once and never removed', TG_TABLE_NAME
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- What the first sealed window of the system records follows: the SHA-256
-- of the seal text's version tag (system_log::seal).
CREATE FUNCTION audit.audit_event_seal_genesis() RETURNS bytea
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$ SELECT sha256(convert_to('ferroehr.system_log.seal.v1', 'UTF8')) $$;
COMMENT ON FUNCTION audit_event_seal_genesis() IS
    'The previous digest of the first sealed window of the system records.';

-- ── the holds ────────────────────────────────────────────────────────────────
-- A write's system records are held until its commit is confirmed (#5762):
-- written in the records' own statement, removed by the release once the
-- write's transaction committed, or with the records by the discard of one
-- that never did. A held record is in no read, no seal and no delivery.
CREATE TABLE audit_event_hold (
    seq         bigint      NOT NULL,
    reserved_by uuid        NOT NULL,
    taken_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_audit_event_hold PRIMARY KEY (seq)
);
CREATE INDEX ix_audit_event_hold_reserved ON audit_event_hold (reserved_by);
COMMENT ON TABLE audit_event_hold IS
    'The system records a write holds until its commit is confirmed (#5762); removed by the release, or with its records by the discard.';

-- ── the reaped positions ─────────────────────────────────────────────────────
-- Retention removes a record past its end (Regulation (EU) 2025/327 Annex II
-- 3.4) and keeps here only its position and its content digest, no content,
-- so the window that covers it still verifies. A row leaves only by the fold
-- of a sealed window every record of which is reaped.
CREATE FUNCTION audit.audit_event_reaped_reject_change() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND coalesce(current_setting('ferroehr.audit_folding', true), 'off') = 'on'
    THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'audit.audit_event_reaped is append-only: a reaped position leaves only by the fold of its sealed window'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TABLE audit_event_reaped (
    seq            bigint      NOT NULL,
    content_digest bytea       NOT NULL,
    reaped_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_audit_event_reaped PRIMARY KEY (seq),
    CONSTRAINT ck_audit_event_reaped_digest CHECK (octet_length(content_digest) = 32)
);
CREATE TRIGGER audit_event_reaped_append_only
    BEFORE UPDATE OR DELETE ON audit_event_reaped
    FOR EACH ROW EXECUTE FUNCTION audit.audit_event_reaped_reject_change();
CREATE TRIGGER audit_event_reaped_no_truncate
    BEFORE TRUNCATE ON audit_event_reaped
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_event_reaped_reject_change();
COMMENT ON TABLE audit_event_reaped IS
    'The position and content digest of each system record retention removed, and nothing of its content, so its window still verifies; left only by the fold of a sealed window every record of which is reaped.';

-- ── the seals ────────────────────────────────────────────────────────────────
-- One row per sealed window: every record position from first_id to last_id,
-- the window after the one before it with no position between, and the
-- SHA-256 of the text system_log::seal documents, signed when the keyring
-- holds a key.
CREATE TABLE audit_event_seal (
    window_no       bigint      NOT NULL,
    first_id        bigint      NOT NULL,
    last_id         bigint      NOT NULL,
    entries         integer     NOT NULL,
    previous_digest bytea       NOT NULL,
    digest          bytea       NOT NULL,
    sealed_at       timestamptz NOT NULL,
    proof           text,
    system_id       text,
    published_at    timestamptz,
    CONSTRAINT pk_audit_event_seal PRIMARY KEY (window_no),
    CONSTRAINT uq_audit_event_seal_last UNIQUE (last_id),
    CONSTRAINT ck_audit_event_seal_span CHECK (last_id >= first_id AND first_id >= 1),
    CONSTRAINT ck_audit_event_seal_entries CHECK (entries >= 0),
    CONSTRAINT ck_audit_event_seal_digests
        CHECK (octet_length(previous_digest) = 32 AND octet_length(digest) = 32),
    CONSTRAINT ck_audit_event_seal_proof
        CHECK ((proof IS NULL) = (system_id IS NULL) AND (proof IS NULL OR proof <> ''))
);
CREATE TRIGGER audit_event_seal_append_only
    BEFORE UPDATE OR DELETE ON audit_event_seal
    FOR EACH ROW EXECUTE FUNCTION audit.seal_reject_change();
CREATE TRIGGER audit_event_seal_no_truncate
    BEFORE TRUNCATE ON audit_event_seal
    FOR EACH STATEMENT EXECUTE FUNCTION audit.seal_reject_change();
COMMENT ON TABLE audit_event_seal IS
    'The sealed windows of the system records, append-only: each covers every position from first_id to last_id and follows the window before it.';
COMMENT ON COLUMN audit_event_seal.digest IS
    'The SHA-256 of the window''s text (system_log::seal): the version tag, the window''s fields, the previous digest, and each record''s position and content digest in position order.';
COMMENT ON COLUMN audit_event_seal.proof IS
    'The armored OpenPGP signature of the window''s seal (system_log::seal::ChainSeal) over digest, named by system_id and sealed_at; NULL when the keyring held no key.';

-- The sealer's place: the last position and window sealed and the window's
-- digest. Its row lock makes one sealer run at a time across replicas.
CREATE TABLE audit_event_seal_state (
    singleton boolean NOT NULL DEFAULT true,
    last_id   bigint  NOT NULL DEFAULT 0,
    window_no bigint  NOT NULL DEFAULT 0,
    digest    bytea   NOT NULL,
    CONSTRAINT pk_audit_event_seal_state PRIMARY KEY (singleton),
    CONSTRAINT ck_audit_event_seal_state_singleton CHECK (singleton)
);
INSERT INTO audit_event_seal_state (singleton, last_id, window_no, digest)
VALUES (true, 0, 0, audit.audit_event_seal_genesis());

-- A window every record of which retention reaped, whose reaped positions
-- the fold removed; its seal stays, and the verifier takes the window as
-- folded.
CREATE TABLE audit_event_seal_fold (
    window_no   bigint      NOT NULL,
    grounds     jsonb       NOT NULL,
    time_folded timestamptz NOT NULL,
    CONSTRAINT pk_audit_event_seal_fold PRIMARY KEY (window_no),
    CONSTRAINT ck_audit_event_seal_fold_grounds
        CHECK (jsonb_typeof(grounds) = 'array' AND jsonb_array_length(grounds) > 0)
);
CREATE TRIGGER audit_event_seal_fold_append_only
    BEFORE UPDATE OR DELETE ON audit_event_seal_fold
    FOR EACH ROW EXECUTE FUNCTION audit.seal_reject_change();
CREATE TRIGGER audit_event_seal_fold_no_truncate
    BEFORE TRUNCATE ON audit_event_seal_fold
    FOR EACH STATEMENT EXECUTE FUNCTION audit.seal_reject_change();
COMMENT ON TABLE audit_event_seal_fold IS
    'The sealed windows of the system records the fold removed, one row each, append-only.';

CREATE TABLE audit_event_seal_verified (
    singleton        boolean     NOT NULL DEFAULT true,
    through_window   bigint      NOT NULL,
    verified_at      timestamptz NOT NULL,
    full_verified_at timestamptz,
    CONSTRAINT pk_audit_event_seal_verified PRIMARY KEY (singleton),
    CONSTRAINT ck_audit_event_seal_verified_singleton CHECK (singleton)
);
COMMENT ON TABLE audit_event_seal_verified IS
    'The newest window of the system records a verification found intact with every window before it: where an incremental verification starts.';

-- Removes the reaped positions of the sealed windows that hold no record and
-- every position of which is reaped, oldest window first, up to p_limit
-- windows, recording each as folded on the ground of retention.
CREATE FUNCTION audit.fold_audit_event_reaped(p_now timestamptz, p_limit integer)
    RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    v_folded bigint;
BEGIN
    IF p_now IS NULL OR coalesce(p_limit, 0) < 1 THEN
        RETURN 0;
    END IF;
    PERFORM pg_advisory_xact_lock(hashtext('audit.audit_event_seal_fold'), 0);
    PERFORM set_config('ferroehr.audit_folding', 'on', true);
    WITH due AS (
        SELECT s.window_no, s.first_id, s.last_id
          FROM audit.audit_event_seal s
         WHERE s.entries > 0
           AND NOT EXISTS (SELECT 1 FROM audit.audit_event_seal_fold f
                            WHERE f.window_no = s.window_no)
           AND NOT EXISTS (SELECT 1 FROM audit.audit_event e
                            WHERE e.seq BETWEEN s.first_id AND s.last_id)
           AND (SELECT count(*) FROM audit.audit_event_reaped r
                 WHERE r.seq BETWEEN s.first_id AND s.last_id) = s.entries
         ORDER BY s.window_no
         LIMIT p_limit
    ),
    written AS (
        INSERT INTO audit.audit_event_seal_fold (window_no, grounds, time_folded)
        SELECT d.window_no, '["retention"]'::jsonb, p_now FROM due d ORDER BY d.window_no
        RETURNING 1
    ),
    removed AS (
        DELETE FROM audit.audit_event_reaped r
         USING due d
         WHERE r.seq BETWEEN d.first_id AND d.last_id
        RETURNING 1
    )
    SELECT (SELECT count(*) FROM written) + 0 * (SELECT count(*) FROM removed) INTO v_folded;
    PERFORM set_config('ferroehr.audit_folding', 'off', true);
    RETURN v_folded;
END;
$$;
COMMENT ON FUNCTION fold_audit_event_reaped(timestamptz, integer) IS
    'The one path a reaped position leaves by: up to p_limit sealed windows of the system records that hold no record and whose positions are all reaped lose their reaped positions, each recorded in audit_event_seal_fold; the seals stay. Returns the windows folded.';
