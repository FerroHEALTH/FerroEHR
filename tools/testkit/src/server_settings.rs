// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The command line of the testkit container's `PostgreSQL` server.

/// The server and its settings: the non-durable settings of a disposable
/// server (`PostgreSQL` docs § "Non-Durable Settings"), room for the
/// connections of as many test processes as a host has cores
/// (`crate::admission` admits by this setting), and `pg_stat_statements` counting nested statements for the
/// statement budget test of `ferroehr-rest` (#5885), as CI's server does
/// (`scripts/lib/test-postgres.sh`), with room for the statements of every
/// clone a parallel run holds at once, so none is evicted while a test counts
/// it (a dropped clone's statements leave with it, [`crate::TestDb`]).
pub(crate) const COMMAND: [&str; 15] = [
    "postgres",
    "-c",
    "fsync=off",
    "-c",
    "synchronous_commit=off",
    "-c",
    "full_page_writes=off",
    "-c",
    "max_connections=400",
    "-c",
    "shared_preload_libraries=pg_stat_statements",
    "-c",
    "pg_stat_statements.track=all",
    "-c",
    "pg_stat_statements.max=100000",
];
