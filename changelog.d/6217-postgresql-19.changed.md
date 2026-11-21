- **Breaking:** FerroEHR requires PostgreSQL 19 and refuses to start against
  an older server: the boot path reads `server_version_num` from each domain
  pool and names the domain, the version found and the 190000 it needs. The
  `ferroehr-postgres` image is built on `postgres:19.0`, and the quickstart,
  the FerroSMART compose files and the test harness run PostgreSQL 19.
  PostgreSQL 18 is no longer supported and there is no upgrade path: a
  volume holding an 18 cluster, the hosted sandbox's included, is removed
  and created again.
