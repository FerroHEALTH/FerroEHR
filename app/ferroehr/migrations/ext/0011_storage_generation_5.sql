-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The storage generation after the node compact markers (#4904).
--
-- 0010 and the node compact migrations of both domains let a node row store a
-- DV_DATE_TIME origin or time as its value string, leave out a bare PARTY_SELF
-- subject, and leave out the root's uid and archetype header, which changes
-- what a composition version costs on disk, so the storage benchmark files its
-- records under a new generation. Nothing in the serving path reads this
-- label. No openEHR spec governs storage layout: our own design.
--
-- Runs with search_path = ext.

CREATE OR REPLACE FUNCTION storage_generation() RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$ SELECT 'generation-5'::text $$;
