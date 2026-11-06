// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `DPoP` proof replay store over the clinical pool (#5072): a proof one
//! replica accepted is refused as a replay on another, every write is one
//! atomic decision, a record past its window is replaced and swept, and a
//! store that cannot answer admits nothing (RFC 9449 §11.1).
//!
//! Each "replica" is a service over a pool of its own to the one test
//! database, as two FerroEHR processes reach one PostgreSQL. The proofs are
//! real: an ES256 key signs each, and the `ferrohealth-smart` proof check reads
//! it, so the store is asked exactly what the server asks it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::panic_in_result_fn,
    reason = "test assertions, diagnostics and fixtures"
)]

use std::sync::Arc;
use std::time::SystemTime;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ferroehr::service::FerroEhrService;
use ferroehr::storage::dpop_replay::{PgReplayStore, SWEEP_EVERY};
use ferrohealth_smart::refusal::Reason;
use ferrohealth_smart::resource::Rejection;
use ferrohealth_smart::sender::Check;
use ferrohealth_smart::sender::proof::{Presented, Proof, ProofCheck, ProofSettings};
use ferrohealth_smart::sender::replay::ReplayStore;
use http::{HeaderMap, HeaderValue, Method};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::json;
use sqlx::PgPool;

/// The base URL the proofs are addressed to.
const EXTERNAL: &str = "https://ehr.example";

/// The request every proof is made for.
const PATH: &str = "/ferroehr/rest/openehr/v1/ehr";

/// The access token every proof is bound to.
const TOKEN: &str = "an-opaque-access-token";

/// One FerroEHR process: its own pool to the shared database, and the proof
/// check over its replay store.
struct Replica {
    /// The store, asked directly by the tests that set a window's end.
    store: Arc<PgReplayStore>,
    /// The check the server runs.
    check: ProofCheck,
}

impl Replica {
    /// A replica over a pool of its own to the database at `url`.
    async fn over(url: &str) -> Self {
        let pool = PgPool::connect(url).await.expect("a pool of the replica's own");
        let service =
            FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
        let store = Arc::new(PgReplayStore::of(&service));
        let settings = ProofSettings::new(EXTERNAL.parse().expect("the external URL"));
        let shared: Arc<dyn ReplayStore> = Arc::<PgReplayStore>::clone(&store);
        let check = ProofCheck::new(settings, shared).expect("the proof settings hold");
        Self { store, check }
    }

    /// Checks `jwt` as the server does before it spends the proof.
    fn read(&self, jwt: &str) -> Proof {
        let mut headers = HeaderMap::new();
        headers.insert("dpop", HeaderValue::from_str(jwt).expect("a header value"));
        self.check
            .check(&Presented {
                method: &Method::GET,
                path: PATH,
                headers: &headers,
                token: TOKEN,
                certificate: None,
            })
            .expect("the proof passes every check but the replay check")
    }
}

/// Signs a `DPoP` proof for [`PATH`] with a fresh ES256 key and `jti`.
fn proof(jti: &str) -> String {
    let key = rcgen::KeyPair::generate().expect("an ES256 key");
    let point = key.public_key_raw();
    let (x, y) = point
        .get(1..33)
        .zip(point.get(33..65))
        .expect("an uncompressed P-256 point");
    let jwk = serde_json::from_value(json!({
        "kty": "EC",
        "crv": "P-256",
        "x": URL_SAFE_NO_PAD.encode(x),
        "y": URL_SAFE_NO_PAD.encode(y),
    }))
    .expect("a public JWK");
    let mut header = Header::new(Algorithm::ES256);
    header.typ = Some("dpop+jwt".to_owned());
    header.jwk = Some(jwk);
    let ath = {
        use sha2::Digest as _;
        URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(TOKEN.as_bytes()))
    };
    let iat = jiff::Timestamp::now().as_second();
    let claims = json!({
        "jti": jti,
        "htm": "GET",
        "htu": format!("{EXTERNAL}{PATH}"),
        "iat": iat,
        "ath": ath,
    });
    let signing = EncodingKey::from_ec_pem(key.serialize_pem().as_bytes()).expect("a PEM key");
    encode(&header, &claims, &signing).expect("a signed proof")
}

/// Whether `outcome` is the replay refusal.
fn replayed(outcome: &Result<(), Rejection>) -> bool {
    matches!(
        outcome,
        Err(Rejection::Invalid {
            reason: Reason::Sender(Check::ProofReplayed)
        })
    )
}

