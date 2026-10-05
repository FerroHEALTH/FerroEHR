-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the domain's own transactional change-event outbox.
--
-- The clinical outbox cannot serve it: that table's foreign key points at the
-- clinical contribution relation, and a party commit does not live there — so
-- a party commit would either violate that key or leave the domain, which is
-- what the separation removes. Written in the same transaction as the commit
-- it announces, exactly as the clinical one is; the envelope is PHI-free by
-- construction (the contribution id, a NULL EHR id, the per-version kind and
-- change type), so pseudonymisation is not weakened by announcing that a party
-- changed.
--
-- The erasure tombstone is the one row that announces no commit. A physical
-- party delete appends it in the same transaction as the delete, and every
-- consumer that derived anything from that party applies it by deleting what
-- it derived: GDPR Art. 19 makes the controller communicate an erasure "to
-- each recipient to whom the personal data have been disclosed"
-- (docs/law/eu/gdpr/text.html Art. 19). It carries no contribution, because
-- the contributions of that party are gone with it.
--
-- No openEHR spec governs eventing: our own extension.
--
-- Runs with search_path = party, ext, public.
CREATE TABLE event_outbox (
    seq             bigint GENERATED ALWAYS AS IDENTITY,
    -- The announced commit; NULL on an erasure tombstone, which announces a
    -- delete rather than a commit and outlives every contribution of the
    -- erased party.
    contribution_id uuid,
    -- Always NULL here (a party has no owning EHR); kept so the envelope and
    -- the drainer read the same column set in both domains.
    ehr_id          uuid,
    envelope        jsonb NOT NULL,
    committed_at    timestamptz NOT NULL,
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

CREATE INDEX idx_event_outbox_pending ON event_outbox (seq)
    WHERE published_at IS NULL;
CREATE INDEX idx_event_outbox_published ON event_outbox (published_at)
    WHERE published_at IS NOT NULL;

COMMENT ON TABLE event_outbox IS 'Contribution-outbox eventing for the party domain: one PHI-free event row per party CONTRIBUTION commit, plus one erasure tombstone per physically deleted party (GDPR Art. 19), written in the same transaction and drained by the same publisher as the clinical outbox. Separate because its foreign key must stay inside this domain.';
COMMENT ON COLUMN event_outbox.contribution_id IS 'The announced commit; NULL on an erasure tombstone, whose party has no contribution left.';

CREATE TABLE event_outbox_reader (
    reader     text        NOT NULL,
    last_seq   bigint      NOT NULL DEFAULT 0,
    active     boolean     NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_event_outbox_reader PRIMARY KEY (reader),
    CONSTRAINT ck_event_outbox_reader_name CHECK (reader ~ '^[a-z][a-z0-9-]{0,62}$'),
    CONSTRAINT ck_event_outbox_reader_seq CHECK (last_seq >= 0)
);

COMMENT ON TABLE event_outbox_reader IS 'Cursor readers over the party outbox: one row per reader with its high-water mark and whether the deployment runs it.';
