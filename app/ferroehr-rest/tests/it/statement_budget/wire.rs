// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The bytes a request sends to `PostgreSQL` and receives from it (#6244),
//! counted by a relay between the server's pools and the database that
//! splits both directions into protocol messages and charges each one to the
//! statement it belongs to.
//!
//! A frontend message belongs to the statement its `Parse`, `Bind` or `Query`
//! names, up to the `Sync` (or the `Query`) that ends its group; a backend
//! message belongs to the oldest group still waiting for its
//! `ReadyForQuery`, since the server answers the groups in order. A
//! `NotificationResponse` is charged to the notifications, whenever it
//! arrives. The startup exchange of a new connection is charged to its own
//! line (PostgreSQL Frontend/Backend Protocol, §Message Flow and §Message
//! Formats). No openEHR spec governs the bytes a request moves: our own
//! design.

use std::collections::{BTreeMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use sqlx::postgres::{PgConnectOptions, PgSslMode};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};

/// The line the startup exchange of a connection is charged to.
pub(super) const STARTUP: &str = "(connection startup)";

/// The line the notifications are charged to.
pub(super) const NOTIFICATIONS: &str = "(notifications)";

/// What one statement moved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Tally {
    /// The `Execute` and `Query` messages that ran it.
    pub(super) calls: u64,
    /// The bytes sent to the database.
    pub(super) sent: u64,
    /// The bytes received from the database.
    pub(super) received: u64,
}

impl Tally {
    /// Both directions together.
    pub(super) fn bytes(self) -> u64 {
        self.sent + self.received
    }

    fn add(&mut self, other: Self) {
        self.calls += other.calls;
        self.sent += other.sent;
        self.received += other.received;
    }
}

/// What the relay moved, by the statement's text, its whitespace collapsed.
#[derive(Debug, Default)]
pub(super) struct Ledger(pub(super) BTreeMap<String, Tally>);

impl Ledger {
    /// Every statement together.
    pub(super) fn total(&self) -> Tally {
        let mut total = Tally::default();
        for tally in self.0.values() {
            total.add(*tally);
        }
        total
    }

    /// Adds `other` into this ledger.
    pub(super) fn merge(&mut self, other: &Self) {
        for (statement, tally) in &other.0 {
            self.0.entry(statement.clone()).or_default().add(*tally);
        }
    }

    fn charge(&mut self, statement: &str, tally: Tally) {
        self.0.entry(statement.to_owned()).or_default().add(tally);
    }
}

/// A relay in front of the database, and what it has counted since the last
/// [`Wire::take`].
#[derive(Debug)]
pub(super) struct Wire {
    at: SocketAddr,
    ledger: Arc<Mutex<Ledger>>,
}

impl Wire {
    /// Starts a relay to the server `upstream` connects to.
    pub(super) async fn start(upstream: &PgConnectOptions) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("the relay binds");
        let at = listener.local_addr().expect("the relay's address");
        let target = (upstream.get_host().to_owned(), upstream.get_port());
        let ledger = Arc::new(Mutex::new(Ledger::default()));
        let shared = Arc::clone(&ledger);
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                let server = TcpStream::connect((target.0.as_str(), target.1))
                    .await
                    .expect("the relay reaches the database");
                drop(client.set_nodelay(true));
                drop(server.set_nodelay(true));
                let (client_read, client_write) = client.into_split();
                let (server_read, server_write) = server.into_split();
                let conn = Arc::new(Mutex::new(Conn::default()));
                tokio::spawn(frontend(
                    client_read,
                    server_write,
                    Arc::clone(&conn),
                    Arc::clone(&shared),
                ));
                tokio::spawn(backend(
                    server_read,
                    client_write,
                    conn,
                    Arc::clone(&shared),
                ));
            }
        });
        Self { at, ledger }
    }

    /// `options` pointed at the relay, without TLS, so the relay reads the
    /// messages.
    pub(super) fn options(&self, options: &PgConnectOptions) -> PgConnectOptions {
        options
            .clone()
            .host(&self.at.ip().to_string())
            .port(self.at.port())
            .ssl_mode(PgSslMode::Disable)
    }

    /// The URL `url` pointed at the relay, without TLS.
    pub(super) fn url(&self, url: &str) -> String {
        let mut url = url::Url::parse(url).expect("the database URL parses");
        url.set_host(Some(&self.at.ip().to_string()))
            .expect("a host");
        url.set_port(Some(self.at.port())).expect("a port");
        url.query_pairs_mut().append_pair("sslmode", "disable");
        url.into()
    }

    /// What the relay counted since the last call, and a fresh count.
    pub(super) fn take(&self) -> Ledger {
        std::mem::take(&mut *self.ledger.lock().unwrap())
    }
}

/// The protocol state of one relayed connection.
#[derive(Default)]
struct Conn {
    /// The text of each named prepared statement.
    named: BTreeMap<String, String>,
    /// The text of the last unnamed prepared statement.
    unnamed: String,
    /// The statement of the group being sent, once a message named it.
    group: Option<String>,
    /// What the group being sent has moved.
    sent: Tally,
    /// The groups sent and not yet answered by `ReadyForQuery`, oldest first.
    waiting: VecDeque<String>,
}

/// A NUL-terminated string at the start of `bytes`, and what follows it.
fn cstr(bytes: &[u8]) -> (String, &[u8]) {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let rest = bytes.get(end + 1..).unwrap_or_default();
    (
        String::from_utf8_lossy(bytes.get(..end).unwrap_or_default()).into_owned(),
        rest,
    )
}

