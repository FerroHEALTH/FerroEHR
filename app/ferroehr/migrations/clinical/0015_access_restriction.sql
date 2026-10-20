-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the EHDS Art. 8 restriction of access by health professionals.
--
-- Regulation (EU) 2025/327 Art. 8 gives natural persons "the right to restrict
-- the access of health professionals and healthcare providers to all or parts
-- of their personal electronic health data", and has the fact of a
-- restriction stay invisible to healthcare providers; Art. 11(5) lets access
-- to the restricted data be granted where necessary to protect the vital
-- interests of the person (docs/law/eu/ehds/text.html). This is a limit on
-- access only, never on storage or other processing, so it is a register and
-- marks of its own, apart from the GDPR Art. 18 `restriction` of
-- 0007_restriction_and_retention.sql.
--
-- `access_restriction` is the register: when the person's instruction was
-- recorded and lifted, at whole-EHR or versioned-object grain.
-- `ehr.access_restricted_at` and `vo_head.access_restricted_at` are the
-- denormalised marks the read paths filter on, recomputed from the register by
-- `crate::storage::marks::refresh_access_restriction_marks`. A versioned object
-- created in an EHR that carries the whole-EHR mark takes the mark with it, so
-- the restriction covers data recorded after it as well.
--
-- No openEHR spec governs restriction of access: our own design/extension.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE access_restriction (
    id           uuid NOT NULL DEFAULT uuidv7(),
    ehr_id       uuid NOT NULL,
    -- NULL restricts the whole EHR; otherwise the one versioned object.
    vo_id        uuid,
    requested_at timestamptz NOT NULL DEFAULT now(),
    lifted_at    timestamptz,
    note         text,
    CONSTRAINT pk_access_restriction PRIMARY KEY (id),
    CONSTRAINT ck_access_restriction_lifted_after CHECK
        (lifted_at IS NULL OR lifted_at >= requested_at),
    CONSTRAINT fk_access_restriction_ehr FOREIGN KEY (ehr_id)
        REFERENCES ehr (id) ON DELETE CASCADE
);

CREATE INDEX idx_access_restriction_in_force ON access_restriction (ehr_id, vo_id)
    WHERE lifted_at IS NULL;

COMMENT ON TABLE access_restriction IS 'The register behind an EHDS Art. 8 access-restriction mark: the person''s instruction, when it was recorded and when it was lifted. Our own design/extension — no openEHR spec governs restriction of access.';
COMMENT ON COLUMN access_restriction.vo_id IS 'The restricted versioned object, or NULL for the whole EHR.';

ALTER TABLE ehr ADD COLUMN access_restricted_at timestamptz;
ALTER TABLE vo_head ADD COLUMN access_restricted_at timestamptz;

COMMENT ON COLUMN ehr.access_restricted_at IS 'When an EHDS Art. 8 restriction of access by health professionals was recorded for the whole EHR; NULL = unrestricted. Our own design/extension.';
COMMENT ON COLUMN vo_head.access_restricted_at IS 'When an EHDS Art. 8 restriction of access by health professionals was recorded for this object, or for its whole EHR; NULL = unrestricted. Our own design/extension.';

-- A new versioned object of a whole-EHR-restricted EHR is restricted from its
-- first version.
CREATE FUNCTION vo_head_inherits_access_restriction() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.access_restricted_at IS NULL AND NEW.ehr_id IS NOT NULL THEN
        SELECT e.access_restricted_at INTO NEW.access_restricted_at
          FROM ehr e WHERE e.id = NEW.ehr_id;
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER trg_vo_head_access_restriction
    BEFORE INSERT ON vo_head
    FOR EACH ROW EXECUTE FUNCTION vo_head_inherits_access_restriction();

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON access_restriction TO ferroehr_clinical;
        GRANT SELECT ON access_restriction TO ferroehr_clinical_reader;
        REVOKE ALL ON access_restriction
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
        GRANT EXECUTE ON FUNCTION vo_head_inherits_access_restriction()
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping access restriction grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
