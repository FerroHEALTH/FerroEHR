-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the three recorded_at indexes ascend.
--
-- recorded_at only grows, so in an index ordered recorded_at DESC every insert
-- lands on the leftmost leaf of its key range, where a B-tree page splits half
-- full; only a split at the rightmost leaf packs the left page full. Measured
-- after the 5.5.0 load test, a rebuild shrank the three indexes by 39 to 44 %,
-- while the ascending primary key did not shrink at all (#4302). A B-tree is
-- scanned backward as cheaply as forward, so ORDER BY recorded_at DESC keeps
-- its plan (PostgreSQL 18 docs, "Indexes and ORDER BY",
-- https://www.postgresql.org/docs/18/indexes-ordering.html).
--
-- No openEHR spec governs index layout: our own design.
--
-- Runs with search_path = audit, ext, public.

DROP INDEX ix_audit_event_recorded_at;
CREATE INDEX ix_audit_event_recorded_at ON audit_event (recorded_at);

DROP INDEX ix_audit_event_patient;
CREATE INDEX ix_audit_event_patient ON audit_event (patient_id, recorded_at)
    WHERE patient_id IS NOT NULL;

DROP INDEX ix_audit_event_principal;
CREATE INDEX ix_audit_event_principal ON audit_event (principal, recorded_at)
    WHERE principal IS NOT NULL;
