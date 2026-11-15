-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: no binding outcome names one country's law (#5691).
--
-- The two Wabvpz Art. 15f binding outcomes 0025 added,
-- refused-wabvpz-role and refused-wabvpz-organisation-type, are no longer
-- written: a controller states such a rule as a deny rule of its
-- ACCESS_POLICY, which the access evaluator refuses at step 4 under
-- refused-policy-denied and records with the rule's legal grounds. No new
-- row may carry either code.
--
-- NOTE: our own design (#5691), the rows already written keep their code because the access log is integrity-protected, so the narrowed check is NOT VALID and binds new rows only.
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
            'refused-scope-beyond-agreement', 'refused-correlation-missing'))
        NOT VALID;
