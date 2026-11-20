-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: restriction of processing and retention.
--
-- The restriction register records why an EHR or an object is restricted
-- (GDPR Art. 18); the marks the read paths filter on are ehr.restricted_at
-- and vo_head.restricted_at. The retention register holds the periods per
-- kind, jurisdiction, category and origin, the anchors they run from, and
-- retention_due is the view of the objects past their period. No openEHR spec
-- governs restriction or retention: our own design/extension.
--
-- Runs with search_path = clinical, ext, public.

-- ── restriction ──────────────────────────────────────────────────────────────
-- The evidence behind a restriction mark: who asked, on what ground, when, and
-- whether it has been lifted. `vo_head.restricted_at` (and `ehr.id` with a
-- NULL vo_id, for the whole-EHR case) is the denormalised mark the read paths
-- filter on; this table is why it is set.
CREATE TABLE restriction (
    id           uuid NOT NULL DEFAULT uuidv7(),
    ehr_id       uuid NOT NULL,
    -- NULL restricts the whole EHR; otherwise the one versioned object.
    vo_id        uuid,
    -- The ground, from a closed list: the GDPR Art. 18(1) points, plus a
    -- deployment-declared ground for a national rule that goes beyond them.
    ground       text NOT NULL,
    requested_at timestamptz NOT NULL DEFAULT now(),
    lifted_at    timestamptz,
    note         text,
    CONSTRAINT pk_restriction PRIMARY KEY (id),
    CONSTRAINT ck_restriction_ground CHECK (ground IN (
        'gdpr-18-1-a', 'gdpr-18-1-b', 'gdpr-18-1-c', 'gdpr-18-1-d', 'national'
    )),
    CONSTRAINT ck_restriction_lifted_after CHECK
        (lifted_at IS NULL OR lifted_at >= requested_at),
    CONSTRAINT fk_restriction_ehr FOREIGN KEY (ehr_id) REFERENCES ehr (id) ON DELETE CASCADE
);
-- The in-force restrictions of one EHR: what a read path resolves a mark
-- against.
CREATE INDEX idx_restriction_in_force ON restriction (ehr_id, vo_id)
    WHERE lifted_at IS NULL;
COMMENT ON TABLE restriction IS 'The register behind a restriction mark: the ground and the dates. GDPR Art. 18; no openEHR spec governs restriction of processing — our own design/extension.';
COMMENT ON COLUMN restriction.vo_id IS 'The restricted versioned object, or NULL for the whole EHR.';
COMMENT ON COLUMN restriction.ground IS 'The Art. 18(1) point the restriction rests on, or `national` for a deployment-declared ground.';

-- ── retention_anchor ─────────────────────────────────────────────────────────
-- The per-EHR facts the period is measured against, and any hold that suspends
-- disposal regardless of it.
CREATE TABLE retention_anchor (
    ehr_id       uuid NOT NULL,
    jurisdiction text NOT NULL,
    -- NULL until the anchor event is known to the deployment.
    anchored_at  timestamptz,
    hold_at      timestamptz,
    hold_ground  text,
    CONSTRAINT pk_retention_anchor PRIMARY KEY (ehr_id),
    CONSTRAINT ck_retention_anchor_hold CHECK
        ((hold_at IS NULL) = (hold_ground IS NULL)),
    CONSTRAINT fk_retention_anchor_ehr FOREIGN KEY (ehr_id)
        REFERENCES ehr (id) ON DELETE CASCADE
);
COMMENT ON TABLE retention_anchor IS 'Per-EHR retention facts: the jurisdiction, the anchor event once known, and any hold that suspends disposal. Our own design/extension.';
COMMENT ON COLUMN retention_anchor.hold_at IS 'When a hold was placed (a litigation hold, or a national obligation to keep); while set, nothing is due.';

-- ── retention_policy ─────────────────────────────────────────────────────────
-- How long a category of content is kept, per jurisdiction, and what the
-- period is measured from. The CDR never deletes clinical content on a timer:
-- indelibility (RM common master06-change_control_package.adoc §Logical
-- Deletion) and the national record-keeping periods both forbid it, and the
-- controller decides. What storage does is know the period and be able to list
-- what is due.
CREATE TABLE retention_policy (
    -- The content category the period applies to.
    kind         text NOT NULL,
    -- The jurisdiction whose rule this is, as an ISO 3166-1 alpha-2 code.
    jurisdiction text NOT NULL,
    period       interval NOT NULL,
    -- What the period is measured from.
    anchor       text NOT NULL,
    -- The legal citation the period comes from, so the register says why.
    source       text NOT NULL,
    -- The data category the period applies to (NULL for every category), as
    -- a `system|code` token.
    category     text,
    -- The origin the period applies to (NULL for every origin).
    origin       text,
    CONSTRAINT uq_retention_policy
        UNIQUE NULLS NOT DISTINCT (kind, jurisdiction, category, origin),
    CONSTRAINT ck_retention_policy_kind CHECK
        (kind IN ('COMPOSITION', 'EHR_STATUS', 'FOLDER', 'EHR')),
    CONSTRAINT ck_retention_policy_anchor CHECK
        (anchor IN ('last_commit', 'death', 'majority')),
    CONSTRAINT ck_retention_policy_period_positive CHECK (period > interval '0'),
    CONSTRAINT ck_retention_policy_category CHECK (
        category IS NULL
        OR category = 'none'
        OR category ~ '^[A-Za-z][A-Za-z0-9+.-]*:[^|[:space:]]+\|[^[:space:]](.*[^[:space:]])?$'
    ),
    CONSTRAINT ck_retention_policy_origin CHECK (
        origin IS NULL OR (origin <> '' AND origin !~ '[[:space:]]')
    )
);
COMMENT ON TABLE retention_policy IS 'The retention period per content category and jurisdiction, with the legal citation it rests on. GDPR Art. 5(1)(e) and Art. 30(1)(f); our own design/extension.';
COMMENT ON COLUMN retention_policy.anchor IS 'What the period is measured from: the last commit, the subject''s death, or their majority.';
COMMENT ON COLUMN retention_policy.source IS 'The legal citation the period comes from, quoted as the vendored act spells it.';
COMMENT ON COLUMN retention_policy.category IS
    'The EHDS category the period is keyed on (Annex II 3.4), as its <system>|<code> token, or NULL for every object of the kind.';
