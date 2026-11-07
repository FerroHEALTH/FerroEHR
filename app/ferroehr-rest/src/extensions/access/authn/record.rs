// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binding outcome every access record of an admitted request carries:
//! how the caller was bound and the fields that outcome needs (EHDS
//! Annex II 3.2(a) and (b), `docs/law/eu/ehds/text.html`). Which fields carry
//! the binding is our own design, as no openEHR spec governs access logging.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use ferroehr::config::agreement::Agreement;
use ferroehr::system_log::binding::{
    AgreementRecord, BindingRecord, Identified, PartyTriple, Representation,
};
use ferroehr::versioning::committer::bound::BoundDetails;
use ferrohealth_smart::federation::{Attribute, External, Field};
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::{Outcome, PartyRef};
use ferrohealth_smart::resource::Caller;

/// The facilities the compositions of one request named, which a system
/// caller's record carries; filled by the commit path, read once the
/// response is built.
#[derive(Debug, Clone, Default)]
pub(crate) struct Facilities(Arc<Mutex<Vec<Identified>>>);

impl Facilities {
    /// The facilities gathered so far.
    pub(crate) fn taken(&self) -> Vec<Identified> {
        std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The collector the commit path pushes into.
    pub(crate) fn sink(&self) -> Arc<Mutex<Vec<Identified>>> {
        Arc::clone(&self.0)
    }
}

/// The binding record of the request `principal` commits as `committer`;
/// `None` for a caller the binding names nothing about.
pub(super) fn of_request(
    principal: &super::Principal,
    committer: &ferroehr::versioning::committer::CommitterIdentity,
) -> Option<BindingRecord> {
    let provider = committer
        .system_client
        .as_ref()
        .and_then(|client| client.provider.as_ref())
        .map(|provider| Identified {
            issuer: provider.issuer.clone(),
            id: provider.id.clone(),
        });
    of(
        principal.access.caller()?,
        committer.bound.as_ref(),
        provider,
        super::declared::admitted(&principal.access).map(Arc::as_ref),
    )
}

/// The binding record of `caller`, with `details` for a bound person,
/// `system_provider` for a system caller and `agreement` for an external
/// one; `None` for a caller the binding names nothing about.
fn of(
    caller: &Caller,
    details: Option<&BoundDetails>,
    system_provider: Option<Identified>,
    agreement: Option<&Agreement>,
) -> Option<BindingRecord> {
    match caller.kind() {
        CallerKind::Patient => Some(BindingRecord::of(
            ferroehr::system_log::binding::BindingCode::SelfPatient,
        )),
        CallerKind::System => Some(BindingRecord::system(system_provider)),
        CallerKind::ExternalPerson(external) => Some(external_record(false, external, agreement?)),
        CallerKind::ExternalOrganisation(external) => {
            Some(external_record(true, external, agreement?))
        }
        kind => {
            let bound = caller.party().and_then(Outcome::bound)?;
            let mut record = BindingRecord::bound(
                triple(&bound.party),
                bound.provisional,
                bound
                    .groups
                    .iter()
                    .map(|group| group.id().to_owned())
                    .collect(),
                details
                    .and_then(|details| details.provider.clone())
                    .map(|(issuer, id)| Identified { issuer, id }),
                bound.guest,
            );
            let inner = match kind {
                CallerKind::Guest(inner) => inner.as_ref(),
                other => other,
            };
            match inner {
                CallerKind::Representative {
                    represented,
                    ground,
                } => {
                    record.representation = Some(Representation {
                        represented: represented.clone(),
                        ground: ground.clone(),
                        relationship: bound
                            .represents
                            .iter()
                            .find_map(ferrohealth_smart::party::Represented::relationship)
                            .map(str::to_owned),
                    });
                }
                CallerKind::Delegated { .. } => record.actor_chain = actor_chain(caller),
                _ => {}
            }
            Some(record)
        }
    }
}

/// The `PARTY_REF` triple of `party`.
fn triple(party: &PartyRef) -> PartyTriple {
    PartyTriple {
        namespace: party.namespace().to_owned(),
        class: party.type_name().to_owned(),
        id: party.id().to_owned(),
    }
}

/// The record of an external accessor admitted under `agreement`.
fn external_record(
    organisation: bool,
    external: &External,
    agreement: &Agreement,
) -> BindingRecord {
    let fields: BTreeMap<String, Vec<String>> = external
        .fields()
        .iter()
        .map(|(field, value)| (field.as_str().to_owned(), strings(value)))
        .collect();
    let text = |field: Field| {
        external
            .field(field)
            .and_then(Attribute::text)
            .map(str::to_owned)
    };
    let provider = text(Field::ProviderHealthcareProviderIdentifier).map(|id| Identified {
        issuer: text(Field::ProviderIssuingAuthorityName).unwrap_or_default(),
        id,
    });
    let record = AgreementRecord {
        name: agreement.name.clone(),
        version: agreement.version.clone(),
        legal_ground: agreement.legal_ground.as_ref().map(ToString::to_string),
        accessor: agreement
            .accessor
            .map_or_else(String::new, |accessor| accessor.to_string()),
        access_class: agreement.access_class.map(|class| {
            match class {
                ferroehr::config::agreement::AccessClass::Automated => "automated",
                ferroehr::config::agreement::AccessClass::Chained => "chained",
            }
            .to_owned()
        }),
    };
    BindingRecord::external(organisation, fields, record, provider)
}

/// Every string an asserted value carries: its text, or each code as
/// `system|code`.
fn strings(value: &Attribute) -> Vec<String> {
    match value {
        Attribute::Text(text) => vec![text.clone()],
        Attribute::Codes(codes) => codes
            .iter()
            .map(|code| match &code.system {
                Some(system) => format!("{system}|{}", code.code),
                None => code.code.clone(),
            })
            .collect(),
    }
}

/// The RFC 8693 §4.1 actor chain of `caller`'s token, outermost first, each
/// actor as `iss sub` (the token's issuer when `act` names none).
fn actor_chain(caller: &Caller) -> Vec<String> {
    let issuer = caller.issuer().unwrap_or_default().to_owned();
    let mut chain = Vec::new();
    let mut current = caller.claim("act");
    while let Some(act) = current.and_then(serde_json::Value::as_object) {
        let Some(sub) = act.get("sub").and_then(serde_json::Value::as_str) else {
            break;
        };
        let iss = act
            .get("iss")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(&issuer);
        chain.push(format!("{iss} {sub}"));
        current = act.get("act");
    }
    chain
}

#[cfg(test)]
mod tests {
    use ferroehr::system_log::binding::BindingCode;
    use serde_json::json;

