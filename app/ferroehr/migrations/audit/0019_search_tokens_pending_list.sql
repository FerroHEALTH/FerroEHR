-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the search token index takes new entries through a pending list.
--
-- 0017 created ix_audit_event_search_tokens with fastupdate off, so every
-- insert rewrote the last segment of the posting tree of each token all records
-- share (their set, the principal, the address), which wrote more WAL than the
-- clinical writes of the same load (#4383). With fastupdate on, a record's
-- entries go to the pending list in one append and move into the posting trees
-- in bulk. A search reads the pending list whole, so its limit is 512 kB rather
-- than the 4 MB default (PostgreSQL 18 docs, "GIN Fast Update Technique",
-- https://www.postgresql.org/docs/18/gin.html#GIN-FAST-UPDATE).
--
-- 0017 shipped in 5.6.0-rc2 and keeps its text, since sqlx refuses a database
-- whose recorded checksum no longer matches the file (#4424).
--
-- No openEHR spec governs index storage: our own design.
--
-- Runs with search_path = audit, ext, public.

ALTER INDEX ix_audit_event_search_tokens
    SET (fastupdate = on, gin_pending_list_limit = 512);
