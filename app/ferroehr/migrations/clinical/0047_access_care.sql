-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the care projection the access condition reads (#5699).
--
-- A query under the access evaluator asks, for each EHR it reads, whether the
-- caller's care teams care for the EHR's subject. The care relationships are
-- facts of the party domain (binding_caller_fact, kind under_care_of), which
-- may live in another database, so a statement cannot join them. Until now
-- every statement bound the subjects of every patient under the caller's
-- teams; this table holds the same facts once, so a statement binds only the
-- caller's team ids and probes one index per EHR row.
--
-- The projector of each replica reads the party side through the party pool
-- and writes here through the clinical pool, so no role reaches both domains.
-- A row holds no name and no identifier: the PERSON and team container UUIDs,
-- the care dates, and the keyed digest of a subject reference that names the
-- PERSON (HMAC-SHA-256 under the clinical lookup subkey of the deployment's
-- [privacy.identifier_protection] key, which never reaches the database). The
-- EHR row carries the keyed digest of its own subject reference, written by
-- the server with the subject. Erasing a PERSON deletes its rows in the same
-- flow, and a read uses the projection only while it is as recent as the
-- facts the reading replica holds, so a removal is never read from a stale
-- row.
--
-- No openEHR spec governs the projection: our own design.
--
-- Runs with search_path = clinical, ext, public.

ALTER TABLE ehr ADD COLUMN subject_digest bytea
    CONSTRAINT ck_ehr_subject_digest CHECK (octet_length(subject_digest) = 32);

COMMENT ON COLUMN ehr.subject_digest IS 'The keyed digest of the subject reference (subject_namespace and subject_id) the care projection is matched by; NULL without a subject or while identifier protection is off. Our own design.';

CREATE TABLE access_care (
    -- The keyed digest of one subject reference that names the PERSON: its
    -- own reference in this CDR's namespace, or a record subject it declares.
    subject_digest bytea NOT NULL
        CONSTRAINT ck_access_care_subject_digest CHECK (octet_length(subject_digest) = 32),
    -- The PERSON under care, by its container.
    person_id      uuid  NOT NULL,
    -- The care team GROUP or organisational unit, by its container.
    team_id        uuid  NOT NULL,
    -- The first date the relationship holds; NULL when unbounded.
    care_from      date,
    -- The first date it no longer holds; NULL when unbounded.
    care_until     date
);

CREATE INDEX ix_access_care_subject ON access_care (subject_digest, team_id);
CREATE INDEX ix_access_care_person ON access_care (person_id);

COMMENT ON TABLE access_care IS 'The care relationships of the party domain, one row per subject reference and relationship, keyed by digest; written only by the care projector. Our own design.';

-- The facts revision of the party domain the rows reflect, and the check
-- digest of the key they were digested under; NULL before the first run.
CREATE TABLE access_care_state (
    singleton boolean     NOT NULL DEFAULT true,
    revision  bigint,
    key_check bytea,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_access_care_state PRIMARY KEY (singleton),
    CONSTRAINT ck_access_care_state_singleton CHECK (singleton)
);

INSERT INTO access_care_state (singleton) VALUES (true);

COMMENT ON TABLE access_care_state IS 'The party facts revision the care projection reflects and the check digest of its key. Our own design.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON access_care TO ferroehr_clinical;
        GRANT SELECT, UPDATE ON access_care_state TO ferroehr_clinical;
        REVOKE ALL ON access_care, access_care_state
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping the care projection grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
