-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: retention periods keyed on the EHDS priority category.
--
-- Annex II 3.4 of Regulation (EU) 2025/327 asks the storing components to
-- "support different retention periods and access rights that take into
-- account the origins and categories of electronic health data"
-- (docs/law/eu/ehds/text.html). The retention register keyed a period on the
-- RM kind alone; this file gives it an optional category dimension.
--
-- A category is a deployment's reading of its own templates, declared in
-- [audit.categories], so the map is configuration rather than data. The server
-- mirrors it into `category_map` at every boot, replacing the previous mirror
-- whole, and the `retention_due` view joins it against each object's template
-- id and root archetype id, template first.
--
-- No openEHR spec governs retention or the category map: our own
-- design/extension.
--
-- Runs with search_path = clinical, ext, public.

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
        category IN ('patient-summary', 'eprescription', 'edispensation', 'imaging',
                     'test-results', 'discharge-report', 'none')
        OR (category ~ '^national:[!-~]+$' AND strpos(category, ',') = 0)
    )
);

COMMENT ON TABLE category_map IS
    'The boot mirror of the [audit.categories] map: template or archetype id (case-folded) to EHDS Art. 14(1) category. Replaced whole at every boot; our own design/extension.';

-- ── retention_policy: the category dimension ─────────────────────────────────
-- NULL keeps the period's earlier meaning: every object of the kind. A
-- category narrows it to the objects the map places in that category.
-- `unclassified` is not a key: an object the map cannot classify takes the
-- longest period configured for its kind, which the view computes.
ALTER TABLE retention_policy
    ADD COLUMN category text;

ALTER TABLE retention_policy
    DROP CONSTRAINT pk_retention_policy,
    ADD CONSTRAINT uq_retention_policy
        UNIQUE NULLS NOT DISTINCT (kind, jurisdiction, category),
    ADD CONSTRAINT ck_retention_policy_category CHECK (
        category IS NULL
        OR category IN ('patient-summary', 'eprescription', 'edispensation', 'imaging',
                        'test-results', 'discharge-report', 'none')
        OR (category ~ '^national:[!-~]+$' AND strpos(category, ',') = 0)
    );

COMMENT ON COLUMN retention_policy.category IS
    'The EHDS priority category the period is keyed on (Annex II 3.4), or NULL for every object of the kind.';

-- ── retention_due: per category ──────────────────────────────────────────────
-- A period keyed on no category lists every object of its kind, as before. A
-- period keyed on a category lists the objects the map places in it, plus the
-- objects it cannot classify when this period is the longest configured for
-- their kind; a category row with no such object is left out. An object's
-- category is read from its latest version that carries content: the template
-- id first, then the root node's archetype id. Every kind other than
-- COMPOSITION holds no priority-category data and reads as `none`.
--
-- Interval comparison treats a month as 30 days (PostgreSQL 18, "Date/Time
-- Functions and Operators", https://www.postgresql.org/docs/18/functions-datetime.html),
-- so the longest of two periods declared in different units is approximate.
CREATE OR REPLACE VIEW retention_due WITH (security_invoker = true) AS
    SELECT a.ehr_id,
           a.jurisdiction,
           p.kind,
           p.source,
           a.anchored_at + p.period AS due_at,
           o.objects_due,
           o.objects_held,
           p.category
    FROM retention_anchor a
    JOIN retention_policy p
      ON p.jurisdiction = a.jurisdiction
    CROSS JOIN LATERAL (
        SELECT count(*) FILTER (WHERE h.retention_hold_at IS NULL) AS objects_due,
               count(*) FILTER (WHERE h.retention_hold_at IS NOT NULL) AS objects_held
          FROM vo_head h
          LEFT JOIN LATERAL (
              SELECT v.template_id, n.archetype
                FROM version v
                LEFT JOIN node n
                  ON n.tier = v.tier AND n.vo_id = v.vo_id
                 AND n.sys_version = v.sys_version AND n.num = 0
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
           AND (p.category IS NULL
                OR p.category = ANY (c.categories)
                OR (c.categories IS NULL
                    AND p.period = (SELECT max(q.period) FROM retention_policy q
                                     WHERE q.jurisdiction = p.jurisdiction
                                       AND (q.kind = 'EHR' OR q.kind = h.kind))))
    ) o
    WHERE a.anchored_at IS NOT NULL
      AND a.hold_at IS NULL
      AND a.anchored_at + p.period <= now()
      AND (p.category IS NULL OR o.objects_due + o.objects_held > 0);

COMMENT ON VIEW retention_due IS 'The EHRs whose retention period has run and which carry no EHR-wide hold, per period and, where the period is keyed on one, per EHDS priority category (Annex II 3.4), with the citation and the per-object hold counts. A list, never a disposal: the CDR deletes no clinical content on a timer (RM common master06 §Logical Deletion).';

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT, INSERT, DELETE ON category_map TO ferroehr_clinical;
        GRANT SELECT ON category_map TO ferroehr_clinical_reader;
        REVOKE ALL ON category_map
            FROM ferroehr_party, ferroehr_party_reader, ferroehr_linkage;
    ELSE
        RAISE NOTICE 'skipping category map grants (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
