- A `chained` organisation request under a federation agreement is served
  only when its token carries the requester's correlation id in the claim or
  attested `authorization_details` member the issuer's `[[auth.federation]]`
  table maps to `correlation_id`. Without it the request is refused with
  `403` and recorded under the binding code `refused-correlation-missing`
  (audit migration `0025`); a request header never stands in for it. The
  `organisation` access record's `agreement` carries the id and its source
  beside the legal ground and the access class. A chained agreement whose
  issuer maps no `correlation_id`, or maps it from a constant, refuses the
  start.
