// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The bytes this replica's in-memory copies and caches hold (#5926).
//!
//! Every copy and cache has a cap in entries or in bytes, and reports what
//! it holds as a gauge in bytes. An entry weighs its key and its value
//! inline, what they own on the heap, and the bookkeeping the cache keeps
//! per entry. The weights are estimates of our own design that leave out
//! the allocator's rounding, and a value shared between entries is weighed
//! in each. No openEHR spec governs them.

use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hash;
use std::sync::Arc;

use moka::future::Cache;
use uuid::Uuid;

/// What a `BTreeMap` spends on each entry beyond its key and value: a leaf
/// node of the standard library holds up to eleven entries beside its
/// parent link and lengths, so an entry carries a share of that header.
pub(crate) const MAP_ENTRY: usize = 8;

/// What a `moka` cache spends on each entry beyond its key and value: its
/// hash table slot, the shared key, the entry's metadata and its place on
/// the eviction order. Measured with moka 0.12.16 at about 220 to 250
/// bytes for caches of 4096 to 100 000 entries, with or without a
/// time-to-live.
pub(crate) const CACHE_ENTRY: usize = 256;

/// What an `Arc` allocation holds beside its value: the two reference
/// counts.
pub(crate) const ARC_COUNTS: usize = 2 * size_of::<usize>();

/// A value whose heap bytes this replica weighs.
pub(crate) trait Weigh {
    /// The bytes the value owns on the heap, beyond its own size.
    fn heap(&self) -> usize;
}

/// The bytes `value` takes inline and on the heap.
pub(crate) fn bytes<T: Weigh>(value: &T) -> usize {
    size_of::<T>().saturating_add(value.heap())
}

/// `bytes` as a gauge's value.
pub(crate) fn gauge(bytes: usize) -> u64 {
    u64::try_from(bytes).unwrap_or(u64::MAX)
}

/// What a copy or a cache holds: its entries and their bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Held {
    /// The entries.
    pub entries: u64,
    /// The bytes they take, as this module's weights estimate them.
    pub bytes: u64,
}

/// What `cache` holds, each entry weighed by its key and value and
/// [`CACHE_ENTRY`].
pub(crate) fn held<K, V>(cache: &Cache<K, V>) -> Held
where
    K: Weigh + Hash + Eq + Send + Sync + 'static,
    V: Weigh + Clone + Send + Sync + 'static,
{
    let (entries, total) =
        cache
            .iter()
            .fold((0_usize, 0_usize), |(entries, total), (key, value)| {
                let entry = bytes(key.as_ref())
                    .saturating_add(bytes(&value))
                    .saturating_add(CACHE_ENTRY);
                (entries.saturating_add(1), total.saturating_add(entry))
            });
    Held {
        entries: gauge(entries),
        bytes: gauge(total),
    }
}

/// The length of `value`'s JSON text, written to no buffer: the weight of a
/// value whose heap is not walked member by member, as the version copy
/// weighs its members (#5778).
pub(crate) fn json_bytes<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    let mut counted = Counted(0);
    match serde_json::to_writer(&mut counted, value) {
        // NOTE: our own design, a value that does not serialize weighs what was written of it.
        Ok(()) | Err(_) => counted.0,
    }
}

/// The length of `value`'s `Debug` rendering, written to no buffer: the
/// weight of a value whose type is too deep to walk member by member, such
/// as a lowered AQL plan. The rendering names every member and its text, so
/// it grows with the value's bytes.
pub(crate) fn debug_bytes<T: std::fmt::Debug + ?Sized>(value: &T) -> usize {
    let mut counted = Counted(0);
    match std::fmt::write(&mut counted, format_args!("{value:?}")) {
        // NOTE: our own design, a rendering that fails weighs what was written of it.
        Ok(()) | Err(_) => counted.0,
    }
}

/// A writer that counts the bytes written to it.
struct Counted(usize);

impl std::fmt::Write for Counted {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        self.0 = self.0.saturating_add(text.len());
        Ok(())
    }
}

