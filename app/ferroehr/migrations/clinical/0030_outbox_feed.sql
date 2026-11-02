-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the outbox becomes a cursor feed (#4786).
--
-- FerroEHR no longer publishes to a message broker; each reader `[outbox]
-- readers` names pulls the rows after its cursor in event_outbox_reader and
-- acknowledges them. Nothing stamps a publish instant any more, so
-- published_at and the two indexes over it go, and the retention prune counts
-- from the commit instant. The subscription filters were the broker's
-- bindings and go with it. 0008_outbox.sql shipped, so the change is here.
--
-- No openEHR spec governs eventing: ITS-REST 1.1.0 defines no change
-- notification, so this whole file is our own extension.
--
-- Runs with search_path = clinical, ext, public.

DROP TABLE event_subscription;

DROP INDEX idx_event_outbox_pending;
DROP INDEX idx_event_outbox_published;
ALTER TABLE event_outbox DROP COLUMN published_at;

-- The retention prune scans by commit age.
CREATE INDEX idx_event_outbox_committed ON event_outbox (committed_at);

COMMENT ON TABLE event_outbox IS 'Contribution-outbox eventing: one PHI-free event row per CONTRIBUTION commit, plus one erasure tombstone per physically deleted EHR (GDPR Art. 19), written in the same transaction and read by each configured reader in seq order over the outbox feed. Not an audit record. Our own extension — no openEHR spec governs eventing.';
COMMENT ON TABLE event_outbox_reader IS 'The outbox feed readers: one row per reader with its acknowledged high-water mark and whether the configuration names it. The retention prune deletes a row only at or below the lowest active last_seq.';
COMMENT ON COLUMN event_outbox_reader.last_seq IS 'The highest event_outbox.seq the reader has acknowledged; moved forward only by an acknowledgement.';
COMMENT ON COLUMN event_outbox_reader.active IS 'Whether [outbox] readers names this reader, reconciled at boot; only active readers hold the prune floor.';
