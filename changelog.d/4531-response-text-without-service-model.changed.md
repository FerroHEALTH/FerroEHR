- Five response messages no longer name the openEHR Service Model. The admin
  dump route's `400` for a missing `file_sys_loc` now reads "request body must
  carry a non-empty `file_sys_loc` string, a location on the server's file
  system (FerroEHR's own dump/load extension)", and its `400` for an
  `encoding` member reads "`encoding` is not accepted: FerroEHR's own
  dump/load extension defines no encoding, so no value names one". The admin
  activity-report `400`s for `a_service` now list the accepted service names
  ("query parameter `a_service` is required: one of Admin, Definitions, …" and
  "query parameter `a_service` names no platform service: … (expected one of
  …)"). The not-implemented answer of `store_query_set` reads "store_query_set
  is not implemented: no openEHR specification defines a query set". The
  `openehr-rm` validation docs for the three `PARTY` and `PARTY_RELATIONSHIP`
  relationship invariants now name the separate relationship container as
  FerroEHR's own design against RM demographic §Party Relationships.
