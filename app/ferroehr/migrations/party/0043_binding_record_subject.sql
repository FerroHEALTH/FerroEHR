-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the EHR subject references each patient PERSON stands for
-- (#5634), so the access evaluator finds the PERSON of an EHR whose
-- EHR_STATUS subject is a pseudonym of an identity source, such as FerroPIX
-- under record.subject_index = "pixm".
--
-- A PERSON declares a reference by an ELEMENT anywhere in its PARTY.details
-- named by the code record_subject of FerroEHR's operation terminology,
-- whose value is a DV_IDENTIFIER: issuer is the
-- reference's namespace (EHR_STATUS.subject.external_ref.namespace) and id
-- the reference's id. An identifier another organisation issued is recorded
-- in PARTY.details (RM Demographic IM §2.1.3), and the cross-reference stays
-- out of the EHR (RM EHR IM §EHR Status). The rows are derived from the party
-- versions in each commit's own transaction, beside the caller facts
-- (0039_caller_facts.sql), and rebuilt by a refill. Reading the element as
-- the link from an EHR to its subject's PERSON is governed by no openEHR
-- spec: our own design.
--
-- Runs with search_path = party, ext, public.

-- ── binding_record_subject ──────────────────────────────────────────────────
-- One row per reference a PERSON head that is not deleted declares.
CREATE TABLE binding_record_subject (
    -- The PERSON container whose head declares the reference.
    holder_id       uuid NOT NULL,
    -- The element's position among the record_subject elements of the
    -- PERSON's details, depth first, from 0.
    ordinal         integer NOT NULL,
    -- The DV_IDENTIFIER issuer: the reference's namespace.
    namespace       text NOT NULL,
    -- The DV_IDENTIFIER id: the reference's id, as written.
    subject_id      text NOT NULL,
    -- The holder head's version_lifecycle_state.
    lifecycle_state text NOT NULL,
    written_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_binding_record_subject PRIMARY KEY (holder_id, ordinal),
    -- One reference names one PERSON. Deferred to the end of the transaction,
    -- so a contribution that moves a reference from one PERSON to another,
    -- such as after an identity merge, commits its versions in any order; the
    -- commit sets it IMMEDIATE before it ends, which reports a collision.
    CONSTRAINT uq_binding_record_subject UNIQUE (namespace, subject_id)
        DEFERRABLE INITIALLY DEFERRED,
    CONSTRAINT ck_binding_record_subject_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);

COMMENT ON TABLE binding_record_subject IS 'The EHR subject references each PERSON head that is not deleted declares by a record_subject element of its details, a code of FerroEHR''s operation terminology. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as the link from an EHR to its subject''s PERSON, our own design.';
