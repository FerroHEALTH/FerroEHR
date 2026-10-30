// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The recorder of the replayed CDR (`tests/fixtures/cdr/`), driven by
//! `scripts/viewer-record.sh`.
//!
//! With `FERROEHR_VIEWER_RECORD_CDR` and `FERROEHR_VIEWER_RECORD_DIR` set,
//! every `ssr_app` test draws its screens over a live FerroEHR: the replayed
//! CDR becomes a proxy that answers a test's stubs first, forwards every
//! other request, answers with exactly the fields a recording keeps, and
//! appends the exchange to a file of the test's own. The ignored
//! [`write_the_recording`] then merges those files into `recording.json`
//! and `manifest.json`, in one order whatever order the tests ran in, with
//! every id and every instant the run stamped replaced by a stand-in, so a
//! second recording of the same screens writes the same bytes.
//!
//! No openEHR specification governs the viewer's test fixtures: our own
//! design.

#![expect(
    clippy::expect_used,
    reason = "the recorder is test tooling: a live CDR that cannot be reached or a recording \
              directory that cannot be written is a broken run, reported by the panic"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the recording is raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

/// The base URL of the live CDR a recording run forwards to.
const CDR_VAR: &str = "FERROEHR_VIEWER_RECORD_CDR";
/// The directory a recording run writes its exchanges and its manifest to.
const DIR_VAR: &str = "FERROEHR_VIEWER_RECORD_DIR";

/// The response headers a recording keeps: the ones the viewer reads.
const KEPT_HEADERS: [&str; 5] = [
    "allow",
    "content-type",
    "etag",
    "last-modified",
    "www-authenticate",
];

/// The first stand-in instant: the seed's instants follow it a second apart,
/// in their order, and every instant of the reads is this one plus an hour.
const STAND_IN_EPOCH: &str = "2026-10-01T09:00:00Z";

/// A recording run: the live CDR and the directory it writes to.
#[derive(Debug, Clone)]
pub(crate) struct Recording {
    /// The live CDR's base URL, without a trailing slash.
    live: String,
    /// The run's directory.
    dir: PathBuf,
}

/// The recording run this process is part of, when the recorder set one up.
pub(crate) fn recording() -> Option<Recording> {
    let live = crate::common::env(CDR_VAR)?;
    let dir = crate::common::env(DIR_VAR)?;
    Some(Recording {
        live: live.trim_end_matches('/').to_owned(),
        dir: PathBuf::from(dir),
    })
}

impl Recording {
    /// The manifest of the ids the recorder seeded.
    pub(crate) fn manifest_path(&self) -> PathBuf {
        self.dir.join("manifest.json")
    }
}

