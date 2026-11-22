// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ids of the search tokens and shared sets the gathered writers learned,
//! with the generation they hold under (#6242). No specification governs
//! them: our own design.

use std::collections::{BTreeMap, BTreeSet};

use sqlx::PgConnection;

use super::{IntakeEntry, IntakeToken, Named};
use crate::access_log::store::AccessLogError;
use crate::system_log::token_digest::token_digest;

/// The ids of the search tokens and shared sets the gathered writes stored,
/// with the generation of the token ids, so a later write names an entry's
/// tokens by id and the statement resolves none (#6242).
///
/// A token id holds while the store's generation is the one it was read
/// under: retention moves the generation each time it deletes a token, and
/// the statement refuses a write that names another (SQLSTATE `FE001`), on
/// which the writer forgets every id and writes again. A shared set is never
/// deleted. No specification governs it: our own design.
#[derive(Debug, Default)]
pub(super) struct TokenIds {
    known: std::sync::Mutex<Option<KnownIds>>,
}

/// The ids read under one generation.
#[derive(Debug, Default)]
pub(super) struct KnownIds {
    generation: i64,
    tokens: BTreeMap<[u8; 16], i32>,
    sets: BTreeMap<Vec<i32>, i32>,
}

impl KnownIds {
    /// The text of the `integer[]` naming `tokens` by id, as the statement
    /// writes it: the shared set's id negated, then the entry's own token ids
    /// in order; `None` when a token or the set is not known.
    pub(super) fn text_of(&self, tokens: &[IntakeToken]) -> Option<String> {
        let mut shared = Vec::new();
        let mut own = Vec::new();
        for token in tokens {
            let digest = token_digest(
                token.parameter,
                token.system.as_deref(),
                token.code.as_deref(),
            );
            let id = *self.tokens.get(&digest)?;
            if token.shares {
                shared.push(id);
            } else {
                own.push(id);
            }
        }
        shared.sort_unstable();
        shared.dedup();
        own.sort_unstable();
        let set = if shared.is_empty() {
            None
        } else {
            Some(*self.sets.get(&shared)?)
        };
        let ids: Vec<String> = set
            .map(i32::saturating_neg)
            .into_iter()
            .chain(own)
            .map(|id| id.to_string())
            .collect();
        Some(format!("{{{}}}", ids.join(",")))
    }
}

/// The most token ids [`TokenIds`] remembers before it starts again.
pub(super) const KNOWN_TOKENS: usize = 65_536;

impl TokenIds {
    pub(super) fn known(&self) -> std::sync::MutexGuard<'_, Option<KnownIds>> {
        self.known
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Names by id the search tokens of each entry whose every token and
    /// shared set is known and whose index row names none yet.
    pub(super) fn resolve(&self, entries: &[IntakeEntry], tokens: &[Vec<IntakeToken>]) -> Named {
        let known = self.known();
        let Some(known) = known.as_ref() else {
            return Named::default();
        };
        let mut named = Named {
            generation: None,
            search_tokens: Vec::with_capacity(entries.len()),
        };
        for (position, entry) in entries.iter().enumerate() {
            let text = entry
                .index
                .as_ref()
                .filter(|index| index.search_tokens.is_empty())
                .and_then(|_| known.text_of(tokens.get(position).map_or(&[][..], Vec::as_slice)));
            if text.is_some() {
                named.generation = Some(known.generation);
            }
            named.search_tokens.push(text);
        }
        named
    }

    /// Learns, after a committed statement, the ids of the tokens and shared
    /// sets of `tokens` with the generation, read in one statement.
    pub(super) async fn learn(
        &self,
        conn: &mut PgConnection,
        tokens: &[Vec<IntakeToken>],
    ) -> Result<(), AccessLogError> {
        let digests: Vec<[u8; 16]> = tokens
            .iter()
            .flatten()
            .map(|token| {
                token_digest(
                    token.parameter,
                    token.system.as_deref(),
                    token.code.as_deref(),
                )
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if digests.is_empty() {
            return Ok(());
        }
        let rows: Vec<(i64, Vec<u8>, i32)> = sqlx::query_as(
            "SELECT g.generation, t.digest, t.id \
               FROM audit.audit_search_token t, audit.audit_search_token_generation g \
              WHERE g.singleton AND t.digest = ANY($1)",
        )
        .bind(digests.iter().map(<[u8; 16]>::as_slice).collect::<Vec<_>>())
        .fetch_all(&mut *conn)
        .await?;
        let Some(generation) = rows.first().map(|(generation, _, _)| *generation) else {
            return Ok(());
        };
        let ids: BTreeMap<[u8; 16], i32> = rows
            .into_iter()
            .filter_map(|(_, digest, id)| {
                <[u8; 16]>::try_from(digest.as_slice())
                    .ok()
                    .map(|d| (d, id))
            })
            .collect();
        let shared: BTreeSet<Vec<i32>> = tokens
            .iter()
            .filter_map(|entry_tokens| {
                let mut set: Vec<i32> = entry_tokens
                    .iter()
                    .filter(|token| token.shares)
                    .map(|token| {
                        ids.get(&token_digest(
                            token.parameter,
                            token.system.as_deref(),
                            token.code.as_deref(),
                        ))
                        .copied()
                    })
                    .collect::<Option<Vec<_>>>()?;
                set.sort_unstable();
                set.dedup();
                (!set.is_empty()).then_some(set)
            })
            .collect();
        let sets: Vec<(String, i32)> = if shared.is_empty() {
            Vec::new()
        } else {
            let texts: Vec<String> = shared
                .iter()
                .map(|set| {
                    let ids: Vec<String> = set.iter().map(ToString::to_string).collect();
                    format!("{{{}}}", ids.join(","))
                })
                .collect();
            sqlx::query_as(
                "SELECT s.tokens::text, s.id FROM unnest($1::text[]) AS t(tokens) \
                   JOIN audit.audit_search_set s ON s.tokens = t.tokens::integer[]",
            )
            .bind(texts)
            .fetch_all(&mut *conn)
            .await?
        };
        let mut known = self.known();
        let fresh = known
            .as_ref()
            .is_none_or(|held| held.generation != generation || held.tokens.len() > KNOWN_TOKENS);
        if fresh {
            *known = Some(KnownIds {
                generation,
                ..KnownIds::default()
            });
        }
        if let Some(held) = known.as_mut() {
            held.tokens.extend(ids);
            for (text, id) in sets {
                let set: Option<Vec<i32>> = text
                    .trim_matches(|c| c == '{' || c == '}')
                    .split(',')
                    .filter(|part| !part.is_empty())
                    .map(|part| part.parse().ok())
                    .collect();
                if let Some(set) = set {
                    held.sets.insert(set, id);
                }
            }
        }
        Ok(())
    }

    /// Forgets every id: the store's generation moved.
    pub(super) fn forget(&self) {
        *self.known() = None;
    }
}
