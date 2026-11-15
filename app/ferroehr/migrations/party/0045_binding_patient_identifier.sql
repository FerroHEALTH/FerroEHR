-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the identifiers each patient PERSON carries (#5648), so a contact
-- point's treatment relationship confirmation, which names the patient by an
-- identifier of another domain (eHDSI XCA Profile 8.0.1 §4.2.1), finds the
-- patient's PERSON, and through its record subjects its EHRs, in the party
-- domain with no call to an identity source on the request path.
--
-- The rows are the DV_IDENTIFIERs at the [authz.party_binding.patient]
-- identifier path of a PERSON of the patient template, each with an issuer:
-- identifiers issued by other organisations or the state are recorded in
-- PARTY.details (RM Demographic IM §2.1.3), and the issuer is the
-- identifier's domain. A protected national identifier is held as the
-- reference the body carries (national_identifier), and found through its
-- lookup digest. The rows are derived from the party versions in each
-- commit's own transaction, beside the record subjects
-- (0043_binding_record_subject.sql), and rebuilt by a refill. Reading them as
-- the way to a patient's EHRs is governed by no openEHR spec: our own design.
--
-- Runs with search_path = party, ext, public.

-- ── binding_patient_identifier ──────────────────────────────────────────────
-- One row per identifier a patient PERSON head that is not deleted carries.
CREATE TABLE binding_patient_identifier (
    -- The PERSON container whose head carries the identifier.
    holder_id       uuid NOT NULL,
    -- The identifier's position among the PERSON's patient identifiers, in
    -- document order, from 0.
    ordinal         integer NOT NULL,
    -- The DV_IDENTIFIER issuer: the identifier's domain.
    issuer          text NOT NULL,
    -- The DV_IDENTIFIER type, empty when the identifier names none.
    identifier_type text NOT NULL,
    -- The DV_IDENTIFIER id as the stored body carries it: the value, or the
    -- reference to its sealed row for a protected scheme.
    value           text NOT NULL,
    -- The holder head's version_lifecycle_state.
    lifecycle_state text NOT NULL,
    written_at      timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_binding_patient_identifier PRIMARY KEY (holder_id, ordinal),
    CONSTRAINT ck_binding_patient_identifier_lifecycle_state
        CHECK (lifecycle_state IN ('532', '553', '800', '801'))
);

-- The lookup by issuer and value; a protected identifier is reached from
-- national_identifier's digest index and the primary key instead.
CREATE INDEX ix_binding_patient_identifier_value
    ON binding_patient_identifier (issuer, value) INCLUDE (holder_id, lifecycle_state);

COMMENT ON TABLE binding_patient_identifier IS 'The identifiers each patient PERSON head that is not deleted carries at the [authz.party_binding.patient] identifier path, by issuer. Derived from the party versions in each commit''s transaction; no openEHR spec governs reading them as the way to a patient''s EHRs, our own design.';