#[tokio::test]
async fn a_proof_one_replica_accepted_is_a_replay_on_another() {
    let db = testkit::db().await.expect("testkit database");
    let first = Replica::over(db.url()).await;
    let second = Replica::over(db.url()).await;
    let jwt = proof("one-proof");

    first
        .check
        .consume(&first.read(&jwt))
        .await
        .expect("the first replica accepts the proof");
    let again = second.check.consume(&second.read(&jwt)).await;
    assert!(
        replayed(&again),
        "RFC 9449 §11.1: the second replica refuses the replay: {again:?}"
    );
    let own = first.check.consume(&first.read(&jwt)).await;
    assert!(replayed(&own), "and so does the first: {own:?}");
    let other = second.check.consume(&second.read(&proof("another-proof"))).await;
    assert!(other.is_ok(), "a proof of its own passes: {other:?}");
}

#[tokio::test]
async fn proofs_racing_over_two_replicas_admit_exactly_one() {
    let db = testkit::db().await.expect("testkit database");
    let replicas = Arc::new([Replica::over(db.url()).await, Replica::over(db.url()).await]);
    let jwt = Arc::new(proof("raced-proof"));
    let mut tasks = Vec::new();
    for index in 0..16_usize {
        let replicas = Arc::clone(&replicas);
        let jwt = Arc::clone(&jwt);
        tasks.push(tokio::spawn(async move {
            let replica = replicas.get(index % 2).expect("two replicas");
            replica.check.consume(&replica.read(&jwt)).await
        }));
    }
    let mut admitted = 0;
    for task in tasks {
        let outcome = task.await.expect("the task runs");
        if outcome.is_ok() {
            admitted += 1;
        } else {
            assert!(replayed(&outcome), "every other one is a replay: {outcome:?}");
        }
    }
    assert_eq!(admitted, 1, "one insert decides for every replica");
}

#[tokio::test]
async fn a_record_past_its_window_is_replaced_and_swept() {
    let db = testkit::db().await.expect("testkit database");
    let replica = Replica::over(db.url()).await;
    let ended = replica.read(&proof("ended-proof"));
    let standing = replica.read(&proof("standing-proof"));
    let past = SystemTime::from(jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1));

    assert!(replica.store.record(ended.id(), past).await.expect("record"));
    assert!(
        replica.store.record(ended.id(), past).await.expect("record"),
        "a record whose window ended is replaced, not held against the proof"
    );
    assert!(
        replica
            .store
            .record(standing.id(), standing.until())
            .await
            .expect("record")
    );
    assert_eq!(replica.store.sweep().await.expect("sweep"), 1, "the ended record goes");
    assert!(
        !replica
            .store
            .record(standing.id(), standing.until())
            .await
            .expect("record"),
        "the standing record stays"
    );
}

#[tokio::test]
async fn periodic_sweeps_bound_the_table() {
    let db = testkit::db().await.expect("testkit database");
    let replica = Replica::over(db.url()).await;
    let past = SystemTime::from(jiff::Timestamp::now() - jiff::SignedDuration::from_hours(1));
    let ended = SWEEP_EVERY * 2;
    for index in 0..ended {
        let proof = replica.read(&proof(&format!("ended-{index}")));
        replica.store.record(proof.id(), past).await.expect("record");
    }
    let pool = db.pool();
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM dpop_proof")
        .fetch_one(&pool)
        .await
        .expect("count the records");
    assert!(
        held < i64::try_from(ended).expect("a small count"),
        "the sweeps that ran on the way removed ended records: {held} of {ended} remain"
    );
}

#[tokio::test]
async fn a_store_that_cannot_answer_admits_nothing() {
    let db = testkit::db().await.expect("testkit database");
    let replica = Replica::over(db.url()).await;
    let jwt = proof("unanswered-proof");
    let proof = replica.read(&jwt);
    sqlx::query("ALTER TABLE dpop_proof RENAME TO dpop_proof_gone")
        .execute(&db.pool())
        .await
        .expect("take the table away");
    let outcome = replica.check.consume(&proof).await;
    assert!(
        matches!(outcome, Err(Rejection::Upstream { .. })),
        "a store fault fails closed, answered 503: {outcome:?}"
    );
}
