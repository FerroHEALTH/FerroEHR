-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- linkage: the one cross-reference relation — which party, and which subject
-- identifier, name which EHR, temporally.
--
-- This is the SM's EHR Index (SM openehr_platform master02-overview.adoc: "EHR
-- Index | EHR id / demographic subject cross-reference service"; the operations
-- are I_EHR_INDEX). master07-ehr_index_service.adoc §Overview says why the
-- relation belongs here rather than in the clinical schema: "In a
-- privacy-supporting environment, this enables EHRs to be persisted with only
-- an EHR id; the EHR Index has to be used to obtain the subject identifier,
-- which will usually be used as a key into a demographic or MPI service".
--
-- Identifiers and the SM's own association metadata only — no name, no address,
-- no plaintext national identifier — because a row here is already the
-- additional information that re-joins a record to a person (GDPR Art. 4(5),
-- https://eur-lex.europa.eu/eli/reg/2016/679/oj), and adding an attribute would
-- make it identifying on its own.
--
-- A row names a party, a subject identifier, or both:
--
--   * party_id + ehr_id            — the demographic party is the subject of
--                                    the EHR (opened by the linkage service).
--   * subject_id + ehr_id          — an EHR Index association (I_EHR_INDEX),
--                                    carrying RESOURCE_STATUS and LOCATION_DESC.
--   * party_id + subject_id + ehr_id — both, once the party's server-minted
--                                    pseudonym has been written onto EHR_STATUS.
--
-- No openEHR spec governs the storage layout: our own design/extension.
--
-- Runs with search_path = linkage, ext, public.
CREATE TABLE subject_ehr (
    -- Surrogate key. The two real keys below are each partial — one applies to
    -- the rows that name a party, the other to the associations in force — and
    -- PostgreSQL has no partial primary key, so the table carries a surrogate
    -- rather than pretending one of them is total.
    id                uuid      NOT NULL DEFAULT uuidv7(),
    -- The party's versioned-object id in the party domain, when the row names
    -- one.
    --
    -- No foreign key, for the reason the whole schema exists: a reference into
    -- the party schema would couple this table to a schema whose role cannot
    -- read it and whose role cannot read this one, and PostgreSQL enforces a
    -- foreign key by reading the referenced row.
    party_id          uuid,
    -- The EHR. No foreign key into the clinical schema either, and for the
    -- stronger form of the same reason: that one would cross the clinical
    -- boundary. linkage.erase_ehr is what closes the gap the missing cascade
    -- leaves.
    ehr_id            uuid      NOT NULL,
    -- The subject identifier the clinical side carries in
    -- EHR_STATUS.subject.external_ref — an opaque pseudonym wherever the
    -- deployment declares subject namespaces — with its namespace and the
    -- OBJECT_REF.type it is referenced as (i_ehr_index.adoc a_subject_id:
    -- OBJECT_REF). NULL on a row that names only a party.
    subject_id        text,
    subject_namespace text,
    subject_type      text,
    -- RESOURCE_STATUS (resource_status.adoc) as canonical JSON: instance_type
    -- plus the optional validity bounds and notes. One column rather than four,
    -- because the service reads and writes it as one structure and nothing here
    -- filters on its parts.
    status            jsonb,
    -- LOCATION_DESC (location_desc.adoc), as canonical JSON: "dynamic location
    -- information for EHRs" (master07 §Overview).
    location          jsonb,
    -- Validity interval of the row, half-open [opened, closed). An open upper
    -- bound is the mapping in force now. Committal time is the only
    -- server-managed temporal axis openEHR speaks about (RM common
    -- master06-change_control_package.adoc §Committal and Audits); the interval
    -- itself is our own storage design.
    sys_period        tstzrange NOT NULL DEFAULT tstzrange(now(), NULL, '[)'),
    CONSTRAINT pk_subject_ehr PRIMARY KEY (id),
    -- A row that names neither a party nor a subject names nothing.
    CONSTRAINT ck_subject_ehr_names_something
        CHECK (party_id IS NOT NULL OR subject_id IS NOT NULL),
    -- A subject identifier without its namespace is not resolvable, and a
    -- namespace without an identifier addresses nothing.
    CONSTRAINT ck_subject_ehr_subject_pair
        CHECK ((subject_id IS NULL) = (subject_namespace IS NULL)),
    -- RESOURCE_INSTANCE_TYPE (resource_instance_type.adoc) is a closed
    -- enumeration, so the database refuses a fourth value rather than trusting
    -- whichever code path happens to write.
    CONSTRAINT ck_subject_ehr_instance_type
        CHECK (status IS NULL
               OR status ->> 'instance_type' IN ('Primary', 'Duplicate', 'Supplementary')),
    -- One party has at most ONE mapping in force at any instant, enforced by
    -- the database rather than by whichever code path happens to write.
    -- PostgreSQL 18 builds a GiST index for a key carrying WITHOUT OVERLAPS,
    -- and the equality key part needs the btree_gist operator classes
    -- (PostgreSQL 18, CREATE TABLE, "UNIQUE",
    -- https://www.postgresql.org/docs/18/sql-createtable.html); the extension
    -- is installed in ext by the bootstrap.
    --
    -- UNIQUE rather than PRIMARY KEY because party_id is nullable: an EHR Index
    -- association that names no party must still be storable, and an exclusion
    -- constraint never conflicts on a NULL key part.
    --
    -- The change-control relations pay a plain btree primary key instead,
    -- because they are the hot write path. This table is written once per EHR
    -- and again only on a merge, a split or an index correction, so the
    -- constraint costs nothing measurable here and the invariant is worth more.
    CONSTRAINT uq_subject_ehr_party UNIQUE (party_id, sys_period WITHOUT OVERLAPS)
);

-- The EHR Index association identity, among the rows in force: one association
-- per (EHR, subject) pair. Deliberately NOT unique per subject and not unique
-- per EHR — master07 §Overview: "There is no limit on the number of subject
-- identifiers associated with a given EHR id, and vice versa, since in real
-- environments both situations commonly occur", and the two N:M states are what
-- the index's own metadata exists "to detect and rectify".
CREATE UNIQUE INDEX uq_subject_ehr_association ON subject_ehr
    (ehr_id, subject_id, subject_namespace)
    WHERE subject_id IS NOT NULL AND upper_inf(sys_period);

-- The two directions the service reads: everything about one EHR (erasure, the
-- subjects of an EHR) and everything about one subject.
CREATE INDEX idx_subject_ehr_by_ehr ON subject_ehr (ehr_id);
CREATE INDEX idx_subject_ehr_by_subject ON subject_ehr (subject_id, subject_namespace)
    WHERE subject_id IS NOT NULL;

COMMENT ON TABLE subject_ehr IS 'The EHR id / demographic subject cross-reference (SM openehr_platform master02; I_EHR_INDEX): which party and which subject identifier name which EHR, temporally. A merge or a split closes a row and opens another rather than deleting. Holds identifiers and the SM association metadata only, because a row here is already the additional information that re-joins a record to a person (GDPR Art. 4(5)).';
COMMENT ON COLUMN subject_ehr.party_id IS 'The party in the party domain, when the row names one; no foreign key, because the referenced schema is unreadable to this role by design.';
COMMENT ON COLUMN subject_ehr.subject_id IS 'The subject identifier the clinical side carries in EHR_STATUS.subject.external_ref — an opaque pseudonym wherever the deployment declares subject namespaces.';
COMMENT ON COLUMN subject_ehr.status IS 'RESOURCE_STATUS (resource_status.adoc) as canonical JSON: instance_type, the optional validity bounds and notes.';
COMMENT ON COLUMN subject_ehr.location IS 'LOCATION_DESC (location_desc.adoc) as canonical JSON — the "dynamic location information for EHRs" of master07 §Overview.';
COMMENT ON COLUMN subject_ehr.sys_period IS 'Validity interval of the row, half-open [opened, closed); an open upper bound is the mapping in force now. Part of the temporal key, so one party cannot hold two mappings at one instant.';
