// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The login of a record's person: a user whose one `PERSON` carries no
//! professional and no representative marker; and the login of a
//! representative: a `RelatedPerson` bound to a representative's `PERSON`, or
//! a user whose one `PERSON` carries the representative marker alone.
//!
//! The shared binding answers a user's login on such a `PERSON`
//! `wrong_marker`, since a user other than a `Patient` or a `RelatedPerson`
//! needs a professional party there. An identity bound to a patient's
//! `PERSON` is that patient (EHDS Art. 3), so FerroEHR admits it as the
//! person themself: it holds no role, and the access evaluator confines it
//! to the EHRs whose `EHR_STATUS.subject` names that `PERSON`. The shared
//! binding never looks a `Patient` up, so FerroEHR looks its login up as a
//! user's ([`as_person`]): the binding stays the one source of own access,
//! and the token's reading is only checked against it. A representative is
//! admitted for a subject its stored representation names, and for its own
//! record as the person themself ([`binds_a_representative`]). A token that
//! names an RFC 8693 §4.1 actor is someone acting for the user, never the
//! person themself. No openEHR spec governs resolving a login to a party:
//! our own design.

use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::{Marker, Outcome, PartyRef};
use ferrohealth_smart::resource::Caller;

use super::super::AuthError;

/// Whether a caller of `kind` is a user the shared binding looks up as a
/// professional: a `Practitioner`, a `PractitionerRole`, a `Person`, or a user
/// of no stated type.
pub(crate) const fn professional_login(kind: &CallerKind) -> bool {
    matches!(
        kind,
        CallerKind::Practitioner
            | CallerKind::PractitionerRole
            | CallerKind::Person
            | CallerKind::User
    )
}

/// Whether `outcome`, the binding of `caller`, binds the person themself: a
/// `Person` or untyped user, or the patient's own token
/// ([`Caller::is_the_person`]), naming no actor, whose one live `PERSON`
/// carries neither marker.
pub(in crate::extensions::access::authn) fn binds_the_person(
    caller: &Caller,
    outcome: &Outcome,
) -> bool {
    let person = match caller.kind() {
        CallerKind::Patient => caller.is_the_person(),
        CallerKind::Person | CallerKind::User => caller.actor().is_none(),
        _ => false,
    };
    person
        && matches!(
            outcome,
            Outcome::WrongMarker {
                party,
                markers,
                needed: Marker::Professional,
            } if party.type_name() == "PERSON" && !markers.professional && !markers.representative
        )
}

/// Whether `outcome`, the binding of `caller`, binds a representative
/// FerroEHR admits from its own facts: a `RelatedPerson` bound to a party, or
/// a `Person` or untyped user naming no actor whose one live `PERSON` carries
/// the representative marker alone.
///
/// The request is admitted only for a subject a live stored representation
/// names this `PERSON` for, or as the person themself when the subject is
/// this `PERSON` ([`super::super::portal`]). No openEHR spec governs
/// resolving a login to a party: our own design.
pub(in crate::extensions::access::authn) fn binds_a_representative(
    caller: &Caller,
    outcome: &Outcome,
) -> bool {
    match caller.kind() {
        CallerKind::RelatedPerson => {
            matches!(outcome, Outcome::Party(bound) if bound.party.type_name() == "PERSON")
        }
        CallerKind::Person | CallerKind::User if caller.actor().is_none() => matches!(
            outcome,
            Outcome::WrongMarker {
                party,
                markers,
                needed: Marker::Professional,
            } if party.type_name() == "PERSON" && !markers.professional && markers.representative
        ),
        _ => false,
    }
}

/// The `PERSON` the login binding bound `caller` to as a representative
/// ([`binds_a_representative`]); `None` for any other caller.
pub(crate) fn representative_person(caller: &Caller) -> Option<&PartyRef> {
    let outcome = caller.party()?;
    if binds_a_representative(caller, outcome) {
        outcome.party()
    } else {
        None
    }
}

/// The `PERSON` the login binding bound `caller` to as the person themself
/// ([`binds_the_person`]); `None` for any other caller.
pub(crate) fn own_person(caller: &Caller) -> Option<&PartyRef> {
    let outcome = caller.party()?;
    if binds_the_person(caller, outcome) {
        outcome.party()
    } else {
        None
    }
}

/// The caller of the login of `caller`, a token read as the patient's own,
/// as an untyped user of the same issuer, client, scopes and claims, which
/// the shared binding looks up under the issuer's `[auth.issuer.<name>.party]` table.
pub(in crate::extensions::access::authn) fn as_person(caller: &Caller) -> Caller {
    let probe = Caller::new(
        caller.subject().to_owned(),
        caller.client_id().map(str::to_owned),
        &caller.scope().join(" "),
        caller.patient().map(str::to_owned),
        CallerKind::User,
    )
    .with_claims(caller.claims().clone());
    match caller.issuer() {
        Some(issuer) => probe.with_issuer(issuer.to_owned()),
        None => probe,
    }
}

