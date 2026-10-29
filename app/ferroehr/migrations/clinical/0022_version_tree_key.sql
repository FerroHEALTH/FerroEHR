-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- version: the version-tree uniqueness key names the object last (#4340).
--
-- RENDERED FROM app/ferroehr/migrations/templates/version_tree_key.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- Every node and attestation row is checked against its version by the
-- foreign key's lookup on (tier, vo_id, sys_version), once per inserted row.
-- With uq_version_tree keyed (tier, vo_id, ...), the planner rated an index
-- scan on (tier, vo_id) with sys_version as a filter as cheap as the primary
-- key whenever it estimated about one version per object, and picked it
-- depending on the statistics of the moment; that scan reads every version
-- of the object, so a supersession wrote more slowly the longer the history
-- it extended. With vo_id last, the key can serve that lookup only by
-- skipping over the trunk and branch columns, and the primary key wins over
-- a populated partition with or without statistics. A lookup by the whole
-- spec tuple still matches every column. No openEHR spec governs index layout: our own design
-- (RM common master06-change_control_package.adoc §Overview admits any
-- physical representation).

ALTER TABLE version DROP CONSTRAINT uq_version_tree;

-- The spec identity tuple {object_id, creating_system_id, version_tree_id}
-- (master06 §Distributed Versioning), with `tier` because a unique constraint
-- on a partitioned table must contain the partition key.
ALTER TABLE version ADD CONSTRAINT uq_version_tree UNIQUE
    (tier, creating_system_id, trunk_version, branch_number, branch_version, vo_id);
