// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The command line of the testkit container's `PostgreSQL` server.

/// The server and its settings: the non-durable settings of a disposable
/// server (`PostgreSQL` docs § "Non-Durable Settings"), room for the clones'
/// connections, and `pg_stat_statements` counting nested statements for the
/// statement budget test of `ferroehr-rest` (#5885), as CI's server does
/// (`scripts/lib/test-postgres.sh`).
pub(crate) const COMMAND: [&str; 15] = [
    "postgres",
    "-c",
    "fsync=off",
    "-c",
    "synchronous_commit=off",
    "-c",
    "full_page_writes=off",
    "-c",
    "max_connections=200",
    "-c",
    "shared_preload_libraries=pg_stat_statements",
    "-c",
    "pg_stat_statements.track=all",
    "-c",
    "pg_stat_statements.max=20000",
];