/// The caller `caller`, a token `ferrohealth-smart` reads as a patient's,
/// carrying the binding of its login when its issuer has an
/// `[auth.issuer.<name>.party]` table; `None` for an issuer with none.
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
    if binds_the_person(caller, &outcome) {
        return Ok(Some(Caller::clone(caller).with_party(outcome)));
    }
    // NOTE: our own design (#6080), a patient's token bound to a party with a marker, or one
    // that is not the patient's own, binds a party of another kind than it claims.
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
/// who reads their record for who they are, and for a representative the
/// binding bound, who reads it for whom they represent; neither holds a
/// professional right (EHDS Art. 3 and Art. 4(2)).
pub(in crate::extensions::access::authn) fn roles(
    caller: &Caller,
    honoured: impl FnOnce() -> Vec<String>,
) -> Vec<String> {
    if own_person(caller).is_some() || representative_person(caller).is_some() {
        Vec::new()
    } else {
        honoured()
    }
}

#[cfg(test)]
mod tests {
    use ferrohealth_smart::kind::Actor;
    use ferrohealth_smart::party::Markers;

    use super::*;

    fn wrong(class: &str, markers: Markers, needed: Marker) -> Outcome {
        Outcome::WrongMarker {
            party: PartyRef::new("ferroehr", class, "p-1"),
            markers,
            needed,
        }
    }

    fn caller(kind: CallerKind) -> Caller {
        Caller::new("login".to_owned(), Some("app".to_owned()), "", None, kind)
    }

    fn acted(kind: CallerKind) -> Caller {
        caller(kind).with_actor(Actor {
            issuer: "https://login.example".to_owned(),
            sub: "agent".to_owned(),
        })
    }

    const REP: Markers = Markers::REPRESENTATIVE;

    #[test]
    fn a_user_login_on_a_plain_person_binds_the_person() {
        let outcome = wrong("PERSON", Markers::default(), Marker::Professional);
        assert!(binds_the_person(&caller(CallerKind::User), &outcome));
        assert!(binds_the_person(&caller(CallerKind::Person), &outcome));
        assert!(!binds_the_person(
            &caller(CallerKind::Practitioner),
            &outcome
        ));
    }

    #[test]
    fn a_representative_row_an_actor_or_another_class_binds_no_person() {
        let user = caller(CallerKind::User);
        assert!(!binds_the_person(
            &user,
            &wrong("PERSON", REP, Marker::Professional)
        ));
        let plain = wrong("PERSON", Markers::default(), Marker::Professional);
        assert!(!binds_the_person(&acted(CallerKind::User), &plain));
        assert!(!binds_the_person(
            &user,
            &wrong("AGENT", Markers::default(), Marker::Professional)
        ));
        assert!(!binds_the_person(
            &user,
            &wrong("PERSON", Markers::default(), Marker::Representative)
        ));
        assert!(!binds_the_person(
            &user,
            &wrong("PERSON", Markers::PROFESSIONAL, Marker::Professional)
        ));
    }

    #[test]
    fn a_user_login_on_a_representative_person_binds_a_representative() {
        let outcome = wrong("PERSON", REP, Marker::Professional);
        assert!(binds_a_representative(&caller(CallerKind::User), &outcome));
        assert!(binds_a_representative(
            &caller(CallerKind::Person),
            &outcome
        ));
        assert!(!binds_a_representative(&acted(CallerKind::User), &outcome));
        assert!(!binds_a_representative(
            &caller(CallerKind::Patient),
            &outcome
        ));
        assert!(!binds_a_representative(
            &caller(CallerKind::User),
            &wrong("AGENT", REP, Marker::Professional)
        ));
        assert!(!binds_a_representative(
            &caller(CallerKind::User),
            &wrong("PERSON", REP, Marker::Representative)
        ));
        for markers in [Markers::default(), Markers::BOTH] {
            assert!(
                !binds_a_representative(
                    &caller(CallerKind::User),
                    &wrong("PERSON", markers, Marker::Professional)
                ),
                "{markers:?}"
            );
        }
    }

    #[test]
    fn a_related_person_the_binding_refuses_binds_neither() {
        let related = caller(CallerKind::RelatedPerson);
        let needs_rep = wrong("PERSON", Markers::default(), Marker::Representative);
        assert!(!binds_a_representative(&related, &needs_rep));
        assert!(!binds_the_person(&related, &needs_rep));
    }

    #[test]
    fn the_patients_own_token_on_a_plain_person_binds_the_person() {
        let own = Caller::new(
            "eva".to_owned(),
            Some("app".to_owned()),
            "patient/composition-*.r",
            Some("p-1".to_owned()),
            CallerKind::Patient,
        )
        .with_claims(
            [("fhirUser".to_owned(), serde_json::json!("Patient/p-1"))]
                .into_iter()
                .collect(),
        );
        let outcome = wrong("PERSON", Markers::default(), Marker::Professional);
        assert!(binds_the_person(&own, &outcome));
        assert!(!binds_the_person(
            &own,
            &wrong("PERSON", Markers::PROFESSIONAL, Marker::Professional)
        ));
        assert!(
            !binds_the_person(&caller(CallerKind::Patient), &outcome),
            "a patient token that is not the patient's own"
        );
    }

    #[test]
    fn the_person_probe_keeps_the_login_and_drops_the_patient_reading() {
        let caller = Caller::new(
            "eva-login".to_owned(),
            Some("app".to_owned()),
            "patient/composition-*.r",
            Some("p-1".to_owned()),
            CallerKind::Patient,
        )
        .with_issuer("https://smart.example".to_owned());
        let probe = as_person(&caller);
        assert_eq!(probe.kind(), &CallerKind::User);
        assert!(!probe.is_the_person());
        assert_eq!(probe.issuer(), Some("https://smart.example"));
        assert_eq!(probe.subject(), "eva-login");
        assert_eq!(probe.client_id(), Some("app"));
    }
}
