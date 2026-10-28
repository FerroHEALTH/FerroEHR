- An AQL query whose `WHERE` clause chains thousands of `NOT`, `AND` or `OR`
  terms no longer overflows the stack and aborts the server. The parser
  refuses a query that nests brackets and `CONTAINS` deeper than 64 levels or
  builds an expression deeper than 512 levels with `400 Bad Request`, before
  any recursive pass runs, on the ad-hoc, stored-query store and execution
  paths alike. A long `OR` over one path is written as `MATCHES {…}`, which
  the bound does not count. A statement PostgreSQL refuses for its own
  complexity limit (SQLSTATE class 54) also answers `400` instead of `500`.
  `openehr-query` gains `ParseError::TooDeep`.
