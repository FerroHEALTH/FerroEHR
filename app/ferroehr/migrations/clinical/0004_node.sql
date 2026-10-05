-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: the decomposed content of every version, one row per RM structure
-- node, nested-set indexed.
--
-- RENDERED FROM app/ferroehr/migrations/templates/node.sql.in by
-- crate::storage::ddl_template. Both pseudonymisation domains carry the same
-- node relation, so one template renders both and neither can drift from the
-- other; edit the template, never this file.
--
-- Decomposing RM containment into rows rather than storing each versioned
-- object as one physical document is explicitly sanctioned by the RM, not
-- merely unaddressed by it — RM common master06-change_control_package.adoc
-- §Overview, of the containment its own figure draws: "Although the figure
-- implies physical containment of Versions by a Versioned object, this is only
-- one possible implementation. Other implementations (e.g. using orthodox
-- relational structures) might use references, separate compressed copies, or
-- any other mechanism." The column set, the nested-set interval and the
-- promoted predicate columns are our own storage design.
--
-- The interval (num, num_cap) is what makes AQL CONTAINS an integer range join
-- rather than a JSON walk (QUERY master03-syntax.adoc §Containment). The promoted
-- name_code and name_terminology columns are what make the AQL node predicate
-- `[atNNNN, 'text']` and its coded form answerable without a JSON probe
-- (master03 §Node predicate).
--
-- Runs with the domain's own search_path, so every relation below is created
-- and referenced unqualified.
CREATE TABLE node (
    -- The storage tier, and the partition key; kept in lockstep with the
    -- version row by the foreign key's ON UPDATE CASCADE.
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
    -- The node's canonical openEHR JSON fragment verbatim, with structure
    -- children pruned — the stored fragment IS the canonical ITS-JSON
    -- encoding, so storage and API are the same bytes. No column compression:
    -- the typical fragment is far under the 2 kB threshold at which PostgreSQL
    -- considers compressing a value at all (PostgreSQL 18, "TOAST",
    -- https://www.postgresql.org/docs/18/storage-toast.html), so the setting
    -- was inert.
    data        jsonb NOT NULL,
    -- Promoted EVENT_CONTEXT.start_time.value on the COMPOSITION root row
    -- (num = 0); NULL elsewhere and for context-less persistent compositions.
    -- Populated at write through ext.openehr_timestamp rather than as a
    -- generated column: a VIRTUAL generated column "must not reference
    -- user-defined functions or types" and a STORED one may not call a STABLE
    -- function (PostgreSQL 18, "Generated Columns",
    -- https://www.postgresql.org/docs/18/ddl-generated-columns.html), and the
    -- decomposer already holds the parsed value. Our own storage design.
    context_start timestamptz,
    CONSTRAINT pk_node PRIMARY KEY (tier, vo_id, sys_version, num),
    CONSTRAINT ck_node_tier CHECK (tier IN ('hot', 'cold')),
    -- The nested-set invariant: a subtree cap is at or after its own number,
    -- and a parent precedes its child in pre-order. The root (num = 0,
    -- parent_num = 0) is exempt from the ordering check.
    CONSTRAINT ck_node_num_cap CHECK (num_cap >= num),
    CONSTRAINT ck_node_parent CHECK (num = 0 OR parent_num < num),
    -- DEFERRABLE so a multi-row version commit can order its statements
    -- freely inside one transaction; INITIALLY IMMEDIATE keeps the default
    -- check-at-statement-end behaviour.
    --
    -- ON UPDATE CASCADE is what makes archival one statement: an UPDATE that
    -- changes a partitioned table's partition key moves the row between
    -- partitions (PostgreSQL 18, "Partitioning",
    -- https://www.postgresql.org/docs/18/ddl-partitioning.html), and the
    -- cascade carries the referencing node rows across with it. The behaviour
    -- is pinned by a test rather than assumed.
    CONSTRAINT fk_node_version FOREIGN KEY (tier, vo_id, sys_version)
        REFERENCES version (tier, vo_id, sys_version)
        ON DELETE CASCADE ON UPDATE CASCADE
        DEFERRABLE INITIALLY IMMEDIATE
) PARTITION BY LIST (tier);

CREATE TABLE node_hot  PARTITION OF node FOR VALUES IN ('hot');
CREATE TABLE node_cold PARTITION OF node FOR VALUES IN ('cold');

-- The hot tier's query paths. The cold partition carries the primary key
-- alone — which is also the foreign key's support index — because archived
-- content leaves the queryable store until it is restored.
--
-- The per-EHR CONTAINS entry: an EHR's nodes of one class, optionally narrowed
-- by an exact archetype id.
CREATE INDEX idx_node_hot_ehr_type ON node_hot (ehr_id, rm_type, archetype);
-- The archetype-subsumption scan, and the class-only anchor as its leading
-- prefix. text_pattern_ops on arch_concept makes the specialisation-child
-- prefix scan (`LIKE 'concept-%'`) index-usable under a non-C collation
-- (PostgreSQL 18, "Operator Classes and Operator Families",
-- https://www.postgresql.org/docs/18/indexes-opclass.html). rm_type leads
-- because the AQL engine emits the CONTAINS class filter alongside every
-- archetype predicate.
CREATE INDEX idx_node_hot_arch_subsume
    ON node_hot (rm_type, arch_entity, arch_major, arch_concept text_pattern_ops);
-- The dashboard ORDER BY: the COMPOSITION roots of one EHR by context start
-- time. COMPOSITION occurs only at a root, so the rm_type predicate is exactly
-- what the query proves.
CREATE INDEX idx_node_hot_context_start ON node_hot (ehr_id, context_start)
    WHERE rm_type = 'COMPOSITION';

COMMENT ON TABLE node IS 'Decomposed versioned-object content: one row per RM structure node, per version (our own storage design — openEHR defines no SQL schema). The nested-set interval num..=num_cap makes AQL CONTAINS an integer range join (QUERY master03-syntax.adoc §Containment).';
COMMENT ON COLUMN node.tier IS 'The storage tier and the partition key, kept in lockstep with the version row by the foreign key''s ON UPDATE CASCADE.';
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
COMMENT ON COLUMN node.data IS 'The node''s canonical openEHR JSON fragment verbatim (ITS-JSON encoding), structure children pruned — storage and API are the same bytes.';
COMMENT ON COLUMN node.context_start IS 'Promoted EVENT_CONTEXT.start_time.value on the COMPOSITION root; NULL elsewhere and for context-less persistent compositions. Our own storage design.';
