-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the retention register reads an object's latest content through
-- the version that holds it (#4415).
--
-- `retention_due` classifies an object by the root archetype of its latest
-- content-carrying version, read off that version's root node row. Since
-- 0032 a version whose content equalled its predecessor's holds no node rows
-- of its own and names the version whose rows it reads in
-- `content_sys_version`, so the join reaches the root through
-- coalesce(content_sys_version, sys_version), as every other read of a
-- version's rows does. The view is otherwise the 0020 text, replaced in
-- place so the grants 0010 gave it stay (PostgreSQL 18, CREATE VIEW,
-- "CREATE OR REPLACE VIEW", https://www.postgresql.org/docs/18/sql-createview.html).
--
-- No openEHR spec governs retention or the physical sharing of stored
-- content: our own design/extension.
--
-- Runs with search_path = clinical, ext, public.

CREATE OR REPLACE VIEW retention_due WITH (security_invoker = true) AS
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
                 AND n.sys_version = coalesce(v.content_sys_version, v.sys_version) AND n.num = 0
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
