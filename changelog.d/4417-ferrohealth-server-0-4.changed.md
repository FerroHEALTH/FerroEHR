- FerroEHR takes `ferrohealth-server` 0.4.0 in place of 0.3.9. The new
  `[server] idle_timeout_ms` (120000 by default) closes a kept-alive
  HTTP/1.1 connection that waits that long for its next request, with a
  clean end of stream, and `[server] header_read_timeout_ms` counts from the
  first byte of each request head on a kept-alive connection (from accept
  for the first one, as before). The head bound used to count from the end
  of the previous answer, so it also closed an idle connection after 30 s,
  sometimes as a client sent its next request into it, which a client such
  as Go's `net/http` saw as `EOF` on a `POST` it does not retry (#4381). A
  deployment that set `header_read_timeout_ms` to bound idle connections
  sets `idle_timeout_ms` instead. The refusals the listener writes itself
  (`408`, `413`, `429`, `503`) and a panic's `500` keep naming the server's
  request id as their `instance` when the client sends its own
  `X-Request-Id`, as every other FerroEHR refusal does.
