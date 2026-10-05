-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the reverse index of PARTY_RELATIONSHIP, and the party item tags.
--
-- RM demographic master02-demographic_package.adoc gives PARTY a
-- `reverse_relationships` attribute whose computation the model expresses as a
-- repository-wide lookup ("repository(\"demographics\").all_party_relationships"):
-- every relationship whose TARGET is this party. Walking the versioned bodies
-- to answer it would read every relationship in the domain, so the target side
-- is indexed here.
--
-- No openEHR spec governs the index relation itself — our own storage design;
-- the attribute it serves is the RM's.
--
-- Runs with search_path = party, ext, public.
CREATE TABLE party_relationship_target (
    -- The party the relationship points AT (PARTY_RELATIONSHIP.target).
    target_party_id uuid NOT NULL,
    -- The PARTY_RELATIONSHIP versioned object that points at it. FK-less for
    -- the reason the change-control relations are keyed per version: this row
    -- names the OBJECT, which outlives any one of its versions.
    source_vo_id    uuid NOT NULL,
    CONSTRAINT pk_party_relationship_target PRIMARY KEY (target_party_id, source_vo_id)
);

-- Maintenance from the source side: a relationship's target changes with a new
-- version, and the old row is removed by source.
CREATE INDEX idx_party_relationship_target_source
    ON party_relationship_target (source_vo_id);

COMMENT ON TABLE party_relationship_target IS 'The target side of PARTY_RELATIONSHIP, indexed so PARTY.reverse_relationships (RM demographic master02-demographic_package.adoc) is a lookup rather than a scan of every relationship in the domain. Our own storage design.';
COMMENT ON COLUMN party_relationship_target.source_vo_id IS 'The PARTY_RELATIONSHIP versioned object pointing at the target; FK-less because it names the object, not one of its versions.';

-- ── item_tag ─────────────────────────────────────────────────────────────────
-- Item tags on a party (RM common master07-tags.adoc ITEM_TAG; the ITS-REST
-- demographic tag routes). Mutable, outside the version chain, requiring no
-- contribution — the same relation the clinical domain carries, minus the EHR
-- it has none of. Our own storage design.
CREATE TABLE item_tag (
    id             uuid NOT NULL DEFAULT uuidv7(),
    -- Always NULL here (a party has no owning EHR); kept so one set of tag
    -- statements serves both domains.
    ehr_id         uuid,
    -- The tagged party's versioned-object uid. Deliberately FK-less: RM common
    -- master07 ITEM_TAG.target "may be a VERSIONED_OBJECT<T> or a VERSION<T>",
    -- so it is outside the version chain by design.
    target_vo_id   uuid NOT NULL,
    -- The '{creating_system_id}::{version_tree_id}' tail of an addressed
    -- VERSION target; NULL = the tag targets the VERSIONED_OBJECT container.
    target_version text,
    target_type    text NOT NULL,
    key            text NOT NULL,
    value          text,
    target_path    text,
    created_at     timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_item_tag PRIMARY KEY (id),
    -- ITEM_TAG identity: ITS-REST specifications/docs/overview/
    -- Requests_and_responses.md §openehr-item-tag — tags are "uniquely
    -- identified by their key and target_path pair", within one target. NULLS
    -- NOT DISTINCT so a NULL target_version or target_path participates in it.
    CONSTRAINT uq_item_tag_identity UNIQUE NULLS NOT DISTINCT
        (ehr_id, target_vo_id, target_version, key, target_path),
    CONSTRAINT ck_item_tag_target_type CHECK (target_type IN (
        'AGENT', 'GROUP', 'ORGANISATION', 'PERSON', 'ROLE'
    ))
);

CREATE INDEX idx_item_tag_target ON item_tag (target_vo_id);

COMMENT ON TABLE item_tag IS 'Item tags on a party (RM common master07-tags.adoc ITEM_TAG; identity per ITS-REST Requests_and_responses.md). Mutable, outside the version chain.';
COMMENT ON COLUMN item_tag.target_vo_id IS 'The tagged party; intentionally FK-less, because RM common master07 lets ITEM_TAG.target name a container OR a specific VERSION.';
