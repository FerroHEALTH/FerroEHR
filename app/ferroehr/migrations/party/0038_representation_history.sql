-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: who ever represented whom (#5532).
--
-- A patient-portal login whose PERSON has ever represented the EHR's subject
-- is judged only as a representative, so ending or deleting the
-- representation never leaves the former representative admitted as the
-- patient. binding_representation keeps its retired rows, and a refill
-- rebuilds them from every version that carried the representation code; this
-- index answers whether a representative has ever represented a subject, live
-- or retired. No openEHR spec governs the side table: our own design.
--
-- Runs with search_path = party, ext, public.

CREATE INDEX ix_binding_representation_history
    ON binding_representation (representative_id, subject_id)
    INCLUDE (retired_at, validity, relationship);

COMMENT ON INDEX ix_binding_representation_history IS 'Whether a representative ever represented a subject, live or retired: the patient-portal check that a former representative is never the patient (#5532).';
