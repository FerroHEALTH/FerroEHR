-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the outbox becomes a cursor feed (#4786), as the clinical one does in
-- its own 0030_outbox_feed.sql.
--
-- Each reader `[outbox] readers` names pulls the rows after its cursor in
-- event_outbox_reader and acknowledges them. Nothing stamps a publish instant
-- any more, so published_at and the two indexes over it go, and the retention
-- prune counts from the commit instant. 0006_outbox.sql shipped, so the change
-- is here.
--
-- No openEHR spec governs eventing: our own extension.
--
-- Runs with search_path = party, ext, public.

DROP INDEX idx_event_outbox_pending;
DROP INDEX idx_event_outbox_published;
ALTER TABLE event_outbox DROP COLUMN published_at;

-- The retention prune scans by commit age.
CREATE INDEX idx_event_outbox_committed ON event_outbox (committed_at);

COMMENT ON TABLE event_outbox IS 'Contribution-outbox eventing for the party domain: one PHI-free event row per party CONTRIBUTION commit, plus one erasure tombstone per physically deleted party (GDPR Art. 19), written in the same transaction and read by each configured reader in seq order over the outbox feed. Separate because its foreign key must stay inside this domain.';
COMMENT ON TABLE event_outbox_reader IS 'The party outbox feed readers: one row per reader with its acknowledged high-water mark and whether the configuration names it.';
