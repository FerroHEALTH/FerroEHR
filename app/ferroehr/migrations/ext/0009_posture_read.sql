-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- ext: the clinical role reads the deployment posture.
--
-- `clinical.subject_pseudonym_guard()` runs as the role that writes
-- `clinical.ehr`, and it reads `ext.posture` to decide whether a subject must
-- be a pseudonym. `0004_grants.sql` granted that role `stamp_posture` and no
-- read of the table, so under a separated clinical credential every EHR create
-- or EHR_STATUS update that names a subject failed with "permission denied for
-- table posture". The clinical role gets SELECT on the one table, and no write:
-- the posture is still written only through `stamp_posture`.
--
-- No openEHR spec governs database roles: our own design/extension.
--
-- Runs with search_path = ext.

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT ON ext.posture TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping ext.posture read grant (roles absent — see the role block NOTICE)';
    END IF;
END $$;