/// Starts the recording proxy over `stubs` and returns its base URL.
///
/// `stub_answer` picks the stub that answers a request, by the replay's own
/// rules; a stub is answered and never recorded.
pub(crate) async fn proxy(
    recording: Recording,
    stubs: Arc<[Value]>,
    stub_answer: for<'a> fn(&'a [Value], &str, &str, &str, bool) -> Option<&'a Value>,
) -> String {
    let test_name = std::thread::current()
        .name()
        .map_or_else(|| std::process::id().to_string(), str::to_owned)
        .replace("::", "__");
    let exchanges = recording.dir.join("exchanges");
    std::fs::create_dir_all(&exchanges).expect("the exchanges directory should be writable");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(exchanges.join(format!("{test_name}.jsonl")))
        .expect("the test's exchange file should open");
    let file = Arc::new(Mutex::new(file));
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("the forwarding client should build");
    let live = recording.live.clone();
    let app = axum::Router::new().fallback(move |request: axum::extract::Request| {
        let stubs = Arc::clone(&stubs);
        let file = Arc::clone(&file);
        let client = client.clone();
        let live = live.clone();
        async move {
            let (parts, body) = request.into_parts();
            let bytes = axum::body::to_bytes(body, 1 << 20)
                .await
                .expect("a test request body fits");
            let text = String::from_utf8_lossy(&bytes).into_owned();
            let path = parts
                .uri
                .path_and_query()
                .map_or_else(|| parts.uri.path().to_owned(), |pq| pq.as_str().to_owned());
            if let Some(stub) = stub_answer(&stubs, parts.method.as_str(), &path, &text, true) {
                return respond(stub);
            }
            let mut forward = client.request(parts.method.clone(), format!("{live}{path}"));
            for (name, value) in &parts.headers {
                if name != http::header::HOST && name != http::header::CONTENT_LENGTH {
                    forward = forward.header(name, value);
                }
            }
            let answer = forward
                .body(bytes.to_vec())
                .send()
                .await
                .expect("the live CDR should answer");
            let status = answer.status();
            let mut headers = serde_json::Map::new();
            for name in KEPT_HEADERS {
                if let Some(value) = answer.headers().get(name).and_then(|v| v.to_str().ok()) {
                    headers.insert(name.to_owned(), Value::from(value));
                }
            }
            let response = answer
                .text()
                .await
                .expect("the live CDR's answer should read");
            let exchange = json!({
                "method": parts.method.as_str(),
                "path": path,
                "body": text,
                "status": status.as_u16(),
                "headers": headers,
                "response": response,
            });
            {
                let mut file = file.lock().expect("the exchange file lock is not poisoned");
                writeln!(file, "{exchange}").expect("the exchange should be written");
            }
            respond(&exchange)
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("the recording proxy serves until the test ends");
    });
    format!("http://{addr}")
}

/// The answer an exchange in the recording's shape gives.
pub(crate) fn respond(exchange: &Value) -> http::Response<axum::body::Body> {
    let mut response = http::Response::builder()
        .status(u16::try_from(exchange["status"].as_u64().unwrap_or(500)).unwrap_or(500));
    if let Some(headers) = exchange["headers"].as_object() {
        for (name, value) in headers {
            if let Some(value) = value.as_str() {
                response = response.header(name.as_str(), value);
            }
        }
    }
    response
        .body(axum::body::Body::from(
            exchange["response"].as_str().unwrap_or("").to_owned(),
        ))
        .expect("a recorded response builds")
}

/// Merges a recording run into `tests/fixtures/cdr/`.
///
/// The exchanges are read test by test in name order, the first of each
/// method, path, body and status is kept, and the kept ones are ordered by
/// path, method, body and status. Every UUID becomes a stand-in numbered by
/// first appearance, as does every base64 SHA-256 digest (a version
/// signature). An instant the seed stamped (between `started` and `seeded`,
/// which the recorder writes around its seed) becomes a stand-in a second
/// after the one before; one the reads stamped (an answer's creation time,
/// whose order depends on the tests' schedule) becomes one stand-in.
#[tokio::test]
#[ignore = "the recorder's last step: scripts/viewer-record.sh runs it after a recording run"]
async fn write_the_recording() {
    let recording = recording().expect("the recorder sets the recording variables");
    let marker = |name: &str| -> jiff::Timestamp {
        std::fs::read_to_string(recording.dir.join(name))
            .expect("the recorder writes its markers")
            .trim()
            .parse()
            .expect("a marker is an RFC 3339 instant")
    };
    let started = marker("started");
    let seeded = marker("seeded");
    let window = (
        started - jiff::SignedDuration::from_secs(1),
        jiff::Timestamp::now() + jiff::SignedDuration::from_secs(1),
    );

    let mut files: Vec<PathBuf> = std::fs::read_dir(recording.dir.join("exchanges"))
        .expect("the run wrote its exchanges")
        .map(|entry| entry.expect("a directory entry reads").path())
        .collect();
    files.sort();
    let mut kept: BTreeMap<(String, String, String, u64), Value> = BTreeMap::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("an exchange file reads");
        for line in text.lines() {
            let exchange: Value = serde_json::from_str(line).expect("an exchange line is JSON");
            let key = (
                exchange["path"].as_str().unwrap_or_default().to_owned(),
                exchange["method"].as_str().unwrap_or_default().to_owned(),
                exchange["body"].as_str().unwrap_or_default().to_owned(),
                exchange["status"].as_u64().unwrap_or_default(),
            );
            kept.entry(key).or_insert(exchange);
        }
    }
    let mut exchanges = Value::Array(kept.into_values().collect());
    let manifest_text =
        std::fs::read_to_string(recording.manifest_path()).expect("the recorder wrote a manifest");
    let mut manifest: Value = serde_json::from_str(&manifest_text).expect("the manifest is JSON");

    let mut stand_ins = StandIns::default();
    stand_ins.collect(&exchanges, window);
    stand_ins.collect(&manifest, window);
    let stand_ins = stand_ins.finish(seeded);
    stand_ins.apply(&mut exchanges);
    stand_ins.apply(&mut manifest);

    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cdr");
    std::fs::write(fixtures.join("recording.json"), pretty(&exchanges))
        .expect("the recording should be written");
    std::fs::write(
        fixtures.join("manifest.json"),
        format!(
            "{}\n",
            serde_json::to_string(&manifest).expect("the manifest serializes")
        ),
    )
    .expect("the manifest should be written");
}

/// `value` as JSON indented by one space, ending in a newline.
fn pretty(value: &Value) -> String {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    serde::Serialize::serialize(value, &mut serializer).expect("the recording serializes");
    out.push(b'\n');
    String::from_utf8(out).expect("serde_json writes UTF-8")
}

