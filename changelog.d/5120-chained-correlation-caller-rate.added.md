- A `chained` organisation request under a federation agreement is served
  only when its token carries the requester's correlation id in the claim or
  attested `authorization_details` member the issuer's `[[auth.federation]]`
  table maps to `correlation_id`, and refused with `403` without it; a request
  header never stands in for it. The `organisation` access record's
  `agreement` carries the id and its source beside the legal ground and the
  access class. A chained agreement whose issuer maps no `correlation_id`, or
  maps it from a constant, refuses the start.
- A federation caller above its agreement's `caller_rate`, a MyHealth@EU
  contact point's included, is answered `429` with `Retry-After` before any
  patient lookup, keyed on its issuer and subject per replica, and every such
  refusal is counted into the caller's summary record.
