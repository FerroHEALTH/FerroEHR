-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the domain's own multimedia blob reference index.
--
-- A party body carries `DV_MULTIMEDIA` like a clinical one — a photograph on a
-- PERSON, a scanned document on an ORGANISATION — so the same externalization
-- and the same garbage collection reach this domain, and the index the
-- collector reads must live here rather than in the clinical schema: a row
-- naming a party's versioned object in the clinical schema would put a
-- demographic identifier where the clinical role can read it (GDPR Art. 4(5),
-- docs/law/eu/gdpr/text.html).
--
-- The table is the clinical `blob_ref` relation, same shape and same foreign
-- key, so one write path serves both domains. Our own extension — no openEHR
-- spec governs multimedia offload.
--
-- Runs with search_path = party, ext, public.
CREATE TABLE blob_ref (
    -- The storage tier, kept in lockstep with the version row by the foreign
    -- key's ON UPDATE CASCADE, exactly as `node` is.
    tier        text NOT NULL DEFAULT 'hot',
    vo_id       uuid NOT NULL,
    sys_version integer NOT NULL,
    -- The referenced blob's URI, as the stored body spells it.
    uri         text NOT NULL,
    CONSTRAINT pk_blob_ref PRIMARY KEY (tier, vo_id, sys_version, uri),
    CONSTRAINT ck_blob_ref_tier CHECK (tier IN ('hot', 'cold')),
    CONSTRAINT fk_blob_ref_version FOREIGN KEY (tier, vo_id, sys_version)
        REFERENCES version (tier, vo_id, sys_version)
        ON DELETE CASCADE ON UPDATE CASCADE
);

-- The collector's question, over both tiers: is this URI still referenced.
CREATE INDEX idx_blob_ref_uri ON blob_ref (uri);

COMMENT ON TABLE blob_ref IS 'Which stored party version references which externalized multimedia blob, so blob garbage collection is a lookup rather than a scan of node. The clinical relation''s twin, because a blob shared with a party must survive the deletion of the clinical record that also referenced it. Our own extension.';
COMMENT ON COLUMN blob_ref.uri IS 'The referenced blob''s URI, as the stored body spells it.';
