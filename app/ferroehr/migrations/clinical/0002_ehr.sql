-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the EHR root and the subject pseudonym guard.
--
-- RM ehr master04-ehr_package.adoc §Root EHR Object: "The root EHR object
-- records three pieces of information that are immutable after creation: the
-- identifier of the system in which the EHR was created, the identifier of the
-- EHR ... and the time of creation of the EHR". The promoted status columns
-- beside them are our own storage design.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE ehr (
    id                uuid NOT NULL,
    -- The system that created this EHR, recorded at creation and never
    -- mutated (RM ehr master04 §Root EHR Object; §EHR Identifier Allocation:
    -- on a cloned EHR "the system_id is from the receiving (cloning)
    -- system"). Distinct from a version's creating_system_id.
    system_id         text NOT NULL,
    time_created      timestamptz NOT NULL DEFAULT now(),
    -- Promoted copy of the current EHR_STATUS subject.external_ref
    -- (id.value + namespace), kept in step by the service on every EHR_STATUS
    -- write. The partial unique index below is what refuses a second EHR for
    -- one subject at the database (ITS-REST 409_EHR.yaml).
    subject_id        text,
    subject_namespace text,
    -- Promoted copy of the current EHR_STATUS.is_queryable (RM ehr master04
    -- §EHR Status). The AQL full-population gate filters this column instead
    -- of probing every current EHR_STATUS root node per query: SM
    -- I_QUERY_SERVICE, with no ehr_ids supplied "a full population query will
    -- be performed on all EHRs whose status has the is_queryable flag set to
    -- True" (i_query_service.adoc). No index: the gate rides the primary key
    -- under ORDER BY id LIMIT n, and almost every EHR is queryable.
    is_queryable      boolean NOT NULL DEFAULT true,
    -- Promoted copy of the current EHR_STATUS.is_modifiable (RM ehr master04
    -- §EHR Active Status: is_modifiable "is used to indicate whether the
    -- contents of an EHR are modifiable"; "an EHR's 'contents' consist of
    -- everything other than the EHR_STATUS object"). The content-write guard
    -- reads this column rather than the current EHR_STATUS root node.
    is_modifiable     boolean NOT NULL DEFAULT true,
    -- Restriction of processing at whole-EHR grain: set while every object of
    -- this EHR may be stored but not otherwise processed. GDPR Art. 4(3) calls
    -- restriction "the marking of stored personal data with the aim of limiting
    -- their processing in the future" and Art. 18(2) leaves only storage
    -- (docs/law/eu/gdpr/text.html). The register that records the ground is
    -- `restriction`; this column and `vo_head.restricted_at` are the marks the
    -- read paths filter on. No openEHR spec governs restriction of processing:
    -- our own design/extension.
    restricted_at     timestamptz,
    -- The subject's objection to processing for research (GDPR Art. 21(6),
    -- docs/law/eu/gdpr/text.html: a right to object to processing "for
    -- scientific or historical research purposes or statistical purposes
    -- pursuant to Article 89(1)"). Set while the objection stands; the
    -- population query, every export and the outbox emitter skip the EHR. A
    -- single-EHR read for care is untouched — Art. 21(6) reaches research
    -- processing, not the care record. Our own design/extension.
    research_objected_at      timestamptz,
    -- The controller's recorded ground for overriding the objection, which
    -- Art. 21(6) admits where "the processing is necessary for the performance
    -- of a task carried out for reasons of public interest". NULL while the
    -- objection stands, so the mark is `research_objected_at IS NOT NULL AND
    -- research_objection_ground IS NULL` and the override is auditable rather
    -- than a silent clearing of the objection.
    research_objection_ground text,
    CONSTRAINT pk_ehr PRIMARY KEY (id),
    CONSTRAINT ck_ehr_objection_ground CHECK
        (research_objection_ground IS NULL OR research_objected_at IS NOT NULL)
) WITH (fillfactor = 90);

CREATE INDEX idx_ehr_time_created ON ehr (time_created DESC, id);

-- Subject uniqueness is wire-hard and RM-soft. The wire refuses a second EHR
-- for a subject (ITS-REST 409_EHR.yaml), while the RM explicitly tolerates it
-- — RM ehr master04 §EHR Identifier Allocation: "providers routinely create
-- new EHRs for a patient regardless of how many other EHRs already exist for
-- that patient". Enforced only where a complete (id, namespace) pair is
-- present.
CREATE UNIQUE INDEX uq_ehr_subject ON ehr (subject_id, subject_namespace)
    WHERE subject_id IS NOT NULL;

COMMENT ON TABLE ehr IS 'One row per EHR. system_id, id and time_created are the three values RM ehr master04-ehr_package.adoc §Root EHR Object makes immutable after creation.';
COMMENT ON COLUMN ehr.system_id IS 'The system that created this EHR, recorded at creation and never mutated (RM ehr master04 §Root EHR Object). A stored value, not the live service configuration.';
COMMENT ON COLUMN ehr.subject_id IS 'Denormalized copy of the current EHR_STATUS subject.external_ref.id.value; backs the one-EHR-per-subject unique index (ITS-REST 409_EHR.yaml). Our own storage design.';
COMMENT ON COLUMN ehr.subject_namespace IS 'Denormalized copy of the current EHR_STATUS subject.external_ref.namespace.';
COMMENT ON COLUMN ehr.is_queryable IS 'Promoted copy of the current EHR_STATUS.is_queryable (RM ehr master04 §EHR Status); backs the AQL full-population gate (SM i_query_service.adoc). Our own storage design.';
COMMENT ON COLUMN ehr.is_modifiable IS 'Promoted copy of the current EHR_STATUS.is_modifiable (RM ehr master04 §EHR Active Status); backs the content-write guard. Our own storage design.';
COMMENT ON COLUMN ehr.restricted_at IS 'When restriction of processing was recorded for the whole EHR (GDPR Art. 4(3), Art. 18(2)); NULL = unrestricted. Our own design/extension — no openEHR spec governs restriction.';
COMMENT ON COLUMN ehr.research_objected_at IS 'When the subject objected to research processing (GDPR Art. 21(6)); while set and unoverridden, population AQL, every export and the outbox emitter skip this EHR. Our own design/extension.';
COMMENT ON COLUMN ehr.research_objection_ground IS 'The controller''s recorded public-interest ground for overriding the objection (GDPR Art. 21(6)); NULL while the objection stands.';

-- No index on either mark, deliberately. Every reader either has the row
-- already (the AQL population gate, which filters the UNMARKED majority on
-- rows its scan has reached) or reaches it by primary key (the mark read, the
-- outbox emitter and the drainer's NOT EXISTS). A partial index over the
-- marked minority would serve none of them.

-- ── the subject pseudonym guard ──────────────────────────────────────────────
-- A deployment that declares subject namespaces holds an OPAQUE pseudonym on
-- the clinical side — never a national identifier, a medical-record number or
-- a name — and this trigger is what refuses anything else at the database
-- rather than trusting every write path. The posture it reads is stamped by
-- the server at boot into ext.posture.
--
-- GDPR Art. 4(5) (https://eur-lex.europa.eu/eli/reg/2016/679/oj). No openEHR
-- spec governs the pseudonym form: our own design/extension.
CREATE FUNCTION subject_pseudonym_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    required text;
BEGIN
    IF NEW.subject_id IS NULL THEN
        RETURN NEW;
    END IF;
    SELECT value INTO required FROM ext.posture WHERE key = 'subject_pseudonyms';
    IF required = 'required'
       AND NEW.subject_id !~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
    THEN
        RAISE EXCEPTION USING
            ERRCODE = 'check_violation',
            MESSAGE = 'ehr.subject_id must be a UUID: this deployment declares privacy.subject_namespaces, so the clinical side holds an opaque subject pseudonym, never a national identifier, a medical-record number or a name',
            CONSTRAINT = 'ehr_subject_pseudonym_guard';
    END IF;
    RETURN NEW;
END $$;

COMMENT ON FUNCTION subject_pseudonym_guard() IS 'Refuses a non-UUID ehr.subject_id when the deployment declares subject namespaces (ext.posture key subject_pseudonyms). GDPR Art. 4(5); our own design/extension.';

CREATE TRIGGER ehr_subject_pseudonym_guard
    BEFORE INSERT OR UPDATE OF subject_id, subject_namespace ON ehr
    FOR EACH ROW EXECUTE FUNCTION subject_pseudonym_guard();

REVOKE ALL ON FUNCTION subject_pseudonym_guard() FROM PUBLIC;
