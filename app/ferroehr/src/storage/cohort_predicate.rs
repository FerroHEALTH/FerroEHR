// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cohort predicate statements: the parameterised read over the
//! DEMOGRAPHIC domain that yields party ids and nothing else.
//!
//! **No openEHR spec governs this — our own design/extension.** Four statement
//! shapes, one per [`PredicateKind`], each a `&'static str` chosen by the
//! kind and never assembled from a caller's value; the archetype, the at-code
//! and the value are all positional binds. The statement selects `vo_id` — a
//! party identifier — and no other column, so the crossing that follows
//! carries identifiers only.
//!
//! Every relation is named UNQUALIFIED and resolves through the party pool's
//! `search_path`, which does not carry `clinical`. Under
//! `[database.party].url` that pool authenticates as `ferroehr_party`,
//! which holds no grant on it.
//!
//! The join is the storage model's own: a node row's nested-set interval
//! `[num, num_cap]` contains its whole subtree (`crate::storage::codec`), so
//! "the `at0012` ELEMENT under an `openEHR-DEMOGRAPHIC-ADDRESS.address.v1`
//! node" is one integer join, not a JSON walk.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::db::acquire::acquire;
use crate::ids::VoId;
use crate::storage::error::StorageError;

/// How a bound predicate's value is matched against the stored leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateKind {
    /// Exact equality against a `DV_TEXT`-shaped leaf's `value/value`.
    Text,
    /// Prefix match against a `DV_TEXT`-shaped leaf's `value/value`.
    TextPrefix,
    /// Exact equality against a `DV_CODED_TEXT` leaf's
    /// `value/defining_code/code_string`.
    Coded,
    /// An inclusive age band in whole years (`"40-49"`) against a `DV_DATE`
    /// leaf's `value/value`.
    BirthDate,
}

impl PredicateKind {
    /// The kind's configuration spelling, for boot logs and refusals.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::TextPrefix => "text_prefix",
            Self::Coded => "coded",
            Self::BirthDate => "birth_date",
        }
    }
}

/// One predicate's match against the leaf, with the values it binds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredicateMatch<'a> {
    /// [`PredicateKind::Text`]: the value the leaf equals.
    Text(&'a str),
    /// [`PredicateKind::TextPrefix`]: the prefix the leaf starts with.
    TextPrefix(&'a str),
    /// [`PredicateKind::Coded`]: the code string the leaf equals.
    Coded(&'a str),
    /// [`PredicateKind::BirthDate`]: the lower age and the upper age plus one,
    /// in whole years.
    BirthDate {
        /// The lowest age matched.
        min: i32,
        /// One above the highest age matched.
        upper: i32,
    },
}

impl PredicateMatch<'_> {
    /// The statement shape this match runs.
    #[must_use]
    pub const fn kind(self) -> PredicateKind {
        match self {
            Self::Text(_) => PredicateKind::Text,
            Self::TextPrefix(_) => PredicateKind::TextPrefix,
            Self::Coded(_) => PredicateKind::Coded,
            Self::BirthDate { .. } => PredicateKind::BirthDate,
        }
    }
}

/// The version-scoping and archetype-join skeleton every predicate shares,
/// spliced with its value clause at compile time so each statement stays one
/// `&'static str`.
///
/// `$1` is the ELEMENT at-code (case-folded), `$2` the archetype HRID of the
/// archetyped ancestor (case-folded), matched through the nested-set interval
/// that contains the element. The version predicate is the object's current
/// trunk version — the head row's own answer — the same currency rule every
/// other read of this store uses, linked through the head's content key so an
/// element row shared by a chain of identical versions is read once (#4944).
macro_rules! predicate_statement {
    ($value_clause:expr) => {
        concat!(
            "SELECT DISTINCT e.vo_id \
             FROM node e \
             JOIN node a \
               ON a.tier = e.tier AND a.vo_id = e.vo_id AND a.sys_version = e.sys_version \
               AND e.num BETWEEN a.num AND a.num_cap \
             JOIN vo_head h \
               ON h.tier = e.tier AND h.vo_id = e.vo_id AND h.trunk_head_content_key = e.sys_version \
             WHERE e.tier = 'hot' \
               AND e.rm_type = 'ELEMENT' AND e.archetype = $1 AND a.archetype = $2 \
               AND ",
            $value_clause
        )
    };
}

/// The ELEMENT's value as canonical JSON, whether the row stores it in its
/// fragment or in the typed value columns (`crate::storage::slim`).
macro_rules! element_value {
    () => {
        "COALESCE(e.data -> 'value', ext.node_value(e.value_type, e.value_num, e.value_text, \
         e.value_code, e.value_terminology))"
    };
}

/// The statement `kind` runs, in full.
///
/// Exposed so a test can pin the four texts: the property that matters is that
/// they are constants, so no caller-supplied byte can ever reach the SQL, and a
/// constant is only checkable if it can be read.
#[must_use]
pub fn predicate_sql(kind: PredicateKind) -> &'static str {
    match kind {
        PredicateKind::Text => {
            predicate_statement!(concat!(element_value!(), " #>> '{value}' = $3"))
        }
        PredicateKind::TextPrefix => {
            predicate_statement!(concat!(
                element_value!(),
                r" #>> '{value}' LIKE $3 ESCAPE '\'"
            ))
        }
        PredicateKind::Coded => {
            predicate_statement!(concat!(
                element_value!(),
                " #>> '{defining_code,code_string}' = $3"
            ))
        }
        PredicateKind::BirthDate => predicate_statement!(concat!(
            "(",
            element_value!(),
            " #>> '{value}')::date > current_date - make_interval(years => $4) AND (",
            element_value!(),
            " #>> '{value}')::date <= current_date - make_interval(years => $3)"
        )),
    }
}

