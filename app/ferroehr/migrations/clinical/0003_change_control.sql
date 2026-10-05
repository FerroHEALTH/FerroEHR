-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- Change control: the commit audit, the CONTRIBUTION envelope, the append-only
-- version store, the one mutable head row per versioned object, and the
-- attestations appended to a version.
--
-- RENDERED FROM app/ferroehr/migrations/templates/change_control.sql.in by
-- crate::storage::ddl_template. Both pseudonymisation domains carry the same
-- change-control relations, so one template renders both and neither can drift
-- from the other; edit the template, never this file.
--
-- openEHR defines no SQL schema: the relational layout is our own design. What
-- the openEHR specifications DO define — the version tree, the CONTRIBUTION as
-- a nested transaction, the audit copied into every version, logical deletion —
-- is what the constraints and comments below cite (RM common
-- master06-change_control_package.adoc). Decomposing a VERSIONED_OBJECT into
-- rows is explicitly sanctioned there (§Overview: "Although the figure implies
-- physical containment of Versions by a Versioned object, this is only one
-- possible implementation. Other implementations (e.g. using orthodox
-- relational structures) might use references, separate compressed copies, or
-- any other mechanism.").
--
-- Runs with the domain's own search_path, so every relation below is created
-- and referenced unqualified.

-- ── commit_audit ─────────────────────────────────────────────────────────────
-- AUDIT_DETAILS of every committed change (RM common
-- master04-generic_package.adoc §Audit Details: system_id / time_committed /
-- change_type / committer 1..1, description 0..1), including the ATTESTATION
-- subtype a version may be committed with (master06 §Attestation).
CREATE TABLE commit_audit (
    id             uuid NOT NULL DEFAULT uuidv7(),
    -- RM common master06 §Committal and Audits: time_committed "should reflect
    -- the time of committal to an EHR server ... It should therefore be
    -- computed on the server". Never client-supplied.
    time_committed timestamptz NOT NULL DEFAULT now(),
    system_id      text NOT NULL,
    -- The openEHR `audit change type` group code — the code_string of
    -- AUDIT_DETAILS.change_type (Change_type_valid, RM common
    -- UML/classes/org.openehr.rm.common.audit_details.adoc §Invariants). The
    -- codes are TERM SupportTerminology
    -- codesets/openehr_terminology-vocabularies.adoc §Audit Change Type.
    change_type    text NOT NULL,
    -- AUDIT_DETAILS.description is a DV_TEXT (0..1) whose DV_CODED_TEXT subtype
    -- carries a defining_code, so the whole canonical fragment is stored — a
    -- bare text column would discard the concrete _type and every coded
    -- attribute.
    description    jsonb COMPRESSION lz4,
    -- The canonical PARTY_PROXY of the committer (AUDIT_DETAILS.committer
    -- 1..1). PostgreSQL grammar: the COMPRESSION clause precedes column
    -- constraints.
    committer      jsonb COMPRESSION lz4 NOT NULL,
    -- The ATTESTATION-declared attributes (attested_view, proof, items, reason,
    -- is_pending — RM common UML/classes/org.openehr.rm.common.attestation.adoc
    -- §Attributes) when this commit audit is an ATTESTATION rather than a plain
    -- AUDIT_DETAILS ("ORIGINAL_VERSION.commit_audit is of type ATTESTATION
    -- rather than AUDIT_DETAILS", master06 §Attestation), else NULL.
    -- ATTESTATION is the only AUDIT_DETAILS subtype RM 1.2.0 declares and both
    -- attributes are mandatory on it, so presence is the concrete class.
    attestation    jsonb COMPRESSION lz4,
    CONSTRAINT pk_commit_audit PRIMARY KEY (id),
    -- AUDIT_DETAILS.System_id_valid: `not system_id.is_empty`.
    CONSTRAINT ck_commit_audit_system_id_nonempty CHECK (system_id <> ''),
    -- DV_TEXT.value is 1..1 (RM data_types
    -- UML/classes/org.openehr.rm.data_types.dv_text.adoc §Attributes).
    CONSTRAINT ck_commit_audit_description_shape CHECK
        (description IS NULL OR description ? 'value'),
    -- ATTESTATION.reason and ATTESTATION.is_pending are both 1..1.
    CONSTRAINT ck_commit_audit_attestation_shape CHECK
        (attestation IS NULL OR (attestation ? 'reason' AND attestation ? 'is_pending')),
    -- The full openEHR `audit change type` group: 249 creation, 250 amendment,
    -- 251 modification, 252 synthesis, 253 unknown, 523 deleted, 666
    -- attestation, 816 restoration, 817 format conversion (TERM
    -- SupportTerminology codesets/openehr_terminology-vocabularies.adoc §Audit
    -- Change Type), not the narrower subset master06 §Contributions enumerates
    -- in prose.
    CONSTRAINT ck_commit_audit_change_type CHECK (change_type IN
        ('249', '250', '251', '252', '253', '523', '666', '816', '817'))
);

-- The admin statistics' time-bounded paths filter and order on the commit
-- instant. No openEHR spec governs indexing: our own design.
CREATE INDEX idx_commit_audit_time_committed ON commit_audit (time_committed);

COMMENT ON TABLE commit_audit IS 'AUDIT_DETAILS of every committed change (RM common master04-generic_package.adoc §Audit Details), including the ATTESTATION subtype a version may be committed with (master06 §Attestation).';
COMMENT ON COLUMN commit_audit.time_committed IS 'Server-computed commit instant: RM common master06 §Committal and Audits — time_committed "should reflect the time of committal to an EHR server ... It should therefore be computed on the server".';
COMMENT ON COLUMN commit_audit.change_type IS 'openEHR `audit change type` group code (AUDIT_DETAILS.Change_type_valid); the CHECK carries the full group as published in TERM SupportTerminology codesets/openehr_terminology-vocabularies.adoc §Audit Change Type.';
COMMENT ON COLUMN commit_audit.committer IS 'Canonical PARTY_PROXY of the committer (AUDIT_DETAILS.committer 1..1).';
COMMENT ON COLUMN commit_audit.description IS 'Canonical AUDIT_DETAILS.description fragment (DV_TEXT or its DV_CODED_TEXT subtype), stored whole so the concrete _type and defining_code survive.';
COMMENT ON COLUMN commit_audit.attestation IS 'The ATTESTATION-declared attributes when the commit audit is an ATTESTATION, else NULL (RM common master06 §Attestation).';

-- ── contribution ─────────────────────────────────────────────────────────────
-- The change-set envelope (RM common master06 §Contributions: "a CONTRIBUTION
-- object will be created, listing the affected VERSION objects, and including
-- its own audit object"), strictly transactional (§Committal and Audits:
-- "An attempt to commit a Contribution should only succeed if each Version
-- and/or Attestation in the Contribution is committed successfully").
CREATE TABLE contribution (
    id              uuid NOT NULL DEFAULT uuidv7(),
    -- The owning EHR, or NULL where no EHR owns the change set (RM demographic
    -- content is not EHR-owned).
    ehr_id          uuid,
    commit_audit_id uuid NOT NULL,
    CONSTRAINT pk_contribution PRIMARY KEY (id),
    CONSTRAINT fk_contribution_ehr FOREIGN KEY (ehr_id) REFERENCES ehr (id) ON DELETE CASCADE,
    CONSTRAINT fk_contribution_commit_audit FOREIGN KEY (commit_audit_id)
        REFERENCES commit_audit (id)
);
CREATE INDEX idx_contribution_ehr_id ON contribution (ehr_id);
CREATE INDEX idx_contribution_commit_audit_id ON contribution (commit_audit_id);

COMMENT ON TABLE contribution IS 'The change-set envelope (RM common master06 §Contributions); one per change set, strictly transactional (§Committal and Audits: a Contribution commits only if every member Version/Attestation commits).';
COMMENT ON COLUMN contribution.ehr_id IS 'Owning EHR, or NULL where no EHR owns the change set.';

-- ── version ──────────────────────────────────────────────────────────────────
-- One WRITE-ONCE row per version of a versioned object. A version row and its
-- node rows are never updated after commit — BASE
-- architecture_overview/master07-security.adoc §Integrity states the property
-- the store realizes ("1 Version = 1 copy"); mutable state lives in the
-- vo_head row below. There is therefore no validity interval and no close-out
-- statement: the validity of version i is [committed_at_i, committed_at_i+1)
-- by construction, and version_at_time(t) is the trunk row with the greatest
-- committed_at <= t.
--
-- Version-tree model (RM common master06 §The 'Virtual Version Tree',
-- §Distributed versioning — "To support branching, a further pair of numbers
-- is added ... branching version identifiers [are required] when local
-- modifications are made to versions copied from elsewhere"):
--   * sys_version is an opaque per-object COMMIT ORDINAL (1..n across trunk
--     AND branch commits) — the node/vo_attestation key, NOT the wire version
--     number;
--   * the spec-facing VERSION_TREE_ID lives in trunk_version +
--     branch_number/branch_version (0/0 = a trunk row, >= 1 each on a branch);
--   * global uniqueness is the spec tuple {object_id, creating_system_id,
--     version_tree_id} (uq_version_tree).
--
-- PARTITION BY LIST (tier): archival is row movement inside one relation set
-- rather than a second, foreign-key-free mirror, so every foreign key and
-- every read path reaches the cold tier without a view. An UPDATE that changes
-- the partition key moves the row (PostgreSQL 18, "Partitioning",
-- https://www.postgresql.org/docs/18/ddl-partitioning.html). The partition key
-- is part of every unique constraint on a partitioned table
-- (https://www.postgresql.org/docs/18/sql-createtable.html), which is why the
-- primary key carries it. No default partition: a tier the schema does not
-- name must fail to insert rather than land somewhere unindexed.
--
-- No fillfactor: the rows are never updated, and "For a table whose entries
-- are never updated, complete packing is the best choice" (PostgreSQL 18,
-- CREATE TABLE, "fillfactor").
CREATE TABLE version (
    -- The storage tier, and the partition key: `hot` or `cold`.
    tier            text NOT NULL DEFAULT 'hot',
    vo_id           uuid NOT NULL,
    kind            text NOT NULL,
    ehr_id          uuid,
    sys_version     integer NOT NULL,
    -- VERSION_TREE_ID columns. On a trunk row trunk_version is the wire
    -- version number; on a branch row the trunk version the branch forks from.
    trunk_version   integer NOT NULL,
    branch_number   integer NOT NULL DEFAULT 0,
    branch_version  integer NOT NULL DEFAULT 0,
    -- ORIGINAL_VERSION.lifecycle_state: the numeric version_lifecycle_state
    -- code (532 complete, 553 incomplete, 523 deleted, 800 inactive, 801
    -- abandoned). A logical delete writes a content-less version with state 523
    -- (master06 §Logical Deletion) — never a physical delete. 553 relaxes
    -- content validity ("mandatory attributes may be absent ... data may be
    -- missing, but it may not be wrong", §Incomplete Content), so content is
    -- never NOT NULL.
    lifecycle_state text NOT NULL DEFAULT '532',
    -- The immutable identity of the system that CREATED this version: the
    -- middle segment of the OBJECT_VERSION_ID {object_id, creating_system_id,
    -- version_tree_id} (master06 §Distributed versioning). Reconstructed from
    -- storage, never re-derived from live configuration, so a configuration
    -- change cannot mutate a historical uid or invalidate a signature.
    creating_system_id text NOT NULL,
    -- ORIGINAL_VERSION.preceding_version_uid (0..1): the full
    -- OBJECT_VERSION_ID of the actual preceding version, stored at commit and
    -- preserved verbatim on import — it cannot be synthesized arithmetically
    -- once branches and imports exist. NULL for a first version.
    preceding_version_uid text,
    -- VERSION.signature (master06 §Digital Signature): 0..1, opaque radix-64.
    -- Canonicalisation is spec-TBD (master06 marks the exact serialisation "To
    -- Be Determined"). On an IMPORTED_VERSION row this is the WRAPPER's own
    -- signature, which "signifies the act of importing and making available
    -- locally an ORIGINAL_VERSION from another system".
    signature       text,
    -- Whether `signature` was supplied verbatim by the committing client (an
    -- author signature over another agreed serialisation) rather than
    -- generated by this server. Client-supplied signatures are stored
    -- byte-for-byte and never re-verified at read, because a foreign canonical
    -- form cannot be recomputed. No openEHR spec governs read-time
    -- verification timing: our own design.
    signature_client_supplied boolean NOT NULL DEFAULT false,
    -- IMPORTED_VERSION discriminator plus the wrapped ORIGINAL_VERSION's own
    -- wrapper-level provenance, held as verbatim canonical openEHR JSON:
    -- { "contribution": <OBJECT_REF>, "commit_audit": <AUDIT_DETAILS>,
    --   "signature": <String, optional> }. NULL means the row is a locally
    -- created ORIGINAL_VERSION. master06 §Committal and Audits: for a Version
    -- committed as an IMPORTED_VERSION "both the contribution and commit_audit
    -- of the latter object correspond to the local act of committal, while the
    -- knowledge of the original Contribution and committal are retained inside
    -- the wrapped ORIGINAL_VERSION instance". The wrapped contribution cannot
    -- be a foreign key: it names a CONTRIBUTION in the source system.
    wrapped_original jsonb COMPRESSION lz4,
    -- ORIGINAL_VERSION.other_input_version_uids: merge provenance (master06
    -- §Version Merging). NULL when the version is not a merge.
    other_input_version_uids jsonb,
    -- The distinct originating systems of this version's body, as a JSON array
    -- of FEEDER_AUDIT.originating_system_audit.system_id values found anywhere
    -- in it, or this server's own system id when it carries none; NULL when
    -- nothing stamped it (a verbatim-replay import or archive load), assessed
    -- at read. EHDS Annex II 3.2(e) (Regulation (EU) 2025/327,
    -- https://eur-lex.europa.eu/eli/reg/2025/327/oj); our own design.
    origins         jsonb,
    contribution_id uuid NOT NULL,
    -- This version's own commit_audit. The RM copies the contribution audit
    -- into every version (master06 §Committal and Audits), which is what makes
    -- committed_at below a property of the version row rather than a join.
    commit_audit_id uuid NOT NULL,
    template_id     text,
    -- Whether the RELEASED openEHR generation set can express this version's
    -- body — the stamp the read-time spec_profile gate consults. NULL = not
    -- stamped at write (a verbatim-replay import or archive-load row),
    -- assessed on the fly at read. No openEHR spec governs runtime generation
    -- selection: our own design/extension.
    stable_compatible boolean,
    -- The commit instant, copied from this version's commit_audit
    -- (master06 §Committal and Audits). The whole temporal axis of the store:
    -- version i is valid over [committed_at_i, committed_at_i+1).
    committed_at    timestamptz NOT NULL,
    -- The version's canonical body BYTES (the openehr-its canonical JSON
    -- encoding), materialized at write from the accepted, uid-stamped value
    -- before node decomposition, so a point read serves these bytes verbatim.
    -- Deliberately text, not jsonb: jsonb re-orders object keys internally and
    -- the served wire must keep the codec's _type-first, BMM-declared field
    -- order, which the ITS-REST datetime rule also depends on
    -- (specifications/docs/overview/Resources.md §Datetime format). NULL on a
    -- logically deleted version (no content, master06 §Logical Deletion).
    body            text COMPRESSION lz4,
    CONSTRAINT pk_version PRIMARY KEY (tier, vo_id, sys_version),
    -- The spec identity tuple {object_id, creating_system_id,
    -- version_tree_id} (master06 §Distributed Versioning). `tier` joins it
    -- because a unique constraint on a partitioned table must contain the
    -- partition key; an archived object moves whole, so no version of one
    -- object is ever split across tiers.
    CONSTRAINT uq_version_tree UNIQUE
        (tier, vo_id, creating_system_id, trunk_version, branch_number, branch_version),
    CONSTRAINT ck_version_tier CHECK (tier IN ('hot', 'cold')),
    CONSTRAINT ck_version_sys_version_positive CHECK (sys_version >= 1),
    CONSTRAINT ck_version_trunk_version_positive CHECK (trunk_version >= 1),
    -- A row is a trunk row (0, 0) or a branch row (>= 1, >= 1), never mixed
    -- (BASE VERSION_TREE_ID: `trunk_version [ '.' branch_number '.'
    -- branch_version ]`, both branch parts starting at 1).
    CONSTRAINT ck_version_branch_pair CHECK (
        (branch_number = 0 AND branch_version = 0)
        OR (branch_number >= 1 AND branch_version >= 1)
    ),
    CONSTRAINT ck_version_kind CHECK (kind IN ('COMPOSITION', 'EHR_STATUS', 'EHR_ACCESS', 'FOLDER')),
    CONSTRAINT ck_version_lifecycle_state CHECK
        (lifecycle_state IN ('532', '553', '523', '800', '801')),
    -- An IMPORTED_VERSION's wrapped original always carries the two mandatory
    -- VERSION attributes (contribution 1..1, commit_audit 1..1, RM common
    -- version.adoc §Attributes); `signature` is 0..1 and may be absent.
    CONSTRAINT ck_version_wrapped_original CHECK (
        wrapped_original IS NULL
        OR (jsonb_typeof(wrapped_original) = 'object'
            AND wrapped_original ? 'contribution'
            AND wrapped_original ? 'commit_audit')
    ),
    CONSTRAINT fk_version_ehr FOREIGN KEY (ehr_id) REFERENCES ehr (id) ON DELETE CASCADE,
    CONSTRAINT fk_version_contribution FOREIGN KEY (contribution_id)
        REFERENCES contribution (id),
    CONSTRAINT fk_version_commit_audit FOREIGN KEY (commit_audit_id)
        REFERENCES commit_audit (id)
) PARTITION BY LIST (tier);

CREATE TABLE version_hot  PARTITION OF version FOR VALUES IN ('hot');
CREATE TABLE version_cold PARTITION OF version FOR VALUES IN ('cold');

-- A TRUNK POSITION is unique across creating systems, while a BRANCH id is
-- not. master06 §Copying §Subsequent Local Modifications: "When new versions
-- are added locally to a copied object, the local system id is recorded in the
-- uid.creating_system_id() attribute, while branching numbering is used in the
-- uid.version_tree_id()" — a second system never extends the trunk of a copied
-- container, it branches; and §Moving Version Containers has the trunk
-- CONTINUE its increment under the new system's id. Declared per partition, so
-- it needs no tier column of its own.
CREATE UNIQUE INDEX uq_version_hot_trunk_position
    ON version_hot (vo_id, trunk_version) WHERE branch_number = 0;
CREATE UNIQUE INDEX uq_version_cold_trunk_position
    ON version_cold (vo_id, trunk_version) WHERE branch_number = 0;

-- The hot tier's access paths. The cold partition deliberately carries the
-- primary key and the tree uniqueness alone: archived content leaves the
-- queryable store, so the indexes that serve querying would be maintenance on
-- rows nothing scans.
--
-- version_at_time(t): the trunk row with the greatest committed_at <= t, one
-- descending probe.
CREATE INDEX idx_version_hot_trunk_at_time
    ON version_hot (vo_id, committed_at DESC) WHERE branch_number = 0;
-- The current tip of one lineage: the greatest commit ordinal within a branch
-- number, the trunk being branch number 0. This is what "is this version still
-- the tip" asks, and the placement read asks it on every supersession.
CREATE INDEX idx_version_hot_lineage_tip
    ON version_hot (vo_id, branch_number, sys_version DESC);
CREATE INDEX idx_version_hot_ehr ON version_hot (ehr_id, kind);
CREATE INDEX idx_version_hot_contribution ON version_hot (contribution_id);
CREATE INDEX idx_version_hot_commit_audit ON version_hot (commit_audit_id);
CREATE INDEX idx_version_hot_template ON version_hot (template_id)
    WHERE template_id IS NOT NULL;
-- Time-range listing (contributions in a window, revision histories) over an
-- append-only heap, where committed_at is physically correlated with insertion
-- order — the case BRIN is for (PostgreSQL 18, "BRIN Indexes",
-- https://www.postgresql.org/docs/18/brin-intro.html).
CREATE INDEX idx_version_hot_committed_brin
    ON version_hot USING brin (committed_at);

COMMENT ON TABLE version IS 'One write-once row per version of a versioned object (RM common master06 version tree); validity is derived from committed_at, so no row is ever updated after commit. Partitioned by storage tier so archival is row movement inside one relation set.';
COMMENT ON COLUMN version.tier IS 'The storage tier and the partition key: hot (served and queryable) or cold (archived). Our own design — no openEHR spec governs storage tiering.';
COMMENT ON COLUMN version.sys_version IS 'Opaque per-object commit ordinal (1..n across trunk AND branch commits) — the node/vo_attestation key. NOT the wire version number: the VERSION_TREE_ID lives in trunk_version/branch_number/branch_version.';
COMMENT ON COLUMN version.trunk_version IS 'VERSION_TREE_ID first part. On a trunk row the wire version number; on a branch row the trunk version the branch forks from.';
COMMENT ON COLUMN version.branch_number IS 'VERSION_TREE_ID second part; 0 = trunk row (RM common master06 §The ''Virtual Version Tree'').';
COMMENT ON COLUMN version.branch_version IS 'VERSION_TREE_ID third part; 0 = trunk row.';
COMMENT ON COLUMN version.committed_at IS 'The commit instant copied from this version''s commit_audit (RM common master06 §Committal and Audits). Version i is valid over [committed_at_i, committed_at_i+1) by construction.';
COMMENT ON COLUMN version.creating_system_id IS 'Immutable per-version creating-system id — the OBJECT_VERSION_ID middle segment (RM common master06 §Distributed versioning). Reconstructed from storage, never live configuration.';
COMMENT ON COLUMN version.preceding_version_uid IS 'ORIGINAL_VERSION.preceding_version_uid (0..1, a full OBJECT_VERSION_ID), stored at commit from the actual preceding row; NULL for a first version.';
COMMENT ON COLUMN version.lifecycle_state IS 'ORIGINAL_VERSION.lifecycle_state from the openEHR `version lifecycle state` group (RM common master06 §Version Lifecycle). 523 = logical delete (§Logical Deletion); 553 relaxes content validity (§Incomplete Content).';
COMMENT ON COLUMN version.signature IS 'VERSION.signature (0..1), opaque radix-64 — on an imported row the IMPORTED_VERSION wrapper''s own signature (RM common master06 §Digital Signature).';
COMMENT ON COLUMN version.signature_client_supplied IS 'True when signature arrived verbatim from the committing client (foreign, never re-verified at read); false for a server-generated signature. Our own design.';
COMMENT ON COLUMN version.wrapped_original IS 'IMPORTED_VERSION discriminator: NULL = a locally created ORIGINAL_VERSION; otherwise the wrapped ORIGINAL_VERSION''s own {contribution, commit_audit, signature?} verbatim, while the row''s own columns carry the LOCAL act of committal (RM common master06 §Committal and Audits, §Copying).';
COMMENT ON COLUMN version.other_input_version_uids IS 'ORIGINAL_VERSION merge provenance (RM common master06 §Version Merging); NULL when the version is not a merge.';
COMMENT ON COLUMN version.origins IS 'The distinct originating systems of this version''s body (FEEDER_AUDIT system ids), or this server''s own when it carries none; NULL when nothing stamped it. EHDS Annex II 3.2(e); our own design.';
COMMENT ON COLUMN version.stable_compatible IS 'Whether the RELEASED openEHR generation set can express this version''s body; NULL = unstamped, assessed at read. No openEHR spec governs runtime generation selection — our own design.';
COMMENT ON COLUMN version.body IS 'The version''s canonical openEHR JSON bytes verbatim, served by point reads without re-aggregation; text rather than jsonb so the codec''s field order survives (ITS-REST Resources.md §Datetime format).';

-- ── vo_head ──────────────────────────────────────────────────────────────────
-- The ONE mutable row per versioned object, and the only row a commit updates.
--
-- It answers every "what is current" question in one primary-key probe: the
-- version the ETag and If-Match are computed from, the latest trunk version
-- (LATEST_VERSION) and the latest version on any branch (the RM's
-- latest_version), the lifecycle state a repeated delete is refused by, the
-- template a template-scoped query filters on, and the two legal marks the
-- read paths consult.
--
-- The columns a commit changes — head_sys_version, trunk_head_sys_version,
-- lifecycle_state, committed_at — appear in NO index, which is what makes the
-- per-commit UPDATE heap-only-tuple eligible: PostgreSQL performs a HOT update
-- when "the update does not modify any columns referenced by the table's
-- indexes" and a new tuple fits on the same page (PostgreSQL 18, "Heap-Only
-- Tuples (HOT)", https://www.postgresql.org/docs/18/storage-hot.html). The
-- fillfactor below reserves that same-page room. The non-HOT updates left are
-- a template change, an archive and the legal marks, each rare by nature.
CREATE TABLE vo_head (
    vo_id                  uuid NOT NULL,
    kind                   text NOT NULL,
    ehr_id                 uuid,
    -- The tier the object's version and node rows sit in; moved in the same
    -- transaction as the rows themselves.
    tier                   text NOT NULL DEFAULT 'hot',
    -- The greatest sys_version on ANY lineage — the RM's latest_version
    -- (master06 §The 'Virtual Version Tree').
    head_sys_version       integer NOT NULL,
    -- The greatest sys_version on the TRUNK — LATEST_VERSION. NULL is
    -- unreachable for a committed object and is left nullable only so a future
    -- branch-only object needs no schema change.
    trunk_head_sys_version integer,
    -- The lifecycle state of the trunk head, promoted so the "already deleted"
    -- refusal needs no version read.
    lifecycle_state        text NOT NULL,
    -- The template of the trunk head, promoted for template-scoped queries.
    template_id            text,
    -- The trunk head's commit instant.
    committed_at           timestamptz NOT NULL,
    -- Restriction of processing: set while the object may be stored but not
    -- otherwise processed. GDPR Art. 18(2) leaves only storage
    -- (https://eur-lex.europa.eu/eli/reg/2016/679/oj); the register that
    -- records the ground is a separate relation, and this column is the mark
    -- every read path filters on. No openEHR spec governs restriction of
    -- processing: our own design/extension.
    restricted_at          timestamptz,
    -- A retention hold: the object is exempt from any retention disposal while
    -- this is set. Our own design/extension.
    retention_hold_at      timestamptz,
    -- When the object was moved to the cold tier, and the caller's reason. The
    -- SM's archive call takes a reason (I_ADMIN_ARCHIVE, "Move ... to archival
    -- storage"), and `tier` alone cannot carry it; both are NULL while the
    -- object is hot. Our own design/extension — no openEHR spec governs
    -- storage tiering.
    archived_at            timestamptz,
    archive_reason         text,
    CONSTRAINT pk_vo_head PRIMARY KEY (vo_id),
    CONSTRAINT fk_vo_head_ehr FOREIGN KEY (ehr_id) REFERENCES ehr (id) ON DELETE CASCADE,
    CONSTRAINT ck_vo_head_tier CHECK (tier IN ('hot', 'cold')),
    CONSTRAINT ck_vo_head_kind CHECK (kind IN ('COMPOSITION', 'EHR_STATUS', 'EHR_ACCESS', 'FOLDER')),
    CONSTRAINT ck_vo_head_head_positive CHECK (head_sys_version >= 1),
    CONSTRAINT ck_vo_head_lifecycle_state CHECK
        (lifecycle_state IN ('532', '553', '523', '800', '801'))
) WITH (fillfactor = 90);

-- The two access paths that are not the primary key: an EHR's objects of one
-- kind (status and directory reads, EHR summaries, the persistent-duplicate
-- probe) and the template-scoped listing. Neither names a column a commit
-- updates, so neither de-optimises the HOT path.
CREATE INDEX idx_vo_head_ehr_kind ON vo_head (ehr_id, kind);
CREATE INDEX idx_vo_head_ehr_template ON vo_head (ehr_id, template_id)
    WHERE template_id IS NOT NULL;

COMMENT ON TABLE vo_head IS 'The one mutable row per versioned object: the current trunk and any-branch heads, the lifecycle state, the tier, and the restriction and retention marks. Its updated columns are in no index, so a commit updates it heap-only (PostgreSQL 18, "Heap-Only Tuples (HOT)").';
COMMENT ON COLUMN vo_head.head_sys_version IS 'The greatest sys_version on any lineage — the RM''s latest_version (RM common master06 §The ''Virtual Version Tree'').';
COMMENT ON COLUMN vo_head.trunk_head_sys_version IS 'The greatest sys_version on the trunk — LATEST_VERSION.';
COMMENT ON COLUMN vo_head.restricted_at IS 'When restriction of processing was recorded for this object (GDPR Art. 18(2) leaves only storage); NULL = unrestricted. Our own design/extension — no openEHR spec governs restriction.';
COMMENT ON COLUMN vo_head.retention_hold_at IS 'When a retention hold was placed on this object, exempting it from disposal; NULL = no hold. Our own design/extension.';
COMMENT ON COLUMN vo_head.archived_at IS 'When the object was moved to the cold tier; NULL while it is hot. Realizes the SM I_ADMIN_ARCHIVE marker — our own design/extension, no openEHR spec governs storage tiering.';
COMMENT ON COLUMN vo_head.archive_reason IS 'The reason the caller gave for archiving (SM I_ADMIN_ARCHIVE); NULL while the object is hot or when no reason was given.';

-- ── vo_attestation ───────────────────────────────────────────────────────────
-- ATTESTATION storage (RM common master06 §Attestation; the class is
-- master04-generic_package.adoc §Attestation): a new ATTESTATION is appended
-- to an existing ORIGINAL_VERSION's attestations list WITHOUT a new version
-- ("Attestations can be added at any time after committal of the content being
-- attested"), committed through a contribution. Partitioned by tier with the
-- versions it attests.
CREATE TABLE vo_attestation (
    tier            text NOT NULL DEFAULT 'hot',
    id              uuid NOT NULL DEFAULT uuidv7(),
    vo_id           uuid NOT NULL,
    sys_version     integer NOT NULL,
    contribution_id uuid NOT NULL,
    time_committed  timestamptz NOT NULL DEFAULT now(),
    -- Whether this ATTESTATION was present ON THE VERSION at the act of
    -- committal (master06 §Attestation, "Signing content at committal"; SM
    -- UML/classes/update_version.adoc carries `attestations` on the
    -- client-supplied UPDATE_VERSION) rather than added later. The signed
    -- canonical form of a VERSION is "the entire Version object" minus
    -- `signature` (master06 §Digital Signature), so an at-committal
    -- attestation is inside it and an after-committal one is outside.
    at_committal    boolean NOT NULL,
    data            jsonb NOT NULL,
    CONSTRAINT pk_vo_attestation PRIMARY KEY (tier, id),
    CONSTRAINT ck_vo_attestation_tier CHECK (tier IN ('hot', 'cold')),
    -- ON UPDATE CASCADE carries the attestations across the tier with the
    -- version they attest, which is what makes archival one statement.
    CONSTRAINT fk_vo_attestation_version FOREIGN KEY (tier, vo_id, sys_version)
        REFERENCES version (tier, vo_id, sys_version)
        ON DELETE CASCADE ON UPDATE CASCADE,
    CONSTRAINT fk_vo_attestation_contribution FOREIGN KEY (contribution_id)
        REFERENCES contribution (id)
) PARTITION BY LIST (tier);

CREATE TABLE vo_attestation_hot  PARTITION OF vo_attestation FOR VALUES IN ('hot');
CREATE TABLE vo_attestation_cold PARTITION OF vo_attestation FOR VALUES IN ('cold');

CREATE INDEX idx_vo_attestation_hot_version
    ON vo_attestation_hot (vo_id, sys_version);
CREATE INDEX idx_vo_attestation_cold_version
    ON vo_attestation_cold (vo_id, sys_version);
CREATE INDEX idx_vo_attestation_hot_contribution
    ON vo_attestation_hot (contribution_id);

COMMENT ON TABLE vo_attestation IS 'ATTESTATIONs appended to a version without a new version (RM common master06 §Attestation); canonical ATTESTATION verbatim in data. Partitioned by tier with the versions it attests.';
COMMENT ON COLUMN vo_attestation.at_committal IS 'True when the ATTESTATION was on the VERSION at the act of committal and is therefore INSIDE its signed canonical form (RM common master06 §Attestation, §Digital Signature); false for one added afterwards.';
