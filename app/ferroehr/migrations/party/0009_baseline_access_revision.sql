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
