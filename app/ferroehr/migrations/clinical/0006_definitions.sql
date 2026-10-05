-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the definition stores — templates in both dialects, ADL 1.4
-- archetypes, ADL2 artefacts and stored queries — and the reference from a
-- committed version to the template it was validated against.
--
-- Runs with search_path = clinical, ext, public.

-- ── template_ref ─────────────────────────────────────────────────────────────
-- The registry of PROVISIONED template wire addresses across BOTH template
-- dialects: `template_store` rows (OPT 1.4 `template_id`) and `adl2_artefact`
-- template-kind rows (ADL2 template / operational_template HRIDs — the AM
-- component keeps the two generations side by side, BASE architecture_overview
-- master05 §Package Structure). `version.template_id` references THIS
-- table, so a committed version's template identity is FK-guarded whichever
-- DEFINITION surface provisioned the template, and a physical delete of an
-- in-use template fails loud even under a concurrent commit (the NO ACTION
-- check at delete time) — the same race guard the former template_store-only
-- FK provided, made dialect-complete. Rows are maintained in the same
-- transaction as the owning store row (insert on provisioning; delete when the
-- last owning store row goes). No openEHR spec governs storage integrity —
-- our own design.
CREATE TABLE template_ref (
    template_id text NOT NULL,
    CONSTRAINT pk_template_ref PRIMARY KEY (template_id)
);

COMMENT ON TABLE template_ref IS 'Registry of provisioned template wire addresses (union of template_store.template_id and template-kind adl2_artefact.hrid). FK target of version.template_id: the dialect-complete delete/commit race guard (an extension — no openEHR spec governs storage integrity).';

-- ── template_store ───────────────────────────────────────────────────────────
-- Operational templates (OPT 1.4 XML); the parsed model is built in the
-- application, not stored here.
-- DUAL IDENTITY (SM I_DEFINITION_ADL14 takes a UUID handle; the ITS-REST wire
-- addresses templates by template_id): the uuid `id` is the
-- SM's OPT-keyed-by-UUID handle (the SM stores OPTs by UUID), while the unique
-- `template_id` is the wire address used by the ITS-REST DEFINITION API and by
-- version.template_id. Both are load-bearing; neither is redundant.
CREATE TABLE template_store (
    id             uuid NOT NULL DEFAULT uuidv7(),
    template_id    text NOT NULL,
    concept        text,
    root_archetype text,
    content        text NOT NULL,
    created_at     timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_template_store PRIMARY KEY (id),
    CONSTRAINT uq_template_store_template_id UNIQUE (template_id)
);