/// The stand-ins of one recording, gathered in the recording's order.
#[derive(Debug, Default)]
struct StandIns {
    /// Every UUID, in order of first appearance.
    ids: Vec<String>,
    /// Every instant the run stamped.
    instants: Vec<jiff::Timestamp>,
    /// Every HTTP date (`Last-Modified`) the run stamped. A whole second is
    /// no rank among the instants, so these all take the reads' stand-in.
    dates: Vec<jiff::Timestamp>,
    /// Every base64 SHA-256 digest (a version signature), in order of first
    /// appearance.
    digests: Vec<String>,
}

/// The stand-ins once gathered: what replaces what.
#[derive(Debug)]
struct Replacements {
    /// A UUID to its stand-in.
    ids: BTreeMap<String, String>,
    /// An instant the run stamped to its stand-in.
    instants: BTreeMap<jiff::Timestamp, jiff::Timestamp>,
    /// An HTTP date the run stamped to its stand-in.
    dates: BTreeMap<jiff::Timestamp, jiff::Timestamp>,
    /// A digest to its stand-in.
    digests: BTreeMap<String, String>,
}

impl StandIns {
    /// Gathers the UUIDs and the run's instants of every string in `value`.
    fn collect(&mut self, value: &Value, window: (jiff::Timestamp, jiff::Timestamp)) {
        walk(value, &mut |text| {
            for id in uuids(text) {
                if !self.ids.iter().any(|known| known == id) {
                    self.ids.push(id.to_owned());
                }
            }
            for digest in digests(text) {
                if !self.digests.iter().any(|known| known == digest) {
                    self.digests.push(digest.to_owned());
                }
            }
            for (_, _, instant) in instants(text) {
                if instant >= window.0 && instant <= window.1 {
                    self.instants.push(instant);
                }
            }
            if let Some(instant) = http_date(text)
                && instant >= window.0
                && instant <= window.1
            {
                self.dates.push(instant);
            }
        });
    }

    /// Numbers the stand-ins; the instants after `seeded` are the reads'.
    fn finish(mut self, seeded: jiff::Timestamp) -> Replacements {
        let ids = self
            .ids
            .iter()
            .zip(1_u64..)
            .map(|(id, n)| (id.clone(), format!("00000000-0000-7000-8000-{n:012x}")))
            .collect();
        self.instants.sort();
        self.instants.dedup();
        let epoch: jiff::Timestamp = STAND_IN_EPOCH
            .parse()
            .expect("the stand-in epoch is an RFC 3339 instant");
        let instants = self
            .instants
            .iter()
            .zip(0_i64..)
            .map(|(instant, n)| {
                let stand_in = if *instant > seeded {
                    epoch + jiff::SignedDuration::from_hours(1)
                } else {
                    epoch + jiff::SignedDuration::from_secs(n)
                };
                (*instant, stand_in)
            })
            .collect();
        let digests = self
            .digests
            .iter()
            .zip(1_u64..)
            .map(|(digest, n)| (digest.clone(), format!("sha256:{n:0>43}=")))
            .collect();
        let read = epoch + jiff::SignedDuration::from_hours(1);
        let dates = self.dates.iter().map(|date| (*date, read)).collect();
        Replacements {
            ids,
            instants,
            dates,
            digests,
        }
    }
}

