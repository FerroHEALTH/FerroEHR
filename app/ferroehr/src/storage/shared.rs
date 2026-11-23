// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A repeated member of a version's node fragments, stored once (#6229).
//!
//! The writer keeps a repeated member verbatim on the first row of the version
//! that carries it (the holder) and leaves it out of every later row whose
//! member is byte-identical, recording the holder's `num` under the member's
//! key in its `shared` column. Every reader puts it back through
//! [`member_data`] or [`resolved_data`], so a fragment reads back exactly as
//! written.
//!
//! The rule, in [`Sharing::take`]: a member is shared when its value is at
//! least [`MIN_BYTES`] bytes, is byte-identical to the same key's value on an
//! earlier row of the same version, and is not one of [`UNSHARED`]. A
//! reference always names the first holder, which never shares that member
//! itself, so no reference leads to another, and a version's root row
//! (`num = 0`) has no earlier row and never shares: a reader of the root row
//! alone reads its `data` as stored. No openEHR spec governs storage layout:
//! our own design.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// The members never shared, because something reads them from the row's own
/// `data` and cannot follow a reference to another row.
///
/// `value` is read by the four value indexes through `ext.node_search_text`,
/// `ext.node_magnitude`, `ext.node_code` and `ext.node_number`; `time` and
/// `origin` are read by the writer into `event_time`. An integration test
/// holds every index on `node` to this set.
pub const UNSHARED: [&str; 3] = ["value", "time", "origin"];

/// The smallest member value, in bytes of its JSON text, that is shared.
///
/// A reader of a shared member pays a function call and a probe of its holder,
/// so only a member larger than that is shared (#6229).
pub const MIN_BYTES: usize = 512;

/// The holders of one version's shared members, as the writer meets its
/// rows in `num` order.
#[derive(Debug, Default)]
pub struct Sharing<'a> {
    /// The first row carrying each eligible member value, by key and value
    /// text.
    holders: BTreeMap<(&'a str, &'a [u8]), i32>,
}

