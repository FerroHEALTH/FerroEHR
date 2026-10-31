-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the access-delay column the shared version reads select.
--
-- The clinical and party change-control relations share one set of version
-- reads. The EHDS Art. 3(3) delay of the person's own access
-- (clinical/0027_access_delay.sql) applies to the clinical record only, so the
-- party column exists for the shared reads and stays NULL.
--
-- No openEHR spec governs a delay of the person's access: our own
-- design/extension.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE vo_head ADD COLUMN person_withheld_until timestamptz;

COMMENT ON COLUMN vo_head.person_withheld_until IS 'Always NULL in the party domain: the EHDS Art. 3(3) delay of the person''s access applies to the clinical record. Present for the version reads both domains share. Our own design/extension.';
