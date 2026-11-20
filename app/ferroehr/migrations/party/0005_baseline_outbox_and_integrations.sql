-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the event outbox and its readers, the multimedia blob references,
-- and the reservations a write holds its audit records under.
--
-- Runs with search_path = party, ext, public.

-- The domain's own transactional change-event outbox.
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
    CONSTRAINT pk_event_outbox PRIMARY KEY (seq),
    -- A contribution-less row is an erasure tombstone and nothing else, so a
    -- write path cannot drop the contribution reference by accident and call
    -- the result an event.
    CONSTRAINT ck_event_outbox_contribution CHECK
        (contribution_id IS NOT NULL OR envelope ->> 'event' = 'erase'),
    CONSTRAINT fk_event_outbox_contribution FOREIGN KEY (contribution_id)
        REFERENCES contribution (id) ON DELETE CASCADE
);
-- The retention prune scans by commit age.
CREATE INDEX idx_event_outbox_committed ON event_outbox (committed_at);
COMMENT ON COLUMN event_outbox.contribution_id IS 'The announced commit; NULL on an erasure tombstone, whose party has no contribution left.';
COMMENT ON TABLE event_outbox IS 'Contribution-outbox eventing for the party domain: one PHI-free event row per party CONTRIBUTION commit, plus one erasure tombstone per physically deleted party (GDPR Art. 19), written in the same transaction and read by each configured reader in seq order over the outbox feed. Separate because its foreign key must stay inside this domain.';

CREATE TABLE event_outbox_reader (
    reader     text        NOT NULL,
    last_seq   bigint      NOT NULL DEFAULT 0,
    active     boolean     NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_event_outbox_reader PRIMARY KEY (reader),
    CONSTRAINT ck_event_outbox_reader_name CHECK (reader ~ '^[a-z][a-z0-9-]{0,62}$'),
    CONSTRAINT ck_event_outbox_reader_seq CHECK (last_seq >= 0)
);
COMMENT ON TABLE event_outbox_reader IS 'The party outbox feed readers: one row per reader with its acknowledged high-water mark and whether the configuration names it.';

-- The domain's own multimedia blob reference index.
--
-- A party body carries `DV_MULTIMEDIA` like a clinical one — a photograph on a
-- PERSON, a scanned document on an ORGANISATION — so the same externalization
-- and the same garbage collection reach this domain, and the index the
-- collector reads must live here rather than in the clinical schema: a row
-- naming a party's versioned object in the clinical schema would put a
-- demographic identifier where the clinical role can read it (GDPR Art. 4(5),
-- docs/law/eu/gdpr/text.html).
--
-- The table is the clinical `blob_ref` relation, same shape and same foreign
-- key, so one write path serves both domains. Our own extension — no openEHR
-- spec governs multimedia offload.
CREATE TABLE blob_ref (
    -- The storage tier, kept in lockstep with the version row by the foreign
    -- key's ON UPDATE CASCADE, exactly as `node` is.
    tier        text NOT NULL DEFAULT 'hot',
    vo_id       uuid NOT NULL,
    sys_version integer NOT NULL,
    -- The referenced blob's URI, as the stored body spells it.
    uri         text NOT NULL,
    CONSTRAINT pk_blob_ref PRIMARY KEY (tier, vo_id, sys_version, uri),
    CONSTRAINT ck_blob_ref_tier CHECK (tier IN ('hot', 'cold')),
    CONSTRAINT fk_blob_ref_version FOREIGN KEY (tier, vo_id, sys_version)
        REFERENCES version (tier, vo_id, sys_version)
        ON DELETE CASCADE ON UPDATE CASCADE
);
-- The collector's question, over both tiers: is this URI still referenced.
CREATE INDEX idx_blob_ref_uri ON blob_ref (uri);
COMMENT ON TABLE blob_ref IS 'Which stored party version references which externalized multimedia blob, so blob garbage collection is a lookup rather than a scan of node. The clinical relation''s twin, because a blob shared with a party must survive the deletion of the clinical record that also referenced it. Our own extension.';
COMMENT ON COLUMN blob_ref.uri IS 'The referenced blob''s URI, as the stored body spells it.';

-- The durable reservation of a write's audit records.
--
-- A request-scoped write commits through one choke point, which first writes
-- the write's audit records into the audit intake held under a reservation
-- id, then inserts that id here in the write's own transaction and commits.
-- After the commit the chainer's next round releases the held records inside
-- its own transaction, so the write costs no third commit. A process that
-- dies before that round leaves the records held, and the reconciler of any
-- replica releases the ones a row here names: the write committed, so its
-- records join the chain, exactly the records the request wrote, once. The
-- reconciler removes a row once its records are released. A row holds no identifier of a person or a caller,
-- only the reservation id the audit side resolves, so the domains stay
-- apart (GDPR Art. 32(1)(a)). No openEHR spec governs audit storage: our own
-- design.
CREATE TABLE audit_reservation (
    id       uuid        NOT NULL,
    taken_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_audit_reservation PRIMARY KEY (id)
);
COMMENT ON TABLE audit_reservation IS
    'The reservation ids of committed writes whose audit records are still held in the audit intake: inserted in the write''s transaction, removed by the reconciler once the records are released.';
