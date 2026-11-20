-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: the decomposed content of every version, one row per RM structure
-- node, nested-set indexed, with the indexes and extended statistics the AQL
-- engine's access paths enter through.
--
-- RENDERED FROM app/ferroehr/migrations/templates/node.sql.in by
-- crate::storage::ddl_template. Both pseudonymisation domains carry the same
-- node relation, so one template renders both and neither can drift from the
-- other; edit the template, never this file.
--
-- Decomposing RM containment into rows rather than storing each versioned
-- object as one physical document is explicitly sanctioned by the RM (RM common
-- master06-change_control_package.adoc §Overview, of the containment its own
-- figure draws: "Although the figure implies physical containment of Versions
-- by a Versioned object, this is only one possible implementation. Other
-- implementations (e.g. using orthodox relational structures) might use
-- references, separate compressed copies, or any other mechanism."). The
-- column set, the nested-set interval and the promoted predicate columns are
-- our own storage design.
--
-- The interval (num, num_cap) is what makes AQL CONTAINS an integer range join
-- rather than a JSON walk (QUERY master03-syntax.adoc §Containment). The
-- promoted name_code and name_terminology columns are what make the AQL node
-- predicate `[atNNNN, 'text']` and its coded form answerable without a JSON
-- probe (master03 §Node predicate).
--
-- Runs with the domain's own search_path, so every relation below is created
-- and referenced unqualified.

CREATE TABLE node (
    -- The storage tier, and the partition key; moved with the version row by
    -- the same statement. No foreign key into version: its
    -- referential-integrity trigger fired once per node row (our own design).
    tier        text NOT NULL DEFAULT 'hot',
    vo_id       uuid NOT NULL,
    sys_version integer NOT NULL,
    -- Pre-order number within the versioned object (root = 0).
    num         integer NOT NULL,
    -- The highest num in this row's subtree: the subtree is the closed
    -- interval num..=num_cap, which is what CONTAINS joins on.
    num_cap     integer NOT NULL,
    -- The num of the parent structure node (the root points at itself).
    parent_num  integer NOT NULL,
    -- The owning EHR, denormalized onto every node for EHR-scoped querying;
    -- NULL where no EHR owns the content.
    ehr_id      uuid,
    -- The node's full RM type name, never an alias.
    rm_type     text NOT NULL,
    -- The node's archetype_node_id when it carries one, stored LOWERCASED:
    -- openEHR identifier equality is case-insensitive (BASE base_types
    -- master05 §Composite Identifiers and Case), so the promoted predicate
    -- column holds the comparison form and AQL equality stays plain indexed
    -- equality with honest statistics. The canonical `data` fragment keeps the
    -- casing the document was written with.
    archetype   text,
    -- Archetype-subsumption columns, parsed from a full archetype HRID and
    -- comparison-normalized (lowercased); NULL on at-code/id-code nodes. Parts
    -- per BASE base_types master05 §Archetype Identifiers:
    --   archetype_id   = qualified_rm_entity '.' domain_concept '.v' version_id
    --   domain_concept = concept_name { '-' specialisation }
    -- They drive query subsumption (BASE architecture_overview master10
    -- §Design-time Relationships: "data created with any specialised archetype
    -- will always be matched by queries based on the parent archetype"): a
    -- query naming a parent matches a specialisation child through a
    -- `concept-%` prefix scan within the same entity and major version, the
    -- major boundary being hard (AM master07 §Querying).
    arch_entity  text,
    arch_concept text,
    arch_major   integer,
    -- The node's name.value.
    name        text,
    -- The node's name/defining_code, promoted so the AQL node predicate can
    -- match a coded name without a JSON probe. QUERY master03-syntax.adoc §Node
    -- predicate matches on the archetype node id and on the name, and the name
    -- of a coded node is a DV_CODED_TEXT whose defining_code is the stable
    -- half; name_terminology is that code's terminology_id.
    name_code        text,
    name_terminology text,
    -- Materialized path from the root; byte order under COLLATE "C" equals
    -- tree order. Used for reassembly, never as an AQL predicate.
    path        text COLLATE "C" NOT NULL,
    -- The node's canonical openEHR JSON fragment, with structure children
    -- pruned and without the keys the row's columns hold
    -- (crate::storage::slim, crate::storage::compact): ext.node_fragment puts
    -- them back. NULL when the columns hold the whole fragment. No column
    -- compression: the typical fragment is far under the 2 kB threshold at
    -- which PostgreSQL considers compressing a value at all (PostgreSQL 18,
    -- "TOAST", https://www.postgresql.org/docs/18/storage-toast.html).
    data        jsonb,
    -- Promoted EVENT_CONTEXT.start_time.value on the COMPOSITION root row
    -- (num = 0); NULL elsewhere and for context-less persistent compositions.
    -- Populated at write through ext.openehr_timestamp rather than as a
    -- generated column: a VIRTUAL generated column "must not reference
    -- user-defined functions or types" and a STORED one may not call a STABLE
    -- function (PostgreSQL 18, "Generated Columns",
    -- https://www.postgresql.org/docs/18/ddl-generated-columns.html), and the
    -- decomposer already holds the parsed value. Our own storage design.
    context_start timestamptz,
    -- The typed ELEMENT value columns: an ELEMENT value of a common shape is
    -- held here rather than in data (crate::storage::slim), and
    -- ext.node_value rebuilds it.
    value_type        text,
    value_num         numeric,
    value_text        text,
    value_code        text,
    value_terminology text,
    -- The archetype header, language and encoding of a row of the common
    -- shape (crate::storage::slim::Header), rebuilt by
    -- ext.node_archetype_details and ext.node_code_phrase.
    rm_version  text,
    language    text,
    encoding    text,
    -- The compact markers: which keys the row left out of data
    -- (crate::storage::compact), put back by ext.node_fragment.
    origin_compact boolean,
    time_compact   boolean,
    party_self     boolean,
    uid_derived    boolean,
    -- The time of an event row or the origin of a history row, read through
    -- ext.openehr_timestamp at write; the column an AQL time filter enters
    -- through (idx_node_hot_event_time).
    event_time  timestamptz,
    CONSTRAINT pk_node PRIMARY KEY (tier, vo_id, sys_version, num),
    CONSTRAINT ck_node_tier CHECK (tier IN ('hot', 'cold')),
    -- The nested-set invariant: a subtree cap is at or after its own number,
    -- and a parent precedes its child in pre-order. The root (num = 0,
    -- parent_num = 0) is exempt from the ordering check.
    CONSTRAINT ck_node_num_cap CHECK (num_cap >= num),
    CONSTRAINT ck_node_parent CHECK (num = 0 OR parent_num < num)
) PARTITION BY LIST (tier);
CREATE TABLE node_hot  PARTITION OF node FOR VALUES IN ('hot');
CREATE TABLE node_cold PARTITION OF node FOR VALUES IN ('cold');
-- The dashboard ORDER BY: the COMPOSITION roots of one EHR by context start
-- time. COMPOSITION occurs only at a root, so the rm_type predicate is exactly
-- what the query proves.
CREATE INDEX idx_node_hot_context_start ON node_hot (ehr_id, context_start)
    WHERE rm_type = 'COMPOSITION';
-- An AQL query over the whole population that orders compositions by
-- c/context/start_time/value and takes the first rows: idx_node_hot_context_start
-- leads with ehr_id and serves one EHR at a time, while this index orders the
-- COMPOSITION roots by start time alone, so a backward scan of it feeds the
-- limit directly (PostgreSQL 18, "Indexes and ORDER BY",
-- https://www.postgresql.org/docs/18/indexes-ordering.html). COMPOSITION
-- occurs only at a root, so the rm_type predicate is exactly what the query
-- proves. No openEHR spec governs indexes: our own design.
CREATE INDEX idx_node_hot_composition_start ON node_hot (context_start)
    WHERE rm_type = 'COMPOSITION';
-- An AQL lookup by an ELEMENT value (`WHERE id/items[at0001]/value/id = $v`)
-- enters here: the AQL engine adds, to such an equality, the condition that
-- some ELEMENT of the same version carries the value as its search text
-- (crate::aql::sql::condition; ext.node_search_text). The index is partial on
-- the ELEMENT rows, the only rows with a value.
CREATE INDEX idx_node_hot_value_search
    ON node_hot (ext.node_search_text(value_type, value_text, data))
    WHERE rm_type = 'ELEMENT';
-- The per-EHR reads of the structure rows by type and archetype.
CREATE INDEX idx_node_hot_ehr_type ON node_hot (ehr_id, rm_type, archetype)
    WHERE rm_type <> 'ELEMENT';
-- Archetype subsumption: a query naming a parent archetype matches a
-- specialisation child through a `concept-%` prefix scan within the same
-- entity and major version (AM master07 §Querying).
CREATE INDEX idx_node_hot_arch_subsume
    ON node_hot (rm_type, arch_entity, arch_major, arch_concept text_pattern_ops)
    WHERE arch_entity IS NOT NULL;
-- An AQL filter on the time of an event or the origin of a history enters
-- here (crate::aql::sql::anchor).
CREATE INDEX idx_node_hot_event_time ON node_hot (rm_type, archetype, event_time)
    WHERE event_time IS NOT NULL;
-- An AQL range filter on a magnitude (`WHERE
-- .../items[at0004]/value/magnitude > $x`) enters here: the AQL engine adds the
-- condition that some ELEMENT of the same version carries a magnitude the
-- comparison keeps (crate::aql::sql::anchor; ext.node_magnitude). It leads
-- with the node id, which the path step names, so a comparison reads one range
-- of it, and is partial on the ELEMENT rows that carry a magnitude
-- (PostgreSQL 18, "Partial Indexes",
-- https://www.postgresql.org/docs/18/indexes-partial.html).
CREATE INDEX idx_node_hot_value_magnitude
    ON node_hot (archetype, ext.node_magnitude(value_type, value_num, data))
    WHERE rm_type = 'ELEMENT'
      AND ext.node_magnitude(value_type, value_num, data) IS NOT NULL;
-- An AQL equality on a code (`WHERE
-- .../items[at0005]/value/defining_code/code_string = $code`) enters here, the
-- same way (ext.node_code). The index costs write-ahead log on every commit
-- that carries a coded value: 72 bytes an entry, 1.08% on the IPS composition
-- (36 entries), measured before it was adopted.
CREATE INDEX idx_node_hot_value_code
    ON node_hot (archetype, ext.node_code(value_type, value_code, data))
    WHERE rm_type = 'ELEMENT'
      AND ext.node_code(value_type, value_code, data) IS NOT NULL;
-- An AQL equality of `value/value` with a number-shaped string (the value of
-- a DV_ORDINAL or a DV_SCALE is a number no text index holds) enters here
-- (ext.node_number).
CREATE INDEX idx_node_hot_value_number
    ON node_hot (archetype, ext.node_number(value_type, value_num, data))
    WHERE rm_type = 'ELEMENT'
      AND ext.node_number(value_type, value_num, data) IS NOT NULL;
-- The AQL engine binds a COMPOSITION, EHR_STATUS or EHR_ACCESS root at
-- num = 0, and each of those rm_types occurs only there, a dependency the
-- planner cannot see by default: it multiplies the two selectivities and
-- expects a handful of root rows where there is one per version. This
-- statistics object records the dependency (PostgreSQL 18, "Extended
-- Statistics", https://www.postgresql.org/docs/18/planner-stats.html).
CREATE STATISTICS st_node_hot_rm_type_num (dependencies, mcv)
    ON rm_type, num FROM node_hot;
-- An AQL archetype predicate binds rm_type, arch_entity, arch_major and
-- arch_concept together, and each archetype fixes the other three columns: a
-- blood_pressure concept is always an openEHR-EHR-OBSERVATION of one major
-- version. The planner multiplies the four selectivities by default and, on a
-- CDR of 545 067 blood-pressure OBSERVATIONs, expected about 15 000, so it
-- built the whole archetype bitmap of idx_node_hot_arch_subsume once per
-- version instead of probing each version's own nodes. This statistics object
-- records the dependency.
CREATE STATISTICS st_node_hot_archetype (dependencies, mcv)
    ON rm_type, arch_entity, arch_major, arch_concept FROM node_hot;
-- The planner takes the distinct values and the spread of an expression from
-- no partial index, so these statistics objects give it those of the three
-- expressions the value indexes read.
CREATE STATISTICS st_node_hot_value_search
    ON (ext.node_search_text(value_type, value_text, data)) FROM node_hot;
CREATE STATISTICS st_node_hot_value_magnitude
    ON (ext.node_magnitude(value_type, value_num, data)) FROM node_hot;
CREATE STATISTICS st_node_hot_value_code
    ON (ext.node_code(value_type, value_code, data)) FROM node_hot;
COMMENT ON TABLE node IS 'Decomposed versioned-object content: one row per RM structure node, per version (our own storage design — openEHR defines no SQL schema). The nested-set interval num..=num_cap makes AQL CONTAINS an integer range join (QUERY master03-syntax.adoc §Containment).';
COMMENT ON COLUMN node.num IS 'Pre-order number within the versioned object (root = 0).';
COMMENT ON COLUMN node.num_cap IS 'The highest num in this node''s subtree: the subtree is num..=num_cap (AQL CONTAINS).';
COMMENT ON COLUMN node.parent_num IS 'The num of the parent structure node (the root points at itself).';
COMMENT ON COLUMN node.name IS 'The node''s name.value, promoted for the AQL node predicate (QUERY master03-syntax.adoc §Node predicate).';
COMMENT ON COLUMN node.archetype IS 'The node''s archetype_node_id, stored lowercased for comparison: openEHR identifier equality is case-insensitive (BASE base_types master05 §Composite Identifiers and Case). The canonical data fragment keeps its own casing.';
COMMENT ON COLUMN node.name_code IS 'The code_string of the node''s name/defining_code when its name is coded; NULL otherwise. Promoted for the AQL node predicate (QUERY master03-syntax.adoc §Node predicate). Our own storage design.';
COMMENT ON COLUMN node.name_terminology IS 'The terminology_id of the node''s name/defining_code when its name is coded; NULL otherwise. Our own storage design.';
COMMENT ON COLUMN node.arch_entity IS 'qualified_rm_entity of a full archetype HRID, lowercased for comparison (BASE base_types master05 §Archetype Identifiers); NULL on at/id-code nodes.';
COMMENT ON COLUMN node.arch_concept IS 'The full domain_concept (specialisation segments included) of a full archetype HRID, lowercased, so a parent query matches a child by prefix (BASE architecture_overview master10 §Design-time Relationships).';
COMMENT ON COLUMN node.arch_major IS 'The major version of a full archetype HRID; the interface-reference major boundary is hard (AM master07 §Querying). NULL on at/id-code nodes.';
COMMENT ON COLUMN node.path IS 'Materialized path from the root; byte order under COLLATE "C" equals tree order. Reassembly only — never an AQL predicate.';
COMMENT ON COLUMN node.context_start IS 'Promoted EVENT_CONTEXT.start_time.value on the COMPOSITION root; NULL elsewhere and for context-less persistent compositions. Our own storage design.';
COMMENT ON COLUMN node.value_type IS 'The RM type of the ELEMENT value the typed value columns hold; NULL when data holds the value. Our own storage design.';
COMMENT ON COLUMN node.value_num IS 'A DV_QUANTITY or DV_COUNT magnitude, or a DV_ORDINAL value, as the canonical JSON carried it.';
COMMENT ON COLUMN node.value_text IS 'A DV_QUANTITY''s units, a DV_ORDINAL symbol''s value, a DV_IDENTIFIER''s id, true or false for a DV_BOOLEAN, otherwise the value''s value.';
COMMENT ON COLUMN node.value_code IS 'The code_string of a DV_CODED_TEXT value or of a DV_ORDINAL symbol.';
COMMENT ON COLUMN node.value_terminology IS 'The terminology_id of value_code.';
COMMENT ON COLUMN node.language IS 'The code_string of a language in ISO_639-1 that data leaves out (ext.node_code_phrase). Our own storage design.';
COMMENT ON COLUMN node.encoding IS 'The code_string of an encoding in IANA_character-sets that data leaves out (ext.node_code_phrase). Our own storage design.';
COMMENT ON COLUMN node.data IS 'The node''s canonical openEHR JSON fragment (ITS-JSON), structure children pruned, without the keys the row''s columns hold (_type, a lowercase archetype_node_id, a plain or simply coded name, a typed ELEMENT value, an archetype header, language and encoding of the common shapes, a bare PARTY_SELF subject, and on the root the uid of the version), with a DV_DATE_TIME origin or time of the plain shape as its value string; NULL when nothing remains. ext.node_fragment rebuilds the whole fragment. Our own storage design.';
COMMENT ON COLUMN node.rm_version IS 'The rm_version of an archetype_details that data leaves out together with archetype_node_id, both rebuilt from archetype (ext.node_archetype_details), on the root with the version''s template_id; NULL when data holds them. Our own storage design.';
COMMENT ON COLUMN node.origin_compact IS 'TRUE when data holds the origin of this row as the value string of a DV_DATE_TIME that carried exactly _type and value (ext.node_date_time); NULL when data holds the origin as written. Our own storage design.';
COMMENT ON COLUMN node.time_compact IS 'TRUE when data holds the time of this row as the value string of a DV_DATE_TIME that carried exactly _type and value (ext.node_date_time); NULL when data holds the time as written. Our own storage design.';
COMMENT ON COLUMN node.party_self IS 'TRUE when data leaves out a subject that was exactly a PARTY_SELF with no external_ref; NULL when data holds the subject as written. Our own storage design.';
COMMENT ON COLUMN node.uid_derived IS 'TRUE when the root row''s data leaves out a uid that was exactly the OBJECT_VERSION_ID of its version row (ext.version_uid); NULL when data holds the uid as written or the row is not the root. Our own storage design.';
COMMENT ON COLUMN node.tier IS 'The storage tier and the partition key, moved with the version row by the same statement (no foreign key: our own design, #5969).';
COMMENT ON COLUMN node.event_time IS 'The time of a POINT_EVENT or INTERVAL_EVENT row, or the origin of a HISTORY row, as ext.openehr_timestamp reads it; NULL on every other row. Our own storage design (#6022).';
