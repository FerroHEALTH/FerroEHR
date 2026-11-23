-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the access log's write and the holds' release and discard.
--
-- Every audit record is written durably before the response that reports the
-- access: a request writes its access-log entries and every other audit
-- record in its own transaction, before it answers, so concurrent requests
-- never wait on a head. An access-log entry is written once into
-- access_log_entry and a system record once into audit_event, each sealed
-- later by window (#6242). A write's records are held under a reservation id
-- until the write's own transaction commits it; the reconciler releases or
-- discards what a process that died left held.
--
-- No openEHR spec governs audit storage mechanics: our own design.
--
-- Runs with search_path = audit, ext, public.

-- Refuses an access-log entry naming a shared part or a render policy the store
-- does not hold, as the foreign keys did; called only for such a row. A part
-- the batch itself carried was removed by a tombstone between the statement
-- that stored it and the one that locks it: that is a serialization failure,
-- which the writer runs again (#5890), and the next run stores the part anew.
CREATE FUNCTION audit.access_log_intake_unknown(p_what text, p_digest text, p_removed boolean)
    RETURNS bytea
    LANGUAGE plpgsql VOLATILE
    SET search_path = pg_catalog
    AS $$
BEGIN
    IF p_removed THEN
        RAISE EXCEPTION 'the % % of an access-log entry was removed while it was written', p_what, p_digest
            USING ERRCODE = 'serialization_failure';
    END IF;
    RAISE EXCEPTION 'an access-log entry names the % %, which the store does not hold', p_what, p_digest
        USING ERRCODE = 'foreign_key_violation';
END;
$$;
COMMENT ON FUNCTION access_log_intake_unknown(text, text, boolean) IS
    'Raises foreign_key_violation for an access-log entry naming a shared part or a render policy the store does not hold, and serialization_failure for a part of the batch removed while it was written (#5887).';

-- Whether the shared part p_digest is gone, read with a snapshot of its own:
-- VOLATILE, so it sees a removal committed while the intake statement waited
-- on the part's row. Called only for a part the statement had to raise and
-- did not (#6241).
CREATE FUNCTION audit.access_log_intake_part_gone(p_digest text) RETURNS boolean
    LANGUAGE plpgsql VOLATILE
    SET search_path = pg_catalog
    AS $$
BEGIN
    RETURN NOT EXISTS (SELECT 1 FROM audit.access_log_shared s
                        WHERE s.digest = decode(p_digest, 'hex'));
END;
$$;
COMMENT ON FUNCTION access_log_intake_part_gone(text) IS
    'Whether a shared part is gone, read with a snapshot of its own; the intake asks it of a part it had to raise and did not (#6241).';

-- Releases the records held under committed reservations, counting them.
CREATE FUNCTION audit.audit_intake_release(p_ids uuid[]) RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    v_entries bigint;
    v_others  bigint;
BEGIN
    WITH released AS (
        DELETE FROM audit.access_log_hold WHERE reserved_by = ANY (p_ids)
        RETURNING 1
    )
    SELECT count(*) INTO v_entries FROM released r;
    WITH released AS (
        DELETE FROM audit.audit_event_hold WHERE reserved_by = ANY (p_ids)
        RETURNING 1
    )
    SELECT count(*) INTO v_others FROM released r;
    RETURN v_entries + v_others;
END;
$$;
COMMENT ON FUNCTION audit_intake_release(uuid[]) IS
    'Release the records held under the reservations named: their writes committed, so the reads, the sealer and the deliveries take them.';

-- Discards the records held under reservations that never committed, which
-- no seal covers.
CREATE FUNCTION audit.audit_intake_discard(p_ids uuid[]) RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    v_entries bigint;
    v_others  bigint;
BEGIN
    PERFORM set_config('ferroehr.audit_discarding', 'on', true);
    WITH removed AS (
        DELETE FROM audit.access_log_entry e
         USING audit.access_log_hold h
         WHERE h.entry_id = e.id AND h.reserved_by = ANY (p_ids)
        RETURNING 1
    ),
    dropped AS (
        DELETE FROM audit.access_log_hold WHERE reserved_by = ANY (p_ids)
        RETURNING 1
    )
    SELECT count(*) INTO v_entries FROM removed, (SELECT count(*) FROM dropped) AS d;
    WITH removed AS (
        DELETE FROM audit.audit_event e
         USING audit.audit_event_hold h
         WHERE h.seq = e.seq AND h.reserved_by = ANY (p_ids)
        RETURNING 1
    ),
    dropped AS (
        DELETE FROM audit.audit_event_hold WHERE reserved_by = ANY (p_ids)
        RETURNING 1
    )
    SELECT count(*) INTO v_others FROM removed, (SELECT count(*) FROM dropped) AS d;
    PERFORM set_config('ferroehr.audit_discarding', 'off', true);
    RETURN v_entries + v_others;
END;
$$;
COMMENT ON FUNCTION audit_intake_discard(uuid[]) IS
    'Discard the records held under the reservations named, whose writes never committed.';
