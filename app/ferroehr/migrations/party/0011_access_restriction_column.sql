-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the access-restriction column the shared version reads select.
--
-- The clinical and party change-control relations share one DDL template and
-- one set of version reads. The EHDS Art. 8 restriction of access by health
-- professionals (clinical/0015_access_restriction.sql) applies to the clinical
-- record only, so the party column exists for the shared reads and stays NULL.
--
-- No openEHR spec governs restriction of access: our own design/extension.
--
-- Runs with search_path = party, ext, public.

ALTER TABLE vo_head ADD COLUMN access_restricted_at timestamptz;

COMMENT ON COLUMN vo_head.access_restricted_at IS 'Always NULL in the party domain: the EHDS Art. 8 restriction of access applies to the clinical record. Present for the version reads both domains share. Our own design/extension.';
