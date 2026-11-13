-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the reverse index keeps when each relationship held (#5473).
--
-- A party read at a version or at a time serves PARTY.reverse_relationships
-- as they stood when the version it serves was committed. The RM is silent on
-- how a derived list behaves on an old version (RM 1.1.0 demographic
-- master02-demographic_package.adoc §Party Relationships; RM 1.2.0 makes it a
-- function, SPECRM-124), so this is our own design. Each row now holds the
-- interval over which the source's trunk named the target: valid_from is the
-- commit instant of the source version that named it, valid_to the commit
-- instant of the one that stopped naming it, NULL while it still does. A
-- point-in-time read is then one indexed range probe per target, never a walk
-- of the source versions.
--
-- The rows are rebuilt from the stored trunk versions, so history committed
-- before this migration is answered too: consecutive trunk versions of a
-- source that name a target form one interval (a gaps-and-islands grouping
-- over trunk_version). The target root is read as
-- crate::party_binding::derive::container reads it: the HIER_OBJECT_ID root
-- before any '::' extension, case-folded, a hyphenated UUID. No openEHR spec
-- governs the index relation: our own storage design.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE party_relationship_target
    ADD COLUMN valid_from timestamptz,
    ADD COLUMN valid_to   timestamptz;

DELETE FROM party_relationship_target;

WITH trunk AS (
    SELECT v.vo_id, v.trunk_version, v.committed_at, v.body, v.lifecycle_state,
           lead(v.committed_at) OVER (PARTITION BY v.vo_id ORDER BY v.trunk_version) AS next_at
    FROM version v
    WHERE v.branch_number = 0 AND v.kind <> 'PARTY_RELATIONSHIP'
), named AS (
    SELECT DISTINCT t.vo_id, t.trunk_version, t.committed_at, t.next_at,
           target.root::uuid AS target
    FROM trunk t
    CROSS JOIN LATERAL jsonb_array_elements(
        CASE WHEN jsonb_typeof(t.body::jsonb -> 'relationships') = 'array'
             THEN t.body::jsonb -> 'relationships' ELSE '[]'::jsonb END
    ) AS r(rel)
    CROSS JOIN LATERAL (SELECT lower(split_part(
        r.rel -> 'target' -> 'id' ->> 'value', '::', 1)) AS root) AS target
    WHERE t.body IS NOT NULL AND t.lifecycle_state <> '523'
      AND target.root ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
), islands AS (
    SELECT n.vo_id, n.target, n.committed_at, n.next_at,
           n.trunk_version - row_number() OVER (
               PARTITION BY n.vo_id, n.target ORDER BY n.trunk_version) AS island
    FROM named n
)
INSERT INTO party_relationship_target (target_party_id, source_vo_id, valid_from, valid_to)
SELECT i.target, i.vo_id, min(i.committed_at),
       CASE WHEN bool_or(i.next_at IS NULL) THEN NULL ELSE max(i.next_at) END
FROM islands i
GROUP BY i.vo_id, i.target, i.island
HAVING bool_or(i.next_at IS NULL) OR max(i.next_at) > min(i.committed_at);

ALTER TABLE party_relationship_target
    ALTER COLUMN valid_from SET NOT NULL,
    DROP CONSTRAINT pk_party_relationship_target,
    ADD CONSTRAINT pk_party_relationship_target
        PRIMARY KEY (target_party_id, source_vo_id, valid_from),
    ADD CONSTRAINT ck_party_relationship_target_interval
        CHECK (valid_to IS NULL OR valid_to > valid_from);

-- The live list, and the one open interval per pair a commit closes or keeps.
CREATE UNIQUE INDEX uq_party_relationship_target_open
    ON party_relationship_target (target_party_id, source_vo_id)
    WHERE valid_to IS NULL;

COMMENT ON TABLE party_relationship_target IS 'The target side of the inline PARTY_RELATIONSHIPs, one row per interval over which a source party''s trunk named the target, so PARTY.reverse_relationships (RM demographic master02-demographic_package.adoc) is a lookup, live or at an instant, rather than a scan of every relationship in the domain. Our own storage design.';
COMMENT ON COLUMN party_relationship_target.valid_from IS 'The commit instant of the source trunk version that began naming the target.';
COMMENT ON COLUMN party_relationship_target.valid_to IS 'The commit instant of the source trunk version that stopped naming the target (a new version without it, or a logical delete); NULL while the current trunk version names it.';

ANALYZE party_relationship_target;
