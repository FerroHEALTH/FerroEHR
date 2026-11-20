-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the outbox of the FHIR Feed (IHE ITI-20, ATX: FHIR Feed).
--
-- Runs with search_path = audit, ext, public.

CREATE TABLE fhir_feed_backlog (
    singleton    boolean NOT NULL DEFAULT true,
    through_seq  bigint  NOT NULL,
    enqueued_at  timestamptz,
    CONSTRAINT pk_fhir_feed_backlog PRIMARY KEY (singleton),
    CONSTRAINT ck_fhir_feed_backlog_singleton CHECK (singleton)
);
INSERT INTO fhir_feed_backlog (through_seq) VALUES (0);
COMMENT ON TABLE fhir_feed_backlog IS
    'The newest chain position written before fhir_feed_outbox existed, and when its pending records were moved into the outbox (NULL until a feed first ran).';

-- ── fhir_feed_outbox ─────────────────────────────────────────────────────────
-- The records the FHIR Feed still owes, each named by the chain it belongs to
-- and its position there.
CREATE TABLE fhir_feed_outbox (
    id        bigint NOT NULL GENERATED ALWAYS AS IDENTITY,
    chain     text   NOT NULL,
    chain_seq bigint NOT NULL,
    CONSTRAINT pk_fhir_feed_outbox PRIMARY KEY (id),
    CONSTRAINT uq_fhir_feed_outbox_record UNIQUE (chain, chain_seq),
    CONSTRAINT ck_fhir_feed_outbox_chain CHECK (chain IN ('system', 'access_log'))
);
COMMENT ON TABLE fhir_feed_outbox IS
    'The records the FHIR Feed still owes, each by its chain and position, in the order they were written: written with the record while the feed is configured, removed when the repository accepted it. The feed delivers by id.';
COMMENT ON COLUMN fhir_feed_outbox.chain IS
    'The chain the record is in: system (audit_event) or access_log (access_log_record).';

CREATE FUNCTION audit.enqueue_fhir_feed_backlog() RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    through bigint;
    moved   bigint;
BEGIN
    SELECT b.through_seq INTO through
      FROM audit.fhir_feed_backlog b
     WHERE b.singleton AND b.enqueued_at IS NULL
       FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 0;
    END IF;
    INSERT INTO audit.fhir_feed_outbox (chain, chain_seq)
    SELECT 'system', e.chain_seq FROM audit.audit_event e
     WHERE e.chain_seq <= through AND e.delivered_fhir_feed_at IS NULL
     ORDER BY e.chain_seq
    ON CONFLICT DO NOTHING;
    GET DIAGNOSTICS moved = ROW_COUNT;
    UPDATE audit.fhir_feed_backlog SET enqueued_at = now() WHERE singleton;
    RETURN moved;
END;
$$;
COMMENT ON FUNCTION enqueue_fhir_feed_backlog() IS
    'Moves the records written before fhir_feed_outbox existed and never delivered by the FHIR Feed into the outbox, once; returns how many it moved.';
