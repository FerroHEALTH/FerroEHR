-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the restriction register, the retention register, and the anchors
-- and holds that decide what is due.
--
-- The relations are the evidence and the register; the marks they drive sit on
-- `vo_head.restricted_at` / `vo_head.retention_hold_at` and on `ehr`, and the
-- service reads them through `crate::storage::marks`.
--
-- No openEHR spec governs restriction of processing or retention: our own
-- design/extension. The obligations the relations exist to answer are GDPR
-- Art. 18 (restriction of processing), Art. 5(1)(e) (storage limitation) and
-- Art. 30(1)(f) (the envisaged erasure periods in the record of processing
-- activities), https://eur-lex.europa.eu/eli/reg/2016/679/oj. Note what the
-- openEHR side does NOT provide: EHR_STATUS.is_queryable is not a restriction
-- mark — RM ehr master04-ehr_package.adoc §EHR Status limits it to population
-- queries.
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
    CONSTRAINT pk_retention_policy PRIMARY KEY (kind, jurisdiction),
    CONSTRAINT ck_retention_policy_kind CHECK
        (kind IN ('COMPOSITION', 'EHR_STATUS', 'FOLDER', 'EHR')),
    CONSTRAINT ck_retention_policy_anchor CHECK
        (anchor IN ('last_commit', 'death', 'majority')),
    CONSTRAINT ck_retention_policy_period_positive CHECK (period > interval '0')
);

COMMENT ON TABLE retention_policy IS 'The retention period per content category and jurisdiction, with the legal citation it rests on. GDPR Art. 5(1)(e) and Art. 30(1)(f); our own design/extension.';
COMMENT ON COLUMN retention_policy.anchor IS 'What the period is measured from: the last commit, the subject''s death, or their majority.';
COMMENT ON COLUMN retention_policy.source IS 'The legal citation the period comes from, quoted as the vendored act spells it.';

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

-- ── retention_due ────────────────────────────────────────────────────────────
-- The EHRs whose period has run and which carry no EHR-wide hold, one row per
-- retention category in force. A VIEW, never a job: listing what is due is a
-- question the controller answers, and the CDR deletes nothing on a timer.
--
-- Two holds exist and both are honoured here. The EHR-wide one
-- (`retention_anchor.hold_at`) keeps the EHR off the list entirely. The
-- per-object one (`vo_head.retention_hold_at`) exempts single objects while the
-- rest of the EHR falls due, which is the shape EPDV Art. 10 Abs. 2 lit. b asks
-- for — the patient may have "Daten von der Vernichtung nach Absatz 1 Buchstabe
-- d ausgenommen" (docs/law/ch/epdv/text-de.html). So the row carries both
-- counts: a controller acting on this list disposes of `objects_due` and leaves
-- `objects_held` where they are.
CREATE VIEW retention_due WITH (security_invoker = true) AS
    SELECT a.ehr_id,
           a.jurisdiction,
           p.kind,
           p.source,
           a.anchored_at + p.period AS due_at,
           o.objects_due,
           o.objects_held
    FROM retention_anchor a
    JOIN retention_policy p
      ON p.jurisdiction = a.jurisdiction
    CROSS JOIN LATERAL (
        SELECT count(*) FILTER (WHERE h.retention_hold_at IS NULL) AS objects_due,
               count(*) FILTER (WHERE h.retention_hold_at IS NOT NULL) AS objects_held
          FROM vo_head h
         WHERE h.ehr_id = a.ehr_id
           AND (p.kind = 'EHR' OR h.kind = p.kind)
    ) o
    WHERE a.anchored_at IS NOT NULL
      AND a.hold_at IS NULL
      AND a.anchored_at + p.period <= now();

COMMENT ON VIEW retention_due IS 'The EHRs whose retention period has run and which carry no EHR-wide hold, with the citation the period rests on and the per-object hold counts. A list, never a disposal: the CDR deletes no clinical content on a timer (RM common master06 §Logical Deletion).';
