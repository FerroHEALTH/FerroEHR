- **The usage report groups a request by the route below the openEHR API
  mount** (`server.base_path` followed by `rest.base_path`). A REST root that
  contains an API family word, such as `/ferroehr/ehr/v1`, no longer places a
  query or definition route under `ehr`, and a route outside the mount counts
  in no group, as before #4128. FerroEHR takes `ferrohealth-usage-report`
  0.2.0, whose window grouping may capture the configured mount.
