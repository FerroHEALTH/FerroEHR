- `ferroehr definitions load <path>...` stores OPT 1.4 (`.opt`) and ADL 2
  (`.adls`) definitions before `serve`, through the validation and versioning
  of the REST upload. A file already stored with the same text is left alone
  and other text under a stored id is refused, so a deployment runs it at
  every start to give its party mapping the templates it needs at boot.
  `deploy/definitions/professional/` holds a professional PERSON and ROLE pair
  adapted from openEHR CKM archetypes for the development lanes and the
  quickstart (#5006).
