-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- vo_head: the root archetype of the trunk head, which the access condition
-- of a query compares where it probed the head's root node row (#5782).
--
-- RENDERED FROM app/ferroehr/migrations/templates/head_root_archetype.sql.in
-- by crate::storage::ddl_template, for both pseudonymisation domains; edit
-- the template, never this file.
--
-- A policy that scopes a directive by archetype reads, for every candidate
-- row of a population query, the archetype_node_id of the object's root.
-- Read from the node rows, that is a probe of the root row by its key per
-- row and per clause that names archetypes, about four buffers each; the
-- benchmark's newest-compositions page paid it for every one of its 7 386
-- candidates. The head row the query already joins now carries the value
-- beside template_id, so the condition compares a column.
--
-- The head upsert of every commit writes it with the trunk head, from the
-- committed body, and an import rebuilding the heads reads it from the root
-- row of the head's content. A content key names one tree whichever version
-- reads it, so the node rebuild that gives a sharing version its own rows
-- leaves it unchanged. The backfill below reads each head's root row once.
-- The nullable column rewrites no table and is in no index, so the
-- per-commit head update stays heap-only (PostgreSQL 18, "Heap-Only Tuples
-- (HOT)", https://www.postgresql.org/docs/18/storage-hot.html). No openEHR
-- spec governs storage access paths: our own design.

-- The archetype_node_id of the trunk head's root, as committed; NULL when
-- the head carries no content (a logical delete) or there is no trunk head.
ALTER TABLE vo_head ADD COLUMN root_archetype text;

UPDATE vo_head h SET root_archetype = (
    SELECT COALESCE(n.data ->> 'archetype_node_id',
                    ext.node_archetype_node_id(n.archetype, n.rm_version))
    FROM node n
    WHERE n.tier = h.tier AND n.vo_id = h.vo_id
      AND n.sys_version = h.trunk_head_content_key AND n.num = 0)
WHERE h.trunk_head_content_key IS NOT NULL;

COMMENT ON COLUMN vo_head.root_archetype IS 'The archetype_node_id of the trunk head''s root node, as committed; NULL when the head carries no content. The access condition of a query compares it. Our own storage design.';

ANALYZE vo_head;