impl Replacements {
    /// Replaces every UUID and every run instant in the strings of `value`.
    fn apply(&self, value: &mut Value) {
        match value {
            Value::String(text) => *text = self.replace(text),
            Value::Array(items) => items.iter_mut().for_each(|item| self.apply(item)),
            Value::Object(map) => map.values_mut().for_each(|item| self.apply(item)),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    /// `text` with its UUIDs and run instants replaced.
    fn replace(&self, text: &str) -> String {
        if let Some(instant) = http_date(text) {
            if let Some(stand_in) = self.dates.get(&instant) {
                return jiff::fmt::rfc2822::DateTimePrinter::new()
                    .timestamp_to_rfc9110_string(stand_in)
                    .expect("a stand-in instant prints as an HTTP date");
            }
            return text.to_owned();
        }
        let mut out = text.to_owned();
        for (id, stand_in) in self.ids.iter().chain(&self.digests) {
            if out.contains(id.as_str()) {
                out = out.replace(id.as_str(), stand_in);
            }
        }
        // Every boundary below is an ASCII byte the scanners matched, so each
        // `get` finds its slice.
        let mut result = String::with_capacity(out.len());
        let mut copied = 0;
        for (start, end, instant) in instants(&out) {
            if let Some(stand_in) = self.instants.get(&instant) {
                result.push_str(out.get(copied..start).unwrap_or_default());
                result.push_str(&rewrite(out.get(start..end).unwrap_or_default(), *stand_in));
                copied = end;
            }
        }
        result.push_str(out.get(copied..).unwrap_or_default());
        result
    }
}

/// Calls `visit` with every string inside `value`, in document order.
fn walk(value: &Value, visit: &mut impl FnMut(&str)) {
    match value {
        Value::String(text) => visit(text),
        Value::Array(items) => items.iter().for_each(|item| walk(item, visit)),
        Value::Object(map) => map.values().for_each(|item| walk(item, visit)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Whether the bytes of `text` from `at` match `shape`, where `9` is a
/// digit, `x` a lowercase hex digit and any other byte itself.
fn shaped(text: &[u8], at: usize, shape: &[u8]) -> bool {
    shape.iter().enumerate().all(|(i, want)| {
        text.get(at + i).is_some_and(|got| match want {
            b'9' => got.is_ascii_digit(),
            b'x' => got.is_ascii_digit() || (b'a'..=b'f').contains(got),
            _ => got == want,
        })
    })
}

/// The lowercase hyphenated UUIDs in `text`, in order.
fn uuids(text: &str) -> Vec<&str> {
    const SHAPE: &[u8] = b"xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx";
    let bytes = text.as_bytes();
    let hex = |at: Option<usize>| {
        at.and_then(|at| bytes.get(at))
            .is_some_and(u8::is_ascii_hexdigit)
    };
    let mut found = Vec::new();
    let mut at = 0;
    while at + SHAPE.len() <= bytes.len() {
        let end = at + SHAPE.len();
        if shaped(bytes, at, SHAPE) && !hex(at.checked_sub(1)) && !hex(Some(end)) {
            found.extend(text.get(at..end));
            at = end;
        } else {
            at += 1;
        }
    }
    found
}

/// The RFC 3339 instants in `text`: start, end and the instant.
fn instants(text: &str) -> Vec<(usize, usize, jiff::Timestamp)> {
    const SHAPE: &[u8] = b"9999-99-99T99:99:99";
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    let mut at = 0;
    while at + SHAPE.len() < bytes.len() {
        if !shaped(bytes, at, SHAPE) {
            at += 1;
            continue;
        }
        let mut end = at + SHAPE.len();
        if bytes.get(end) == Some(&b'.') {
            end += 1;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
        }
        if bytes.get(end) == Some(&b'Z') {
            end += 1;
        } else if shaped(bytes, end + 1, b"99:99") && matches!(bytes.get(end), Some(b'+' | b'-')) {
            end += 6;
        } else {
            at += SHAPE.len();
            continue;
        }
        if let Some(instant) = text
            .get(at..end)
            .and_then(|written| written.parse::<jiff::Timestamp>().ok())
        {
            found.push((at, end, instant));
        }
        at = end;
    }
    found
}

/// `text` as an HTTP date (RFC 9110 §5.6.7), when it is one whole.
fn http_date(text: &str) -> Option<jiff::Timestamp> {
    // NOTE: a string that is no HTTP date is legitimately absent here: the
    // probe's refusal is the answer.
    if !text.ends_with(" GMT") {
        return None;
    }
    jiff::fmt::rfc2822::DateTimeParser::new()
        .parse_timestamp(text)
        .ok()
}

/// `stand_in` with a six-digit fraction, in the offset `written` carries.
fn rewrite(written: &str, stand_in: jiff::Timestamp) -> String {
    let (offset, suffix) = match written
        .len()
        .checked_sub(6)
        .and_then(|at| written.get(at..))
    {
        Some(suffix) if !written.ends_with('Z') => {
            let sign = if suffix.starts_with('-') { -1 } else { 1 };
            let part = |range: std::ops::Range<usize>| -> i32 {
                suffix
                    .get(range)
                    .and_then(|digits| digits.parse().ok())
                    .expect("an offset the scanner matched has digits")
            };
            let seconds = sign * (part(1..3) * 3600 + part(4..6) * 60);
            let offset = jiff::tz::Offset::from_seconds(seconds)
                .expect("a parsed instant's offset is in range");
            (offset, suffix)
        }
        _ => (jiff::tz::Offset::UTC, "Z"),
    };
    let mut out = offset
        .to_datetime(stand_in)
        .strftime("%Y-%m-%dT%H:%M:%S")
        .to_string();
    // The CDR drops a fraction's trailing zeros, so its width varies per run:
    // a stand-in always carries six.
    out.push_str(".000000");
    out.push_str(suffix);
    out
}

/// The base64 SHA-256 digests (`sha256:` and 44 base64 characters) in `text`.
fn digests(text: &str) -> Vec<&str> {
    const PREFIX: &str = "sha256:";
    const LENGTH: usize = PREFIX.len() + 44;
    let base64 = |b: &u8| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=');
    text.match_indices(PREFIX)
        .filter_map(|(at, _)| {
            let digest = text.get(at..at + LENGTH)?;
            let shaped = digest.bytes().skip(PREFIX.len()).all(|b| base64(&b))
                && digest.ends_with('=')
                && !text.as_bytes().get(at + LENGTH).is_some_and(base64);
            shaped.then_some(digest)
        })
        .collect()
}
