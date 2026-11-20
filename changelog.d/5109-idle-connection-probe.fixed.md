- The first request after a database restart, a failover or a terminated
  session is served again: the connection pools test a connection that sat
  idle for a second or longer before they hand it out, and replace one the
  database closed, where that request used to fail on the dead session.
  Two replicas over one database answer `POST /ehr` with `201`, `409` for a
  subject another create took, or `503` while the database cannot serve it.
