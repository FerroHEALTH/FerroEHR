-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: the time of an event and the origin of a history as a column an AQL
-- filter enters through (#6022).
--
-- RENDERED FROM app/ferroehr/migrations/templates/node_event_time.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- A population query filtered on an event's time (`WHERE
-- o/data[at0001]/events[at0002]/time/value >= $t`) had no access path: the
-- planner read every node of the archetype, walked to each one's event row
-- and compared the time last, about 17 buffers a composition, whatever the
-- filter kept. The time lives in the event row's fragment and is read
-- through ext.openehr_timestamp, which is STABLE and so legal in no index
-- expression. The node insert now stores that function's value of the row's
-- `time` (POINT_EVENT, INTERVAL_EVENT) or `origin` (HISTORY) in event_time,
-- as it stores context_start, and the AQL engine adds, to such a comparison,
-- the condition that some row of the same version carries a time the
-- comparison keeps (crate::aql::sql::anchor), which the index serves. A
-- time the function reads as NULL stays out of the index and matches
-- nothing, as before. The UPDATEs fill the rows stored before this
-- migration from the same expression.
--
-- The index is partial on the rows that carry a time, and leads with the
-- RM type and the node id, which the path step names, so a comparison reads
-- one range of it (PostgreSQL 18, "Partial Indexes",
-- https://www.postgresql.org/docs/18/indexes-partial.html).
--
-- No openEHR spec governs columns or indexes: our own design.

ALTER TABLE node ADD COLUMN event_time timestamptz;

UPDATE node
   SET event_time = ext.openehr_timestamp(
           CASE WHEN time_compact THEN data ->> 'time' ELSE data #>> '{time,value}' END)
 WHERE rm_type IN ('POINT_EVENT', 'INTERVAL_EVENT');

UPDATE node
   SET event_time = ext.openehr_timestamp(
           CASE WHEN origin_compact THEN data ->> 'origin' ELSE data #>> '{origin,value}' END)
 WHERE rm_type = 'HISTORY';

CREATE INDEX idx_node_hot_event_time ON node_hot (rm_type, archetype, event_time)
    WHERE event_time IS NOT NULL;

ANALYZE node_hot;

COMMENT ON COLUMN node.event_time IS 'The time of a POINT_EVENT or INTERVAL_EVENT row, or the origin of a HISTORY row, as ext.openehr_timestamp reads it; NULL on every other row. Our own storage design (#6022).';
