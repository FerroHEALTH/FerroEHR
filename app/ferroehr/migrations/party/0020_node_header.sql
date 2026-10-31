-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: the header columns, so a node below the root stores its repeated
-- archetype header, language and encoding once as short codes (#4525).
--
-- RENDERED FROM app/ferroehr/migrations/templates/node_header.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- Every archetype root below the versioned object's root repeats its
-- `archetype_node_id` inside `archetype_details`, with the same `rm_version`
-- on every node, and every ENTRY repeats the same `language` and `encoding`
-- CODE_PHRASE. On a row below the root, `data` leaves out
-- (crate::storage::slim::Header):
--
-- * `archetype_details` and `archetype_node_id` together, when the details are
--   exactly the node's own archetype id and an `rm_version`, and that id is
--   the recased form of the lowercase `archetype` column
--   (ext.node_archetype_node_id); `rm_version` keeps the version;
-- * `language`, when it is a code in `ISO_639-1`, kept in `language`;
-- * `encoding`, when it is a code in `IANA_character-sets`, kept in
--   `encoding`.
--
-- ext.node_fragment rebuilds the omitted keys. The root row keeps its
-- fragment unchanged. A row written before this migration has the three
-- columns NULL and its whole fragment, and reads the same. Adding a nullable
-- column with no default rewrites no table (PostgreSQL 18, ALTER TABLE, Notes).
-- No openEHR spec governs storage layout: our own design (RM common
-- master06-change_control_package.adoc §Overview admits any physical
-- representation).

-- `archetype_details/rm_version` of a row whose archetype header `data`
-- leaves out; NULL when `data` holds the header or the row has none.
ALTER TABLE node ADD COLUMN rm_version text;
-- `language/code_string` of a language in ISO_639-1 that `data` leaves out.
ALTER TABLE node ADD COLUMN language text;
-- `encoding/code_string` of an encoding in IANA_character-sets that `data`
-- leaves out.
ALTER TABLE node ADD COLUMN encoding text;

COMMENT ON COLUMN node.data IS 'The node''s canonical openEHR JSON fragment (ITS-JSON), structure children pruned, without the keys the row''s columns hold (_type, a lowercase archetype_node_id, a plain or simply coded name, a typed ELEMENT value, and below the root an archetype header, language and encoding of the common shapes); NULL when nothing remains. ext.node_fragment rebuilds the whole fragment. Our own storage design.';
COMMENT ON COLUMN node.rm_version IS 'The rm_version of an archetype_details that data leaves out together with archetype_node_id, both rebuilt from archetype (ext.node_archetype_details); NULL when data holds them. Our own storage design.';
COMMENT ON COLUMN node.language IS 'The code_string of a language in ISO_639-1 that data leaves out (ext.node_code_phrase). Our own storage design.';
COMMENT ON COLUMN node.encoding IS 'The code_string of an encoding in IANA_character-sets that data leaves out (ext.node_code_phrase). Our own storage design.';
