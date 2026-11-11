- A `PERSON` binds a login only while it performs a professional `ROLE` of
  the party mapping's role template that names it as `performer`; a
  `PERSON` that carries the binding key and performs no such `ROLE` is
  refused like an unbound one (`403`, `refused-unbound`). SCIM provisioning
  and just-in-time creation commit the `ROLE` with the `PERSON`. Upgrade:
  give each existing `PERSON` its `ROLE` with `ferroehr party-binding adopt`
  before the first SCIM run, and list the persons still without one with
  `ferroehr party-binding check`.
