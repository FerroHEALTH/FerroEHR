- A response to a request whose body FerroEHR left unread, such as a `404`
  to a `POST` of a path it does not serve or a declared-length `413`, now
  carries `Connection: close`. The connection closed after such a response
  without saying so, and a client that sent its next request on it met a
  closed socket, which a non-idempotent request cannot safely retry
  (RFC 9112 §9.6).
- A `[server.caller_rate]` above 1 000 000 000 requests per second no longer
  answers `429` to concurrent requests: such a rate now limits nothing, as
  its operator meant.
- Every request leaves one request line, a timeout `408`, a declared-length
  `413` and the `500` of a panicked handler included, and a `413` for a
  body with no declared length that passes `[server] body_limit_bytes` is
  answered with the openEHR error body, as the declared-length `413` is.
