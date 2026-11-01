-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The storage generation after the node header columns (#4525).
--
-- 0007 moved the archetype header, language and encoding of a node row below
-- the root into columns, which changes what a composition version costs on
-- disk, so the storage benchmark files its records under a new generation.
-- Nothing in the serving path reads this label. No openEHR spec governs
-- storage layout: our own design.
--
-- Runs with search_path = ext.

CREATE OR REPLACE FUNCTION storage_generation() RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$ SELECT 'generation-4'::text $$;
