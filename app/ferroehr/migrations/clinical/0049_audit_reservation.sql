-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the durable reservation of a write's audit records (#5762).
--
-- A request-scoped write commits through one choke point, which first writes
-- the write's audit records into the audit intake held under a reservation
-- id, then inserts that id here in the write's own transaction and commits.
-- After the commit the request releases the held records and deletes the row.
-- A process that dies between the commit and the release leaves the row,
-- and the reconciler of any replica releases the held records it names: the
-- write committed, so its records join the chain, exactly the records the
-- request wrote, once. A row holds no identifier of a person or a caller,
-- only the reservation id the audit side resolves, so the domains stay
-- apart (GDPR Art. 32(1)(a)). No openEHR spec governs audit storage: our own
-- design.
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE audit_reservation (
    id       uuid        NOT NULL,
    taken_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_audit_reservation PRIMARY KEY (id)
);

COMMENT ON TABLE audit_reservation IS
    'The reservation ids of committed writes whose audit records are still held in the audit intake: inserted in the write''s transaction, deleted once the records are released.';

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, DELETE ON audit_reservation TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the audit reservation grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
