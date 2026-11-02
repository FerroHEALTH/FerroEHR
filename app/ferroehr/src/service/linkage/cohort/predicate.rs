// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Step one of a cohort query: a bound predicate, read over the DEMOGRAPHIC
//! domain, that yields party ids and nothing else.
//!
//! **No openEHR spec governs this — our own design/extension.** The caller's
//! value is checked here and handed to
//! [`crate::storage::cohort_predicate`], whose four constant statements bind
//! it positionally.

use std::collections::BTreeSet;

use sqlx::PgPool;

use crate::ids::VoId;
use crate::service::linkage::cohort::CohortError;
use crate::service::linkage::cohort::config::PredicateBinding;
use crate::storage::cohort_predicate::{self, PredicateKind, PredicateMatch};
use crate::storage::error::StorageError;

/// Run one bound predicate and return the parties it matches.
///
/// # Errors
/// [`CohortError::InvalidValue`] when an age band does not parse,
/// [`CohortError::Acquire`] when no connection can be obtained, and
/// [`CohortError::Storage`] when the read fails.
pub(crate) async fn matching_parties(
    pool: &PgPool,
    name: &str,
    binding: &PredicateBinding,
    value: &str,
) -> Result<BTreeSet<VoId>, CohortError> {
    let matched = match binding.kind {
        PredicateKind::Text => PredicateMatch::Text(value),
        PredicateKind::TextPrefix => PredicateMatch::TextPrefix(value),
        PredicateKind::Coded => PredicateMatch::Coded(value),
        PredicateKind::BirthDate => {
            let (min, upper) = age_band(name, value)?;
            PredicateMatch::BirthDate { min, upper }
        }
    };
    cohort_predicate::matching_parties(pool, &binding.archetype, &binding.node, matched)
        .await
        .map_err(|error| match error {
            StorageError::Acquire(acquire) => CohortError::Acquire(acquire),
            other => CohortError::Storage(other),
        })
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
