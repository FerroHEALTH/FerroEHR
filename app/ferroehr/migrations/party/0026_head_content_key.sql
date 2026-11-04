-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- vo_head: the trunk head's content key, which a LATEST_VERSION read joins
-- the node rows on, and the planner's estimate of an object's versions
-- (#4944).
--
-- RENDERED FROM app/ferroehr/migrations/templates/head_content_key.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- A version whose content equals its predecessor's reads its predecessor's
-- node rows (version.content_key, #4415), so one node row belongs to every
-- version of an identical-content chain. A read joining node to version on
-- the content key reaches each such row through every version of the chain
-- before the head filter keeps one; the storage bench's 1 200-version chains
-- read fourteen times the pages so. The head row names the one version a
-- LATEST_VERSION read answers, and with its content key beside it the read
-- links the node rows to the head, one row per object, and reaches the
-- version row through the head's primary key.
--
-- The head upsert of every commit writes the column with the trunk head, and
-- the node rebuild, which gives a sharing version its own rows again, sets it
-- to that version's own sys_version. The backfill below reads each object's
-- trunk head once. The nullable column rewrites no table, and the column is
-- in no index, so the per-commit head update stays heap-only (PostgreSQL 18,
-- "Heap-Only Tuples (HOT)",
-- https://www.postgresql.org/docs/18/storage-hot.html).
--
-- The node's, the head's and the version's vo_id are one equivalence class,
-- so the planner can still probe the version by vo_id alone, and on
-- statistics taken before a chain grew it priced that probe at one row and
-- took it. The distinct count declared below holds the estimate at two
-- versions an object whatever ANALYZE samples, so the exact path through the
-- head row always costs less than a probe that may read a whole chain; the
-- partitioned parent keeps its sampled statistics, which the join sizes read
-- (PostgreSQL 18, ALTER TABLE, "n_distinct",
-- https://www.postgresql.org/docs/18/sql-altertable.html). No openEHR spec
-- governs storage access paths: our own design.

-- The trunk head's version.content_key; NULL exactly when the object has no
-- trunk head.
ALTER TABLE vo_head ADD COLUMN trunk_head_content_key integer;

UPDATE vo_head h SET trunk_head_content_key = v.content_key
FROM version v
WHERE v.vo_id = h.vo_id AND v.sys_version = h.trunk_head_sys_version;

-- The key names the head or an earlier version of the same object, and is
-- set exactly when the head is.
ALTER TABLE vo_head ADD CONSTRAINT ck_vo_head_trunk_head_content_key
    CHECK ((trunk_head_content_key IS NULL) = (trunk_head_sys_version IS NULL)
           AND (trunk_head_content_key IS NULL
                OR trunk_head_content_key BETWEEN 1 AND trunk_head_sys_version));

COMMENT ON COLUMN vo_head.trunk_head_content_key IS 'The trunk head''s version.content_key: the sys_version whose node rows hold the head''s content. A LATEST_VERSION read joins the node rows on it. Our own storage design.';

-- Half as many distinct objects as version rows: two versions an object.
ALTER TABLE version_hot ALTER COLUMN vo_id SET (n_distinct = -0.5);
ALTER TABLE version_cold ALTER COLUMN vo_id SET (n_distinct = -0.5);

ANALYZE version;
ANALYZE vo_head;
