- A `+` in a query string is a literal plus, as RFC 3986 §2.1 and §3.4
  decode a URI query, on every ITS-REST operation and every extension route:
  a date-time offset such as `2026-10-28T10:00:00+01:00` or an AQL parameter
  sent with an unencoded `+` reaches the server unchanged, where it was read
  as a space. A space in a query value is sent as `%20`. This reverses the
  form-decoding of #3891: a client that writes a space as `+` now sends a
  plus. The `openehr-*` crates move to 0.0.114.
