-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the projection of the access_directives settings of each EHR's
-- current EHR_ACCESS (#5509).
--
-- EHR_ACCESS is the access-decision authority of an EHR (RM ehr
-- org.openehr.rm.ehr.ehr_access.adoc) and every change to it is a version in
-- a contribution (RM common change control). Each commit of an EHR_ACCESS
-- version writes, in its own transaction, the settings the access gate reads,
-- one row per directive and exception with the facts the access evaluator
-- matches on, and one row per sensitivity label; a refill rebuilds all three
-- from the current versions. A change to an EHR's settings notifies every
-- replica on ferroehr_ehr_access when it commits, so each drops its copy of
-- that EHR's settings. An EHR whose EHR_ACCESS carries no settings has no row.
--
-- No openEHR spec governs the projection: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE access_settings (
    ehr_id         uuid    NOT NULL,
    -- The EHR_ACCESS versioned object and the storage ordinal of its trunk
    -- head the rows were projected from.
    vo_id          uuid    NOT NULL,
    sys_version    integer NOT NULL,
    -- The stored settings do not read as the access_directives scheme, so the
    -- EHR is held closed and no directive or label row is written.
    held_closed    boolean NOT NULL,
    -- The access default code; restricted for an EHR held closed.
    default_access text    NOT NULL,
    -- The ACCESS_DIRECTIVE_SETTINGS in canonical JSON; NULL when held closed.
    settings       jsonb,
    CONSTRAINT pk_access_settings PRIMARY KEY (ehr_id),
    CONSTRAINT fk_access_settings_vo FOREIGN KEY (vo_id)
        REFERENCES vo_head (vo_id) ON DELETE CASCADE,
    CONSTRAINT ck_access_settings_held CHECK (held_closed = (settings IS NULL)),
    CONSTRAINT ck_access_settings_default
        CHECK (default_access IN ('open', 'restricted', 'consent_required'))
);

CREATE INDEX ix_access_settings_vo ON access_settings (vo_id);

COMMENT ON TABLE access_settings IS 'The access_directives settings of each EHR''s current EHR_ACCESS, written in the commit''s transaction; no row for an EHR whose EHR_ACCESS carries none. Our own design.';

-- One row per ACCESS_DIRECTIVE and per exception, in depth-first order. The
-- rows of an EHR are replaced whole in one statement, so they carry no key
-- of their own.
CREATE TABLE access_directive (
    ehr_id             uuid      NOT NULL,
    vo_id              uuid      NOT NULL,
    -- The directive's position in a depth-first walk of the settings, from 0.
    ordinal            integer   NOT NULL,
    -- The ordinal of the directive this one is an exception of.
    parent             integer,
    -- The access directive kind and access effect codes.
    kind               text      NOT NULL,
    effect             text      NOT NULL,
    emergency_override boolean   NOT NULL,
    -- The applies_to scopes: party id values, roles as terminology::code,
    -- and the access relationship and access caller kind codes.
    parties            text[]    NOT NULL,
    roles              text[]    NOT NULL,
    relationships      text[]    NOT NULL,
    caller_kinds       text[]    NOT NULL,
    -- The access operation codes.
    operations         text[]    NOT NULL,
    -- The purposes, and the categories and sensitivities of the data scopes,
    -- as terminology::code of the jurisdiction's terminology.
    purposes           text[]    NOT NULL,
    categories         text[]    NOT NULL,
    sensitivities      text[]    NOT NULL,
    -- The instants time_validity holds; unbounded when absent, NULL when a
    -- bound is not a full date and time with an offset.
    validity           tstzrange,
    CONSTRAINT fk_access_directive_vo FOREIGN KEY (vo_id)
        REFERENCES vo_head (vo_id) ON DELETE CASCADE,
    CONSTRAINT ck_access_directive_kind
        CHECK (kind IN ('consent', 'restriction', 'opt_out', 'delegation')),
    CONSTRAINT ck_access_directive_effect CHECK (effect IN ('permit', 'deny'))
);

CREATE INDEX ix_access_directive_ehr ON access_directive (ehr_id);
CREATE INDEX ix_access_directive_vo ON access_directive (vo_id);
CREATE INDEX ix_access_directive_kind ON access_directive (kind, effect);
CREATE INDEX ix_access_directive_parties ON access_directive USING gin (parties);
CREATE INDEX ix_access_directive_roles ON access_directive USING gin (roles);
CREATE INDEX ix_access_directive_relationships ON access_directive USING gin (relationships);
CREATE INDEX ix_access_directive_categories ON access_directive USING gin (categories);
CREATE INDEX ix_access_directive_sensitivities ON access_directive USING gin (sensitivities);
CREATE INDEX ix_access_directive_validity ON access_directive USING gist (validity);

COMMENT ON TABLE access_directive IS 'One row per ACCESS_DIRECTIVE and exception of each EHR''s current EHR_ACCESS settings, with the facts the access evaluator matches on. Our own design.';
COMMENT ON COLUMN access_directive.validity IS 'The instants time_validity holds: unbounded when absent, NULL when a bound cannot be read as an instant, which the evaluator never reads as permissive.';

-- One row per ACCESS_LABEL.
CREATE TABLE access_label (
    ehr_id           uuid    NOT NULL,
    vo_id            uuid    NOT NULL,
    ordinal          integer NOT NULL,
    -- The labelled object: its OBJECT_REF namespace, type and id value.
    target_namespace text    NOT NULL,
    target_type      text    NOT NULL,
    target_id        text    NOT NULL,
    -- The sensitivities, as terminology::code.
    sensitivities    text[]  NOT NULL,
    CONSTRAINT fk_access_label_vo FOREIGN KEY (vo_id)
        REFERENCES vo_head (vo_id) ON DELETE CASCADE
);

CREATE INDEX ix_access_label_ehr ON access_label (ehr_id);
CREATE INDEX ix_access_label_vo ON access_label (vo_id);
CREATE INDEX ix_access_label_target ON access_label (target_id);
CREATE INDEX ix_access_label_sensitivities ON access_label USING gin (sensitivities);

COMMENT ON TABLE access_label IS 'One row per ACCESS_LABEL of each EHR''s current EHR_ACCESS settings. Our own design.';

-- Whether the projection was filled from the stored versions: the first boot
-- after this migration fills it once.
CREATE TABLE access_projection_state (
    singleton boolean     NOT NULL DEFAULT true,
    filled_at timestamptz,
    CONSTRAINT pk_access_projection_state PRIMARY KEY (singleton),
    CONSTRAINT ck_access_projection_state_singleton CHECK (singleton)
);

INSERT INTO access_projection_state (singleton, filled_at) VALUES (true, NULL);

COMMENT ON TABLE access_projection_state IS 'When the access projection was last filled from the stored EHR_ACCESS versions; NULL until the first boot fills it. Our own design.';

-- A change to an EHR's settings row, the projection's write or the cascade of
-- an erasure, notifies every replica at commit (PostgreSQL 18, NOTIFY).
CREATE FUNCTION access_settings_notify() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM pg_notify('ferroehr_ehr_access', OLD.ehr_id::text);
    ELSE
        PERFORM pg_notify('ferroehr_ehr_access', NEW.ehr_id::text);
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER tg_access_settings_notify
    AFTER INSERT OR UPDATE OR DELETE ON access_settings
    FOR EACH ROW EXECUTE FUNCTION access_settings_notify();

-- PUBLIC holds EXECUTE on a new function by default (PostgreSQL 18, GRANT,
-- "Notes"); only the trigger calls it, so no role of any domain reaches it.
REVOKE ALL ON FUNCTION access_settings_notify() FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE
            ON access_settings, access_directive, access_label TO ferroehr_clinical;
        GRANT SELECT, UPDATE ON access_projection_state TO ferroehr_clinical;
        REVOKE ALL ON access_settings, access_directive, access_label, access_projection_state
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping the access projection grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
