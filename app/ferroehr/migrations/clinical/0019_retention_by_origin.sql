-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: retention periods keyed on the origin of the data.
--
-- Annex II 3.4 of Regulation (EU) 2025/327 asks the storing components to
-- "support different retention periods and access rights that take into
-- account the origins and categories of electronic health data"
-- (docs/law/eu/ehds/text.html). 0012 gave the retention register its category
-- dimension; this file gives it the origin dimension. An origin is the id of
-- an origin the deployment registers in [[origins.registered]], which the
-- service checks before the row is written, and it is matched against the
-- origins each version recorded at commit (`version.origins`, the creating
-- system for an unstamped row).
--
-- A period keyed on an origin lists the objects whose latest version carrying
-- content came from it; NULL keeps the earlier meaning, every origin. An object
-- the category map cannot classify takes the longest period configured for its
-- kind among the periods that reach its origin.
--
-- No openEHR spec governs retention or the origin register: our own
-- design/extension.
--
-- Runs with search_path = clinical, ext, public.

ALTER TABLE retention_policy
    ADD COLUMN origin text,
    ADD CONSTRAINT ck_retention_policy_origin CHECK (
        origin IS NULL OR (origin <> '' AND origin !~ '[[:space:]]')
    ),
    DROP CONSTRAINT uq_retention_policy,
    ADD CONSTRAINT uq_retention_policy
        UNIQUE NULLS NOT DISTINCT (kind, jurisdiction, category, origin);

COMMENT ON COLUMN retention_policy.origin IS
    'The registered origin the period is keyed on (Annex II 3.4), as its id in [[origins.registered]], or NULL for every origin.';

-- Replaced in place, the new column last, so the grants 0010 gave the view stay
-- (PostgreSQL 18, CREATE VIEW, "CREATE OR REPLACE VIEW",
-- https://www.postgresql.org/docs/18/sql-createview.html).
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
