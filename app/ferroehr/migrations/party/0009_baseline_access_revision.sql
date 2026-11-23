-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the view of the revisions a demographic query's access condition was
-- judged on.
--
-- Runs with search_path = party, ext, public.

-- The clinical schema reads an EHR's access revision from the view
-- access_revision_of. A demographic object belongs to no EHR, so the party
-- schema keeps a view of the same shape with no rows, and a statement both
-- domains run reads NULL there in its scalar subquery.
--
-- No openEHR spec governs the projection or its revision: our own design.
CREATE VIEW access_revision_of AS
SELECT NULL::uuid AS ehr_id, NULL::text AS revision WHERE false;
COMMENT ON VIEW access_revision_of IS 'No rows: no party object belongs to an EHR, so none carries EHR access settings. Our own design.';

-- The clinical schema reads the policy and definition revisions from the view
-- request_revisions beside an EHR's access revision. The party schema keeps
-- the policy copies in no store of its own, so its view has the same shape and
-- no rows (#6251).
CREATE VIEW request_revisions AS
SELECT NULL::bigint AS policy, NULL::bigint AS definitions WHERE false;
COMMENT ON VIEW request_revisions IS 'No rows: the party schema holds no policy or definition revision. Our own design.';

-- The one door from the clinical domain into this one (#6320). Where the two
-- domains share a database, a clinical read confirms the party copies its
-- decision rests on in its own statement, by reading the grant, facts and
-- binding revisions here. The schema holds this one function and nothing
-- else; it runs as its owner, the owner of this schema, with a fixed
-- search_path, and returns three counters that carry no personal data. The
-- clinical roles may run it and read no table of this schema
-- (db::verify_domain_isolation holds both).
--
-- No openEHR spec governs the confirmation: our own design.
CREATE SCHEMA party_door;
COMMENT ON SCHEMA party_door IS 'The one function of the party domain the clinical roles may run: the party store''s revisions. Our own design (#6320).';
REVOKE ALL ON SCHEMA party_door FROM PUBLIC;

CREATE FUNCTION party_door.party_revisions()
RETURNS TABLE (grants bigint, facts bigint, bindings bigint)
LANGUAGE sql STABLE SECURITY DEFINER
SET search_path = party, pg_catalog
AS $$
    SELECT r.revision, r.facts_revision, r.binding_revision FROM party.binding_grant_revision r
$$;
COMMENT ON FUNCTION party_door.party_revisions() IS 'The party store''s grant, facts and binding revisions, the only party facts a clinical statement reads. Our own design (#6320).';
REVOKE ALL ON FUNCTION party_door.party_revisions() FROM PUBLIC;
