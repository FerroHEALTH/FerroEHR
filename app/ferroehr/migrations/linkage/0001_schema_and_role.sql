-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- linkage: the schema that holds the map between the two pseudonymisation
-- domains, and nothing else.
--
-- A row here is the additional information that re-joins a pseudonymised
-- clinical record to a person (GDPR Art. 4(5),
-- https://eur-lex.europa.eu/eli/reg/2016/679/oj), so it is held apart from
-- both domains it joins and reachable only by its own role. Neither the
-- clinical nor the party search path carries this schema, and the role that
-- does carries neither of theirs.
--
-- No openEHR spec governs storage layout or database roles: our own
-- design/extension. The role itself is created by the ext set, which runs
-- first.
--
-- Runs with search_path = linkage, ext, public.

CREATE SCHEMA IF NOT EXISTS linkage;

COMMENT ON SCHEMA linkage IS 'The linkage pseudonymisation domain: the map from a party to the EHR whose subject it is. The additional information that re-joins a pseudonymised clinical record to a person (GDPR Art. 4(5)), held apart from both domains it joins. No openEHR spec governs storage layout — our own design/extension.';
