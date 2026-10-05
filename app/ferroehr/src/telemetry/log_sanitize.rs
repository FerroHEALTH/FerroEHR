// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Log-record integrity for the TEXT log formats: CR and LF inside a logged
//! record are neutralised, so no value that reaches a field can forge a second
//! log line.
//!
//! The OWASP Logging Cheat Sheet §Log Injection asks that event data have
//! "carriage return (CR), line feed (LF) and delimiter characters" sanitized
//! (<https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html>).
//! Three of the four sinks satisfy that by construction: the `json` format
//! escapes control characters, ATNA syslog framing is datagram- or
//! octet-counted, and the Audit Record Repository is parameterized SQL. The text
//! format does not: `tracing_subscriber`'s default field visitor renders a
//! `%`-sigil field and an interpolated event message verbatim, and a line break
//! is the record delimiter of a line-oriented log.
//!
//! The delimiters within a record cannot forge a record and are left alone;
//! escaping them would mangle every message. ANSI and C1 escapes are
//! `tracing_subscriber`'s own concern (`Writer::sanitizes_ansi_escapes`).
//! [`crate::telemetry::layers`] wraps only the `pretty` and `auto` text `fmt`
//! layer with [`LineSafe`], so a JSON record keeps its own single `\n` escape.
//!
//! A genuine line break in a value is escaped rather than dropped, so a
//! multi-line clinical text appears as `line one\nline two` on one physical
//! line with every character preserved. The cost is that a value that literally
//! contained `\` and `n` reads the same as an escaped break; the `json` format
//! keeps values byte-exact.
//!
//! No openEHR spec governs logging — our own design/extension.

use std::borrow::Cow;
use std::io::{self, Write};

use tracing::Metadata;
use tracing_subscriber::fmt::MakeWriter;

/// Wraps a [`MakeWriter`] so every record it writes carries no interior CR or
/// LF byte.
#[derive(Debug)]
pub(super) struct LineSafe<M>(M);

impl<M> LineSafe<M> {
    /// Wraps `inner`.
    pub(super) const fn new(inner: M) -> Self {
        Self(inner)
    }
}

impl<'a, M> MakeWriter<'a> for LineSafe<M>
where
    M: MakeWriter<'a>,
{
    type Writer = LineSafeWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        LineSafeWriter(self.0.make_writer())
    }

    fn make_writer_for(&'a self, meta: &Metadata<'_>) -> Self::Writer {
        LineSafeWriter(self.0.make_writer_for(meta))
    }
}

/// The writer [`LineSafe`] hands to the `fmt` layer.
#[derive(Debug)]
pub(super) struct LineSafeWriter<W>(W);

