-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the end a care relationship written without one takes from the
-- access policy (#6073).
--
-- The draft access control decision procedure, Writing Relationships: "A care
-- relationship written without an end takes the default_period of the first
-- ACCESS_POLICY.relationship_validity entry that covers its role and provider
-- type". The committed version stays exactly as it was sent, so its body,
-- digest and signature are untouched; the end it takes is computed once, in
-- the commit's transaction, from the policy version the request confirmed,
-- and kept here with that version. A refill of the caller facts reads it and
-- never recomputes it, so a later policy version applies only to the
-- relationships written after it. The caller fact of the relationship takes
-- it as the upper bound of its validity. No openEHR spec governs the side
-- table: our own design.
--
-- Runs with search_path = party, ext, public.

CREATE TABLE binding_care_validity (
    -- The care relationship's PARTY_RELATIONSHIP version container.
    vo_id          uuid PRIMARY KEY,
    -- The OBJECT_VERSION_ID of the version the end was computed for.
    version_uid    text NOT NULL,
    -- The first date the relationship no longer holds.
    effective_until date NOT NULL,
    -- The OBJECT_VERSION_ID of the ACCESS_POLICY version that set it.
    policy_version text NOT NULL,
    written_at     timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE binding_care_validity IS 'The end each care relationship head written without one takes from the access policy version its commit confirmed, with that version. Written in the commit''s transaction and kept by a refill, which never recomputes it; no openEHR spec governs it, our own design.';
COMMENT ON COLUMN binding_care_validity.effective_until IS 'The first date the relationship no longer holds: its start, or the commit''s date when it names none, plus the default period.';
COMMENT ON COLUMN binding_care_validity.policy_version IS 'The OBJECT_VERSION_ID of the ACCESS_POLICY version whose relationship_validity entry gave the period.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON binding_care_validity TO ferroehr_party;
        GRANT SELECT ON binding_care_validity TO ferroehr_party_reader;
    END IF;
END $$;
