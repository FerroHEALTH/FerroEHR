- The `openehr-its` client reads an RFC 9457 problem details refusal:
  `ErrorBody::problem()` exposes its `type`, `title`, `status`, `detail` and
  `instance`, ignoring a member of the wrong type (RFC 9457 §3.1), and
  `ErrorBody::message()` answers the problem's `detail`, else its `title`,
  where it answered only the ITS-REST `Error`'s `message`. The `openehr-*`
  crates move to 0.0.115.
