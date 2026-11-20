// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login of a record's person: a `persons` caller whose one `PERSON`
//! carries no professional and no representative marker.
//!
//! The shared binding answers such a login `wrong_marker`, since a `Person`
//! or a `Delegated` caller needs a professional party there. An identity bound to a
//! patient's `PERSON` is that patient (EHDS Art. 3), so FerroEHR admits it
//! as the person themself: it holds no role, and the access evaluator
//! confines it to the EHRs whose `EHR_STATUS.subject` names that `PERSON`.
//! A token `ferrohealth-smart` reads as the patient's own (`CallerKind::Patient`)
//! is never looked up by the shared binding, so FerroEHR looks its login up
//! as a person's ([`as_person`]): the binding stays the one source of own
//! access, and the token's reading is only checked against it.
//! No openEHR spec governs resolving a login to a party: our own design
//! (#6106, #6080, #6146).

use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::{Marker, Outcome, PartyRef};
use ferrohealth_smart::resource::Caller;

use super::super::AuthError;

/// Whether `outcome`, the binding of a caller of `kind`, binds the person
/// themself: a plain `persons` login, through a first-party client or a
/// SMART app the CDR grants `act_for_persons`, never a guest nor an RFC 8693
/// actor's, or a token read as the patient's own, whose one live `PERSON`
/// carries neither marker.
pub(in crate::extensions::access::authn) fn binds_the_person(
    kind: &CallerKind,
    outcome: &Outcome,
) -> bool {
    // NOTE: our own design (#6146), an openEHR launch reaches the CDR through a delegated
    // SMART app, and its login is the person's as a first-party login is (EHDS Art. 3).
    matches!(
        kind,
        CallerKind::Person | CallerKind::Patient | CallerKind::Delegated { actor: None }
    ) && matches!(
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

/// The caller of the login of `caller`, a token read as the patient's own,
/// as a `persons` login of the same issuer, client, scopes and claims, which
/// the shared binding looks up under the issuer's `[[auth.party]]` table.
pub(in crate::extensions::access::authn) fn as_person(caller: &Caller) -> Caller {
    let probe = Caller::new(
        caller.subject().to_owned(),
        caller.client_id().map(str::to_owned),
        &caller.scope().join(" "),
        caller.patient().map(str::to_owned),
        false,
        CallerKind::Person,
    )
    .with_person_key(
        caller.person_key().map(str::to_owned),
        caller.tenant().map(str::to_owned),
    )
    .with_claims(caller.claims().clone());
    match caller.issuer() {
        Some(issuer) => probe.with_issuer(issuer.to_owned()),
        None => probe,
    }
}

/// The caller `caller`, a token `ferrohealth-smart` reads as the patient's
/// own, carrying the binding of its login when its issuer is a `persons`
/// issuer with an `[[auth.party]]` table; `None` for a `patients` issuer's
/// token, whose login the patient-portal binding reads ([`super::super::portal`]).
///
/// The token's reading never grants: its login has to bind the person
/// themself ([`binds_the_person`]), looked up as a person's
/// ([`as_person`]). No openEHR spec governs resolving a login to a
/// party: our own design (#6080).
///
/// # Errors
/// [`AuthError::PartyRefused`] when the login binds no `PERSON`
/// (`refused-unbound`, `refused-key-claim-missing`), one with a marker
/// (`refused-wrong-marker`) or one the binding refuses for its state, and
/// [`AuthError::PartyUnavailable`] when the source could not answer.
pub(super) async fn own_patient(
    binding: &super::Binding,
    caller: &Caller,
) -> Result<Option<Caller>, AuthError> {
    let Some(issuer) = caller
        .issuer()
        .filter(|issuer| binding.identifier_issuers.contains_key(*issuer))
    else {
        return Ok(None);
    };
    let outcome = binding
        .shared
        .resolve(&as_person(caller))
        .await
        .map_err(|error| super::closed::unanswered(&error))?
        .unwrap_or(Outcome::Unbound);
    if binds_the_person(caller.kind(), &outcome) {
        return Ok(Some(Caller::clone(caller).with_party(outcome)));
    }
    // NOTE: our own design (#6080), a patient's token bound to a party with a marker binds a
    // party of another kind than the token claims, so it reads as `refused-wrong-marker`.
    let refusal = match outcome {
        Outcome::Party(_) => wrong_marker(),
        outcome => super::refusal(&outcome).unwrap_or_else(wrong_marker),
    };
    tracing::warn!(
        issuer,
        outcome = refusal.label(),
        "the login binding names no person for a patient's own token"
    );
    Err(refusal)
}

/// The refusal of a login bound to a party of another kind than its caller
/// claims.
const fn wrong_marker() -> AuthError {
    AuthError::PartyRefused {
        code: "refused-wrong-marker",
        principal: None,
        party: None,
        named: None,
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

    #[test]
    fn a_login_through_a_delegated_app_binds_the_person_and_an_actor_does_not() {
        let plain = wrong("PERSON", Markers::default(), Marker::Professional);
        assert!(binds_the_person(
            &CallerKind::Delegated { actor: None },
            &plain
        ));
        let professional = wrong(
            "PERSON",
            Markers {
                professional: true,
                representative: false,
            },
            Marker::Professional,
        );
        assert!(!binds_the_person(
            &CallerKind::Delegated { actor: None },
            &professional
        ));
    }

    #[test]
    fn a_patient_token_on_a_plain_person_binds_the_person() {
        let outcome = wrong("PERSON", Markers::default(), Marker::Professional);
        assert!(binds_the_person(&CallerKind::Patient, &outcome));
        let professional = wrong(
            "PERSON",
            Markers {
                professional: true,
                representative: false,
            },
            Marker::Professional,
        );
        assert!(!binds_the_person(&CallerKind::Patient, &professional));
    }

    #[test]
    fn the_person_probe_keeps_the_login_and_drops_the_patient_reading() {
        let caller = Caller::new(
            "eva-login".to_owned(),
            Some("app".to_owned()),
            "patient/composition-*.r",
            Some("p-1".to_owned()),
            false,
            CallerKind::Patient,
        )
        .with_issuer("https://smart.example".to_owned())
        .with_person_key(Some("eva-key".to_owned()), None);
        let probe = as_person(&caller);
        assert_eq!(probe.kind(), &CallerKind::Person);
        assert!(!probe.is_the_person());
        assert_eq!(probe.issuer(), Some("https://smart.example"));
        assert_eq!(probe.person_key(), Some("eva-key"));
        assert_eq!(probe.subject(), "eva-login");
        assert_eq!(probe.client_id(), Some("app"));
    }
}
