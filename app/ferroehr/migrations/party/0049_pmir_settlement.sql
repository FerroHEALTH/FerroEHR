-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the operator's retry and settlement of a refused ITI-93 message
-- (#5725). A refused message keeps its splits in force until an operator
-- retries it, after fixing its cause, or settles it: discarded with its
-- splits kept, or closed with its splits lifted by a recorded decision.
-- PMIR leaves the Consumer's handling to its capabilities (PMIR 1.6.0
-- §2:3.93.4.1.3): our own design.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE pmir_message
    -- The Registry's Patient.ids the message names, its survivors included,
    -- kept after it applies, so a retry of an older message that names one of
    -- them is refused. Resource ids of the Registry, never a business
    -- identifier.
    ADD COLUMN identities      text[] NOT NULL DEFAULT '{}',
    -- The apply attempts that ended, refused or applied.
    ADD COLUMN attempts        integer NOT NULL DEFAULT 0,
    -- When the newest attempt refused the message.
    ADD COLUMN refused_at      timestamptz,
    -- The operator's settlement: 'discard' keeps the splits, 'lift-splits'
    -- removes them.
    ADD COLUMN decision        text,
    -- Who decided: the token subject, or the operator name the CLI was given.
    ADD COLUMN decided_by      text,
    -- Why, as the operator wrote it.
    ADD COLUMN decision_reason text;

-- The column the earlier states called the settlement time is the time a
-- message reached its last state; the refusal time is its own column now.
UPDATE pmir_message SET refused_at = settled_at, settled_at = NULL WHERE state = 'refused';

UPDATE pmir_message
   SET identities = ARRAY(
       SELECT DISTINCT value FROM (
           SELECT e ->> 'id' AS value FROM jsonb_array_elements(events) e
           UNION
           SELECT e ->> 'replaced_by' FROM jsonb_array_elements(events) e
       ) named WHERE value IS NOT NULL AND value <> '' ORDER BY value)
 WHERE events IS NOT NULL;

ALTER TABLE pmir_message
    DROP CONSTRAINT ck_pmir_message_state,
    DROP CONSTRAINT ck_pmir_message_events,
    ADD CONSTRAINT ck_pmir_message_state
        CHECK (state IN ('received', 'applied', 'refused', 'settled')),
    ADD CONSTRAINT ck_pmir_message_events
        CHECK ((state IN ('applied', 'settled')) = (events IS NULL)),
    ADD CONSTRAINT ck_pmir_message_decision
        CHECK ((state = 'settled') = (decision IS NOT NULL)
               AND (decision IS NULL OR decision IN ('discard', 'lift-splits'))
               AND (decision IS NULL OR (decided_by IS NOT NULL AND decision_reason IS NOT NULL)));

-- The refused messages an operator still has to act on.
CREATE INDEX ix_pmir_message_refused ON pmir_message (sequence) WHERE state = 'refused';
-- The applied messages naming an identity, for the retry guard.
CREATE INDEX ix_pmir_message_identities ON pmir_message USING gin (identities);

COMMENT ON COLUMN pmir_message.decision IS 'The operator''s settlement of a refused message: discard keeps its splits, lift-splits removes them; our own design.';
