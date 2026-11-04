-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- version: a version whose content equals its predecessor's stores that
-- content once (#4415).
--
-- RENDERED FROM app/ferroehr/migrations/templates/version_content.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- Every committed version keeps its own row, commit audit, contribution,
-- attestations and uid-stamped body: the commit is recorded in full (RM
-- common master06-change_control_package.adoc §Committal and Audits). What a
-- version whose content equals the preceding tip's no longer writes is a
-- second copy of the node rows: `content_sys_version` names the version in
-- the same object whose rows hold the content, NULL meaning the version's
-- own rows, which is every row written before this migration, every logical
-- delete and every loaded or replayed version. `content_key` stores
-- coalesce(content_sys_version, sys_version), and every read of a version's
-- rows joins `node` on it: a plain column the planner joins on, probes an
-- index with and estimates from its statistics, where an expression would
-- carry no statistics and anchor no index condition.
--
-- `content_digest` is SHA-256 over the canonical ITS-JSON bytes of the body
-- with the root `uid` removed (crate::versioning::change::content_digest):
-- a digest a crafted change cannot collide. NULL on a logical delete and on
-- rows written before this migration, which are never shared.
--
-- The two nullable columns rewrite no table; the stored generated column
-- rewrites `version` once, as a stored generated column added to an
-- existing table is computed for every row (PostgreSQL 18, ALTER TABLE,
-- Notes, https://www.postgresql.org/docs/18/sql-altertable.html), and the
-- ANALYZE that closes the migration gives the planner its statistics at
-- once. No openEHR spec governs the physical sharing of stored content: our
-- own design (master06 §Overview admits any physical representation).

-- SHA-256 over the canonical body with the root uid removed; NULL on a
-- logical delete and on a row written before this migration.
ALTER TABLE version ADD COLUMN content_digest bytea;
-- The sys_version, in the same object, whose node rows hold this version's
-- content; NULL when its own rows do.
ALTER TABLE version ADD COLUMN content_sys_version integer;
-- The key every read of a version's rows joins node on: the version whose
-- rows hold its content (PostgreSQL 18, "Generated Columns",
-- https://www.postgresql.org/docs/18/ddl-generated-columns.html).
ALTER TABLE version ADD COLUMN content_key integer NOT NULL
    GENERATED ALWAYS AS (coalesce(content_sys_version, sys_version)) STORED;

ALTER TABLE version ADD CONSTRAINT ck_version_content_digest_length
    CHECK (content_digest IS NULL OR octet_length(content_digest) = 32);
-- A version shares only an earlier version's rows, and a logical delete
-- (no body) shares none.
ALTER TABLE version ADD CONSTRAINT ck_version_content_sys_version
    CHECK (content_sys_version IS NULL
           OR (content_sys_version >= 1 AND content_sys_version < sys_version AND body IS NOT NULL));

COMMENT ON COLUMN version.content_digest IS 'SHA-256 over the canonical ITS-JSON body with the root uid removed, compared with the preceding tip''s to store identical content once; NULL on a logical delete and on a row written before the column existed. Our own storage design.';
COMMENT ON COLUMN version.content_sys_version IS 'The sys_version in the same versioned object whose node rows hold this version''s content, when the content equalled the preceding tip''s at commit; NULL when the version''s own rows hold it. Our own storage design.';
COMMENT ON COLUMN version.content_key IS 'The sys_version whose node rows hold this version''s content: content_sys_version where set, else the version''s own. Every read of a version''s rows joins node on this column. Our own storage design.';

ANALYZE version;
