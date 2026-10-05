-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the optional integrations' stores — the FHIR mapping registry and
-- the multimedia blob reference index.
--
-- Both are extensions with no openEHR spec behind them, kept in the clinical
-- schema because their rows are EHR-scoped and reached through the clinical
-- pool.
--
-- Where the FHIR R4 surface of a deployment lives — this product's `fhir`
-- feature or FerroBRIDGE's facade — was ruled on 2026-09-03: it is FerroBRIDGE's (#3080).
-- `fhir_mapping` is the mapping registry that surface reads.
-- TODO(#3080): fhir_mapping leaves with the in-tree FHIR connector, retired
-- in favour of FerroBRIDGE (owner ruling 2026-09-03).
--
-- Runs with search_path = clinical, ext, public.

CREATE TABLE fhir_mapping (
    id            uuid NOT NULL DEFAULT uuidv7(),
    -- The stable, addressable deployable identity.
    name          text NOT NULL,
    -- The FHIR resource type this mapping consumes; the inbound router
    -- resolves POST /fhir/r4/{resourceType} by this plus the profile.
    resource_type text NOT NULL,
    -- The FHIR profile canonical URL matched against the resource's
    -- meta.profile; NULL = the default mapping for the resource type.
    profile_url   text,
    -- The openEHR template the built COMPOSITION targets.
    template_id   text NOT NULL,
    -- The mapping definition: the field bindings, code-system translations and
    -- subject/context rules. Validated on upload, stored verbatim so it
    -- round-trips.
    definition    jsonb NOT NULL,
    enabled       boolean NOT NULL DEFAULT true,
    created_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_fhir_mapping PRIMARY KEY (id),
    CONSTRAINT uq_fhir_mapping_name UNIQUE (name),
    CONSTRAINT fk_fhir_mapping_template FOREIGN KEY (template_id)
        REFERENCES template_store (template_id)
);

CREATE INDEX idx_fhir_mapping_resolve ON fhir_mapping (resource_type, profile_url)
    WHERE enabled;

COMMENT ON TABLE fhir_mapping IS 'FHIR-connector mapping store: deployable artefacts binding one openEHR template to one FHIR resource profile. Our own extension — no openEHR spec governs FHIR interop. Moving to FerroBRIDGE with the rest of the FHIR facade.';
COMMENT ON COLUMN fhir_mapping.definition IS 'The mapping definition (field bindings, code-system translations, subject/context rules), validated on upload and stored verbatim.';

-- ── blob_ref ─────────────────────────────────────────────────────────────────
-- Which stored version references which externalized multimedia blob.
--
-- `DV_MULTIMEDIA` may carry its content by reference
-- (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.dv_multimedia.adoc`:
-- `uri` is a "URI reference to electronic information stored outside the
-- record as a file, database entry etc"), and the multimedia store holds those
-- bytes outside the database. Garbage collection after a physical delete
-- then has to answer "does any surviving version still reference this blob",
-- which a scan of every `node` row answers slowly and a lookup here answers as
-- an index probe. Our own extension — no openEHR spec governs multimedia
-- offload.
--
-- One row per (version, URI). The rows are written by the node write path, in
-- the same transaction as the nodes they describe, so the index is never
-- behind the content it indexes; and the foreign key below carries them across
-- the tier move and removes them with the version, so nothing else maintains
-- them.
CREATE TABLE blob_ref (
    -- The storage tier, kept in lockstep with the version row by the foreign
    -- key's ON UPDATE CASCADE, exactly as `node` is.
    tier        text NOT NULL DEFAULT 'hot',
    vo_id       uuid NOT NULL,
    sys_version integer NOT NULL,
    -- The referenced blob's URI, as the stored body spells it.
    uri         text NOT NULL,
    CONSTRAINT pk_blob_ref PRIMARY KEY (tier, vo_id, sys_version, uri),
    CONSTRAINT ck_blob_ref_tier CHECK (tier IN ('hot', 'cold')),
    CONSTRAINT fk_blob_ref_version FOREIGN KEY (tier, vo_id, sys_version)
        REFERENCES version (tier, vo_id, sys_version)
        ON DELETE CASCADE ON UPDATE CASCADE
);

-- The collector's question, over both tiers: is this URI still referenced.
CREATE INDEX idx_blob_ref_uri ON blob_ref (uri);

COMMENT ON TABLE blob_ref IS 'Which stored version references which externalized multimedia blob, so blob garbage collection is a lookup rather than a scan of node. Written by the node write path in the same transaction; carried across the tier move and removed with the version by its foreign key. Our own extension.';
COMMENT ON COLUMN blob_ref.uri IS 'The referenced blob''s URI, as the stored body spells it.';
