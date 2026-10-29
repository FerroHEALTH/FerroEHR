- A statement the database cancels, by its `statement_timeout` or a cancel
  request (SQLSTATE `57014`), answers `503` with `Retry-After` instead of
  `500`: the statement rolled back, so the request can be repeated. By default
  the request timeout answers before the 60 s statement timeout fires, so on a
  request path it is raised by an operator's cancel or by a forward step of
  the database host's clock, which fires every running statement's timeout at
  once.
