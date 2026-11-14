-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: drop the EHDS Art. 8 restriction and Art. 10 opt-out registers
-- (#5510).
--
-- The person's restriction of access by health professionals (Regulation (EU)
-- 2025/327 Art. 8) and opt-out from the access services (Art. 10) are now
-- `restriction` and `opt_out` directives of the `access_directives` settings
-- in the EHR's own EHR_ACCESS, versioned with it (RM ehr `ehr_access.adoc`
-- §EHR_ACCESS Class) and carried by an EHR Extract. The two registers are
-- dropped with no conversion. The marks the read paths filter on stay:
-- `ehr.access_restricted_at`, `vo_head.access_restricted_at` and
-- `ehr.opted_out_at` become the projection of the directives in force,
-- written in the transaction of every EHR_ACCESS commit
-- (`crate::service::ehr::access_marks`). They are cleared here, since the
-- registers that set them are gone, and the next boot projects them again from
-- every stored EHR_ACCESS.
--
-- No openEHR spec governs the registers, the directives or the projection: our
-- own design.
--
-- Runs with search_path = clinical, ext, public.

DROP TABLE access_restriction;
DROP TABLE access_opt_out;

UPDATE ehr SET access_restricted_at = NULL, opted_out_at = NULL
 WHERE access_restricted_at IS NOT NULL OR opted_out_at IS NOT NULL;
UPDATE vo_head SET access_restricted_at = NULL WHERE access_restricted_at IS NOT NULL;

COMMENT ON COLUMN ehr.access_restricted_at IS 'When a whole-EHR EHDS Art. 8 restriction directive of the current EHR_ACCESS was first projected; NULL = no such directive in force. Our own design/extension.';
COMMENT ON COLUMN vo_head.access_restricted_at IS 'When an EHDS Art. 8 restriction directive of the current EHR_ACCESS covering this object, or its whole EHR, was first projected; NULL = none in force. Our own design/extension.';
COMMENT ON COLUMN ehr.opted_out_at IS 'When an EHDS Art. 10 opt-out directive of the current EHR_ACCESS was first projected; NULL = no opt-out in force. Our own design/extension.';
