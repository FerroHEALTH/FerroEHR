-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the transactional change-event outbox, its reader cursors and the
-- subscription filters over it.
--
-- One outbox row is written in the SAME transaction as every CONTRIBUTION
-- commit: no commit without its event, no event without its commit. The row
-- carries a PHI-FREE envelope (the contribution id, the EHR id, the commit
-- instant and the per-version kind/change type/template) — clinical content is
-- never in an event; a consumer fetches bodies through the authenticated API.
--
-- The erasure tombstone is the one row that announces no commit. A physical
-- EHR delete appends it in the same transaction as the delete, and every
-- consumer that derived anything from that EHR applies it by deleting what it
-- derived: GDPR Art. 19 makes the controller communicate an erasure "to each
-- recipient to whom the personal data have been disclosed"
-- (docs/law/eu/gdpr/text.html Art. 19). It carries no contribution, because
-- the contributions of that EHR are gone with it.
--
-- No openEHR spec governs eventing: ITS-REST 1.1.0 defines no change
-- notification, so this whole file is our own extension.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE event_outbox (
    -- The monotonic delivery sequence and the per-EHR ordering axis. A
    -- generated identity rather than a uuidv7, so a drainer can ORDER BY it
    -- and a consumer can reason about ordering.
    seq             bigint GENERATED ALWAYS AS IDENTITY,
    -- The announced commit; NULL on an erasure tombstone, which announces a
    -- delete rather than a commit and outlives every contribution of the
    -- erased EHR.
    contribution_id uuid,
    -- The owning EHR, mirroring contribution.ehr_id; the per-EHR ordering
    -- group.
    ehr_id          uuid,
    envelope        jsonb NOT NULL,
    -- The contribution's commit instant, copied so a consumer or the pruner
    -- needs no join back to the commit audit.
    committed_at    timestamptz NOT NULL,
    -- NULL = pending; the publish instant once the broker confirm lands
    -- (at-least-once delivery).
    published_at    timestamptz,
    CONSTRAINT pk_event_outbox PRIMARY KEY (seq),
    -- A contribution-less row is an erasure tombstone and nothing else, so a
    -- write path cannot drop the contribution reference by accident and call
    -- the result an event.
    CONSTRAINT ck_event_outbox_contribution CHECK
        (contribution_id IS NOT NULL OR envelope ->> 'event' = 'erase'),
    CONSTRAINT fk_event_outbox_contribution FOREIGN KEY (contribution_id)
        REFERENCES contribution (id) ON DELETE CASCADE
);

-- The drainer's working set: pending rows only, ordered for per-EHR delivery,
-- so the ordered SELECT ... FOR UPDATE SKIP LOCKED reads straight off the
-- index and the index stays small as rows drain.
CREATE INDEX idx_event_outbox_pending ON event_outbox (ehr_id, seq)
    WHERE published_at IS NULL;
-- Retention pruning scans published rows by age.
CREATE INDEX idx_event_outbox_published ON event_outbox (published_at)
    WHERE published_at IS NOT NULL;

COMMENT ON TABLE event_outbox IS 'Contribution-outbox eventing: one PHI-free event row per CONTRIBUTION commit, plus one erasure tombstone per physically deleted EHR (GDPR Art. 19), written in the same transaction and drained at-least-once in (ehr_id, seq) order. Not an audit record. Our own extension — no openEHR spec governs eventing.';
COMMENT ON COLUMN event_outbox.contribution_id IS 'The announced commit; NULL on an erasure tombstone, whose EHR has no contribution left.';
COMMENT ON COLUMN event_outbox.envelope IS 'The PHI-free payload: contribution id, ehr_id, committed_at, and the per-version (vo_id, kind, sys_version, change_type, template_id). An erasure tombstone carries event = erase and the erased ehr_id. No clinical content.';
COMMENT ON COLUMN event_outbox.published_at IS 'NULL = pending; the publish instant once the broker acknowledges.';

-- ── event_outbox_reader ──────────────────────────────────────────────────────
-- One row per registered reader over the outbox. The retention prune deletes a
-- published row only at or below the lowest active high-water mark, so a
-- reader that is behind is never overtaken. The instance is single-tenant, so
-- a reader is exactly one row.
CREATE TABLE event_outbox_reader (
    reader     text        NOT NULL,
    last_seq   bigint      NOT NULL DEFAULT 0,
    active     boolean     NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_event_outbox_reader PRIMARY KEY (reader),
    CONSTRAINT ck_event_outbox_reader_name CHECK (reader ~ '^[a-z][a-z0-9-]{0,62}$'),
    CONSTRAINT ck_event_outbox_reader_seq CHECK (last_seq >= 0)
);

COMMENT ON TABLE event_outbox_reader IS 'Cursor readers over event_outbox: one row per reader with its high-water mark and whether the deployment runs it. The retention prune deletes a published row only at or below the lowest active last_seq.';
COMMENT ON COLUMN event_outbox_reader.last_seq IS 'The highest event_outbox.seq the reader has fully processed; advanced monotonically, never moved back.';
COMMENT ON COLUMN event_outbox_reader.active IS 'Whether the deployment runs this reader, reconciled from the configuration at boot; only active readers hold the prune floor.';

-- ── event_subscription ───────────────────────────────────────────────────────
-- A server-side filter over the PHI-free event stream. Each enabled row maps
-- to an AMQP topic binding key `<kind>.<change_type>.<template_id|->` on the
-- ferroehr.events exchange and a durable queue ferroehr.events.<name>, so the
-- broker does the fan-out. A NULL predicate is a wildcard.
CREATE TABLE event_subscription (
    id          uuid NOT NULL DEFAULT uuidv7(),
    -- The subscription name, and the suffix of its broker queue.
    name        text NOT NULL,
    -- The versioned-object RM type; NULL = any kind.
    kind        text,
    -- The audit change-type group code; NULL = any change type.
    change_type text,
    -- The OPT template a COMPOSITION was committed against; NULL = any.
    template_id text,
    -- The publisher declares and binds a queue only for enabled rows.
    enabled     boolean NOT NULL DEFAULT true,
    created_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_event_subscription PRIMARY KEY (id),
    CONSTRAINT uq_event_subscription_name UNIQUE (name)
);

CREATE INDEX idx_event_subscription_enabled ON event_subscription (enabled)
    WHERE enabled;

COMMENT ON TABLE event_subscription IS 'Event-filter subscriptions: server-side predicate filters over the PHI-free event stream, each enabled row mapping to one AMQP topic binding key and durable queue. Our own extension.';
COMMENT ON COLUMN event_subscription.kind IS 'Predicate on the versioned-object RM type; NULL = wildcard.';
COMMENT ON COLUMN event_subscription.change_type IS 'Predicate on the audit change-type group code; NULL = wildcard.';
COMMENT ON COLUMN event_subscription.template_id IS 'Predicate on the OPT template id; NULL = wildcard.';
