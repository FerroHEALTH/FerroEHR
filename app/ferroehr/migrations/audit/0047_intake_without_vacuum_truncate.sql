-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: autovacuum never truncates an intake (#5970).
--
-- Each intake row is inserted by a request and deleted by the chainer a
-- moment later, so the tail pages of both intakes empty again and again.
-- Truncating them takes an ACCESS EXCLUSIVE lock, which every request's
-- intake write then waits behind (PostgreSQL 18 docs, CREATE TABLE, "Storage
-- Parameters", vacuum_truncate). The next inserts reuse the space a truncate
-- would return.
--
-- No openEHR spec governs audit storage: our own design.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE access_log_intake SET (vacuum_truncate = off);
ALTER TABLE audit_event_intake SET (vacuum_truncate = off);
