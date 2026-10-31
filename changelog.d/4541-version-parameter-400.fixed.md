- A versioned read whose `version` path segment is no SEMVER version and no
  `{major}` or `{major}.{minor}` prefix, such as `v1.x`, `1.x` or `01`, now
  answers `400` before any lookup, where it answered `404`
  (`parameters/path/version.yaml`, `responses/400.yaml`). This covers the ADL 2
  template read `GET /definition/template/adl2/{template_id}/{version}`, the
  stored-query read `GET /definition/query/{qualified_query_name}/{version}` and
  both stored-query executions at a version, `GET` and `POST
  /query/{qualified_query_name}/{version}`. A well-formed version the server
  does not hold still answers `404`.
