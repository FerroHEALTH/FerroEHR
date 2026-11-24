// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binding outcome every access record of an admitted request carries:
//! how the caller was bound and the fields that outcome needs (EHDS
//! Annex II 3.2(a) and (b), `docs/law/eu/ehds/text.html`). Which fields carry
//! the binding is our own design, as no openEHR spec governs access logging.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use ferroehr::party_binding::grants::agreement::Agreement;
use ferroehr::party_binding::grants::capabilities::Capabilities;
use ferroehr::system_log::binding::{
    AgreementRecord, BindingRecord, Identified, PartyTriple, Representation,
};
use ferroehr::versioning::committer::bound::BoundDetails;
use ferrohealth_smart::federation::{Attribute, External, Field};
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::{Outcome, PartyRef};
use ferrohealth_smart::resource::Caller;

/// Runs the request `fut`, gathering the facilities its commits name, with
/// `binding` published for the records the service layer writes itself, and
/// puts `binding`, with those facilities, on its response.
pub(super) async fn recorded<F>(binding: Option<BindingRecord>, fut: F) -> axum::response::Response
where
    F: Future<Output = axum::response::Response>,
{
    use ferroehr::system_log::access_context::{with_binding, with_facilities};
    let sink: Arc<Mutex<Vec<Identified>>> = Arc::default();
    let published = Box::pin(with_binding(binding.clone(), fut));
    let mut resp = with_facilities(Arc::clone(&sink), published).await;
    if let Some(mut binding) = binding {
        binding.facilities =
            std::mem::take(&mut *sink.lock().unwrap_or_else(PoisonError::into_inner));
        resp.extensions_mut().insert(binding);
    }
    resp
}

/// The binding record of the refusal `code`, naming `party`, the key claim
/// a token lacked when `named` is one and, for a representative, the subject `principal`
/// asserted; `None` for a code no binding value spells.
pub(super) fn refused(
    code: &str,
    party: Option<&PartyTriple>,
    named: Option<&super::party::Named>,
    principal: Option<&super::Principal>,
) -> Option<BindingRecord> {
    let code = ferroehr::system_log::binding::BindingCode::parse(code)?;
    let mut detail = BTreeMap::new();
    if let Some(claim) = named.and_then(super::party::Named::claim) {
        detail.insert("claim".to_owned(), vec![claim.to_owned()]);
    }
    let caller = principal.and_then(|principal| principal.access.caller());
    // NOTE: EHDS Art. 4(2), a refused representative's record names the patient its
    // token's launch context asserts.
    if let Some(represented) = caller.and_then(Caller::represented) {
        detail.insert("represented".to_owned(), vec![represented.to_owned()]);
    }
    // NOTE: master07 §Context Selection, a patient's token names its patient by the EHR of
    // its launch context.
    if caller.is_some_and(|caller| matches!(caller.kind(), CallerKind::Patient))
        && let Some(ehr) = caller.and_then(Caller::ehr_id)
    {
        detail.insert("represented".to_owned(), vec![ehr.to_owned()]);
    }
    let mut record = BindingRecord::refused(code, detail);
    record.party = party.cloned();
    Some(record)
}

