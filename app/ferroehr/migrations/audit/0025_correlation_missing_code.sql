-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: a chained organisation request without its correlation id (#5120),
-- and the Wabvpz Art. 15f refusals of an NL agreement's caller (#5107).
--
-- A `chained` organisation request whose token carries no attested
-- correlation id is refused with 403 under refused-correlation-missing, and
-- the agreement of an admitted chained request records the id and its
-- source, as 0023's column comment states. Under an NL agreement a caller
-- asserting the UZI role of an occupational or insurance physician is refused
-- under refused-wabvpz-role, and one asserting no care provider type under
-- refused-wabvpz-organisation-type (docs/law/nl/wabvpz/text.html, Art. 15f).
-- 0023 and 0024 are left as they were applied.
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
            'refused-scope-beyond-agreement', 'refused-correlation-missing',
            'refused-wabvpz-role', 'refused-wabvpz-organisation-type'));

