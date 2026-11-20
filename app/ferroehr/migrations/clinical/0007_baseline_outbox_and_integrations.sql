-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the event outbox and its readers, the multimedia blob references,
-- and the reservations a write holds its audit records under.
--
-- Runs with search_path = clinical, ext, public.

-- The transactional change-event outbox, its reader cursors and the
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
COMMENT ON COLUMN event_outbox.contribution_id IS 'The announced commit; NULL on an erasure tombstone, whose EHR has no contribution left.';
COMMENT ON COLUMN event_outbox.envelope IS 'The PHI-free payload: contribution id, ehr_id, committed_at, and the per-version (vo_id, kind, sys_version, change_type, template_id). An erasure tombstone carries event = erase and the erased ehr_id. No clinical content.';
COMMENT ON TABLE event_outbox IS 'Contribution-outbox eventing: one PHI-free event row per CONTRIBUTION commit, plus one erasure tombstone per physically deleted EHR (GDPR Art. 19), written in the same transaction and read by each configured reader in seq order over the outbox feed. Not an audit record. Our own extension — no openEHR spec governs eventing.';

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
COMMENT ON TABLE event_outbox_reader IS 'The outbox feed readers: one row per reader with its acknowledged high-water mark and whether the configuration names it. The retention prune deletes a row only at or below the lowest active last_seq.';
COMMENT ON COLUMN event_outbox_reader.last_seq IS 'The highest event_outbox.seq the reader has acknowledged; moved forward only by an acknowledgement.';
COMMENT ON COLUMN event_outbox_reader.active IS 'Whether [outbox] readers names this reader, reconciled at boot; only active readers hold the prune floor.';

-- ── blob_ref ─────────────────────────────────────────────────────────────────
-- Which stored version references which externalized multimedia blob.
--
-- `DV_MULTIMEDIA` may carry its content by reference
-- (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_multimedia.adoc`:
-- `uri` is a "URI reference to electronic information stored outside the
-- record as a file, database entry etc"), and the multimedia store holds those
-- bytes outside the database. Garbage collection after a physical delete
-- then has to answer "does any surviving version still reference this blob",
-- which a scan of every `node` row answers slowly and a lookup here answers as
-- an index probe. Our own extension — no openEHR spec governs multimedia
-- offload.
--
-- One row per (version, URI). The rows are written by the node write path, in
-- the same transaction as the nodes they describe, so the index is never
-- behind the content it indexes; and the foreign key below carries them across
-- the tier move and removes them with the version, so nothing else maintains
-- them.
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
COMMENT ON TABLE blob_ref IS 'Which stored version references which externalized multimedia blob, so blob garbage collection is a lookup rather than a scan of node. Written by the node write path in the same transaction; carried across the tier move and removed with the version by its foreign key. Our own extension.';
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
