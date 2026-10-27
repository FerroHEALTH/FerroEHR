- **The usage report groups a request by the route below
  `server.base_path`.** A base path that contains an API family word, such
  as `/ferroehr/ehr/v1`, no longer places a query or definition route under
  `ehr`, and a route outside the base path counts in no group, as before
  #4128. FerroEHR takes `ferrohealth-usage-report` 0.2.0, whose window
  grouping may capture the configured base path.
