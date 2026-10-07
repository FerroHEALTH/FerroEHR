-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the emergency-access mark on an access record.
--
-- Art. 11(5) of Regulation (EU) 2025/327 lets a health professional be granted
-- access to data a natural person restricted under Art. 8 "where necessary in
-- order to protect the vital interests of the data subject", and requires that
-- "such cases shall be logged in a clear and understandable format and shall
-- be easily accessible for the data subject"; Art. 9(1) gives the person
-- information on every access "including access provided in accordance with
-- Article 11(5)" (docs/law/eu/ehds/text.html). An access whose declared purpose
-- of use is one of the deployment's [audit] emergency_purpose_codes is marked
-- here. The mark records the declaration; it lifts no restriction.
--
-- false means the declared purpose was not an emergency code, or nothing was
-- declared, or the record predates the column.
--
-- Append-only needs no new machinery: the BEFORE UPDATE trigger the tamper
-- chain installs compares the whole row minus the two delivery stamps, so the
-- column cannot be changed after the insert. The column itself is not an input
-- of audit.audit_event_digest; the FHIR document the digest covers carries the
-- same mark as AuditEvent.purposeOfEvent. The table-level grants of the audit
-- grants file cover new columns.
--
-- No openEHR spec governs access-log content: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event
    ADD COLUMN emergency_access boolean NOT NULL DEFAULT false;

COMMENT ON COLUMN audit_event.emergency_access IS
    'Whether the access was declared as an emergency access in the vital interest of the data subject (EHDS Art. 11(5)): the declared purpose of use is one of [audit] emergency_purpose_codes. Records the declaration only; no restriction is lifted by it.';
