-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the outbox of the FHIR Feed (IHE ITI-20, ATX: FHIR Feed).
--
-- Runs with search_path = audit, ext, public.

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
    'The records the FHIR Feed still owes, each by its chain and position, in the order they were written: written with the record while the feed is configured, removed when the repository accepted it, so a row is the delivery state of its record. The feed delivers by id.';
COMMENT ON COLUMN fhir_feed_outbox.chain IS
    'The chain the record is in: system (audit_event) or access_log (access_log_record).';
