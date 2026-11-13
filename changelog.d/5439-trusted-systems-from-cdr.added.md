- The `ferroehr-operations` terminology gains the operations `data_origin`,
  `read_change_feed` and `scim_provision` and the scope elements `creating_system`,
  `origin`, `reader`, `binding_issuer` and `token_scope`. A change an administrator commits
  through the Demographic API decides the next request with no restart: the outbox is
  written while a reader is granted and only that reader's client reads it, a retention
  period may be keyed on an origin a grant names, and a SCIM request is admitted for a
  client whose grant is in force. A `ROLE` that names one of these elements other than as
  its operation reads it is refused with `400`.
- `ferroehr party-binding grant` commits a client's first grant on the operator's
  authority, such as the `scim_provision` of the client that provisions the administrators,
  who cannot log in before their `PERSON` exists. The compose quickstart, the Keycloak
  overlay and the hosted sandbox run it in a `ferroehr-grant` one-shot, and
  `scripts/scim-provision.sh` takes the client's binding issuer and runs it through
  `SCIM_GRANT`.
