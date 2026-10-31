-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the EHDS Art. 3(3) delay of the person's own access.
--
-- Regulation (EU) 2025/327 Art. 3(1) gives natural persons access to their
-- data "immediately after the personal electronic health data have been
-- registered in an EHR system", and Art. 3(3) lets a Member State restrict
-- that right "by delaying access to their personal electronic health data for
-- a limited period of time until a health professional is able to properly
-- communicate and explain to the natural persons concerned information that
-- can have a significant impact on their health" (docs/law/eu/ehds/text.html,
-- recital 10). This withholds data from the person, the opposite direction of
-- the Art. 8 restriction of 0015_access_restriction.sql, which withholds it
-- from professionals.
--
-- `access_delay` is the register: one row per hold on one versioned object,
-- the instant it ends, and when a professional released it early. Every hold
-- ends (Art. 3(3) "for a limited period of time"), so `withheld_until` is
-- never NULL. `vo_head.person_withheld_until` is the denormalised mark the
-- patient-context read paths filter on: the latest end of the holds not
-- released, recomputed by `crate::storage::access_delay`. A mark in the past
-- withholds nothing, so a hold expires without a write; `expiry_recorded_at`
-- is when the access record of that expiry was written.
--
-- No openEHR spec governs a delay of the person's access: our own
-- design/extension.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE access_delay (
    id                 uuid NOT NULL DEFAULT uuidv7(),
    ehr_id             uuid NOT NULL,
    vo_id              uuid NOT NULL,
    -- `category`: the deployment's configured category held it at creation;
    -- `professional`: a health professional held it.
    basis              text NOT NULL,
    withheld_at        timestamptz NOT NULL DEFAULT now(),
    withheld_until     timestamptz NOT NULL,
    released_at        timestamptz,
    expiry_recorded_at timestamptz,
    note               text,
    CONSTRAINT pk_access_delay PRIMARY KEY (id),
    CONSTRAINT ck_access_delay_basis CHECK (basis IN ('category', 'professional')),
    CONSTRAINT ck_access_delay_until_after CHECK (withheld_until > withheld_at),
    CONSTRAINT ck_access_delay_released_after CHECK
        (released_at IS NULL OR released_at >= withheld_at),
    CONSTRAINT fk_access_delay_ehr FOREIGN KEY (ehr_id)
        REFERENCES ehr (id) ON DELETE CASCADE
);

CREATE INDEX idx_access_delay_object ON access_delay (vo_id)
    WHERE released_at IS NULL;
CREATE INDEX idx_access_delay_ehr ON access_delay (ehr_id, withheld_at);
CREATE INDEX idx_access_delay_unrecorded_expiry ON access_delay (withheld_until)
    WHERE released_at IS NULL AND expiry_recorded_at IS NULL;

COMMENT ON TABLE access_delay IS 'The register behind an EHDS Art. 3(3) delay of the person''s own access: when each hold began, when it ends, and when a professional released it. Our own design/extension; no openEHR spec governs it.';
COMMENT ON COLUMN access_delay.withheld_until IS 'When the hold ends by itself: Art. 3(3) allows a delay "for a limited period of time" only.';
COMMENT ON COLUMN access_delay.expiry_recorded_at IS 'When the access record of the hold''s expiry was written; NULL while it is in force, released, or not yet recorded.';

ALTER TABLE vo_head ADD COLUMN person_withheld_until timestamptz;

COMMENT ON COLUMN vo_head.person_withheld_until IS 'Until when an EHDS Art. 3(3) delay withholds this object from the person''s own access; NULL or past = not withheld. Our own design/extension.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON access_delay TO ferroehr_clinical;
        GRANT SELECT ON access_delay TO ferroehr_clinical_reader;
        REVOKE ALL ON access_delay
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping access delay grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
