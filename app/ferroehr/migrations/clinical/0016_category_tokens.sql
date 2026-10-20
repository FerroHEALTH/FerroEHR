-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the retention register keys a category on its ehds-logging token.
--
-- The [audit.categories] map is now classified by the shared crate
-- ehds-logging (FerroAUDIT), which names each priority category of
-- Regulation (EU) 2025/327 Art. 14(1) by its code in HL7 Europe's
-- EEHRxFDocumentPriorityCategoryCS and each national category by the
-- <system>|<code> the deployment declares (docs/law/eu/ehds/text.html). The
-- boot mirror (`category_map`) and a retention period keyed on a category
-- (`retention_policy.category`, Annex II 3.4) carry that `<system>|<code>`
-- token, or `none`.
--
-- The mirror is replaced whole at every boot. A period already keyed on one of
-- the six priority spellings moves to its token here. A period keyed on a
-- `national:<code>` spelling names no system, so it cannot be moved: the
-- migration refuses, and the operator deletes the period and declares it again
-- under the national category's `<system>|<code>`.
--
-- No openEHR spec governs the register: our own design/extension.
--
-- Runs with search_path = clinical, ext, public.

DO $$
BEGIN
    IF EXISTS (SELECT FROM retention_policy WHERE category LIKE 'national:%') THEN
        RAISE EXCEPTION 'retention_policy holds a period keyed on a national:<code> category, '
            'which names no code system; delete it and declare it again under the national '
            'category''s <system>|<code> as [audit.categories] national declares it';
    END IF;
END $$;

ALTER TABLE category_map
    DROP CONSTRAINT ck_category_map_category;
ALTER TABLE retention_policy
    DROP CONSTRAINT ck_retention_policy_category;

DELETE FROM category_map;

UPDATE retention_policy
   SET category = 'http://hl7.eu/fhir/health-data-api/CodeSystem/eehrxf-document-priority-category-cs|'
       || CASE category
              WHEN 'patient-summary' THEN 'Patient-Summaries'
              WHEN 'eprescription' THEN 'Electronic-Prescriptions'
              WHEN 'edispensation' THEN 'Electronic-Dispensations'
              WHEN 'imaging' THEN 'Medical-Imaging'
              WHEN 'test-results' THEN 'Laboratory-Reports'
              WHEN 'discharge-report' THEN 'Discharge-Reports'
          END
 WHERE category IN ('patient-summary', 'eprescription', 'edispensation', 'imaging',
                    'test-results', 'discharge-report');

-- A token is an absolute URI (an RFC 3986 scheme and no `|`), a `|`, and a
-- FHIR R4 code: the form ehds-logging writes.
ALTER TABLE category_map
    ADD CONSTRAINT ck_category_map_category CHECK (
        category = 'none'
        OR category ~ '^[A-Za-z][A-Za-z0-9+.-]*:[^|[:space:]]+\|[^[:space:]](.*[^[:space:]])?$'
    );
ALTER TABLE retention_policy
    ADD CONSTRAINT ck_retention_policy_category CHECK (
        category IS NULL
        OR category = 'none'
        OR category ~ '^[A-Za-z][A-Za-z0-9+.-]*:[^|[:space:]]+\|[^[:space:]](.*[^[:space:]])?$'
    );

COMMENT ON TABLE category_map IS
    'The boot mirror of the [audit.categories] map: template or archetype id (case-folded) to EHDS Art. 14(1) category as its <system>|<code> token, or none. Replaced whole at every boot; our own design/extension.';
COMMENT ON COLUMN retention_policy.category IS
    'The EHDS category the period is keyed on (Annex II 3.4), as its <system>|<code> token, or NULL for every object of the kind.';