impl std::io::Write for Counted {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 = self.0.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Values that own nothing on the heap.
macro_rules! inline {
    ($($t:ty),*) => {
        $(impl Weigh for $t {
            fn heap(&self) -> usize {
                0
            }
        })*
    };
}

inline!(
    (),
    bool,
    i32,
    i64,
    u64,
    usize,
    Uuid,
    jiff::civil::Date,
    jiff::Timestamp
);

impl Weigh for String {
    fn heap(&self) -> usize {
        self.capacity()
    }
}

impl<T: Weigh> Weigh for Option<T> {
    fn heap(&self) -> usize {
        self.as_ref().map_or(0, Weigh::heap)
    }
}

impl<T: Weigh> Weigh for Box<T> {
    fn heap(&self) -> usize {
        bytes(self.as_ref())
    }
}

impl<T: Weigh> Weigh for Arc<T> {
    fn heap(&self) -> usize {
        ARC_COUNTS.saturating_add(bytes(self.as_ref()))
    }
}

impl<T: Weigh> Weigh for Arc<[T]> {
    fn heap(&self) -> usize {
        ARC_COUNTS.saturating_add(slice(self))
    }
}

impl<T: Weigh> Weigh for Vec<T> {
    fn heap(&self) -> usize {
        let spare = self.capacity().saturating_sub(self.len());
        slice(self).saturating_add(spare.saturating_mul(size_of::<T>()))
    }
}

impl<A: Weigh, B: Weigh> Weigh for (A, B) {
    fn heap(&self) -> usize {
        self.0.heap().saturating_add(self.1.heap())
    }
}

impl<K: Weigh, V: Weigh> Weigh for BTreeMap<K, V> {
    fn heap(&self) -> usize {
        self.iter().fold(0_usize, |total, (key, value)| {
            total
                .saturating_add(MAP_ENTRY)
                .saturating_add(bytes(key))
                .saturating_add(bytes(value))
        })
    }
}

impl<T: Weigh> Weigh for BTreeSet<T> {
    fn heap(&self) -> usize {
        self.iter().fold(0_usize, |total, item| {
            total.saturating_add(MAP_ENTRY).saturating_add(bytes(item))
        })
    }
}

#[expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments the copies \
              hold, weighed member by member"
)]
impl Weigh for serde_json::Value {
    fn heap(&self) -> usize {
        match self {
            Self::Null | Self::Bool(_) | Self::Number(_) => 0,
            Self::String(text) => text.capacity(),
            Self::Array(items) => items.heap(),
            Self::Object(members) => members.iter().fold(0_usize, |total, (key, value)| {
                total
                    .saturating_add(MAP_ENTRY)
                    .saturating_add(bytes(key))
                    .saturating_add(bytes(value))
            }),
        }
    }
}

/// The heap bytes a typed RM value owns, estimated by walking it: each object
/// it holds and the text of each string attribute. No spec governs memory
/// accounting: our own estimate.
fn rm_heap(node: &dyn openehr_rm::v1_2::access::RmObject) -> usize {
    use openehr_rm::v1_2::access::{RmRef, schema};
    let mut total = 0_usize;
    for name in schema::attribute_names(node.rm_type()).unwrap_or_default() {
        if let Some(RmRef::String(text)) = node.attribute(name) {
            total = total.saturating_add(text.len());
        }
    }
    for (_, _, child) in node.children() {
        total = total
            .saturating_add(RM_OBJECT)
            .saturating_add(rm_heap(child));
    }
    total
}

/// The bytes one boxed or listed RM object takes beside its strings: no spec
/// governs memory accounting, our own estimate of a small RM struct.
const RM_OBJECT: usize = 64;

impl Weigh for openehr_rm::prelude::Ehr {
    fn heap(&self) -> usize {
        rm_heap(openehr_rm::v1_2::access::as_rm_object(self))
    }
}

/// The bytes of the items of `items`, inline and on the heap.
fn slice<T: Weigh>(items: &[T]) -> usize {
    items
        .iter()
        .fold(0_usize, |total, item| total.saturating_add(bytes(item)))
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