/// The parties on `pool`, the party domain's, whose current version holds an
/// ELEMENT `node` under an `archetype` node that `matched` selects.
///
/// # Errors
/// [`StorageError::Acquire`] when no connection can be obtained, and
/// [`StorageError::Database`] when the read fails.
pub async fn matching_parties(
    pool: &PgPool,
    archetype: &str,
    node: &str,
    matched: PredicateMatch<'_>,
) -> Result<BTreeSet<VoId>, StorageError> {
    // The two identifier binds are case-folded because `node.archetype` is
    // stored case-folded (`crate::storage::codec`; BASE `base_types`
    // master05 §"Composite Identifiers and Case").
    let query = sqlx::query_scalar::<_, VoId>(predicate_sql(matched.kind()))
        .bind(node.to_ascii_lowercase())
        .bind(archetype.to_ascii_lowercase());
    let query = match matched {
        PredicateMatch::Text(value) | PredicateMatch::Coded(value) => query.bind(value.to_owned()),
        PredicateMatch::TextPrefix(prefix) => query.bind(like_prefix(prefix)),
        PredicateMatch::BirthDate { min, upper } => query.bind(min).bind(upper),
    };
    let ids: Vec<VoId> = query.fetch_all(&mut *acquire(pool).await?).await?;
    Ok(ids.into_iter().collect())
}

/// The `LIKE` pattern matching every value starting with `value`.
///
/// The three `LIKE` metacharacters PostgreSQL recognises are escaped with the
/// default escape character, which the statement names explicitly
/// (<https://www.postgresql.org/docs/18/functions-matching.html>), so a
/// caller's `%` matches a literal percent sign rather than everything.
fn like_prefix(value: &str) -> String {
    let mut pattern = String::with_capacity(value.len() + 1);
    for ch in value.chars() {
        if matches!(ch, '\\' | '%' | '_') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every statement is the shared skeleton plus its own value clause, and
    /// none of them names another domain's schema or a clinical archetype.
    /// The skeleton reads the current trunk version by linking the element
    /// row to the head's content key, one head row per object (#4944).
    #[test]
    fn every_statement_is_a_constant_over_the_demographic_domain() {
        for kind in [
            PredicateKind::Text,
            PredicateKind::TextPrefix,
            PredicateKind::Coded,
            PredicateKind::BirthDate,
        ] {
            let sql = predicate_sql(kind);
            assert!(
                sql.starts_with("SELECT DISTINCT e.vo_id FROM node e ")
                    && sql.contains("e.num BETWEEN a.num AND a.num_cap")
                    && sql.contains("h.trunk_head_content_key = e.sys_version"),
                "{} must share the skeleton: {sql}",
                kind.as_str()
            );
            for forbidden in [
                "clinical.",
                "party.",
                "linkage.",
                "ehr.",
                "demographic.",
                "subject_ehr",
                "openehr-ehr-",
            ] {
                assert!(
                    !sql.contains(forbidden),
                    "{} must not name `{forbidden}`: {sql}",
                    kind.as_str()
                );
            }
        }
    }

    #[test]
    fn like_metacharacters_in_a_prefix_are_escaped() {
        assert_eq!(like_prefix("97"), "97%");
        assert_eq!(like_prefix("9%7_a\\b"), r"9\%7\_a\\b%");
    }
}
