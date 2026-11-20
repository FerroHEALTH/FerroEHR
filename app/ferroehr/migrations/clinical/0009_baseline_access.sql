-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the access decision's stores: the DPoP replay store, the
-- operational-policy registers, the controller's ACCESS_POLICY, the EHDS
-- Art. 3(3) access delay, the EHR_ACCESS projection with its settings classes,
-- the care projection, and the view of the revisions a query's access
-- condition was judged on.
--
-- No openEHR spec governs these stores: our own design/extension. The access
-- control package they realize is FerroEHR's draft, which is no openEHR
-- release.
--
-- Runs with search_path = clinical, ext, public.

-- ── dpop_proof ───────────────────────────────────────────────────────────────
--
-- RFC 9449 §11.1 asks a resource server to accept a DPoP proof once within the
-- window it accepts proofs in, and notes that single use "may not always be
-- feasible in practice, e.g., when multiple servers behind a single endpoint
-- have no shared state". Every replica of an instance shares the clinical
-- pool, so the record lives here: one row per accepted proof, keyed by the
-- base64url SHA-256 of its `jti` (43 characters, never the `jti` itself),
-- kept until the end of its window. A record past its end may be replaced
-- and is swept.
--
-- No openEHR spec governs authentication: our own design/extension.
CREATE TABLE dpop_proof (
    proof      text        NOT NULL,
    expires_at timestamptz NOT NULL,
    CONSTRAINT pk_dpop_proof PRIMARY KEY (proof),
    CONSTRAINT ck_dpop_proof_digest CHECK (char_length(proof) = 43)
);
-- The sweep reads the records whose window ended, oldest first.
CREATE INDEX idx_dpop_proof_expires ON dpop_proof (expires_at);
COMMENT ON TABLE dpop_proof IS
    'The DPoP proofs every replica accepted (RFC 9449 §11.1): the base64url SHA-256 of each proof''s jti and the end of the window it is accepted in. No request content.';

-- ── policy_register ──────────────────────────────────────────────────────────
-- The audit category map and the EHDS Art. 3(3) access delay are operational
-- policy with no home in the openEHR demographic model, so each is an admin
-- register: every write appends one version with
-- its committer, and the request path reads the newest version of each
-- register, so a change applies on the next request on every replica. A
-- register with no version holds its default.
--
-- No openEHR spec governs operational policy: our own design/extension.
CREATE TABLE policy_register (
    register     text        NOT NULL,
    version      bigint      NOT NULL,
    document     jsonb       NOT NULL,
    committer    text        NOT NULL,
    committed_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_policy_register PRIMARY KEY (register, version),
    CONSTRAINT ck_policy_register_name CHECK (
        register IN ('audit-categories', 'access-delay')
    ),
    CONSTRAINT ck_policy_register_version CHECK (version > 0)
);
COMMENT ON TABLE policy_register IS
    'The operational-policy admin registers: one row per version of the audit category map and the EHDS Art. 3(3) access delay, with its committer. Append-only; our own design/extension.';

-- ── access_policy_version ────────────────────────────────────────────────────
-- The controller's ACCESS_POLICY.
--
-- The access policy holds the facts that hold for every EHR the controller
-- keeps: the sensitivity of kinds of data, whether clinical access needs a
-- care relationship and a declared purpose, and the named directives a
-- subject can accept by reference. It is the one VERSIONED_ACCESS_POLICY of
-- the instance, kept outside every EHR and outside the demographic service.
-- Each row is one ORIGINAL_VERSION on the trunk, committed in its own
-- CONTRIBUTION with its AUDIT_DETAILS (RM common change_control, Common IM
-- master06 §Contributions and §Committal and Audits). Append-only.
--
-- ACCESS_POLICY is a class of FerroEHR's draft access control package, which
-- is no openEHR release, and ITS-REST defines no resource for it: our own
-- design/extension.
CREATE TABLE access_policy_version (
    version          integer     NOT NULL,
    vo_id            uuid        NOT NULL,
    system_id        text        NOT NULL,
    contribution_id  uuid        NOT NULL,
    commit_audit     jsonb       NOT NULL,
    time_committed   timestamptz NOT NULL,
    lifecycle_state  text        NOT NULL,
    data             jsonb       NOT NULL,
    CONSTRAINT pk_access_policy_version PRIMARY KEY (version),
    CONSTRAINT uq_access_policy_version_contribution UNIQUE (contribution_id),
    CONSTRAINT ck_access_policy_version_trunk CHECK (version > 0)
);
COMMENT ON TABLE access_policy_version IS
    'The controller ACCESS_POLICY: one ORIGINAL_VERSION per row on the trunk of the one VERSIONED_ACCESS_POLICY, each in its own CONTRIBUTION with its AUDIT_DETAILS. Append-only; our own design/extension.';