COMMENT ON COLUMN retention_policy.origin IS
    'The registered origin the period is keyed on (Annex II 3.4), as its id in [[origins.registered]], or NULL for every origin.';

-- ── category_map ─────────────────────────────────────────────────────────────
-- The boot mirror of [audit.categories]: one row per (key, category) pair, the
-- key case-folded the way the node table stores archetype ids (BASE
-- base_types master05 §Composite Identifiers and Case).
CREATE TABLE category_map (
    key_kind text NOT NULL,
    key      text NOT NULL,
    category text NOT NULL,
    CONSTRAINT pk_category_map PRIMARY KEY (key_kind, key, category),
    CONSTRAINT ck_category_map_key_kind CHECK (key_kind IN ('template', 'archetype')),
    CONSTRAINT ck_category_map_key_folded CHECK (key = lower(key COLLATE "C")),
    CONSTRAINT ck_category_map_category CHECK (
        category = 'none'
        OR category ~ '^[A-Za-z][A-Za-z0-9+.-]*:[^|[:space:]]+\|[^[:space:]](.*[^[:space:]])?$'
    )
);
COMMENT ON TABLE category_map IS
    'The boot mirror of the [audit.categories] map: template or archetype id (case-folded) to EHDS Art. 14(1) category as its <system>|<code> token, or none. Replaced whole at every boot; our own design/extension.';

-- ── retention_due ────────────────────────────────────────────────────────────
-- The EHRs whose retention period has run, per period. An object is
-- classified by the template or root archetype of its latest content-carrying
-- version; a version whose content equalled its predecessor's holds no node
-- rows of its own, so the join reaches the root through the stored
-- `content_key`, as every other read of a version's rows does.
--
-- No openEHR spec governs retention or the physical sharing of stored
-- content: our own design/extension.
CREATE VIEW retention_due WITH (security_invoker = true) AS
    SELECT a.ehr_id,
           a.jurisdiction,
           p.kind,
           p.source,
           a.anchored_at + p.period AS due_at,
           o.objects_due,
           o.objects_held,
           p.category,
           p.origin
    FROM retention_anchor a
    JOIN retention_policy p
      ON p.jurisdiction = a.jurisdiction
    CROSS JOIN LATERAL (
        SELECT count(*) FILTER (WHERE h.retention_hold_at IS NULL) AS objects_due,
               count(*) FILTER (WHERE h.retention_hold_at IS NOT NULL) AS objects_held
          FROM vo_head h
          LEFT JOIN LATERAL (
              SELECT v.template_id, n.archetype,
                     coalesce(v.origins, jsonb_build_array(v.creating_system_id)) AS origins
                FROM version v
                LEFT JOIN node n
                  ON n.tier = v.tier AND n.vo_id = v.vo_id
                 AND n.sys_version = v.content_key AND n.num = 0
               WHERE v.vo_id = h.vo_id AND v.lifecycle_state <> '523'
               ORDER BY v.sys_version DESC
               LIMIT 1
          ) lv ON true
          CROSS JOIN LATERAL (
              SELECT CASE
                       WHEN h.kind <> 'COMPOSITION' THEN ARRAY['none']
                       ELSE coalesce(
                           (SELECT array_agg(m.category) FROM category_map m
                             WHERE m.key_kind = 'template'
                               AND m.key = lower(lv.template_id COLLATE "C")),
                           (SELECT array_agg(m.category) FROM category_map m
                             WHERE m.key_kind = 'archetype' AND m.key = lv.archetype))
                     END AS categories
          ) c
         WHERE h.ehr_id = a.ehr_id
           AND (p.kind = 'EHR' OR h.kind = p.kind)
           AND (p.origin IS NULL OR coalesce(lv.origins ? p.origin, false))
           AND (p.category IS NULL
                OR p.category = ANY (c.categories)
                OR (c.categories IS NULL
                    AND p.period = (SELECT max(q.period) FROM retention_policy q
                                     WHERE q.jurisdiction = p.jurisdiction
                                       AND (q.kind = 'EHR' OR q.kind = h.kind)
                                       AND (q.origin IS NULL
                                            OR coalesce(lv.origins ? q.origin, false)))))
    ) o
    WHERE a.anchored_at IS NOT NULL
      AND a.hold_at IS NULL
      AND a.anchored_at + p.period <= now()
      AND ((p.category IS NULL AND p.origin IS NULL) OR o.objects_due + o.objects_held > 0);
COMMENT ON VIEW retention_due IS 'The EHRs whose retention period has run and which carry no EHR-wide hold, per period and, where the period is keyed on one, per EHDS category and per registered origin (Annex II 3.4), with the citation and the per-object hold counts. A list, never a disposal: the CDR deletes no clinical content on a timer (RM common master06 §Logical Deletion).';
