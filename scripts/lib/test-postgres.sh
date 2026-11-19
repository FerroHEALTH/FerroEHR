#!/usr/bin/env bash
# SPDX-FileCopyrightText: Cadasto B.V.
# SPDX-License-Identifier: BUSL-1.1
# The settings of the PostgreSQL server a CI lane runs the test suite on
# (tools/testkit, FERROEHR_TEST_PG_URL), applied to the lane's service
# container: the non-durable settings the PostgreSQL docs give for throwaway
# data ("Non-Durable Settings"), and pg_stat_statements loaded with
# track = all, which the statement budget of an AQL request reads (#5885),
# and 1 GB of past WAL kept, so a checkpoint removes no segment a WAL budget
# is about to read with pg_walinspect (#6035).
# The testkit container starts with the same settings.
#
# shared_preload_libraries takes effect only at server start (PostgreSQL 18
# docs, "Shared Library Preloading"), so the container is restarted and
# waited for.
#
# Usage:
#   scripts/lib/test-postgres.sh <service-container-id>
# with PGPASSWORD set and the server on localhost:5432 as openehr.
#
# No specification governs this: our own design.
set -Eeuo pipefail

container="${1:?usage: scripts/lib/test-postgres.sh <service-container-id>}"

psql -h localhost -U openehr -d postgres -v ON_ERROR_STOP=1 \
  -c "ALTER SYSTEM SET fsync = off" \
  -c "ALTER SYSTEM SET synchronous_commit = off" \
  -c "ALTER SYSTEM SET full_page_writes = off" \
  -c "ALTER SYSTEM SET shared_preload_libraries = 'pg_stat_statements'" \
  -c "ALTER SYSTEM SET pg_stat_statements.track = 'all'" \
  -c "ALTER SYSTEM SET pg_stat_statements.max = 100000" \
  -c "ALTER SYSTEM SET wal_keep_size = '1GB'" >/dev/null
docker restart "$container" >/dev/null
for _ in $(seq 1 60); do
  if pg_isready -h localhost -U openehr -q; then
    loaded=$(psql -h localhost -U openehr -d postgres -Atc "SHOW shared_preload_libraries")
    [[ "$loaded" == *pg_stat_statements* ]] || {
      echo "::error::the test server did not load pg_stat_statements: '$loaded'" >&2
      exit 1
    }
    exit 0
  fi
  sleep 1
done
echo "::error::the test server did not come back within 60 s of its restart" >&2
exit 1
