- The management surface answers a caller the login binding refuses with
  `403` and its binding code, such as `refused-unbound`, as every other
  route does, in place of `401`: the credential authenticated, so new
  credentials cannot help (RFC 9110 §15.5.4). An absent or invalid
  credential still answers `401` with its challenge.
