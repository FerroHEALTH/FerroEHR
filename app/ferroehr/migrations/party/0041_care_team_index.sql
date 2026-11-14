-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the patients a care team cares for, read by the team (#5615).
--
-- A replica's copy of the caller facts no longer holds every patient's care
-- relationships. When a query's access condition needs the patients a
-- professional's teams care for, the replica reads each team's care
-- relationships once and holds them in a bounded cache, so the read is
-- answered by the team rather than by a scan of every fact.
--
-- No openEHR spec governs the side table or its index: our own design.
--
-- Runs with search_path = party, ext, public.

CREATE INDEX ix_binding_caller_fact_care_team ON binding_caller_fact (target_id)
    WHERE kind = 'under_care_of';
