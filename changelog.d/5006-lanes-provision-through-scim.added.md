- The compose quickstart binds its user to a professional `PERSON`: the
  `ferroehr-definitions` one-shot loads the PERSON and ROLE templates the
  party mapping names before the server starts, and `ferroehr-provision`
  creates the user through `/scim/v2/Users` with a token of the development
  issuer, then reports healthy, so `docker compose up --wait` returns once
  the login is bound. The Helm chart (23.3.0) takes `definitions.files`,
  which an init container stores with `ferroehr definitions load` before the
  server container starts. The conformance, browser and Kubernetes lanes
  load the same templates and provision every login they use through SCIM,
  with the committed test client of `docs/conformance/party/scim/`.
