-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the FHIR Feed owes only the records written while it is configured.
--
-- A record was pending for a sink while its delivery stamp was unset, and the
-- two partial indexes over the unset stamps held every record of a deployment
-- with no sink configured, growing with the trail. The syslog sink delivers
-- from ihe-atna's spool and never reads or stamps delivered_syslog_at, so its
-- index served no query. The FHIR Feed (ITI-20 ATX: FHIR Feed) now reads the
-- records it owes from fhir_feed_outbox, which the store writes in the same
-- statement as the record only while the feed is configured, and which
-- delivery empties. A deployment with no feed keeps an empty outbox.
--
-- A record written before this migration names no outbox entry. Those still
-- pending stay owed to a feed configured later: fhir_feed_backlog keeps the
-- newest position written before the migration, and the first delivery run
-- moves the pending records up to it into the outbox, once.
--
-- No openEHR spec governs audit storage mechanics: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

DROP INDEX ix_audit_event_pending_syslog;
DROP INDEX ix_audit_event_pending_fhir_feed;

-- The foreign key cascades, so retention reaping still removes a record the
-- feed owed, as it did before.
CREATE TABLE fhir_feed_outbox (
    chain_seq bigint NOT NULL,
    CONSTRAINT pk_fhir_feed_outbox PRIMARY KEY (chain_seq),
    CONSTRAINT fk_fhir_feed_outbox_record FOREIGN KEY (chain_seq)
        REFERENCES audit_event (chain_seq) ON DELETE CASCADE
);

COMMENT ON TABLE fhir_feed_outbox IS
    'The chain positions of the records the FHIR Feed still owes: written with the record while the feed is configured, removed when the repository accepted it.';

CREATE TABLE fhir_feed_backlog (
    singleton    boolean NOT NULL DEFAULT true,
    through_seq  bigint  NOT NULL,
    enqueued_at  timestamptz,
    CONSTRAINT pk_fhir_feed_backlog PRIMARY KEY (singleton),
    CONSTRAINT ck_fhir_feed_backlog_singleton CHECK (singleton)
);

COMMENT ON TABLE fhir_feed_backlog IS
    'The newest chain position written before fhir_feed_outbox existed, and when its pending records were moved into the outbox (NULL until a feed first ran).';

INSERT INTO fhir_feed_backlog (through_seq)
SELECT coalesce(max(chain_seq), 0) FROM audit_event;

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
    INSERT INTO audit.fhir_feed_outbox (chain_seq)
    SELECT e.chain_seq FROM audit.audit_event e
     WHERE e.chain_seq <= through AND e.delivered_fhir_feed_at IS NULL
    ON CONFLICT DO NOTHING;
    GET DIAGNOSTICS moved = ROW_COUNT;
    UPDATE audit.fhir_feed_backlog SET enqueued_at = now() WHERE singleton;
    RETURN moved;
END;
$$;

COMMENT ON FUNCTION audit.enqueue_fhir_feed_backlog() IS
    'Moves the records written before fhir_feed_outbox existed and never delivered by the FHIR Feed into the outbox, once; returns how many it moved.';

COMMENT ON COLUMN audit_event.delivered_syslog_at IS
    'Unused: the syslog sink delivers from its own spool and never stamps a record.';

-- Functions are executable by PUBLIC unless revoked (PostgreSQL 18, GRANT,
-- "Notes", https://www.postgresql.org/docs/18/sql-grant.html).
REVOKE ALL ON FUNCTION audit.enqueue_fhir_feed_backlog() FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON fhir_feed_outbox, fhir_feed_backlog
            FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT, INSERT, DELETE ON fhir_feed_outbox TO ferroehr_clinical;
        GRANT SELECT ON fhir_feed_backlog TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.enqueue_fhir_feed_backlog() TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping audit FHIR feed outbox grants (roles absent — see the ext role block NOTICE)';
    END IF;
END
$grants$;
