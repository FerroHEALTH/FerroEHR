-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- linkage: drop the subject-to-EHR cross-reference.
--
-- Which EHR belongs to which subject has one source: the EHR's own
-- EHR_STATUS.subject.external_ref (RM ehr master04 §EHR Status), promoted to
-- the clinical `ehr` table's subject columns on every EHR create and
-- EHR_STATUS update. Nothing a route reaches wrote subject_ehr, so a read of it
-- answered nothing; the cohort, the AQL demographic join and the EHR index
-- providers now read the promoted columns. The definer function that erased
-- this table's rows goes with it.
--
-- A deployment that wrote rows here through the service API loses them: they
-- named EHRs whose own EHR_STATUS is the record of their subject.
--
-- No openEHR spec governs storage layout: our own design/extension.
--
-- Runs with search_path = linkage, ext, public.

DROP FUNCTION erase_ehr(uuid);

DROP TABLE subject_ehr;
