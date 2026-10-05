-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the EHR-scoped relations that stand OUTSIDE change control —
-- folder membership and item tags.
--
-- Each is mutable in place and carries no version chain, which is what keeps
-- them out of the change-control file. Runs with
-- search_path = clinical, ext, public.

-- ── ehr_folder ───────────────────────────────────────────────────────────────
-- One row per folder hierarchy of an EHR (RM ehr master04-ehr_package.adoc
-- §Folders: "at any time, an entirely new Folder hierarchy may be added, which
-- will be referenced by a new member of the EHR._folders_ attribute"). `rank`
-- is the EHR.folders list order (1-based); EHR.directory is the first LIVE
-- hierarchy (RM ehr §EHR Class Directory_in_folders: `folders /= Void implies
-- folders.item(1) = directory`). Ranks are append-only and never reused — a
-- deleted hierarchy keeps its slot. Each referenced hierarchy is its own
-- versioned object; this table records membership and order only, so vo_id
-- carries no foreign key (version is keyed per version, not per object) and a
-- service-wide UNIQUE stands in. Our own storage design.
CREATE TABLE ehr_folder (
    ehr_id uuid  NOT NULL REFERENCES ehr (id) ON DELETE CASCADE,
    rank   int   NOT NULL,
    vo_id  uuid  NOT NULL,
    CONSTRAINT pk_ehr_folder PRIMARY KEY (ehr_id, rank),
    CONSTRAINT uq_ehr_folder_vo UNIQUE (vo_id),
    CONSTRAINT ck_ehr_folder_rank_positive CHECK (rank >= 1)
);

COMMENT ON TABLE ehr_folder IS 'One row per folder hierarchy of an EHR (RM ehr master04 §Folders); rank order = EHR.folders order, EHR.directory = the first live hierarchy (RM ehr §EHR Class Directory_in_folders: folders.item(1) = directory). Ranks are append-only, never reused. No openEHR spec governs this table (our own storage design).';
COMMENT ON COLUMN ehr_folder.rank IS 'EHR.folders position (1-based, append-only). The lowest-rank LIVE hierarchy is EHR.directory (folders.item(1)).';
COMMENT ON COLUMN ehr_folder.vo_id IS 'The VERSIONED_FOLDER versioned-object id (a member of EHR.folders). FK-less (vo_version is keyed per version); UNIQUE service-wide instead.';

-- ── item_tag ─────────────────────────────────────────────────────────────────
-- Item tags (RM common master07-tags.adoc ITEM_TAG; the ITS-REST tags routes).
-- Mutable, EHR-scoped, outside the version chain, requiring no contribution.
-- Our own storage design.
CREATE TABLE item_tag (
    id           uuid NOT NULL DEFAULT uuidv7(),
    ehr_id       uuid,
    -- The tagged target's uid. Deliberately FK-less: RM common master07
    -- ITEM_TAG.target "may be a VERSIONED_OBJECT<T> or a VERSION<T>", so it is
    -- outside the version chain by design and a key into `version` (which is
    -- keyed per version) would be wrong.
    target_vo_id uuid NOT NULL,
    -- The '{creating_system_id}::{version_tree_id}' tail of an addressed
    -- VERSION target, verbatim as addressed; NULL = the tag targets the
    -- VERSIONED_OBJECT container.
    target_version text,
    target_type  text NOT NULL,
    key          text NOT NULL,
    value        text,
    target_path  text,
    created_at   timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_item_tag PRIMARY KEY (id),
    -- ITEM_TAG identity: ITS-REST specifications/docs/overview/
    -- Requests_and_responses.md §openehr-item-tag — tags are "uniquely
    -- identified by their key and target_path pair", within one target
    -- (container or a specific VERSION). NULLS NOT DISTINCT so a NULL
    -- target_version or target_path participates in the identity.
    CONSTRAINT uq_item_tag_identity UNIQUE NULLS NOT DISTINCT
        (ehr_id, target_vo_id, target_version, key, target_path),
    -- The change-controlled resource types a tag may target. The ITS-REST
    -- wrapper headers are defined "for associating tags with change-controlled
    -- resources (e.g. COMPOSITION, EHR_STATUS, FOLDER, etc.)"
    -- (Requests_and_responses.md §openehr-item-tag and
    -- §openehr-version-item-tag).
    CONSTRAINT ck_item_tag_target_type CHECK (target_type IN (
        'COMPOSITION', 'EHR_STATUS', 'FOLDER',
        'AGENT', 'GROUP', 'ORGANISATION', 'PERSON', 'ROLE'
    )),
    CONSTRAINT fk_item_tag_ehr FOREIGN KEY (ehr_id) REFERENCES ehr (id) ON DELETE CASCADE
);
-- No dedicated index on `key` alone. RM ehr master04-ehr_package.adoc §Tags
-- states the indexing OBLIGATION ("in a typical implementation, tags would be
-- indexed") but names no index and no access path, so the choice is ours.
-- uq_item_tag_identity already serves every addressed read the released wire
-- defines, and the one key-leading access — the EHR-wide listing's optional
-- tag_key filter — is scoped to one EHR by that index's leading column, with
-- PostgreSQL 18 B-tree skip scan covering the remainder
-- (https://www.postgresql.org/docs/18/indexes-multicolumn.html).

COMMENT ON TABLE item_tag IS 'Item tags (RM common master07-tags.adoc ITEM_TAG; identity per ITS-REST Requests_and_responses.md: key + target_path within one target). Mutable, EHR-scoped, outside the version chain.';
COMMENT ON COLUMN item_tag.target_vo_id IS 'The tagged object; intentionally FK-less, because RM common master07 lets ITEM_TAG.target name a container OR a specific VERSION.';
COMMENT ON COLUMN item_tag.target_version IS 'The {creating_system_id}::{version_tree_id} tail of a VERSION-addressed target; NULL = container target.';
