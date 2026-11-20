-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the schema.
--
-- This is the v6.0.0 baseline of the party set: every object of the schema is
-- created once, in its final shape, by the files 0001 to 0010. A later change
-- is a new file after them.
--
-- The party pseudonymisation domain holds the demographic versioned objects,
-- physically separated from the clinical schema so no runtime role reads both
-- (GDPR Art. 4(5) and Art. 32(1)(a),
-- https://eur-lex.europa.eu/eli/reg/2016/679/oj). Its change-control and node
-- relations are rendered from the same templates as the clinical ones
-- (0002, 0003), so the two cannot drift. The pool serving this domain opens
-- each connection with `search_path = party, ext, public`, and every relation
-- below is created and referenced unqualified.
--
-- No openEHR spec governs storage layout or database roles: our own design.
--
-- Runs with search_path = party, ext, public. The schema itself is created by
-- the server's bootstrap (crate::db).

COMMENT ON SCHEMA party IS 'The party pseudonymisation domain: the demographic versioned objects and their change control, physically separated from the clinical schema so no single runtime role reads both (GDPR Art. 4(5), Art. 32(1)(a); EDPB 01/2025). No openEHR spec governs storage layout — our own design/extension.';
