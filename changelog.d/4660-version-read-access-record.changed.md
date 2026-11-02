- The access record of a VERSION read (`GET /ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version`,
  its `/version/{version_uid}` form, and the same two `versioned_ehr_status`
  routes) now names the content it served, and for a COMPOSITION the origins
  of its data, as the bare reads already did. Its `ehr_id` is the parsed EHR
  id, as on every other read, rather than the path text as sent. The
  response is unchanged.
