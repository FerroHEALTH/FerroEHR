-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the refusals of a caller kind carry a binding code (#5140).
--
-- A system caller holding a user/ or patient/ scope, a federation caller
-- whose issuer has no agreement for its accessor kind, and one holding a
-- scope its agreement does not read are refused with 403; each record now
-- names which rule refused it. 0023 is left as it was applied.
--
-- No openEHR spec governs access-log content: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event
    DROP CONSTRAINT ck_audit_event_binding,
    ADD CONSTRAINT ck_audit_event_binding
        CHECK (binding IS NULL OR binding IN (
            'bound', 'provisional', 'external', 'organisation', 'system', 'self',
            'refused-unbound', 'refused-ambiguous', 'refused-inactive', 'refused-abandoned',
            'refused-deprovisioned', 'refused-wrong-marker',
            'refused-representation-unrecorded', 'refused-key-claim-missing',
            'refused-groups-unavailable', 'refused-guest', 'refused-external-incomplete',
            'refused-trc-patient-mismatch', 'refused-trc-missing',
            'refused-system-personal-scope', 'refused-no-agreement',
            'refused-scope-beyond-agreement'));
