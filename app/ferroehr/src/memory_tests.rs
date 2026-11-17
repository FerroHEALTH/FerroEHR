// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The weights the copies and caches report their bytes by (#5926).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): a stored canonical fragment weighed"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use moka::future::Cache;
use serde_json::json;

use super::{CACHE_ENTRY, MAP_ENTRY, Weigh, bytes, held, json_bytes};

#[test]
fn text_weighs_its_capacity() {
    let text = String::with_capacity(100);
    assert_eq!(text.heap(), 100);
    assert_eq!(bytes(&text), size_of::<String>() + 100);
}

#[test]
fn a_vector_weighs_its_spare_capacity_and_its_items() {
    let mut items: Vec<String> = Vec::with_capacity(4);
    items.push("abc".to_owned());
    let item = items.first().map_or(0, String::capacity);
    assert_eq!(items.heap(), 4 * size_of::<String>() + item);
}

#[test]
fn a_map_weighs_each_entry_and_its_header_share() {
    let map: BTreeMap<String, Arc<[u64]>> =
        BTreeMap::from([("k".to_owned(), Arc::from(vec![1_u64, 2, 3]))]);
    let key = bytes(&"k".to_owned());
    let value = size_of::<Arc<[u64]>>() + 2 * size_of::<usize>() + 3 * size_of::<u64>();
    assert_eq!(map.heap(), MAP_ENTRY + key + value);
}

#[test]
fn json_weighs_its_members_and_text() {
    let value = json!({"name": "a long enough name", "items": [1, 2, 3]});
    assert!(
        bytes(&value) > "a long enough name".len() + 4 * size_of::<serde_json::Value>(),
        "{}",
        bytes(&value)
    );
    assert_eq!(json_bytes(&value), value.to_string().len());
}

#[tokio::test]
async fn a_cache_weighs_every_entry_with_its_bookkeeping() {
    let cache: Cache<u64, String> = Cache::builder().max_capacity(16).build();
    for n in 0..4_u64 {
        cache.insert(n, "x".repeat(100)).await;
    }
    cache.run_pending_tasks().await;
    let weighed = held(&cache);
    assert_eq!(weighed.entries, 4);
    let each = size_of::<u64>() + size_of::<String>() + 100 + CACHE_ENTRY;
    assert_eq!(weighed.bytes, u64::try_from(4 * each).expect("fits"));
}
