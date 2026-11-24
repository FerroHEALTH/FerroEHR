# syntax=docker/dockerfile:1
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
#
# The PostgreSQL the CI database jobs run as a service container (#6520). A
# test image, never shipped. A service container takes an image and options
# but no command
# (https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#jobsjob_idservices),
# so the settings come from here: the non-durable settings the PostgreSQL
# documentation names for throwaway data
# (https://www.postgresql.org/docs/current/non-durability.html),
# pg_stat_statements preloaded for the statement budgets, and the connection
# ceiling the parallel suite needs. The base is docker/postgres's, digest for
# digest; ci-postgres.yml refuses a drift between the two.
FROM postgres:19.0@sha256:bb6e70add742cd868fb62cb3b2f67d1dd2bdd2b65a521e778420748753687c60

LABEL org.opencontainers.image.title="FerroEHR CI PostgreSQL" \
      org.opencontainers.image.description="PostgreSQL 19 for FerroEHR's CI database jobs: non-durable, pg_stat_statements preloaded. Not for deployment." \
      org.opencontainers.image.source="https://github.com/FerroHEALTH/FerroEHR" \
      org.opencontainers.image.vendor="Cadasto B.V." \
      org.opencontainers.image.licenses="BUSL-1.1 AND PostgreSQL"

CMD ["postgres", \
     "-c", "fsync=off", \
     "-c", "synchronous_commit=off", \
     "-c", "full_page_writes=off", \
     "-c", "max_connections=400", \
     "-c", "shared_preload_libraries=pg_stat_statements", \
     "-c", "pg_stat_statements.track=all", \
     "-c", "pg_stat_statements.max=100000", \
     "-c", "wal_keep_size=1GB"]
