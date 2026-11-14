-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: settings classes and facts in place of the directive and label
-- rows of the EHR_ACCESS projection (#5588).
--
-- A query is judged over every EHR it could reach. Each EHR's settings are
-- stored as a class, the settings with every value particular to the EHR set
-- aside, and the facts those values are, so a replica holds the classes and a
-- query reads each EHR's facts in SQL. Many EHRs share one class, so what a
-- replica holds does not grow with the EHRs that carry settings. The
-- directive and label rows had no reader and go. A new class notifies every
-- replica on ferroehr_ehr_access with the payload "class".
--
-- Every existing projection row is deleted and the projection is marked
-- unfilled, so the next boot rebuilds it from the stored EHR_ACCESS versions
-- under the new shape.
--
-- No openEHR spec governs the projection: our own design.
--
-- Runs with search_path = clinical, ext, public.

DROP TABLE access_directive;
DROP TABLE access_label;

DELETE FROM access_settings;
UPDATE access_projection_state SET filled_at = NULL WHERE singleton;

CREATE TABLE access_class (
    id       bigint GENERATED ALWAYS AS IDENTITY,
    -- The SHA-256 of the class's JSON text.
    digest   bytea NOT NULL,
    -- The settings class: ACCESS_DIRECTIVE_SETTINGS in canonical JSON, with
    -- what the decision does not read left out and every value particular
    -- to an EHR replaced by a fixed one.
    settings jsonb NOT NULL,
    CONSTRAINT pk_access_class PRIMARY KEY (id),
    CONSTRAINT uq_access_class_digest UNIQUE (digest)
);

COMMENT ON TABLE access_class IS 'The settings classes of the EHR_ACCESS projection: one row per distinct class, shared by every EHR whose settings make it. Our own design.';

ALTER TABLE access_settings ADD COLUMN class_id bigint;
ALTER TABLE access_settings ADD COLUMN facts jsonb;
ALTER TABLE access_settings
    ADD CONSTRAINT fk_access_settings_class FOREIGN KEY (class_id) REFERENCES access_class (id);
ALTER TABLE access_settings
    ADD CONSTRAINT ck_access_settings_class
    CHECK ((class_id IS NULL) = held_closed AND (facts IS NULL) = held_closed);

CREATE INDEX ix_access_settings_class ON access_settings (class_id);

COMMENT ON COLUMN access_settings.class_id IS 'The settings class of the EHR; NULL when the EHR is held closed.';
COMMENT ON COLUMN access_settings.facts IS 'The values of the EHR''s settings its class sets aside, as a JSON array in the order of a depth-first walk; NULL when the EHR is held closed.';

-- A new class notifies every replica at commit (PostgreSQL 18, NOTIFY), so
-- each copies it before a query reaches an EHR of it.
CREATE FUNCTION access_class_notify() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog
    AS $$
BEGIN
    PERFORM pg_notify('ferroehr_ehr_access', 'class');
    RETURN NULL;
END $$;

CREATE TRIGGER tg_access_class_notify
    AFTER INSERT OR DELETE ON access_class
    FOR EACH ROW EXECUTE FUNCTION access_class_notify();

-- PUBLIC holds EXECUTE on a new function by default (PostgreSQL 18, GRANT,
-- "Notes"); only the trigger calls it, so no role of any domain reaches it.
REVOKE ALL ON FUNCTION access_class_notify() FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, DELETE ON access_class TO ferroehr_clinical;
        REVOKE ALL ON access_class FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping the access class grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
