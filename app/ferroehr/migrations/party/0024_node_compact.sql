-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: the compact markers, so a row stores a DV_DATE_TIME origin or time
-- as its value string, leaves out a bare PARTY_SELF subject, and the root
-- leaves out the uid the version row already holds (#4904).
--
-- RENDERED FROM app/ferroehr/migrations/templates/node_compact.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- Each marker is TRUE when `data` leaves the key out and NULL otherwise
-- (crate::storage::compact), so an unset marker costs nothing and a row
-- written before this migration reads as it did:
--
-- * `origin_compact`: `data -> 'origin'` is the `value` string of a
--   DV_DATE_TIME that carried exactly `_type` and `value` (RM data_structures
--   HISTORY.origin);
-- * `time_compact`: the same for `data -> 'time'` (RM data_structures
--   EVENT.time, RM composition ACTION.time);
-- * `party_self`: `data` leaves out a `subject` that was exactly
--   `{"_type": "PARTY_SELF"}` (RM composition ENTRY.subject, RM common
--   PARTY_SELF);
-- * `uid_derived`: the root row's `data` leaves out a `uid` that was exactly
--   the OBJECT_VERSION_ID of its version row (RM common LOCATABLE.uid,
--   ext.version_uid).
--
-- ext.node_fragment rebuilds every one of them. A marker, never an absent key
-- or a JSON shape, is what the rebuild reads, so a fragment the write paths
-- accepted with any other shape reads back as stored. Adding a nullable column
-- with no default rewrites no table (PostgreSQL 18, ALTER TABLE, Notes). No
-- openEHR spec governs storage layout: our own design (RM common
-- master06-change_control_package.adoc §Overview admits any physical
-- representation).

-- TRUE when `data -> 'origin'` is the value string of a DV_DATE_TIME.
ALTER TABLE node ADD COLUMN origin_compact boolean;
-- TRUE when `data -> 'time'` is the value string of a DV_DATE_TIME.
ALTER TABLE node ADD COLUMN time_compact boolean;
-- TRUE when `data` leaves out a `subject` of exactly `{"_type": "PARTY_SELF"}`.
ALTER TABLE node ADD COLUMN party_self boolean;
-- TRUE when the root row's `data` leaves out the `uid` of its version.
ALTER TABLE node ADD COLUMN uid_derived boolean;

COMMENT ON COLUMN node.data IS 'The node''s canonical openEHR JSON fragment (ITS-JSON), structure children pruned, without the keys the row''s columns hold (_type, a lowercase archetype_node_id, a plain or simply coded name, a typed ELEMENT value, an archetype header, language and encoding of the common shapes, a bare PARTY_SELF subject, and on the root the uid of the version), with a DV_DATE_TIME origin or time of the plain shape as its value string; NULL when nothing remains. ext.node_fragment rebuilds the whole fragment. Our own storage design.';
COMMENT ON COLUMN node.rm_version IS 'The rm_version of an archetype_details that data leaves out together with archetype_node_id, both rebuilt from archetype (ext.node_archetype_details), on the root with the version''s template_id; NULL when data holds them. Our own storage design.';
COMMENT ON COLUMN node.origin_compact IS 'TRUE when data holds the origin of this row as the value string of a DV_DATE_TIME that carried exactly _type and value (ext.node_date_time); NULL when data holds the origin as written. Our own storage design.';
COMMENT ON COLUMN node.time_compact IS 'TRUE when data holds the time of this row as the value string of a DV_DATE_TIME that carried exactly _type and value (ext.node_date_time); NULL when data holds the time as written. Our own storage design.';
COMMENT ON COLUMN node.party_self IS 'TRUE when data leaves out a subject that was exactly a PARTY_SELF with no external_ref; NULL when data holds the subject as written. Our own storage design.';
COMMENT ON COLUMN node.uid_derived IS 'TRUE when the root row''s data leaves out a uid that was exactly the OBJECT_VERSION_ID of its version row (ext.version_uid); NULL when data holds the uid as written or the row is not the root. Our own storage design.';