/// `statement` with its whitespace collapsed.
fn collapsed(statement: &str) -> String {
    statement.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The length a message header at the start of `buf` declares, from its
/// four length bytes at `at`, when they have arrived.
fn declared(buf: &[u8], at: usize) -> Option<usize> {
    let bytes: [u8; 4] = buf.get(at..at + 4)?.try_into().ok()?;
    usize::try_from(u32::from_be_bytes(bytes)).ok()
}

/// Relays the client's bytes to the database, charging each message.
async fn frontend(
    mut from: OwnedReadHalf,
    mut to: OwnedWriteHalf,
    conn: Arc<Mutex<Conn>>,
    ledger: Arc<Mutex<Ledger>>,
) {
    let mut buf = Vec::new();
    let mut started = false;
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let Ok(n) = from.read(&mut chunk).await else {
            return;
        };
        if n == 0 {
            drop(to.shutdown().await);
            return;
        }
        buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
        loop {
            if !started {
                let Some(len) = declared(&buf, 0) else { break };
                if buf.len() < len {
                    break;
                }
                buf.drain(..len);
                started = true;
                let mut conn = conn.lock().unwrap();
                conn.waiting.push_back(STARTUP.to_owned());
                ledger.lock().unwrap().charge(
                    STARTUP,
                    Tally {
                        sent: u64::try_from(len).expect("a message length"),
                        ..Tally::default()
                    },
                );
                continue;
            }
            let Some(len) = declared(&buf, 1) else { break };
            if buf.len() < len + 1 {
                break;
            }
            let message: Vec<u8> = buf.drain(..=len).collect();
            sent(
                &message,
                &mut conn.lock().unwrap(),
                &mut ledger.lock().unwrap(),
            );
        }
        if to
            .write_all(chunk.get(..n).unwrap_or_default())
            .await
            .is_err()
        {
            return;
        }
    }
}

/// Charges the frontend message `message` (its type byte first).
fn sent(message: &[u8], conn: &mut Conn, ledger: &mut Ledger) {
    let bytes = u64::try_from(message.len()).expect("a message length");
    let kind = message.first().copied().unwrap_or_default();
    let body = message.get(5..).unwrap_or_default();
    match kind {
        b'p' => ledger.charge(
            STARTUP,
            Tally {
                sent: bytes,
                ..Tally::default()
            },
        ),
        b'Q' => {
            let (query, _) = cstr(body);
            let query = collapsed(&query);
            conn.waiting.push_back(query.clone());
            ledger.charge(
                &query,
                Tally {
                    calls: 1,
                    sent: bytes,
                    received: 0,
                },
            );
        }
        b'S' => {
            conn.sent.sent += bytes;
            let group = conn.group.take().unwrap_or_else(|| "(sync)".to_owned());
            let tally = std::mem::take(&mut conn.sent);
            ledger.charge(&group, tally);
            conn.waiting.push_back(group);
        }
        _ => {
            match kind {
                b'P' => {
                    let (name, rest) = cstr(body);
                    let (query, _) = cstr(rest);
                    let query = collapsed(&query);
                    if name.is_empty() {
                        conn.unnamed.clone_from(&query);
                    } else {
                        conn.named.insert(name, query.clone());
                    }
                    conn.group = Some(query);
                }
                b'B' => {
                    let (_, rest) = cstr(body);
                    let (name, _) = cstr(rest);
                    let query = if name.is_empty() {
                        conn.unnamed.clone()
                    } else {
                        conn.named.get(&name).cloned().unwrap_or(name)
                    };
                    conn.group = Some(query);
                }
                b'C' if conn.group.is_none() => conn.group = Some("(close)".to_owned()),
                b'E' => conn.sent.calls += 1,
                _ => {}
            }
            conn.sent.sent += bytes;
        }
    }
}

/// Relays the database's bytes to the client, charging each message.
async fn backend(
    mut from: OwnedReadHalf,
    mut to: OwnedWriteHalf,
    conn: Arc<Mutex<Conn>>,
    ledger: Arc<Mutex<Ledger>>,
) {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        let Ok(n) = from.read(&mut chunk).await else {
            return;
        };
        if n == 0 {
            drop(to.shutdown().await);
            return;
        }
        buf.extend_from_slice(chunk.get(..n).unwrap_or_default());
        while let Some(len) = declared(&buf, 1) {
            if buf.len() < len + 1 {
                break;
            }
            let kind = buf.first().copied().unwrap_or_default();
            buf.drain(..=len);
            let tally = Tally {
                received: u64::try_from(len).expect("a message length") + 1,
                ..Tally::default()
            };
            let mut ledger = ledger.lock().unwrap();
            if kind == b'A' {
                ledger.charge(NOTIFICATIONS, Tally { calls: 1, ..tally });
                continue;
            }
            let mut conn = conn.lock().unwrap();
            let group = conn
                .waiting
                .front()
                .cloned()
                .unwrap_or_else(|| "(unsolicited)".to_owned());
            ledger.charge(&group, tally);
            if kind == b'Z' {
                conn.waiting.pop_front();
            }
        }
        if to
            .write_all(chunk.get(..n).unwrap_or_default())
            .await
            .is_err()
        {
            return;
        }
    }
}
