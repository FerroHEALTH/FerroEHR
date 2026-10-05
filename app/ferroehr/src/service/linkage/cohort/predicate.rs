// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Step one of a cohort query: the parameterised predicate over the
//! DEMOGRAPHIC domain that yields party ids and nothing else.
//!
//! **No openEHR spec governs this — our own design/extension.** Four statement
//! shapes, one per [`PredicateKind`], each a `&'static str` chosen by the
//! binding's kind and never assembled from a caller's value; the archetype, the
//! at-code and the value are all positional binds. The statement selects
//! `vo_id` — a party identifier — and no other column, so the crossing that
//! follows carries identifiers only.
//!
//! Every relation is named UNQUALIFIED and resolves through the party pool's
//! `search_path`, which carries neither `clinical` nor `linkage`. Under
//! `[storage.party].url` that pool authenticates as `ferroehr_party`,
//! which holds no grant on either.
//!
//! The join is the storage model's own: a node row's nested-set interval
//! `[num, num_cap]` contains its whole subtree (`crate::storage::codec`), so
//! "the `at0012` ELEMENT under an `openEHR-DEMOGRAPHIC-ADDRESS.address.v1`
//! node" is one integer join, not a JSON walk.

use std::collections::BTreeSet;

use sqlx::PgPool;
use uuid::Uuid;

use crate::ids::VoId;
use crate::service::linkage::cohort::CohortError;
use crate::service::linkage::cohort::config::{PredicateBinding, PredicateKind};

/// The version-scoping and archetype-join skeleton every predicate shares,
/// spliced with its value clause at compile time so each statement stays one
/// `&'static str`.
///
/// `$1` is the ELEMENT at-code (case-folded), `$2` the archetype HRID of the
/// archetyped ancestor (case-folded), matched through the nested-set interval
/// that contains the element. The version predicate is the object's current
/// trunk version — the head row's own answer — the same currency rule every
/// other read of this store uses.
macro_rules! predicate_statement {
    ($value_clause:expr) => {
        concat!(
            "SELECT DISTINCT e.vo_id \
             FROM node e \
             JOIN node a \
               ON a.tier = e.tier AND a.vo_id = e.vo_id AND a.sys_version = e.sys_version \
               AND e.num BETWEEN a.num AND a.num_cap \
             JOIN version v \
               ON v.tier = e.tier AND v.vo_id = e.vo_id AND v.sys_version = e.sys_version \
             JOIN vo_head h \
               ON h.vo_id = v.vo_id AND h.trunk_head_sys_version = v.sys_version \
             WHERE e.tier = 'hot' \
               AND e.rm_type = 'ELEMENT' AND e.archetype = $1 AND a.archetype = $2 \
               AND ",
            $value_clause
        )
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
        PredicateKind::Text => predicate_statement!("e.data #>> '{value,value}' = $3"),
        PredicateKind::TextPrefix => {
            predicate_statement!(r"e.data #>> '{value,value}' LIKE $3 ESCAPE '\'")
        }
        PredicateKind::Coded => {
            predicate_statement!("e.data #>> '{value,defining_code,code_string}' = $3")
        }
        PredicateKind::BirthDate => predicate_statement!(
            "(e.data #>> '{value,value}')::date > current_date - make_interval(years => $4) \
             AND (e.data #>> '{value,value}')::date <= current_date - make_interval(years => $3)"
        ),
    }
}

/// Run one bound predicate and return the parties it matches.
///
/// # Errors
/// [`CohortError::InvalidValue`] when an age band does not parse, and
/// [`CohortError::Database`] when the read fails.
pub(crate) async fn matching_parties(
    pool: &PgPool,
    name: &str,
    binding: &PredicateBinding,
    value: &str,
) -> Result<BTreeSet<VoId>, CohortError> {
    // The two identifier binds are case-folded because `node.archetype` is
    // stored case-folded (`crate::storage::codec`; BASE `base_types`
    // master05 §"Composite Identifiers and Case").
    let mut query = sqlx::query_scalar::<_, Uuid>(predicate_sql(binding.kind))
        .bind(binding.node.to_ascii_lowercase())
        .bind(binding.archetype.to_ascii_lowercase());
    query = match binding.kind {
        PredicateKind::Text | PredicateKind::Coded => query.bind(value.to_owned()),
        PredicateKind::TextPrefix => query.bind(like_prefix(value)),
        PredicateKind::BirthDate => {
            let (min, max) = age_band(name, value)?;
            query.bind(min).bind(max)
        }
    };
    let ids: Vec<Uuid> = query.fetch_all(pool).await.map_err(CohortError::Database)?;
    Ok(ids.into_iter().map(VoId).collect())
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

/// Parse an inclusive age band in whole years (`"40-49"`) into the two
/// `make_interval` operands: the lower age, and the upper age plus one.
///
/// Both ends are inclusive to the caller, which the half-open date comparison
/// expresses as `> today - (max + 1) years` and `<= today - min years`.
///
/// # Errors
/// [`CohortError::InvalidValue`] for anything that is not two non-negative
/// whole numbers separated by `-` with the lower first.
fn age_band(name: &str, value: &str) -> Result<(i32, i32), CohortError> {
    let invalid = |reason: &str| CohortError::InvalidValue {
        name: name.to_owned(),
        reason: reason.to_owned(),
    };
    let (low, high) = value
        .split_once('-')
        .ok_or_else(|| invalid("an age band is two whole years separated by `-`, e.g. `40-49`"))?;
    let min: i32 = low
        .trim()
        .parse()
        .map_err(|_ignored: std::num::ParseIntError| {
            invalid("the lower age is not a whole number")
        })?;
    let max: i32 = high
        .trim()
        .parse()
        .map_err(|_ignored: std::num::ParseIntError| {
            invalid("the upper age is not a whole number")
        })?;
    if min < 0 || max < 0 {
        return Err(invalid("an age band carries no negative age"));
    }
    if min > max {
        return Err(invalid("the lower age is above the upper age"));
    }
    let upper = max
        .checked_add(1)
        .ok_or_else(|| invalid("the upper age is out of range"))?;
    Ok((min, upper))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every statement is the shared skeleton plus its own value clause, and
    /// none of them names another domain's schema or a clinical archetype.
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
                    && sql.contains("h.trunk_head_sys_version"),
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

    #[test]
    fn an_age_band_is_inclusive_at_both_ends() {
        assert_eq!(age_band("age_band", "40-49").expect("a band"), (40, 50));
        assert_eq!(age_band("age_band", " 0 - 0 ").expect("a band"), (0, 1));
    }

    #[test]
    fn a_malformed_age_band_is_refused_typed() {
        for bad in ["40", "40-", "-49", "forty-nine", "49-40", "-1--1"] {
            assert!(
                matches!(
                    age_band("age_band", bad),
                    Err(CohortError::InvalidValue { .. })
                ),
                "{bad} must be refused"
            );
        }
    }
}
