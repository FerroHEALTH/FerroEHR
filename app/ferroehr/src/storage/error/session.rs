// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Which phase of a database session a driver error comes from: the
//! establishing of a connection, or an open session cut under a request.

/// Returns whether `e` is a failure only establishing a new connection
/// produces, so no statement of the request reached the database.
///
/// It serves the errors with no acquire phase ([`super::classify_acquire`]
/// classifies the others):
///
/// - [`sqlx::Error::Tls`], raised only at connect time
///   (<https://docs.rs/sqlx/0.9.0/sqlx/enum.Error.html>).
/// - an I/O error of kind `ConnectionRefused` or `AddrNotAvailable`.
/// - a refused session start-up
///   (<https://www.postgresql.org/docs/18/errcodes-appendix.html>): class `28`,
///   `3D000`, `53300` and `57P03`.
///
/// No openEHR spec governs this: our own design.
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
/// network cut, an outage a repeated request outlives.
///
/// - SQLSTATE class `08` but `08P01`, and `57P01`, `57P02`, `57P03` and
///   `57P05` (<https://www.postgresql.org/docs/18/errcodes-appendix.html>);
///   `57014` and `57P04` are no outage, and [`super::classify_sqlx`] answers
///   each on its own.
/// - an I/O error of kind `ConnectionReset`, `ConnectionAborted`,
///   `BrokenPipe`, `NotConnected` or `UnexpectedEof`.
///
/// No openEHR spec governs this: our own design.
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
