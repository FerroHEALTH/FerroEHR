- A REST request whose body cannot be read is refused with `400`, and one
  whose path parameter is not valid UTF-8 with `400`, instead of running as
  a request with an empty body or no path parameters. The body-limit check
  answers `413` only for a body over its limit; any other read failure is
  `400`.
