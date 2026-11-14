-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: drop the marks of the EHDS Art. 8 restriction and Art. 10
-- opt-out directives (#5587).
--
-- `ehr.access_restricted_at`, `vo_head.access_restricted_at` and
-- `ehr.opted_out_at` projected the `restriction` and `opt_out` directives of
-- each EHR's current EHR_ACCESS for the event outbox, reading every directive
-- as binding everyone. The outbox now judges each event with the access
-- evaluator against the EHR's own settings, as a system caller, so the marks
-- have no reader and go with the trigger that copied a whole-EHR mark onto a
-- new versioned object. Nothing is converted: the settings remain the record.
--
-- The GDPR Art. 18 restriction of processing (`restricted_at`) is a separate
-- right with its own register and stays.
--
-- No openEHR spec governs the marks or their removal: our own design.
--
-- Runs with search_path = clinical, ext, public.

DROP TRIGGER trg_vo_head_access_restriction ON vo_head;
DROP FUNCTION vo_head_inherits_access_restriction();

ALTER TABLE vo_head DROP COLUMN access_restricted_at;
ALTER TABLE ehr DROP COLUMN access_restricted_at;
ALTER TABLE ehr DROP COLUMN opted_out_at;
