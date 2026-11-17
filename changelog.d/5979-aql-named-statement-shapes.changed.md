- A repeated AQL query now runs as a named statement, prepared once on each
  pooled connection, so PostgreSQL can reuse its plan instead of planning it on
  every request. A query whose SQL is built for one request still runs
  unnamed. Each connection now keeps up to 356 prepared statements: sqlx's
  default 100, plus one per cached AQL plan.
