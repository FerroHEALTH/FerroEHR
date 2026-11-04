-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The storage generation after shared version content (#4415, #4944).
--
-- clinical/0032 and party/0025 let a version whose content equals its
-- predecessor's read its predecessor's node rows, and clinical/0034 and
-- party/0026 give the head row the trunk head's content key, which changes
-- what a superseding version costs on disk and how a LATEST_VERSION read
-- reaches its rows, so the storage benchmark files its records under a new
-- generation. Nothing in the serving path reads this label. No openEHR spec
-- governs storage layout: our own design.
--
-- Runs with search_path = ext.

CREATE OR REPLACE FUNCTION storage_generation() RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$ SELECT 'generation-6'::text $$;
