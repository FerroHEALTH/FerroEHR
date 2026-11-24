// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared part of a commit audit (#6395): the members no record subject
//! can be named by, stored once per distinct content in
//! `commit_audit_shared`, and the ids of the parts this process has seen.
//!
//! A commit whose part this process knows binds its id; one it does not
//! know inserts the part in its own statement (`ON CONFLICT DO SELECT`) and
//! learns the id once another transaction's commit made the row. No openEHR
//! spec governs the storage: our own design.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;

use arc_swap::ArcSwap;
use sha2::{Digest as _, Sha256};

/// The most shared parts a process keeps ids for; a part past it is inserted
/// by its statement every time, which stays correct.
const KEPT: usize = 4096;

/// A shared part: its members and the SHA-256 of its JSON text, its key. The
/// text is written only for a statement that inserts the part.
#[derive(Debug, Clone)]
pub struct SharedPart<'a> {
    system_id: &'a str,
    committer: Option<String>,
    description: Option<String>,
    digest: [u8; 32],
}

impl<'a> SharedPart<'a> {
    /// The part of `system_id`, with the canonical JSON text of the committer
    /// and of the description when they are shared.
    #[must_use]
    pub fn new(system_id: &'a str, committer: Option<String>, description: Option<String>) -> Self {
        let mut part = Self {
            system_id,
            committer,
            description,
            digest: [0; 32],
        };
        let mut hasher = Sha256::new();
        part.write(|piece| hasher.update(piece));
        part.digest = hasher.finalize().into();
        part
    }

    /// Hands the part's JSON text to `out` piece by piece.
    fn write(&self, mut out: impl FnMut(&str)) {
        out("{\"committer\":");
        out(self.committer.as_deref().unwrap_or("null"));
        out(",\"description\":");
        out(self.description.as_deref().unwrap_or("null"));
        out(",\"system_id\":\"");
        let mut rest = self.system_id;
        while let Some((at, c, escaped)) = rest
            .char_indices()
            .find_map(|(at, c)| escape(c).map(|escaped| (at, c, escaped)))
        {
            let (plain, special) = rest.split_at(at);
            out(plain);
            out(escaped);
            rest = special.split_at(c.len_utf8()).1;
        }
        out(rest);
        out("\"}");
    }

    /// The part's JSON text, which its insert binds.
    #[must_use]
    pub fn content(&self) -> String {
        let mut text = String::new();
        self.write(|piece| text.push_str(piece));
        text
    }

    /// The part's key.
    #[must_use]
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

/// The JSON escape of `c` when it needs one: a quote, a backslash or a
/// control character (RFC 8259 §7).
fn escape(c: char) -> Option<&'static str> {
    const CONTROL: [&str; 32] = [
        "\\u0000", "\\u0001", "\\u0002", "\\u0003", "\\u0004", "\\u0005", "\\u0006", "\\u0007",
        "\\u0008", "\\u0009", "\\u000a", "\\u000b", "\\u000c", "\\u000d", "\\u000e", "\\u000f",
        "\\u0010", "\\u0011", "\\u0012", "\\u0013", "\\u0014", "\\u0015", "\\u0016", "\\u0017",
        "\\u0018", "\\u0019", "\\u001a", "\\u001b", "\\u001c", "\\u001d", "\\u001e", "\\u001f",
    ];
    match c {
        '"' => Some("\\\""),
        '\\' => Some("\\\\"),
        _ => u8::try_from(c)
            .ok()
            .and_then(|byte| CONTROL.get(usize::from(byte)))
            .copied(),
    }
}

/// The ids of the shared parts one domain's table holds that this process
/// has seen committed, read without a lock.
#[derive(Debug, Default)]
pub struct SharedIds {
    ids: ArcSwap<BTreeMap<[u8; 32], i32>>,
}

impl SharedIds {
    /// The id of the part `digest`, when this process has seen it.
    #[must_use]
    pub fn get(&self, digest: &[u8; 32]) -> Option<i32> {
        self.ids.load().get(digest).copied()
    }

    /// Keeps the id of a part another transaction committed.
    pub fn remember(&self, digest: [u8; 32], id: i32) {
        self.ids.rcu(|ids| {
            if ids.len() >= KEPT || ids.contains_key(&digest) {
                return Arc::clone(ids);
            }
            let mut next = BTreeMap::clone(ids);
            next.insert(digest, id);
            Arc::new(next)
        });
    }
}

/// The shared parts of both pseudonymisation domains, each table numbering
/// its own.
#[derive(Debug, Default)]
pub struct AuditShares {
    clinical: SharedIds,
    party: SharedIds,
}

impl AuditShares {
    /// The ids of the party domain's parts when `party`, else the clinical
    /// domain's.
    #[must_use]
    pub fn of(&self, party: bool) -> &SharedIds {
        if party { &self.party } else { &self.clinical }
    }
}

/// How one audit of a statement names its shared part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedRef {
    /// The part's id, bound in the audit's first parameter.
    Known(i32),
    /// The part's JSON text, bound in the audit's first parameter, inserted
    /// by the CTE of the given index unless the table holds it.
    New(usize),
}

/// The shared parts of one statement's audits: how each audit names its
/// part, and the parts the statement inserts, once each.
#[derive(Debug, Default)]
pub(crate) struct SharedPlan {
    refs: Vec<SharedRef>,
    new: Vec<[u8; 32]>,
}