// NOTE: the `fmt` layer formats a whole event into one buffer and writes it in
// a single call, and the text format it is configured with is single-line, so
// the one legitimate line break is the trailing byte.
impl<W> Write for LineSafeWriter<W>
where
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_all(&escape_record(buf))?;
        // The escaped form is longer than the input; a `Write` implementation
        // reports how much of the INPUT it consumed, which is all of it.
        Ok(buf.len())
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.0.write_all(&escape_record(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Escapes every CR and LF in one formatted record as the two-character
/// sequences `\r` and `\n`, preserving a single trailing LF — the record
/// terminator the formatter itself wrote.
///
/// Returns the input untouched (no allocation) when it carries no interior
/// break, which is every ordinary record.
fn escape_record(buf: &[u8]) -> Cow<'_, [u8]> {
    let (body, terminator): (&[u8], &[u8]) = match buf.split_last() {
        Some((b'\n', head)) => (head, b"\n"),
        _ => (buf, b""),
    };
    if !body.contains(&b'\n') && !body.contains(&b'\r') {
        return Cow::Borrowed(buf);
    }
    let mut out = Vec::with_capacity(buf.len() + 8);
    for byte in body {
        match *byte {
            b'\n' => out.extend_from_slice(br"\n"),
            b'\r' => out.extend_from_slice(br"\r"),
            other => out.push(other),
        }
    }
    out.extend_from_slice(terminator);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt;
    use tracing_subscriber::layer::SubscriberExt;

    use super::{LineSafe, escape_record};

    /// A value carrying CR/LF and a forged record of its own — the shape a
    /// client-supplied identifier, header, or driver-error text would take.
    const FORGED: &str = "ok\r\n2026-08-06T00:00:00Z ERROR forged: the patient record was deleted";

    /// An in-memory sink shared with the test body.
    #[derive(Clone, Debug, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);

    impl Capture {
        fn contents(&self) -> String {
            let guard = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            String::from_utf8_lossy(&guard).into_owned()
        }
    }

    impl<'a> fmt::MakeWriter<'a> for Capture {
        type Writer = Capture;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    impl std::io::Write for Capture {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let mut guard = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Emit one event carrying [`FORGED`] as a `Display` field and as the
    /// interpolated message, on a SCOPED subscriber (never the global one —
    /// tests share the process).
    fn emit<L>(layer: L)
    where
        L: tracing_subscriber::Layer<tracing_subscriber::Registry> + Send + Sync + 'static,
    {
        let subscriber = tracing_subscriber::Registry::default().with(layer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::error!(value = %FORGED, "a client value reached a logged field: {FORGED}");
        });
    }

    /// The property: a CR/LF-bearing value cannot forge a second log line in
    /// the text format — the whole event is ONE line.
    #[test]
    fn a_crlf_bearing_value_cannot_forge_a_log_line() {
        let sink = Capture::default();
        emit(
            fmt::layer()
                .with_target(true)
                .with_ansi(false)
                .with_writer(LineSafe::new(sink.clone())),
        );
        let captured = sink.contents();
        assert_eq!(
            captured.lines().count(),
            1,
            "the event must occupy exactly one line: {captured:?}"
        );
        assert!(
            captured.ends_with('\n'),
            "the record terminator is preserved: {captured:?}"
        );
        assert!(
            !captured.contains("ok\r\n"),
            "the raw break must not survive: {captured:?}"
        );
        assert!(
            captured.contains(r"ok\r\n"),
            "the break is escaped, not dropped: {captured:?}"
        );
        assert!(
            captured.contains("forged"),
            "the value's text is still logged in full: {captured:?}"
        );
    }

    /// The counter-proof that the guard is load-bearing: the SAME event through
    /// an unwrapped text layer spans three lines, two of which a reader would
    /// take for records the server emitted.
    #[test]
    fn the_unguarded_text_format_would_admit_a_forged_line() {
        let sink = Capture::default();
        emit(
            fmt::layer()
                .with_target(true)
                .with_ansi(false)
                .with_writer(sink.clone()),
        );
        let captured = sink.contents();
        assert!(
            captured.lines().count() > 1,
            "without the guard the record breaks apart — if this ever becomes one \
             line, tracing-subscriber started escaping CR/LF itself and the guard \
             can be re-adjudicated: {captured:?}"
        );
    }

    /// The `json` format needs no guard: CR/LF are JSON string escapes there.
    /// Pinned so the "safe by construction" half of the claim is asserted too.
    #[test]
    fn the_json_format_escapes_control_characters_itself() {
        let sink = Capture::default();
        emit(fmt::layer().json().with_writer(sink.clone()));
        let captured = sink.contents();
        assert_eq!(
            captured.lines().count(),
            1,
            "one JSON object per line: {captured:?}"
        );
        assert!(
            captured.contains(r"ok\r\n"),
            "serde_json escapes the break: {captured:?}"
        );
    }

    #[test]
    fn escape_record_preserves_ordinary_records_untouched() {
        let plain = b"INFO ferroehr: nothing to escape\n";
        assert!(matches!(
            escape_record(plain),
            std::borrow::Cow::Borrowed(_)
        ));
        assert_eq!(escape_record(plain).as_ref(), plain);
        // A record with no terminator at all is still escaped.
        assert_eq!(escape_record(b"a\rb").as_ref(), br"a\rb");
        // Only the FINAL newline survives.
        assert_eq!(escape_record(b"a\nb\n").as_ref(), b"a\\nb\n");
    }
}