COMMENT ON TABLE template_store IS 'Operational templates (OPT 1.4 XML). Dual identity: uuid id = the SM UUID handle (SM I_DEFINITION_ADL14); template_id = the ITS-REST wire address. Template versioning is not spec-required — replace-in-place.';
COMMENT ON COLUMN template_store.id IS 'The SM OPT-by-UUID handle (SM openehr_platform master04-definition_package.adoc §Archetypes and Templates: "In ADL 1.4 ... OPTs are identified with UUIDs").';
-- The ITS-REST TemplateMetadata.version (optional + deprecated) is NOT stored:
-- it is the `.vN` version axis of template_id (filter_version: "taken from
-- template_id"), a pure function of the id, so it is derived on read rather than
-- denormalised into a column (see crate::templates::identity::template_version).
COMMENT ON COLUMN template_store.template_id IS 'The wire address (ITS-REST DEFINITION API; registered in template_ref, the version.template_id FK target); also the source of the reported TemplateMetadata.version (its `.vN` axis).';
-- Case-insensitive uniqueness of TEMPLATE_ID (BASE base_types master05
-- §Composite Identifiers and Case: identifier equality — and thus uniqueness —
-- is case-insensitive, so a case variant is the SAME template id and the
-- upload endpoint rejects it as a duplicate, ITS-REST
-- 409_template_already_exists). The exact-case UNIQUE above stays as the
-- natural-key anchor; this functional unique index is the race-free
-- case-insensitive guard.
CREATE UNIQUE INDEX ux_template_store_template_id_ci
    ON template_store (lower(template_id));


-- ── archetype_store (SM-2, I_DEFINITION_ADL14) ───────────────────────────────
-- ADL 1.4 source archetypes, keyed by their human-readable ARCHETYPE_ID (not a
-- UUID — reserved for OPTs/ADL2). Source ADL text stored verbatim; upload
-- replaces an existing id (ON CONFLICT DO UPDATE). Separate identity scheme per
-- formalism (SM I_DEFINITION_QUERY: stored queries are addressed by qualified
-- name + version).
CREATE TABLE archetype_store (
    archetype_id text NOT NULL,
    adl          text NOT NULL,
    created_at   timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_archetype_store PRIMARY KEY (archetype_id)
);

COMMENT ON TABLE archetype_store IS 'SM-2 ADL 1.4 source archetypes (I_DEFINITION_ADL14), keyed by ARCHETYPE_ID; verbatim ADL text.';

-- ── adl2_artefact (SM-2, I_DEFINITION_ADL2) ──────────────────────────────────
-- ADL2 artefacts (source archetype / template / OPT), all keyed uniformly by
-- ARCHETYPE_HRID; kind discriminates for the per-type list/count calls. Source
-- ADL2 text verbatim; upload replaces an existing HRID.
CREATE TABLE adl2_artefact (
    hrid       text NOT NULL,
    kind       text NOT NULL,
    adl        text NOT NULL,
    -- The artefact's declared `specialize` parent (AUTHORED_ARCHETYPE
    -- .parent_archetype_id), extracted from the validated source at upload;
    -- NULL for a non-specialised artefact. This is the ONLY lineage source for
    -- an AOM2-era identifier: AM Identification master03 §Legacy ADL 1.4
    -- Semantics strips the '-' concept separator of all meaning, and master07
    -- §Supporting Archetype-based Querying rules that "for specialised
    -- archetypes, the specialisation lineage can only be obtained from the
    -- operational form of the archetype". The AQL archetype predicate reads
    -- the (hrid, parent_hrid) edge set to widen a parent query to its stored
    -- specialisation children.
    parent_hrid text,
    created_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_adl2_artefact PRIMARY KEY (hrid),
    CONSTRAINT ck_adl2_artefact_kind CHECK (kind IN ('archetype', 'template', 'operational_template'))
);

COMMENT ON TABLE adl2_artefact IS 'SM-2 ADL2 artefacts (I_DEFINITION_ADL2), keyed by ARCHETYPE_HRID; kind ∈ archetype/template/operational_template; verbatim ADL2 text.';

-- ── stored_query ─────────────────────────────────────────────────────────────
-- Stored AQL queries, addressed by qualified name + SEMVER version (ITS-REST
-- specifications/docs/query/Qualified_query_name.md: "Stored queries are
-- identified by their name, used as `qualified_query_name`, and an optional
-- `version` number"; "`[{namespace}::]{query-name}`"; "The `version` identifier
-- is in the format specified by SEMVER"). Columns: reverse-domain namespace
-- (default "misc" — the namespace is optional on the wire, so an unqualified
-- name needs a stored stand-in; no openEHR spec governs the substitute value,
-- our own design), semantic id, semver, formalism.
CREATE TABLE stored_query (
    reverse_domain_name text NOT NULL,
    semantic_id         text NOT NULL,
    semver              text NOT NULL DEFAULT '0.0.0',
    query_type          text NOT NULL DEFAULT 'AQL',
    query_text          text NOT NULL,
    created_at          timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_stored_query PRIMARY KEY (reverse_domain_name, semantic_id, semver)
);

COMMENT ON TABLE stored_query IS 'Stored AQL queries, addressed by qualified name + SEMVER version (ITS-REST specifications/docs/query/Qualified_query_name.md).';

-- ── item_tag ─────────────────────────────────────────────────────────────────

-- ── the clinical references out of the templated change-control relations ────
-- `version` is rendered from the DDL template both pseudonymisation domains
-- share, so it can only carry the foreign keys the party domain also has. The
-- template reference is clinical-only and is attached here, once template_ref
-- exists: a physical delete of an in-use template then fails loud even under a
-- concurrent commit (the NO ACTION check at delete time), whichever DEFINITION
-- surface provisioned it. Our own design — no openEHR spec governs storage
-- integrity.
ALTER TABLE version
    ADD CONSTRAINT fk_version_template FOREIGN KEY (template_id)
        REFERENCES template_ref (template_id);
