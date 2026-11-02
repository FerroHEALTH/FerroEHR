-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: drop resolve_national_identifier (#4697).
--
-- No server path calls the function since the linkage domain went (#4684):
-- the identifier store reads its own rows as the party writer. Dropping it
-- also drops the EXECUTE grant 0008 gave and the denials it recorded.
DROP FUNCTION IF EXISTS resolve_national_identifier(text, bytea);
