-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the categories of an access as ehds-logging classifies them.
--
-- The EHDS access record's categories (Regulation (EU) 2025/327 Annex II
-- 3.2(c), docs/law/eu/ehds/text.html) are classified by the shared crate
-- ehds-logging (FerroAUDIT). A record written from now on carries each priority
-- category of Art. 14(1) as its `<system>|<code>` token in HL7 Europe's
-- EEHRxFDocumentPriorityCategoryCS, each national category as the
-- `<system>|<code>` the deployment declares, `none` and `unclassified` as
-- before, and says what the classification was read from: the objects the
-- access returned or wrote, the query's constraints, or the resource kind.
-- A record already written keeps the spellings and the basis it was written
-- with: the table is append-only and the chain digest covers both columns.
--
-- No openEHR spec governs audit storage: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event
    DROP CONSTRAINT ck_audit_event_category_basis,
    ADD CONSTRAINT ck_audit_event_category_basis
        CHECK (category_basis IS NULL
               OR category_basis IN ('returned', 'written', 'queried', 'resource-kind',
                                     'template', 'archetype', 'query'));

COMMENT ON COLUMN audit_event.categories IS
    'The EHDS Art. 14(1) categories of the data the access served or wrote (Annex II 3.2(c)), as a sorted JSON array: each category as its <system>|<code> token, none for data of no category, unclassified when the map could not classify all of it; records written before 0012 carry the earlier spellings; NULL when the operation touched no clinical content or the record predates the column.';
COMMENT ON COLUMN audit_event.category_basis IS
    'What the classification was read from: returned, written, queried or resource-kind; records written before 0012 carry template, archetype, query or resource-kind.';
COMMENT ON COLUMN audit_event.category_evidence IS
    'The template ids, archetype ids (in comparison form) and RM class names the classification read, as a JSON array, capped.';