COMMENT ON COLUMN access_policy_version.vo_id IS 'VERSIONED_ACCESS_POLICY.uid: the same in every row.';
COMMENT ON COLUMN access_policy_version.commit_audit IS 'ORIGINAL_VERSION.commit_audit, the canonical AUDIT_DETAILS.';
COMMENT ON COLUMN access_policy_version.data IS 'ORIGINAL_VERSION.data, the canonical ACCESS_POLICY.';

-- ── access_delay ─────────────────────────────────────────────────────────────
-- The EHDS Art. 3(3) delay of the person's own access.
--
-- Regulation (EU) 2025/327 Art. 3(1) gives natural persons access to their
-- data "immediately after the personal electronic health data have been
-- registered in an EHR system", and Art. 3(3) lets a Member State restrict
-- that right "by delaying access to their personal electronic health data for
-- a limited period of time until a health professional is able to properly
-- communicate and explain to the natural persons concerned information that
-- can have a significant impact on their health" (docs/law/eu/ehds/text.html,
-- recital 10). This withholds data from the person, the opposite direction of
-- the Art. 8 restriction, which withholds it from professionals.
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
CREATE TRIGGER tg_access_class_notify
    AFTER INSERT OR DELETE ON access_class
    FOR EACH ROW EXECUTE FUNCTION access_class_notify();
COMMENT ON TABLE access_class IS 'The settings classes of the EHR_ACCESS projection: one row per distinct class, shared by every EHR whose settings make it. Our own design.';

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

-- ── access_settings ──────────────────────────────────────────────────────────
-- The projection of the access_directives settings of each EHR's current
-- EHR_ACCESS. EHR_ACCESS is the access-decision authority of an EHR (RM ehr
-- org.openehr.rm.ehr.ehr_access.adoc) and every change to it is a version in
-- a contribution (RM common change control). Each commit of an EHR_ACCESS
-- version writes, in its own transaction, the settings row the access gate
-- reads, with its settings class and the facts the class sets aside; a refill
-- rebuilds the rows from the current versions. A change to an EHR's settings
-- notifies every replica on ferroehr_ehr_access when it commits, so each
-- drops its copy of that EHR's settings. An EHR whose EHR_ACCESS carries no
-- settings has no row.
--
-- No openEHR spec governs the projection: our own design.
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
    -- The settings class the EHR's facts belong to; NULL when held closed.
    class_id       bigint,
    -- The access facts the settings read as; NULL when held closed.
    facts          jsonb,
    CONSTRAINT pk_access_settings PRIMARY KEY (ehr_id),
    CONSTRAINT fk_access_settings_vo FOREIGN KEY (vo_id)
        REFERENCES vo_head (vo_id) ON DELETE CASCADE,
    CONSTRAINT fk_access_settings_class FOREIGN KEY (class_id) REFERENCES access_class (id),
    CONSTRAINT ck_access_settings_class
        CHECK ((class_id IS NULL) = held_closed AND (facts IS NULL) = held_closed),
    CONSTRAINT ck_access_settings_held CHECK (held_closed = (settings IS NULL)),
    CONSTRAINT ck_access_settings_default
        CHECK (default_access IN ('open', 'restricted', 'consent_required'))
);
CREATE INDEX ix_access_settings_vo ON access_settings (vo_id);
CREATE INDEX ix_access_settings_class ON access_settings (class_id);
CREATE TRIGGER tg_access_settings_notify
    AFTER INSERT OR UPDATE OR DELETE ON access_settings
    FOR EACH ROW EXECUTE FUNCTION access_settings_notify();
COMMENT ON TABLE access_settings IS 'The access_directives settings of each EHR''s current EHR_ACCESS, written in the commit''s transaction; no row for an EHR whose EHR_ACCESS carries none. Our own design.';
COMMENT ON COLUMN access_settings.class_id IS 'The settings class of the EHR; NULL when the EHR is held closed.';
COMMENT ON COLUMN access_settings.facts IS 'The values of the EHR''s settings its class sets aside, as a JSON array in the order of a depth-first walk; NULL when the EHR is held closed.';

-- Whether the projection was filled from the stored versions: the first boot
-- fills it once.
CREATE TABLE access_projection_state (
    singleton boolean     NOT NULL DEFAULT true,
    filled_at timestamptz,
    CONSTRAINT pk_access_projection_state PRIMARY KEY (singleton),
    CONSTRAINT ck_access_projection_state_singleton CHECK (singleton)
);
INSERT INTO access_projection_state (singleton, filled_at) VALUES (true, NULL);
COMMENT ON TABLE access_projection_state IS 'When the access projection was last filled from the stored EHR_ACCESS versions; NULL until the first boot fills it. Our own design.';

