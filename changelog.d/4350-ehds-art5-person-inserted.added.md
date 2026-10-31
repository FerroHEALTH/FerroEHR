- EHDS Art. 5: what a person inserts in their own EHR is marked as theirs, and
  they cannot alter what a professional committed. A write under a token whose
  SMART resource scopes are all `patient/` scopes records `PARTY_SELF` as its
  `AUDIT_DETAILS.committer`, its `external_ref` naming the login, whatever
  committer the request names; a COMPOSITION it commits must name `PARTY_SELF`
  or `PARTY_RELATED` as composer; and its update or deletion of a version whose
  committer is not `PARTY_SELF`, directly or inside a CONTRIBUTION, is refused
  with `403`. A professional's writes are unchanged. The contribution list
  extension gains `inserted_by_person` per row, and the viewer names a
  `PARTY_SELF` committer "The person or their representative". A service
  `auth_failure` status now answers `403` where it answered `500`.
