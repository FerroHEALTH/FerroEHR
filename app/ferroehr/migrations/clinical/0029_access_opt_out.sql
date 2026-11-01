-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the EHDS Art. 10 opt-out from access through the access services.
--
-- Regulation (EU) 2025/327 Art. 10(1) lets a Member State's law give natural
-- persons "the right to opt out from the access to their personal electronic
-- health data registered in an EHR system through the electronic health data
-- access services referred to in Articles 4 and 12", and has the Member State
-- "ensure that the exercise of that right is reversible"; Art. 10(2) lets a
-- healthcare provider or health professional reach the data where processing
-- is necessary to protect vital interests (docs/law/eu/ehds/text.html). It
-- limits access through those services only, never storage or the provider's
-- own care, so it is a register and a mark of its own, apart from the Art. 8
-- `access_restriction` of 0015_access_restriction.sql.
--
-- `access_opt_out` is the register: when the person's opt-out was recorded
-- and when it was reversed, always for the whole EHR. A reversal writes
-- `reversed_at` on the row, so the history survives. `ehr.opted_out_at` is the
-- denormalised mark the read paths filter on, recomputed from the register by
-- `crate::storage::opt_out::refresh_mark`.
--
-- No openEHR spec governs an opt-out: our own design/extension.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE access_opt_out (
    id           uuid NOT NULL DEFAULT uuidv7(),
    ehr_id       uuid NOT NULL,
    opted_out_at timestamptz NOT NULL DEFAULT now(),
    reversed_at  timestamptz,
    note         text,
    CONSTRAINT pk_access_opt_out PRIMARY KEY (id),
    CONSTRAINT ck_access_opt_out_reversed_after CHECK
        (reversed_at IS NULL OR reversed_at >= opted_out_at),
    CONSTRAINT fk_access_opt_out_ehr FOREIGN KEY (ehr_id)
        REFERENCES ehr (id) ON DELETE CASCADE
);

CREATE INDEX idx_access_opt_out_in_force ON access_opt_out (ehr_id)
    WHERE reversed_at IS NULL;
CREATE INDEX idx_access_opt_out_ehr ON access_opt_out (ehr_id, opted_out_at);

COMMENT ON TABLE access_opt_out IS 'The register behind an EHDS Art. 10 opt-out from access through the access services of Art. 4 and Art. 12: when the person''s opt-out was recorded and when it was reversed. Our own design/extension; no openEHR spec governs it.';
COMMENT ON COLUMN access_opt_out.reversed_at IS 'When the opt-out was reversed (Art. 10(1): the exercise of the right is reversible); NULL while it stands.';

ALTER TABLE ehr ADD COLUMN opted_out_at timestamptz;

COMMENT ON COLUMN ehr.opted_out_at IS 'When an EHDS Art. 10 opt-out from access through the access services was recorded for this EHR; NULL = no opt-out stands. Our own design/extension.';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON access_opt_out TO ferroehr_clinical;
        GRANT SELECT ON access_opt_out TO ferroehr_clinical_reader;
        REVOKE ALL ON access_opt_out
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping access opt-out grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
