// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Which phase of a database session a driver error comes from: the
//! establishing of a connection, or an open session cut under a request.

/// Returns whether `e` is a failure that only establishing a new connection
/// produces, so no statement of the request reached the database.
///
/// Every failure of [`crate::db::acquire::acquire`] is classified by its phase
/// in [`super::classify_acquire`]; this identity test serves the errors that carry no
/// phase, from a statement executed on a pool rather than on an acquired
/// connection, by identity: only the identities an established session cannot
/// produce qualify.
///
/// - [`sqlx::Error::Tls`]: raised only while configuring and negotiating TLS at
///   connect time (sqlx 0.9, <https://docs.rs/sqlx/0.9.0/sqlx/enum.Error.html>).
/// - an I/O error of kind `ConnectionRefused` or `AddrNotAvailable`: the
///   `connect(2)` errors `ECONNREFUSED` and `EADDRNOTAVAIL`; a peer resetting an
///   established session reports `ConnectionReset` instead.
/// - a server refusal of a new session's start-up, by SQLSTATE
///   (<https://www.postgresql.org/docs/18/errcodes-appendix.html>): class `28`
///   (invalid authorization), `3D000` `invalid_catalog_name`, `53300`
///   `too_many_connections` and `57P03` `cannot_connect_now`. None of them
///   answers a statement this server issues on an open session.
///
/// No openEHR spec governs this — our own design.
pub(crate) fn is_connect_failure(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Tls(_) => true,
        sqlx::Error::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::AddrNotAvailable
        ),
        sqlx::Error::Database(db) => db.code().is_some_and(|code| {
            code.starts_with("28") || matches!(&*code, "3D000" | "53300" | "57P03")
        }),
        _ => false,
    }
}

/// Returns whether `e` reports an open session that the database or the
/// network cut, an outage that a repeated request outlives.
///
/// - SQLSTATE class `08` connection exception, except `08P01`
///   `protocol_violation`, and the class `57` codes a shutdown, a crash or a
///   start-up raises: `57P01` `admin_shutdown`, `57P02` `crash_shutdown`,
///   `57P03` `cannot_connect_now` and `57P05` `idle_session_timeout`
///   (<https://www.postgresql.org/docs/18/errcodes-appendix.html>). `57014`
///   `query_canceled` (a statement timeout or cancel) and `57P04`
///   `database_dropped` are no outage; [`super::classify_sqlx`] answers each on its
///   own.
/// - an I/O error of kind `ConnectionReset`, `ConnectionAborted`,
///   `BrokenPipe`, `NotConnected` or `UnexpectedEof`: the socket closed under
///   the session.
///
/// No openEHR spec governs this — our own design.
pub(crate) fn is_cut_session(e: &sqlx::Error) -> bool {
    match e {
        sqlx::Error::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::NotConnected
                | std::io::ErrorKind::UnexpectedEof
        ),
        sqlx::Error::Database(db) => db.code().is_some_and(|code| {
            (code.starts_with("08") && &*code != "08P01")
                || matches!(&*code, "57P01" | "57P02" | "57P03" | "57P05")
        }),
        _ => false,
    }
}
