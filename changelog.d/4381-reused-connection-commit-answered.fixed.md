- A request sent on a kept-alive connection that sat idle longer than
  `[server] header_read_timeout_ms` (30 s by default) is answered. The server
  used to close such a connection, sometimes while the request was arriving,
  so a client that reused it got `EOF` with no answer and no server log line,
  and a client that never retries a `POST` on a reused connection, such as
  Go's `net/http`, reported a composition commit as failed. An idle
  connection now stays open until `[server] idle_timeout_ms` (120 s by
  default) and then ends cleanly before any new request byte; keep that
  limit longer than the idle timeout of your clients (90 s for Go) and of any
  load balancer in front.
