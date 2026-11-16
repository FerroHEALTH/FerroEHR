// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The folds the access condition's emitter applies to a verdict before it
//! renders it ([`super::access`]): what the SQL would decide alike is
//! decided here, so the planner plans no subquery that cannot change a row.
//! No openEHR spec governs this: our own design (#5667).

use crate::access_eval::scope::{DataMatch, Pred};

/// `pred` with what its SQL would decide alike folded away: a scope that
/// names an empty list covers nothing, a repeated conjunct or disjunct counts
/// once, and a two-valued predicate beside its own negation decides the
/// conjunction (false) or the disjunction (true).
///
/// The folds keep SQL's three values: only a predicate whose SQL is never
/// unknown ([`two_valued`]) is folded against its negation. The planner
/// plans every `EXISTS` the condition names, so each one folded away is
/// planning a query no longer pays for. No openEHR spec governs this: our own
/// design (#5667).
pub(super) fn simplified(pred: &Pred) -> Pred {
    match pred {
        Pred::Not(inner) => Pred::negate(simplified(inner)),
        Pred::All(preds) => folded(Pred::all(preds.iter().map(simplified).collect()), true),
        Pred::Any(preds) => folded(Pred::any(preds.iter().map(simplified).collect()), false),
        Pred::Data(data, _) if covers_nothing(data) => Pred::False,
        Pred::True | Pred::False | Pred::Data(..) | Pred::Ehr(_) => pred.clone(),
    }
}

/// `pred`, a conjunction when `all` and a disjunction otherwise, its
/// repeated members counted once and decided by a two-valued member beside
/// its negation.
fn folded(pred: Pred, all: bool) -> Pred {
    let members = match pred {
        Pred::All(members) if all => members,
        Pred::Any(members) if !all => members,
        other => return other,
    };
    let mut kept: Vec<Pred> = Vec::with_capacity(members.len());
    for member in members {
        if !kept.contains(&member) {
            kept.push(member);
        }
    }
    let decided = kept
        .iter()
        .any(|member| two_valued(member) && kept.contains(&Pred::negate(member.clone())));
    match (decided, kept.len(), all) {
        (true, _, all) => Pred::of(!all),
        (false, 1, _) => kept.pop().unwrap_or(Pred::of(all)),
        (false, _, true) => Pred::All(kept),
        (false, _, false) => Pred::Any(kept),
    }
}

/// Whether the SQL of `pred` is never unknown: every data atom is null-safe,
/// and a fact of the EHR is read from its settings row, where it may be
/// absent.
fn two_valued(pred: &Pred) -> bool {
    match pred {
        Pred::True | Pred::False | Pred::Data(..) => true,
        Pred::Ehr(_) => false,
        Pred::Not(inner) => two_valued(inner),
        Pred::All(preds) | Pred::Any(preds) => preds.iter().all(two_valued),
    }
}

/// Whether the data scope names an empty list of objects, templates,
/// archetypes or categories, which no object is among.
fn covers_nothing(data: &DataMatch) -> bool {
    let empty = |list: Option<usize>| list == Some(0);
    empty(
        data.objects
            .as_ref()
            .and_then(|objects| objects.known())
            .map(Vec::len),
    ) || empty(data.templates.as_ref().map(Vec::len))
        || empty(data.archetypes.as_ref().map(Vec::len))
        || empty(data.categories.as_ref().map(Vec::len))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::simplified;
    use crate::access_eval::scope::{DataMatch, EhrFact, Polarity, Pred};

    /// The objects of archetype `archetype`, read as a permit reads them.
    fn of_archetype(archetype: &str) -> Pred {
        Pred::Data(
            Arc::new(DataMatch {
                archetypes: Some(vec![archetype.to_owned()]),
                ..DataMatch::default()
            }),
            Polarity::Permit,
        )
    }

    /// The fact at position 0 names a party of the caller.
    fn party() -> Pred {
        Pred::Ehr(EhrFact::Party(0))
    }

    #[test]
    fn a_data_scope_beside_its_negation_decides_the_conjunction_and_the_disjunction() {
        let lab = of_archetype("openehr-ehr-observation.laboratory_test_result.v1");
        let both = Pred::All(vec![lab.clone(), Pred::negate(lab.clone())]);
        assert_eq!(simplified(&both), Pred::False);
        assert_eq!(simplified(&Pred::negate(both)), Pred::True);
        let either = Pred::Any(vec![Pred::negate(lab.clone()), lab]);
        assert_eq!(simplified(&either), Pred::True);
    }

    #[test]
    fn a_fact_of_the_ehr_beside_its_negation_stays_for_the_sql_to_decide() {
        let both = Pred::All(vec![party(), Pred::negate(party())]);
        assert_eq!(simplified(&both), both);
        let either = Pred::Any(vec![party(), Pred::negate(party())]);
        assert_eq!(simplified(&either), either);
    }

    #[test]
    fn a_repeated_member_counts_once_and_the_rest_keep_their_order() {
        let lab = of_archetype("openehr-ehr-observation.laboratory_test_result.v1");
        let note = of_archetype("openehr-ehr-evaluation.clinical_synopsis.v1");
        let repeated = Pred::All(vec![lab.clone(), party(), lab.clone(), note.clone()]);
        assert_eq!(
            simplified(&repeated),
            Pred::All(vec![lab.clone(), party(), note])
        );
        assert_eq!(simplified(&Pred::Any(vec![lab.clone(), lab.clone()])), lab);
    }

    #[test]
    fn a_scope_naming_an_empty_list_covers_nothing() {
        for data in [
            DataMatch {
                templates: Some(Vec::new()),
                ..DataMatch::default()
            },
            DataMatch {
                archetypes: Some(Vec::new()),
                ..DataMatch::default()
            },
            DataMatch {
                categories: Some(Vec::new()),
                ..DataMatch::default()
            },
        ] {
            let scope = Pred::Data(Arc::new(data), Polarity::Deny);
            assert_eq!(simplified(&scope), Pred::False);
            assert_eq!(simplified(&Pred::negate(scope)), Pred::True);
        }
        let unset = Pred::Data(Arc::new(DataMatch::default()), Polarity::Deny);
        assert_eq!(simplified(&unset), unset);
    }

    #[test]
    fn the_folds_reach_inside_nested_predicates() {
        let lab = of_archetype("openehr-ehr-observation.laboratory_test_result.v1");
        let nested = Pred::All(vec![
            party(),
            Pred::negate(Pred::All(vec![lab.clone(), Pred::negate(lab.clone())])),
            Pred::negate(lab.clone()),
        ]);
        assert_eq!(
            simplified(&nested),
            Pred::All(vec![party(), Pred::negate(lab)])
        );
    }
}