impl SharedPlan {
    /// The plan of `parts`, the statement's audits in order, against the ids
    /// `known` holds.
    pub(crate) fn of<'p, 'a: 'p>(
        known: &SharedIds,
        parts: impl Iterator<Item = &'p SharedPart<'a>>,
    ) -> Self {
        let mut plan = Self::default();
        for part in parts {
            let shared = match known.get(&part.digest) {
                Some(id) => SharedRef::Known(id),
                None => SharedRef::New(
                    plan.new
                        .iter()
                        .position(|digest| *digest == part.digest)
                        .unwrap_or_else(|| {
                            plan.new.push(part.digest);
                            plan.new.len() - 1
                        }),
                ),
            };
            plan.refs.push(shared);
        }
        plan
    }

    /// The plan whose one audit's part is new, or known, by its kind alone:
    /// what a statement text cached per kind is built from.
    pub(crate) fn single(new: bool) -> Self {
        if new {
            Self {
                refs: vec![SharedRef::New(0)],
                new: vec![[0; 32]],
            }
        } else {
            Self {
                refs: vec![SharedRef::Known(0)],
                new: Vec::new(),
            }
        }
    }

    /// How the audit at `index` names its part.
    #[expect(
        clippy::indexing_slicing,
        reason = "a plan holds one reference per audit of the statement it was made for"
    )]
    pub(crate) fn audit(&self, index: usize) -> SharedRef {
        self.refs[index]
    }

    /// Whether the statement inserts a part.
    pub(crate) fn inserts(&self) -> bool {
        !self.new.is_empty()
    }

    /// The CTEs inserting the statement's new parts, each from the first
    /// parameter of the first audit naming it, the audits' first parameters
    /// being `firsts`; empty when every part is known.
    pub(crate) fn ctes(&self, firsts: &[usize]) -> String {
        let mut sql = String::new();
        for at in 0..self.new.len() {
            let Some(first) = self
                .refs
                .iter()
                .zip(firsts)
                .find(|(shared, _)| **shared == SharedRef::New(at))
                .map(|(_, first)| *first)
            else {
                continue;
            };
            let _ = write!(sql, ", {}", insert_cte(at, first));
        }
        sql
    }

    /// The result column naming, for each new part in order, its id when
    /// another transaction committed it, else NULL; empty when every part is
    /// known.
    pub(crate) fn learned_column(&self) -> String {
        if self.new.is_empty() {
            return String::new();
        }
        let ids: Vec<String> = (0..self.new.len())
            .map(|at| format!("(SELECT CASE WHEN ours THEN NULL ELSE id END FROM s{at})"))
            .collect();
        format!(", ARRAY[{}]::int4[] AS shared_learned", ids.join(", "))
    }

    /// Keeps in `known` each id the statement's result `learned` names.
    pub(crate) fn remember(&self, known: &SharedIds, learned: &[Option<i32>]) {
        for (digest, id) in self.new.iter().zip(learned) {
            if let Some(id) = id {
                known.remember(*digest, *id);
            }
        }
    }
}

/// The value an audit's insert stores as its shared part, for an audit whose
/// first parameter is `first`.
pub(crate) fn shared_value(shared: SharedRef, first: usize) -> String {
    match shared {
        SharedRef::Known(_) => format!("${first}"),
        SharedRef::New(at) => format!("(SELECT id FROM s{at})"),
    }
}

/// The CTE `s{at}` that stores the part whose JSON text is `$first` unless
/// the table holds it, and answers its id and whether this transaction
/// wrote the row: a part a concurrent commit stores first is answered once
/// that commit ends (PostgreSQL 19, `INSERT`, "ON CONFLICT DO SELECT ...
/// allows an atomic INSERT or SELECT outcome").
fn insert_cte(at: usize, first: usize) -> String {
    format!(
        "s{at} AS ( \
             INSERT INTO commit_audit_shared (digest, system_id, description, committer) \
             SELECT sha256(convert_to(${first}, 'UTF8')), j->>'system_id', \
                    nullif(j->'description', 'null'::jsonb), nullif(j->'committer', 'null'::jsonb) \
             FROM (SELECT ${first}::jsonb AS j) p \
             ON CONFLICT (digest) DO SELECT \
             RETURNING id, xmin = pg_current_xact_id()::xid AS ours \
         )"
    )
}

#[cfg(test)]
mod tests {
    use super::{SharedIds, SharedPart, SharedPlan, SharedRef};

    #[test]
    fn a_part_is_keyed_by_its_text() {
        let part = SharedPart::new("sys", Some(r#"{"_type":"PARTY_SELF"}"#.to_owned()), None);
        assert_eq!(
            part.content(),
            r#"{"committer":{"_type":"PARTY_SELF"},"description":null,"system_id":"sys"}"#
        );
        let quoted = SharedPart::new("a\"b\\c\u{1}", None, None);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&quoted.content()).expect("JSON")["system_id"],
            "a\"b\\c\u{1}"
        );
        assert_ne!(
            part.digest(),
            SharedPart::new("sys", None, None).digest(),
            "a different part has a different key"
        );
    }

    #[test]
    fn a_statement_inserts_each_unknown_part_once() {
        let known = SharedIds::default();
        let first = SharedPart::new("a", None, None);
        let second = SharedPart::new("b", None, None);
        known.remember(*second.digest(), 7);
        let plan = SharedPlan::of(&known, [&first, &second, &first].into_iter());
        assert!(plan.inserts());
        assert_eq!(plan.audit(0), SharedRef::New(0));
        assert_eq!(plan.audit(1), SharedRef::Known(7));
        assert_eq!(plan.audit(2), SharedRef::New(0));
        assert_eq!(plan.ctes(&[1, 6, 11]).matches(" AS (").count(), 1);
        plan.remember(&known, &[Some(9)]);
        assert_eq!(known.get(first.digest()), Some(9));
    }
}
