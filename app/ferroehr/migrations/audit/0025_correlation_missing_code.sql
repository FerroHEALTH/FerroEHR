-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: a chained organisation request without its correlation id (#5120).
--
-- A `chained` organisation request whose token carries no attested
-- correlation id is refused with 403 under refused-correlation-missing, and
-- the agreement of an admitted chained request records the id and its
-- source. 0024 is left as it was applied.
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
            'refused-scope-beyond-agreement', 'refused-correlation-missing'));

COMMENT ON COLUMN audit_event.agreement IS
    'The federation agreement an external accessor was admitted under: {name, version, legal_ground, accessor, access_class}, and for a chained request {correlation_id, correlation_source}.';