impl<'a> Sharing<'a> {
    /// Takes the shared members out of the members of row `num`, and returns
    /// the row's `shared` column: a JSON object of each taken key and its
    /// holder's `num`, or `None` when the row shares nothing.
    ///
    /// `member` gives a member's key and the JSON text of its value as the
    /// row would store it, or `None` for a member the row stores in another
    /// form, which is never shared. A key outside `[A-Za-z0-9_]` is never
    /// shared, so `shared` needs no escaping, and a row whose fragment carries
    /// a key more than once shares nothing: the fragment keeps the key's last
    /// value.
    pub fn take<T>(
        &mut self,
        num: i32,
        members: &mut Vec<T>,
        member: impl Fn(&T) -> Option<(&'a str, &'a [u8])>,
    ) -> Option<String> {
        let eligible = |key: &str, value: &[u8]| {
            value.len() >= MIN_BYTES
                && !UNSHARED.contains(&key)
                && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        };
        let offered = || members.iter().filter_map(&member);
        // Most rows carry no member this long, and leave without allocating.
        if !offered().any(|(key, value)| eligible(key, value))
            || offered()
                .enumerate()
                .any(|(i, (key, _))| offered().skip(i + 1).any(|(other, _)| other == key))
        {
            return None;
        }
        let mut taken: Vec<(&'a str, i32)> = Vec::new();
        members.retain(|candidate| {
            let Some((key, value)) = member(candidate) else {
                return true;
            };
            if !eligible(key, value) {
                return true;
            }
            if let Some(holder) = self.holders.get(&(key, value)) {
                taken.push((key, *holder));
                return false;
            }
            self.holders.insert((key, value), num);
            true
        });
        if taken.is_empty() {
            return None;
        }
        taken.sort_unstable();
        let mut shared = String::from("{");
        for (i, (key, holder)) in taken.iter().enumerate() {
            if i > 0 {
                shared.push(',');
            }
            let _ = write!(shared, "\"{key}\":{holder}");
        }
        shared.push('}');
        Some(shared)
    }
}

/// The `data` of the row `alias` as a path that starts at the member `key`
/// reads it (an SQL expression).
///
/// The row's own `data`, or, when the row shares that member, an object of
/// the member alone, read from its holder by the domain's
/// `node_shared_member`. `key` is an SQL expression (a quoted literal or a
/// bound parameter). This and [`resolved_data`] are the only readers of
/// `node.data` but the index expressions, which read only [`UNSHARED`]
/// members, the writer, and a reader of a root row alone.
#[must_use]
pub fn member_data(alias: &str, key: &str) -> String {
    format!(
        "CASE WHEN (\"{alias}\".shared ->> {key}) IS NOT NULL \
         THEN jsonb_build_object({key}, node_shared_member({key}, \
         (\"{alias}\".shared ->> {key})::integer, \"{alias}\".tier, \"{alias}\".vo_id, \
         \"{alias}\".sys_version)) ELSE \"{alias}\".data END"
    )
}

/// The whole `data` of the row `alias`, every shared member put back from its
/// holder by the domain's `node_shared` (an SQL expression).
#[must_use]
pub fn resolved_data(alias: &str) -> String {
    format!(
        "CASE WHEN \"{alias}\".shared IS NULL THEN \"{alias}\".data \
         ELSE COALESCE(\"{alias}\".data, '{{}}'::jsonb) || node_shared(\"{alias}\".shared, \
         \"{alias}\".tier, \"{alias}\".vo_id, \"{alias}\".sys_version) END"
    )
}

/// Whether every reference of the row `alias` holds (an SQL boolean).
///
/// Each names an earlier row of the same tier and version that stores the
/// member itself and does not share it. A row that shares nothing holds
/// trivially.
#[must_use]
pub fn references_hold(alias: &str) -> String {
    format!(
        "(\"{alias}\".shared IS NULL OR NOT EXISTS (SELECT 1 FROM \
         jsonb_each_text(\"{alias}\".shared) AS s(key, num) WHERE NOT EXISTS (SELECT 1 \
         FROM node h WHERE h.tier = \"{alias}\".tier AND h.vo_id = \"{alias}\".vo_id \
         AND h.sys_version = \"{alias}\".sys_version AND h.num = s.num::int \
         AND h.num < \"{alias}\".num AND (h.data -> s.key) IS NOT NULL \
         AND (h.shared ->> s.key) IS NULL)))"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A member as the writer offers it: a key and its value text.
    type Offered = (&'static str, &'static [u8]);

    fn long(byte: u8) -> &'static [u8] {
        Box::leak(vec![byte; MIN_BYTES].into_boxed_slice())
    }

    #[test]
    fn a_repeated_member_is_taken_from_every_later_row_and_names_the_first_holder() {
        let list = long(b'a');
        let mut sharing = Sharing::default();
        let rows: [Vec<Offered>; 3] = [
            vec![("other_participations", list)],
            vec![("other_participations", list), ("subject", b"{}")],
            vec![("other_participations", list)],
        ];
        let mut out = Vec::new();
        for (num, row) in (1..).zip(rows) {
            let mut members = row;
            let shared = sharing.take(num, &mut members, |m| Some(*m));
            out.push((members, shared));
        }
        assert_eq!(out[0], (vec![("other_participations", list)], None));
        assert_eq!(
            out[1],
            (
                vec![("subject", &b"{}"[..])],
                Some(r#"{"other_participations":1}"#.to_owned())
            )
        );
        assert_eq!(
            out[2],
            (Vec::new(), Some(r#"{"other_participations":1}"#.to_owned()))
        );
    }

    #[test]
    fn short_unshared_differing_duplicated_and_unusual_members_stay() {
        let mut sharing = Sharing::default();
        let short: &[u8] = &[b'x'; MIN_BYTES - 1];
        let first: Vec<Offered> = vec![
            ("value", long(b'v')),
            ("time", long(b't')),
            ("origin", long(b'o')),
            ("narrative", short),
            ("feeder_audit", long(b'f')),
            ("other-key", long(b'k')),
        ];
        let mut members = first.clone();
        assert_eq!(sharing.take(1, &mut members, |m| Some(*m)), None);
        let mut members: Vec<Offered> = first
            .iter()
            .map(|m| {
                if m.0 == "feeder_audit" {
                    ("feeder_audit", long(b'g'))
                } else {
                    *m
                }
            })
            .collect();
        assert_eq!(
            sharing.take(2, &mut members, |m| Some(*m)),
            None,
            "{members:?}"
        );
        let mut members = first;
        let shared = sharing.take(3, &mut members, |m| Some(*m));
        assert_eq!(shared, Some(r#"{"feeder_audit":1}"#.to_owned()));
        for kept in ["value", "time", "origin", "narrative", "other-key"] {
            assert!(members.iter().any(|m| m.0 == kept), "{kept} stays");
        }
        let mut twice: Vec<Offered> = vec![("links", long(b'l')), ("links", long(b'l'))];
        assert_eq!(
            sharing.take(4, &mut twice, |m| Some(*m)),
            None,
            "a repeated key"
        );
        assert_eq!(twice.len(), 2);
    }
}