    use super::*;

    fn caller(kind: CallerKind, claims: &serde_json::Value) -> Caller {
        Caller::new(
            "sub-1".to_owned(),
            Some("app".to_owned()),
            "",
            None,
            false,
            kind,
        )
        .with_issuer("https://login.example".to_owned())
        .with_claims(claims.as_object().cloned().unwrap_or_default())
    }

    #[test]
    fn a_patient_is_self_and_a_system_caller_carries_its_provider() {
        let patient =
            of(&caller(CallerKind::Patient, &json!({})), None, None, None).expect("a record");
        assert_eq!(patient.code, BindingCode::SelfPatient);
        let provider = Identified {
            issuer: "urn:oid:2.999.7.9".to_owned(),
            id: "dev".to_owned(),
        };
        let system = of(
            &caller(CallerKind::System, &json!({})),
            None,
            Some(provider.clone()),
            None,
        )
        .expect("a record");
        assert_eq!(system.code, BindingCode::System);
        assert_eq!(system.provider, Some(provider));
    }

    #[test]
    fn an_unbound_person_carries_no_binding() {
        assert!(of(&caller(CallerKind::Person, &json!({})), None, None, None).is_none());
    }

    #[test]
    fn the_actor_chain_is_read_outermost_first() {
        let delegated = caller(
            CallerKind::Delegated { actor: None },
            &json!({ "act": { "sub": "gateway", "act": { "iss": "https://idp.example",
                                                           "sub": "portal" } } }),
        );
        assert_eq!(
            actor_chain(&delegated),
            vec![
                "https://login.example gateway".to_owned(),
                "https://idp.example portal".to_owned()
            ]
        );
    }
}
