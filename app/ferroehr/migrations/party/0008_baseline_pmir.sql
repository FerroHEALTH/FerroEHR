-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the IHE PMIR consumer: the subscriptions, the received feed messages,
-- the identities the Registry names, the merges and the suppressed parties.
--
-- Runs with search_path = party, ext, public.

-- ── pmir_subscription ───────────────────────────────────────────────────────
-- The subscription FerroEHR holds at the Registry: one row, whose location
-- every replica reads, so a second replica adopts it in place of creating
-- another.
CREATE TABLE pmir_subscription (
    -- Always true: the table holds one row.
    singleton   boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    -- The Subscription's URL at the Registry (§2:3.94.4.2.2, Location).
    location    text NOT NULL,
    -- The channel endpoint the subscription names: this server's feed route.
    endpoint    text NOT NULL,
    -- The status the Registry answered last (§2:3.94.4.3).
    status      text NOT NULL,
    checked_at  timestamptz NOT NULL DEFAULT now()
);

-- ── pmir_message ────────────────────────────────────────────────────────────
-- One row per ITI-93 message received, by its MessageHeader.id.
CREATE TABLE pmir_message (
    -- The MessageHeader.id, which a redelivery repeats.
    message_id   text PRIMARY KEY,
    -- Arrival order: the Registry sends a subscription's messages one at a
    -- time and in order, so this is the order they are applied in.
    sequence     bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
    received_at  timestamptz NOT NULL DEFAULT now(),
    -- The token issuer and client the message was authenticated as.
    issuer       text NOT NULL,
    client_id    text NOT NULL,
    -- 'received' until applied; 'applied'; 'refused' with the reason; or
    -- 'settled' once an operator decided a refused message.
    state        text NOT NULL DEFAULT 'received',
    -- The changes, as the feed reads them: the Registry's resource ids and the
    -- business identifiers, never a demographic. Cleared once the message is
    -- applied, so an identifier stays no longer than the apply needs it; a
    -- refused message keeps them until it is settled.
    events       jsonb,
    -- Why the message was refused: a fixed text that names no value.
    reason       text,
    -- The replica holding the apply lease, and until when.
    leased_by    uuid,
    leased_until timestamptz,
    settled_at   timestamptz,
    -- The Registry's Patient.ids the message names, its survivors included,
    -- kept after it applies, so a retry of an older message that names one of
    -- them is refused. Resource ids of the Registry, never a business
    -- identifier.
    identities      text[] NOT NULL DEFAULT '{}',
    -- The apply attempts that ended, refused or applied.
    attempts        integer NOT NULL DEFAULT 0,
    -- When the newest attempt refused the message.
    refused_at      timestamptz,
    -- The operator's settlement: 'discard' keeps the splits, 'lift-splits'
    -- removes them.
    decision        text,
    -- Who decided: the token subject, or the operator name the CLI was given.
    decided_by      text,
    -- Why, as the operator wrote it.
    decision_reason text,
    CONSTRAINT ck_pmir_message_state
        CHECK (state IN ('received', 'applied', 'refused', 'settled')),
    CONSTRAINT ck_pmir_message_events
        CHECK ((state IN ('applied', 'settled')) = (events IS NULL)),
    CONSTRAINT ck_pmir_message_decision
        CHECK ((state = 'settled') = (decision IS NOT NULL)
               AND (decision IS NULL OR decision IN ('discard', 'lift-splits'))
               AND (decision IS NULL OR (decided_by IS NOT NULL AND decision_reason IS NOT NULL)))
);
-- The head of the queue: the oldest message still to apply.
CREATE INDEX ix_pmir_message_pending ON pmir_message (sequence) WHERE state = 'received';
-- The refused messages an operator still has to act on.
CREATE INDEX ix_pmir_message_refused ON pmir_message (sequence) WHERE state = 'refused';
-- The applied messages naming an identity, for the retry guard.
CREATE INDEX ix_pmir_message_identities ON pmir_message USING gin (identities);
COMMENT ON TABLE pmir_message IS 'The ITI-93 messages received from the PMIR Patient Identity Registry, stored before they are acknowledged and applied in arrival order; no specification governs the store, our own design.';
COMMENT ON COLUMN pmir_message.decision IS 'The operator''s settlement of a refused message: discard keeps its splits, lift-splits removes them; our own design.';

-- ── pmir_identity ───────────────────────────────────────────────────────────
-- Each Patient Master Identity the feed named, with the PERSON it is bound
-- to and the identity it was merged into.
CREATE TABLE pmir_identity (
    -- The Registry's Patient.id.
    pix_id      text PRIMARY KEY,
    -- The PERSON container the identity is bound to, once one carries one of
    -- its identifiers alone.
    person_id   uuid,
    -- The surviving identity while this one is merged (§2:3.93.4.1.2.4).
    merged_into text REFERENCES pmir_identity (pix_id),
    updated_at  timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX ix_pmir_identity_person ON pmir_identity (person_id);

-- ── pmir_merge ──────────────────────────────────────────────────────────────
-- Each merge applied, with the record subjects it moved from the merged
-- identity's PERSON to the survivor's, which an unmerge moves back.
CREATE TABLE pmir_merge (
    id              uuid PRIMARY KEY,
    message_id      text NOT NULL REFERENCES pmir_message (message_id),
    merged_pix      text NOT NULL REFERENCES pmir_identity (pix_id),
    survivor_pix    text NOT NULL REFERENCES pmir_identity (pix_id),
    merged_person   uuid,
    survivor_person uuid,
    -- The record subjects moved, as [[namespace, id], …].
    subjects        jsonb NOT NULL,
    -- The message that reverted the merge.
    reverted_by     text REFERENCES pmir_message (message_id),
    applied_at      timestamptz NOT NULL DEFAULT now()
);
-- At most one merge of an identity stands unreverted.
CREATE UNIQUE INDEX uq_pmir_merge_standing ON pmir_merge (merged_pix) WHERE reverted_by IS NULL;

-- ── pmir_suppression ────────────────────────────────────────────────────────
-- An identifier or record subject that stops resolving from the receipt of
-- the message that splits it until that message is applied: the patient
-- identifier lookup and the record subject reads skip it, so the window in
-- which a split identity reaches the other patient is the delivery time.
CREATE TABLE pmir_suppression (
    message_id text NOT NULL REFERENCES pmir_message (message_id),
    -- The PERSON whose identifier or record subject stops resolving.
    holder_id  uuid NOT NULL,
    -- 'identifier' (binding_patient_identifier) or 'subject'
    -- (binding_record_subject).
    kind       text NOT NULL,
    -- The identifier's issuer, or the subject reference's namespace.
    namespace  text NOT NULL,
    -- The identifier's id as the stored body carries it, or the subject id.
    value      text NOT NULL,
    CONSTRAINT pk_pmir_suppression PRIMARY KEY (holder_id, kind, namespace, value, message_id),
    CONSTRAINT ck_pmir_suppression_kind CHECK (kind IN ('identifier', 'subject'))
);
CREATE INDEX ix_pmir_suppression_message ON pmir_suppression (message_id);
CREATE INDEX ix_pmir_suppression_subject ON pmir_suppression (namespace, lower(value))
    WHERE kind = 'subject';
COMMENT ON TABLE pmir_suppression IS 'The identifiers and record subjects an unmerge or unlink splits, which stop resolving from the receipt of the message until it is applied; our own design.';
