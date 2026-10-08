-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: how the accessing person authenticated, and who that person is.
--
-- Annex II 3.1 of Regulation (EU) 2025/327 asks an EHR system used by health
-- professionals for "reliable mechanisms for the identification and
-- authentication of health professionals", and Annex II 3.2(b) asks the access
-- record for "identification of the specific natural person or persons having
-- accessed the personal electronic health data" (docs/law/eu/ehds/text.html).
-- A bearer token may name a client application rather than a person, so the
-- record says which it was, which professional a client acted for, and the
-- authentication assurance level the token carried.
--
-- NULL means "not established": a deployment that configures no
-- [auth.oidc.professional] records no mode and no professional, one that maps
-- no assurance values records no level, and every record written before this
-- migration carries none of the four.
--
-- Append-only needs no new machinery: the BEFORE UPDATE trigger the tamper
-- chain installs compares the whole row minus the two delivery stamps. The
-- chain digest covers the FHIR document, which carries the mode and the
-- professional as agents; the level has no FHIR R4 AuditEvent element, so it
-- lives in these columns only. The table-level grants of the audit grants file
-- cover new columns.
--
-- No openEHR spec governs access-log content: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event
    ADD COLUMN acting_mode      text,
    ADD COLUMN assurance_level  text,
    ADD COLUMN assurance_value  text,
    ADD COLUMN professional_id  text;

ALTER TABLE audit_event
    ADD CONSTRAINT ck_audit_event_acting_mode
        CHECK (acting_mode IS NULL OR acting_mode IN ('person', 'client')),
    ADD CONSTRAINT ck_audit_event_assurance_level
        CHECK (assurance_level IS NULL
               OR assurance_level IN ('low', 'substantial', 'high'));

COMMENT ON COLUMN audit_event.acting_mode IS
    'Whether the credential named a natural person (person) or a client application (client); NULL when [auth.oidc.professional] is not configured or the record predates the column.';
COMMENT ON COLUMN audit_event.assurance_level IS
    'The eIDAS level of assurance (low, substantial, high) the credential''s assurance claim maps to under [auth.oidc.assurance] levels; NULL when unmapped or not configured.';
COMMENT ON COLUMN audit_event.assurance_value IS
    'The raw assurance claim value (usually acr) the credential carried; NULL when absent.';
COMMENT ON COLUMN audit_event.professional_id IS
    'The identifier of the natural person who accessed the data (EHDS Annex II 3.2(b)): the person''s own claim, or the professional a client token acted for.';
