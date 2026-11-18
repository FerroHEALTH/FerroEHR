-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: no foreign key into version, and indexes over only the rows their
-- readers reach (#5969).
--
-- RENDERED FROM app/ferroehr/migrations/templates/node_lean.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- The foreign key fired its referential-integrity trigger once per node row:
-- 402 nested lookups of the version row on an IPS commit, about 5.6 ms of
-- server time, for a key the writer already holds. Every node row is written
-- in the transaction that writes its version (the commit statement, the
-- import, the archive load and the rebuild), every statement that removes a
-- version removes its rows, and the tier moves name the node rows beside
-- the version rows (crate::storage::version_repo::tier). The storage-parity
-- sweep reports a node row whose version is gone, and a test holds the
-- invariant across commit, delete, archive and erasure.
--
-- The per-EHR class index leaves the ELEMENT rows out, 59% of an IPS
-- commit's rows: an AQL source of class ELEMENT is reached from the
-- structure node that contains it, through the primary key's nested-set
-- range, never from the EHR. The subsumption index keeps only the rows that
-- carry a parsed archetype id, the only rows its archetype predicates match;
-- an equality on arch_entity proves the partial predicate, whether the value
-- is a literal or a bound parameter (PostgreSQL 18, "Partial Indexes",
-- https://www.postgresql.org/docs/18/indexes-partial.html). Together they
-- write about 17% less WAL per commit.
--
-- No openEHR spec governs foreign keys or indexes: our own design.

ALTER TABLE node DROP CONSTRAINT fk_node_version;

DROP INDEX idx_node_hot_ehr_type;
CREATE INDEX idx_node_hot_ehr_type ON node_hot (ehr_id, rm_type, archetype)
    WHERE rm_type <> 'ELEMENT';

DROP INDEX idx_node_hot_arch_subsume;
CREATE INDEX idx_node_hot_arch_subsume
    ON node_hot (rm_type, arch_entity, arch_major, arch_concept text_pattern_ops)
    WHERE arch_entity IS NOT NULL;

COMMENT ON COLUMN node.tier IS 'The storage tier and the partition key, moved with the version row by the same statement (no foreign key: our own design, #5969).';