CREATE TABLE access_care (
    -- The keyed digest of one subject reference that names the PERSON: its
    -- own reference in this CDR's namespace, or a record subject it declares.
    subject_digest bytea NOT NULL
        CONSTRAINT ck_access_care_subject_digest CHECK (octet_length(subject_digest) = 32),
    -- The PERSON under care, by its container.
    person_id      uuid  NOT NULL,
    -- The carer, by its container: a PERSON, a ROLE, a GROUP or an
    -- ORGANISATION.
    carer_id        uuid    NOT NULL,
    -- The first date the relationship holds; NULL when unbounded.
    care_from       date,
    -- The first date it no longer holds; NULL when unbounded.
    care_until      date,
    -- The provider DV_IDENTIFIER in whose care the relationship is.
    provider_issuer text    NOT NULL,
    provider_id     text    NOT NULL,
    -- The PERSON or client ACTOR that wrote the relationship, by its
    -- container; NULL when its committer names none.
    writer_id       uuid,
    -- Whether the relationship's basis is self_declared.
    self_declared   boolean NOT NULL DEFAULT false
);
CREATE INDEX ix_access_care_person ON access_care (person_id);
CREATE INDEX ix_access_care_subject ON access_care (subject_digest, carer_id);
COMMENT ON TABLE access_care IS 'The care relationships of the party domain, one row per subject reference and relationship, keyed by digest; written only by the care projector. Our own design.';
COMMENT ON COLUMN access_care.subject_digest IS 'The SHA-256 digest of one subject reference that names the PERSON: its own reference in this CDR''s namespace, or a record subject it declares. Our own design.';
COMMENT ON COLUMN access_care.carer_id IS 'The carer of the relationship, by its container: a PERSON, a ROLE, a GROUP or an ORGANISATION. Our own design.';
COMMENT ON COLUMN access_care.provider_issuer IS 'The issuer of the provider DV_IDENTIFIER in whose care the relationship is. Our own design.';
COMMENT ON COLUMN access_care.provider_id IS 'The id of the provider DV_IDENTIFIER in whose care the relationship is. Our own design.';
COMMENT ON COLUMN access_care.writer_id IS 'The PERSON or client ACTOR that wrote the relationship, by its container; NULL when its committer names none. Our own design.';
COMMENT ON COLUMN access_care.self_declared IS 'Whether the relationship''s basis is self_declared. Our own design.';

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
COMMENT ON COLUMN access_care_state.key_check IS 'The check value of the digest form the rows were taken in; a different value makes the projector rebuild them. Our own design.';

CREATE FUNCTION access_class_unknown(ehr uuid) RETURNS boolean
    LANGUAGE plpgsql STABLE PARALLEL SAFE
    AS $$
BEGIN
    RAISE EXCEPTION 'a query''s access condition does not hold the settings class of an EHR it reads'
        USING ERRCODE = 'ZA002';
END
$$;
COMMENT ON FUNCTION access_class_unknown(uuid) IS 'Raises SQLSTATE ZA002: a query reached an EHR whose settings class its access condition does not hold. Our own design.';

CREATE FUNCTION access_stale_copy() RETURNS boolean
    LANGUAGE plpgsql VOLATILE PARALLEL SAFE
    AS $$
BEGIN
    RAISE EXCEPTION 'the access copy a query was judged on is older than the store'
        USING ERRCODE = 'ZA001';
END
$$;
COMMENT ON FUNCTION access_stale_copy() IS 'Raises SQLSTATE ZA001: the settings or subject of a pinned EHR are no longer the revision its verdict was judged on. Our own design.';

-- The access revision of each EHR, read inside the statement that selects it
-- in a scalar subquery the planner merges with the view, so no read runs a
-- statement of its own per row: the JSON array [<ordinal>, <namespace>, <id>],
-- each null when absent. The party schema keeps a view of the same shape with
-- no rows, so a statement both domains run reads NULL there. The stale-copy
-- check of a query pinned to one EHR compares the revision the same way, and
-- calls access_stale_copy() only when it moved.
--
-- No openEHR spec governs the projection or its revision: our own design.
CREATE VIEW access_revision_of WITH (security_invoker = true) AS
SELECT e.id AS ehr_id,
       json_build_array(s.sys_version, e.subject_namespace, e.subject_id)::text AS revision
  FROM clinical.ehr e
  LEFT JOIN clinical.access_settings s ON s.ehr_id = e.id;
COMMENT ON VIEW access_revision_of IS 'The storage ordinal of the EHR_ACCESS version each EHR''s settings row was projected from and the EHR''s promoted subject_namespace and subject_id, as the JSON array [<ordinal>, <namespace>, <id>], each null when absent; read in a scalar subquery by the statement that selects it. Our own design.';