/// The binding record of the request `principal` commits as `committer`,
/// holding `granted`, whose client `ACTOR` they name when it is a system
/// caller, under `auth`, whose login binding names the provider the
/// deployment acts for; `None` for a caller the binding names nothing about.
///
/// A system caller's record names its client `ACTOR` as the party, and the
/// identifier of the provider its acts-for relationship targets or, when it
/// names none, the deployment's provider (EHDS Annex II 3.2(a)). A
/// patient-portal representative's record names their `PERSON` as the party
/// and the subject they represent.
pub(super) fn of_request(
    principal: &super::Principal,
    committer: &ferroehr::versioning::committer::CommitterIdentity,
    granted: &Capabilities,
    auth: Option<&super::Authenticator>,
) -> Option<BindingRecord> {
    if let Some(represented) = &principal.represented
        && let Some(portal) = &represented.portal
    {
        return Some(portal_record(portal, represented.representation));
    }
    // NOTE: EHDS Annex II 3.2(a), the record's person is recorded as the subject of care,
    // naming the `PERSON` the party binding bound them to, whatever their issuer (#6080).
    if let Some(own) = &principal.own {
        return Some(BindingRecord {
            party: Some(triple(own)),
            ..BindingRecord::of(ferroehr::system_log::binding::BindingCode::SelfPatient)
        });
    }
    let client = granted.client().filter(|_| committer.system);
    let deployment = auth
        .zip(principal.access.caller())
        .and_then(|(auth, _)| auth.deployment.provider());
    let provider = client
        .and_then(|client| client.provider.as_ref())
        .and_then(|provider| provider.identifier.clone())
        .or(deployment.filter(|_| committer.system))
        .map(|(issuer, id)| Identified { issuer, id });
    let mut record = of(
        principal.access.caller()?,
        committer.bound.as_ref(),
        provider,
        super::declared::admitted(&principal.access).map(|agreement| {
            (
                agreement.as_ref(),
                super::declared::correlation_source(&principal.access),
            )
        }),
    )?;
    if let (Some(representation), Some(represented)) =
        (&mut record.representation, &principal.represented)
    {
        representation.relationship_id = Some(represented.representation.to_string());
    }
    if let Some(client) = client {
        // NOTE: BASE base_types PARTY_REF Type_validity admits "ACTOR", and a client is
        // an ACTOR of any concrete kind.
        record.party = Some(PartyTriple {
            namespace: "demographic".to_owned(),
            class: "ACTOR".to_owned(),
            id: client.party.to_string(),
        });
    }
    Some(record)
}

/// The binding record of `caller`, with `details` for a bound person,
/// `system_provider` for a system caller and `agreement`, with where its
/// issuer reads the correlation id from, for an external one; `None` for a
/// caller the binding names nothing about.
fn of(
    caller: &Caller,
    details: Option<&BoundDetails>,
    system_provider: Option<Identified>,
    agreement: Option<(&Agreement, Option<&str>)>,
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
        // NOTE: EHDS Annex II 3.2(a), the record's person bound by the party binding is
        // recorded as the subject of care, naming their `PERSON` (#6106).
        _ if let Some(party) = super::party::own::own_person(caller) => Some(BindingRecord {
            party: Some(triple(party)),
            ..BindingRecord::of(ferroehr::system_log::binding::BindingCode::SelfPatient)
        }),
        _ => {
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
                false,
            );
            if caller.actor().is_some() {
                record.actor_chain = actor_chain(caller);
            }
            Some(record)
        }
    }
}

/// The record of the patient-portal representative `portal`: bound to their
/// `PERSON`, representing the subject's `PERSON` (EHDS Annex II 3.2(a)).
fn portal_record(
    portal: &super::portal::PortalRepresentative,
    representation: uuid::Uuid,
) -> BindingRecord {
    let mut record = BindingRecord::bound(
        PartyTriple {
            namespace: portal.namespace.clone(),
            class: "PERSON".to_owned(),
            id: portal.person.to_string(),
        },
        false,
        Vec::new(),
        None,
        false,
    );
    record.representation = Some(Representation {
        represented: portal.subject.to_string(),
        ground: None,
        relationship: portal.relationship.clone(),
        relationship_id: Some(representation.to_string()),
    });
    record
}

/// The `PARTY_REF` triple of `party`.
pub(super) fn triple(party: &PartyRef) -> PartyTriple {
    PartyTriple {
        namespace: party.namespace().to_owned(),
        class: party.type_name().to_owned(),
        id: party.id().to_owned(),
    }
}

/// The record of an external accessor admitted under `agreement`, whose
/// issuer reads the correlation id from `correlation_source`.
fn external_record(
    organisation: bool,
    external: &External,
    (agreement, correlation_source): (&Agreement, Option<&str>),
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
        accessor: agreement.accessor.to_string(),
        access_class: agreement
            .access_class
            .map(|class| class.as_str().to_owned()),
        correlation_id: external.correlation_id().map(|id| id.as_str().to_owned()),
        correlation_source: external
            .correlation_id()
            .and(correlation_source.map(str::to_owned)),
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
        Caller::new("sub-1".to_owned(), Some("app".to_owned()), "", None, kind)
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
        assert!(of(&caller(CallerKind::User, &json!({})), None, None, None).is_none());
    }

    #[test]
    fn the_actor_chain_is_read_outermost_first() {
        let delegated = caller(
            CallerKind::User,
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
