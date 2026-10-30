-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- version: an AQL page ordered by the version uid reads it from an index (#4429).
--
-- RENDERED FROM app/ferroehr/migrations/templates/version_uid_order.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- ORDER BY c/uid/value sorts on the uid the AQL engine synthesizes from the
-- stored identity columns (crate::aql::sql::value::version_field_expr), so
-- no index could hand the version rows over in that order. The planner's one
-- way to sort them before the joins and stop at the limit was a Gather Merge,
-- and a pruned partition is offered a parallel scan however small it is: every
-- such query started a parallel worker, about 19 ms on a CI runner, and
-- whether it did turned on the statistics (PostgreSQL 18, "Parallel Query").
-- This index holds the same expression, so a serial scan of it feeds the limit
-- in uid order (PostgreSQL 18, "Indexes and ORDER BY"). The expression must
-- stay identical to the engine's, which the storage_plans test holds.
-- No openEHR spec governs index layout: our own design.

CREATE INDEX idx_version_hot_uid
    ON version_hot ((
        vo_id::text || '::' || creating_system_id || '::' || trunk_version::text
        || CASE WHEN branch_number > 0
                THEN '.' || branch_number::text || '.' || branch_version::text
                ELSE '' END
    ));
