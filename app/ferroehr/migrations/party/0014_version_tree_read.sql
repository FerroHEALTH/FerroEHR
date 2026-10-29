-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- version: a read by version tree id probes one version (#4420).
--
-- RENDERED FROM app/ferroehr/migrations/templates/version_tree_read.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- A read of one version by its VERSION_TREE_ID binds the object and the three
-- tree columns as parameters. The only index that matched all of them was the
-- trunk position, which is partial on branch_number = 0, and a generic plan
-- cannot prove a parameter equal to 0 (PostgreSQL 18, "Partial Indexes"), so
-- from a session's sixth execution on the read scanned the lineage tip index
-- on (vo_id, branch_number) and filtered out every other version of the
-- object. This index matches the four columns for any branch, in both tiers,
-- so an archived object stays readable by version id at the same cost. The
-- object id comes last, as in uq_version_tree (#4340): an index led by vo_id
-- would tie with the primary key for the foreign-key lookup on (tier, vo_id,
-- sys_version) and could be chosen to scan every version of the object.
-- No openEHR spec governs index layout: our own design (RM common
-- master06-change_control_package.adoc §Overview admits any physical
-- representation).

CREATE INDEX idx_version_hot_tree
    ON version_hot (trunk_version, branch_number, branch_version, vo_id);
CREATE INDEX idx_version_cold_tree
    ON version_cold (trunk_version, branch_number, branch_version, vo_id);
