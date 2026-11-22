-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the delivery of both logs to the sinks that deliver from the store
-- (IHE ITI-20: the ATX: FHIR Feed and the syslog route).
--
-- A sink delivers each log in id order from its cursor up to the delivery
-- frontier (#6242): every record at or below its cursor was accepted by the
-- repository or parked, and none above it was. The frontier round moves
-- the delivery frontier while a sink is configured, never past a record a
-- write holds. A record a sink cannot render or its repository refuses is
-- parked, never skipped: it stays until it is delivered or an operator acts.
-- After a crash a sink repeats at most the record it had in flight.
--
-- No specification governs where a record waits for its delivery: our own
-- design.
--
-- Runs with search_path = audit, ext, public.

-- ── the delivery frontier ────────────────────────────────────────────────────
-- Per log, the highest id every sink may deliver up to: the log's frontier,
-- below every record a write holds.
CREATE TABLE delivery_frontier (
    chain      text   NOT NULL,
    through_id bigint NOT NULL DEFAULT 0,
    CONSTRAINT pk_delivery_frontier PRIMARY KEY (chain),
    CONSTRAINT ck_delivery_frontier_chain CHECK (chain IN ('system', 'access_log'))
) WITH (fillfactor = 10);
INSERT INTO delivery_frontier (chain, through_id) VALUES ('system', 0), ('access_log', 0);
COMMENT ON TABLE delivery_frontier IS
    'Per log, the highest id the sinks may deliver up to: the frontier, below every record a write holds; moved by the frontier round while a sink is configured.';

-- ── the cursors ──────────────────────────────────────────────────────────────
-- One row per sink and log: every record at or below through_id was accepted
-- by the sink's repository or is parked. A sink's first frontier round
-- starts it at the delivery frontier then; a sink taken out of the
-- configuration keeps its cursor until an operator forgets it.
CREATE TABLE delivery_cursor (
    sink       text        NOT NULL,
    chain      text        NOT NULL,
    through_id bigint      NOT NULL,
    started_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_delivery_cursor PRIMARY KEY (sink, chain),
    CONSTRAINT ck_delivery_cursor_sink CHECK (sink IN ('fhir_feed', 'syslog')),
    CONSTRAINT ck_delivery_cursor_chain CHECK (chain IN ('system', 'access_log')),
    CONSTRAINT ck_delivery_cursor_through CHECK (through_id >= 0)
) WITH (fillfactor = 10);
COMMENT ON TABLE delivery_cursor IS
    'Per sink and log, the id at or below which every record was accepted by the sink''s repository or parked; moved after the repository accepted a record.';

-- ── the parked records ───────────────────────────────────────────────────────
-- A record a sink cannot render, its repository refused, or retention removed
-- before the sink delivered it. It stays until it is delivered, after an
-- operator asked to retry it, or an operator dismisses it.
CREATE TABLE delivery_parked (
    sink      text        NOT NULL,
    chain     text        NOT NULL,
    record_id bigint      NOT NULL,
    reason    text        NOT NULL,
    parked_at timestamptz NOT NULL DEFAULT now(),
    retry     boolean     NOT NULL DEFAULT false,
    CONSTRAINT pk_delivery_parked PRIMARY KEY (sink, chain, record_id),
    CONSTRAINT ck_delivery_parked_sink CHECK (sink IN ('fhir_feed', 'syslog')),
    CONSTRAINT ck_delivery_parked_chain CHECK (chain IN ('system', 'access_log')),
    CONSTRAINT ck_delivery_parked_reason
        CHECK (reason IN ('unrenderable', 'refused', 'reaped', 'tombstoned'))
);
COMMENT ON TABLE delivery_parked IS
    'The records a sink did not deliver: one it could not render, its repository refused, or retention removed or replaced by a tombstone first; kept until it is delivered on a retry or an operator dismisses it.';

-- Parks, for every sink whose cursor of p_chain is below p_record, that record
-- with p_reason: called as retention removes or replaces a record a sink has
-- not delivered.
CREATE FUNCTION audit.delivery_park_undelivered(p_chain text, p_records bigint[], p_reason text)
    RETURNS bigint
    LANGUAGE sql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    WITH parked AS (
        INSERT INTO audit.delivery_parked (sink, chain, record_id, reason)
        SELECT c.sink, c.chain, r.id, p_reason
          FROM audit.delivery_cursor c
          JOIN unnest(p_records) AS r(id) ON r.id > c.through_id
         WHERE c.chain = p_chain
        ON CONFLICT (sink, chain, record_id) DO NOTHING
        RETURNING 1
    )
    SELECT count(*) FROM parked
$$;
COMMENT ON FUNCTION delivery_park_undelivered(text, bigint[], text) IS
    'Parks each of p_records of the log p_chain for every sink whose cursor has not passed it, with p_reason; retention calls it as it removes or replaces records.';
