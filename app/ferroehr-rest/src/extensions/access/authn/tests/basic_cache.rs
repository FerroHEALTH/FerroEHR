// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A burst of requests carrying one Basic credential that the verified-
//! credential cache does not hold, at first sight or once its entry expired,
//! runs one Argon2id derivation, and every request of the burst is admitted
//! (#5742). A request that ran no derivation of its own is no fresh
//! authentication, so the count of fresh answers is the count of derivations.
//! No openEHR spec governs the cache: our own design.

use super::*;

/// The requests of one burst, more than the runtime has workers so every
/// worker takes one at once.
const BURST: usize = 32;

/// Sends `BURST` requests for `alice` / `pw` at once and returns how many
/// were fresh authentications; every request must be admitted.
async fn burst(auth: &Arc<Authenticator>) -> usize {
    let gate = Arc::new(tokio::sync::Barrier::new(BURST));
    let mut set = tokio::task::JoinSet::new();
    for _ in 0..BURST {
        let auth = Arc::clone(auth);
        let gate = Arc::clone(&gate);
        set.spawn(async move {
            gate.wait().await;
            auth.authenticate(&headers("Basic YWxpY2U6cHc="))
                .await
                .expect("every request of the burst is admitted")
                .fresh
        });
    }
    let mut fresh = 0;
    while let Some(joined) = set.join_next().await {
        fresh += usize::from(joined.expect("the request task completes"));
    }
    fresh
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_burst_at_first_sight_derives_the_credential_once() {
    let auth = Authenticator::new(&basic_config(60), None, memory())
        .await
        .expect("a Basic-only authenticator");
    assert_eq!(
        burst(&auth).await,
        1,
        "the misses of one credential share one derivation"
    );
    assert_eq!(
        burst(&auth).await,
        0,
        "the kept credential admits the next burst"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn a_burst_after_the_entry_expired_derives_the_credential_once() {
    let auth = Authenticator::new(&basic_config(1), None, memory())
        .await
        .expect("a Basic-only authenticator");
    assert_eq!(burst(&auth).await, 1, "the first burst derives once");
    // NOTE: an entry lives at most `cache_ttl` from its derivation, so past it
    // the cache holds nothing for the credential.
    tokio::time::sleep(Duration::from_millis(1_100)).await;
    assert_eq!(
        burst(&auth).await,
        1,
        "the misses at the entry's expiry share one derivation"
    );
}
