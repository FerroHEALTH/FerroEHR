-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the EHDS priority category of each access.
--
-- Annex II 3.2(c) of Regulation (EU) 2025/327 asks the logging component to
-- record "the categories of data accessed", and Art. 14(1) names the priority
-- categories (docs/law/eu/ehds/text.html). AQL selects archetypes and
-- templates, never categories, so the server classifies each access at the
-- moment of access through a map the deployment declares ([audit.categories]),
-- and these columns carry the outcome beside the map that produced it.
--
-- Classification at access, never at commit: the map is a deployment's
-- reading of its own templates, not a fact of the content, and a stamp written
-- at commit would go stale the moment the map changed. The digest column says
-- which map classified a record, so a later map never re-reads an old one.
--
-- NULL means "not classified": an operation that touched no clinical content
-- (authentication, template provisioning, node management), and every record
-- written before this migration. An access the map could not classify is not
-- NULL; it carries the category `unclassified` with its identifiers as
-- evidence.
--
-- Append-only needs no new machinery: the BEFORE UPDATE trigger the tamper
-- chain installs compares the whole row minus the two delivery stamps, and the
-- FHIR document the chain digests carries the same classification as entity
-- details. The table-level grants of the audit grants file cover new columns.
--
-- No openEHR spec governs the classification: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event
    -- The category spellings, sorted: patient-summary, eprescription,
    -- edispensation, imaging, test-results, discharge-report,
    -- national:<code>, none, unclassified.
    ADD COLUMN categories          jsonb,
    -- What the classification rests on.
    ADD COLUMN category_basis      text,
    -- The template ids, archetype ids or RM class names it rests on, capped.
    ADD COLUMN category_evidence   jsonb,
    -- The digest of the map in force when the record was written.
    ADD COLUMN category_map_digest text;

ALTER TABLE audit_event
    ADD CONSTRAINT ck_audit_event_categories
        CHECK (categories IS NULL OR jsonb_typeof(categories) = 'array'),
    ADD CONSTRAINT ck_audit_event_category_basis
        CHECK (category_basis IS NULL
               OR category_basis IN ('template', 'archetype', 'query', 'resource-kind')),
    ADD CONSTRAINT ck_audit_event_category_evidence
        CHECK (category_evidence IS NULL OR jsonb_typeof(category_evidence) = 'array'),
    -- A classification is the categories, the basis and the evidence together;
    -- a record never carries one without the others.
    ADD CONSTRAINT ck_audit_event_category_whole
        CHECK ((categories IS NULL) = (category_basis IS NULL)
               AND (categories IS NULL) = (category_evidence IS NULL));

COMMENT ON COLUMN audit_event.categories IS
    'The EHDS Art. 14(1) priority categories of the data the access served or wrote (Annex II 3.2(c)), as a sorted JSON array of spellings; unclassified when the map could not classify it; NULL when the operation touched no clinical content or the record predates the column.';
COMMENT ON COLUMN audit_event.category_basis IS
    'What the classification rests on: template, archetype, query or resource-kind.';
COMMENT ON COLUMN audit_event.category_evidence IS
    'The identifiers the classification rests on (template ids, archetype ids, RM class names), as a JSON array, capped.';
COMMENT ON COLUMN audit_event.category_map_digest IS
    'The digest of the [audit.categories] map in force when the record was written; NULL on records that predate the column.';

-- "Who accessed this person's test results" filters the patient index's result
-- by category and needs no index of its own.
