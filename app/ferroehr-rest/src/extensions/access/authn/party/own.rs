// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login of a record's person: a `persons` caller whose one `PERSON`
//! carries no professional and no representative marker.
//!
//! The shared binding answers such a login `wrong_marker`, since a `Person`
//! caller needs a professional party there. An identity bound to a
//! patient's `PERSON` is that patient (EHDS Art. 3), so FerroEHR admits it
//! as the person themself: it holds no role, and the access evaluator
//! confines it to the EHRs whose `EHR_STATUS.subject` names that `PERSON`.
//! No openEHR spec governs resolving a login to a party: our own design
//! (#6106).

use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::{Marker, Outcome, PartyRef};
use ferrohealth_smart::resource::Caller;

/// Whether `outcome`, the binding of a caller of `kind`, binds the person
/// themself: a plain `persons` login, neither a guest nor a delegate, whose
/// one live `PERSON` carries neither marker.
pub(in crate::extensions::access::authn) fn binds_the_person(
    kind: &CallerKind,
    outcome: &Outcome,
) -> bool {
    matches!(kind, CallerKind::Person)
        && matches!(
            outcome,
            Outcome::WrongMarker {
                party,
                markers,
                needed: Marker::Professional,
            } if party.type_name() == "PERSON" && !markers.professional && !markers.representative
        )
}

/// The `PERSON` the login binding bound `caller` to as the person themself
/// ([`binds_the_person`]); `None` for any other caller.
pub(crate) fn own_person(caller: &Caller) -> Option<&PartyRef> {
    let outcome = caller.party()?;
    if binds_the_person(caller.kind(), outcome) {
        outcome.party()
    } else {
        None
    }
}

/// The roles `caller` holds: `honoured`, or none for the person themself,
/// who reads their record for who they are and holds no professional right
/// (EHDS Art. 3).
pub(in crate::extensions::access::authn) fn roles(
    caller: &Caller,
    honoured: impl FnOnce() -> Vec<String>,
) -> Vec<String> {
    if own_person(caller).is_some() {
        Vec::new()
    } else {
        honoured()
    }
}

#[cfg(test)]
mod tests {
    use ferrohealth_smart::party::Markers;

    use super::*;

    fn wrong(class: &str, markers: Markers, needed: Marker) -> Outcome {
        Outcome::WrongMarker {
            party: PartyRef::new("ferroehr", class, "p-1"),
            markers,
            needed,
        }
    }

    #[test]
    fn a_person_login_on_a_plain_person_binds_the_person() {
        let outcome = wrong("PERSON", Markers::default(), Marker::Professional);
        assert!(binds_the_person(&CallerKind::Person, &outcome));
    }

    #[test]
    fn a_representative_row_a_delegate_or_another_class_binds_no_person() {
        let rep = wrong(
            "PERSON",
            Markers {
                professional: false,
                representative: true,
            },
            Marker::Professional,
        );
        assert!(!binds_the_person(&CallerKind::Person, &rep));
        let plain = wrong("PERSON", Markers::default(), Marker::Professional);
        assert!(!binds_the_person(
            &CallerKind::Guest(Box::new(CallerKind::Person)),
            &plain
        ));
        let agent = wrong("AGENT", Markers::default(), Marker::Professional);
        assert!(!binds_the_person(&CallerKind::Person, &agent));
        let needs_rep = wrong("PERSON", Markers::default(), Marker::Representative);
        assert!(!binds_the_person(&CallerKind::Person, &needs_rep));
    }
}
