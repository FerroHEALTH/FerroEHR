- A request's scopes (the connection scope, the access evaluator, the handed read, the party
  revisions read, the capabilities and the binding guards) wrap the request's future once each
  instead of copying it into every layer, so a care-decided composition read no longer overflows
  a 2 MiB stack in a debug build: it needed about 2,000 KiB and now needs about 860 KiB.
