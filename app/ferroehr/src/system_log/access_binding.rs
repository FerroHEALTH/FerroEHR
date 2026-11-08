// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binding fields of one access record (Regulation (EU) 2025/327 Annex
//! II 3.2(a), (b)) as `ehds-logging` holds them, which the BALP `AuditEvent`
//! and the DICOM audit message both render.
//!
//! Each recorded field goes to the `ehds-logging` member of the same meaning,
//! and both renderings write the members through the crate, so the two never
//! disagree. No openEHR specification governs access logging: our own design.

use std::collections::BTreeMap;

use ehds_logging::detail::{self, Entry};
use ehds_logging::record::{
    AccessClass, Accessor, Actor, Agreement, AuthenticationMethod, Binding, BoundParty, Coded,
    Correlation, CorrelationSource, Gateways, IssuedId, PartyRef, Professional, Relayed,
    RelayedError, RelayedParts, RelayedProfessional, RelayedProvider, Representation, Via,
};

use crate::system_log::binding::{
    AgreementRecord, BindingCode, BindingRecord, Identified, PartyTriple,
};

/// The binding members of one record's accessor, and the correlation id of
/// its request.
#[derive(Default)]
pub(crate) struct Members {
    /// The binding outcome.
    pub(crate) binding: Option<Binding>,
    /// The RFC 8693 actor chain, outermost first.
    pub(crate) actors: Vec<Actor>,
    /// Whom a representative acts for.
    pub(crate) represents: Option<Representation>,
    /// The healthcare provider the accessor acts for.
    pub(crate) acts_for: Option<IssuedId>,
    /// The facilities a system caller's commit names.
    pub(crate) facilities: Vec<IssuedId>,
    /// What a federation relayed of an external accessor.
    pub(crate) relayed: Option<Relayed>,
    /// The correlation id of a `chained` request.
    pub(crate) correlation: Option<Correlation>,
}

/// Why a binding record has no form in the access record.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BindingError {
    /// An outcome that names its party carries none.
    #[error("the binding outcome {0} names no party")]
    PartyMissing(&'static str),
    /// A refusal lacks the detail its code names.
    #[error("the binding outcome {code} names no {name}")]
    DetailMissing {
        /// The binding code.
        code: &'static str,
        /// The detail, such as `claim`.
        name: &'static str,
    },
    /// An external accessor's token names no issuer.
    #[error("the external accessor's token names no issuer")]
    IssuerMissing,
    /// The agreement names an access class the record has no form for.
    #[error("the agreement names an access class the access record has no form for")]
    AccessClass,
    /// The correlation id was read from a source the record cannot name.
    #[error("the correlation id source is none the access record names")]
    CorrelationSource,
    /// The external accessor breaks a rule of the relayed record.
    #[error("the external accessor is not one the access record holds")]
    Relayed(#[from] RelayedError),
}

/// The members `record` gives an accessor whose token `issuer` issued.
///
/// # Errors
/// A [`BindingError`] when the record lacks a field its code needs, or its
/// external accessor breaks a rule of the relayed record.
pub(crate) fn members(
    record: &BindingRecord,
    issuer: Option<&str>,
) -> Result<Members, BindingError> {
    let detail = |name: &str| record.detail.get(name).and_then(|values| values.first());
    let represents = record
        .representation
        .as_ref()
        .map(|representation| Representation {
            represented: representation.represented.clone(),
            ground: representation.ground.clone().map(uncoded),
            relationship: representation.relationship.clone().map(uncoded),
        })
        .or_else(|| {
            detail("represented").map(|represented| Representation {
                represented: represented.clone(),
                ground: None,
                relationship: None,
            })
        });
    let external = matches!(
        record.code,
        BindingCode::External | BindingCode::Organisation
    );
    let relayed = match (&record.agreement, external) {
        (Some(agreement), true) => Some(relayed(record, agreement, issuer)?),
        _ => None,
    };
    Ok(Members {
        binding: Some(binding(record)?),
        actors: record
            .actor_chain
            .iter()
            .map(|actor| actor_of(actor))
            .collect(),
        represents,
        acts_for: record.provider.as_ref().map(issued),
        facilities: record.facilities.iter().map(issued).collect(),
        relayed,
        correlation: record
            .agreement
            .as_ref()
            .map(correlation)
            .transpose()?
            .flatten(),
    })
}

/// The entries of the accessor and the relayed entity of `members`, by
/// entity name, as `ehds-logging` writes them for an accessor named
/// `subject`; an entity with no entry is left out. The correlation id of a
/// `chained` request rides in the relayed entity, beside the agreement.
pub(crate) fn entities(members: Members, subject: String) -> Vec<(&'static str, Vec<Entry>)> {
    let accessor = Accessor {
        issuer: None,
        subject,
        client_id: None,
        audience: None,
        provider: None,
        acting: None,
        roles: Vec::new(),
        token_id: None,
        assurance: None,
        professional: Professional::default(),
        alt_id: None,
        purposes: Vec::new(),
        policies: Vec::new(),
        relayed: None,
        binding: members.binding,
        actors: members.actors,
        represents: members.represents,
        acts_for: members.acts_for,
        facilities: members.facilities,
    };
    let mut entities = Vec::new();
    let entries = detail::accessor(&accessor);
    if !entries.is_empty() {
        entities.push((detail::ACCESSOR_ENTITY, entries));
    }
    if let Some(relayed) = &members.relayed {
        let mut entries = detail::relayed(relayed);
        entries.extend(members.correlation.into_iter().flat_map(|correlation| {
            [
                Entry::new(detail::CORRELATION, correlation.value),
                Entry::new(detail::CORRELATION_SOURCE, correlation.source.code()),
            ]
        }));
        entities.push((detail::RELAYED_ENTITY, entries));
    }
    entities
}

/// The binding outcome of `record`.
fn binding(record: &BindingRecord) -> Result<Binding, BindingError> {
    let code = record.code;
    let party = || {
        record
            .party
            .as_ref()
            .map(party_ref)
            .ok_or(BindingError::PartyMissing(code.as_str()))
    };
    let bound = || -> Result<BoundParty, BindingError> {
        let party = party()?;
        let groups = record
            .groups
            .iter()
            .map(|id| PartyRef {
                namespace: party.namespace.clone(),
                kind: "GROUP".to_owned(),
                id: id.clone(),
            })
            .collect();
        Ok(BoundParty { party, groups })
    };
    Ok(match code {
        BindingCode::Bound => Binding::Bound {
            party: bound()?,
            guest: record.guest,
        },
        BindingCode::Provisional => Binding::Provisional { party: bound()? },
        BindingCode::External => Binding::External,
        BindingCode::Organisation => Binding::Organisation,
        BindingCode::System => Binding::System,
        BindingCode::SelfPatient => Binding::PartySelf,
        BindingCode::RefusedUnbound => Binding::RefusedUnbound,
        BindingCode::RefusedAmbiguous => Binding::RefusedAmbiguous,
        BindingCode::RefusedInactive => Binding::RefusedInactive { party: party()? },
        BindingCode::RefusedAbandoned => Binding::RefusedAbandoned { party: party()? },
        BindingCode::RefusedDeprovisioned => Binding::RefusedDeprovisioned,
        BindingCode::RefusedWrongMarker => Binding::RefusedWrongMarker,
        BindingCode::RefusedRepresentationUnrecorded => Binding::RefusedRepresentationUnrecorded,
        BindingCode::RefusedKeyClaimMissing => Binding::RefusedKeyClaimMissing {
            claim: record
                .detail
                .get("claim")
                .and_then(|values| values.first())
                .cloned()
                .ok_or(BindingError::DetailMissing {
                    code: code.as_str(),
                    name: "claim",
                })?,
        },
        BindingCode::RefusedGroupsUnavailable => Binding::RefusedGroupsUnavailable,
        BindingCode::RefusedGuest => Binding::RefusedGuest,
        // NOTE: the authentication layer names no field beside this refusal, so its
        // record carries none (no specification governs the outcomes: our own design).
        BindingCode::RefusedExternalIncomplete => Binding::RefusedExternalIncomplete {
            fields: record.detail.get("fields").cloned().unwrap_or_default(),
        },
        BindingCode::RefusedTrcPatientMismatch => Binding::RefusedTrcPatientMismatch,
        BindingCode::RefusedTrcMissing => Binding::RefusedTrcMissing,
        BindingCode::RefusedSystemPersonalScope => Binding::RefusedSystemPersonalScope,
        BindingCode::RefusedNoAgreement => Binding::RefusedNoAgreement,
        BindingCode::RefusedScopeBeyondAgreement => Binding::RefusedScopeBeyondAgreement,
        BindingCode::RefusedCorrelationMissing => Binding::RefusedCorrelationMissing,
        BindingCode::RefusedWabvpzRole => Binding::RefusedWabvpzRole,
        BindingCode::RefusedWabvpzOrganisationType => Binding::RefusedWabvpzOrganisationType,
    })
}

/// A recorded code as a coded value no system names.
fn uncoded(code: String) -> Coded {
    Coded { system: None, code }
}

/// A value written `system|code`, or a bare code.
fn coded(value: &str) -> Coded {
    match value.split_once('|') {
        Some((system, code)) => Coded {
            system: Some(system.to_owned()),
            code: code.to_owned(),
        },
        None => uncoded(value.to_owned()),
    }
}

/// An actor recorded as `iss sub`.
fn actor_of(actor: &str) -> Actor {
    match actor.split_once(' ') {
        Some((issuer, subject)) => Actor {
            issuer: Some(issuer.to_owned()),
            subject: subject.to_owned(),
        },
        None => Actor {
            issuer: None,
            subject: actor.to_owned(),
        },
    }
}

fn party_ref(party: &PartyTriple) -> PartyRef {
    PartyRef {
        namespace: party.namespace.clone(),
        kind: party.class.clone(),
        id: party.id.clone(),
    }
}

fn issued(id: &Identified) -> IssuedId {
    IssuedId {
        issuer: id.issuer.clone(),
        id: id.id.clone(),
    }
}

/// The correlation id `agreement` carries, read from the claim or the
/// `authorization_details` member its issuer maps.
fn correlation(agreement: &AgreementRecord) -> Result<Option<Correlation>, BindingError> {
    let Some(value) = agreement.correlation_id.clone() else {
        return Ok(None);
    };
    // NOTE: FerroEHR reads a chained correlation id only from what the token attests,
    // never a header (config-auth `access_class`), and records the source as `<kind>:<name>`.
    let source = match agreement
        .correlation_source
        .as_deref()
        .and_then(|s| s.split_once(':'))
    {
        Some(("claim", _)) => CorrelationSource::TokenClaim,
        Some(("authorization_details", _)) => CorrelationSource::AuthorizationDetails,
        _ => return Err(BindingError::CorrelationSource),
    };
    Ok(Some(Correlation { value, source }))
}

/// What a federation `issuer` relayed of the external accessor of `record`,
/// admitted under `agreement`: each Implementing Regulation (EU) 2026/2099
/// Annex field and eHDSI SAML Profile 10.0.0 §2.3 attribute at its member, and
/// every other asserted field by its name.
fn relayed(
    record: &BindingRecord,
    agreement: &AgreementRecord,
    issuer: Option<&str>,
) -> Result<Relayed, BindingError> {
    let mut fields: BTreeMap<String, Vec<String>> = record.external.clone();
    let mut take = |name: &str| fields.remove(name).unwrap_or_default();
    let one = |values: Vec<String>| values.into_iter().next();
    let professional = RelayedProfessional {
        name: one(take("professional.name")),
        family_name: one(take("professional.family_name")),
        given_name: one(take("professional.given_name")),
        identifier: one(take("professional.hp_identifier")),
        issuing_authority: one(take("professional.issuing_authority_name")),
        roles: take("professional.hp_professional_role")
            .iter()
            .map(|role| coded(role))
            .collect(),
        healthcare_provider_identifier: one(take("professional.healthcare_provider_identifier")),
        name_id: None,
        authentication_method: one(take("professional.authentication_method"))
            .as_deref()
            .and_then(AuthenticationMethod::from_uri),
        on_behalf_of: one(take("professional.on_behalf_of")).as_deref().map(coded),
        permissions: take("professional.permissions"),
    };
    let provider_identifier = one(take("provider.healthcare_provider_identifier"));
    let issuing_authority = one(take("provider.issuing_authority_name"));
    let provider_name = one(take("provider.healthcare_provider_name"));
    let address = one(take("provider.healthcare_provider_address"));
    let facility_type = one(take("provider.facility_type")).as_deref().map(coded);
    let home_community_id = one(take("provider.home_community_id"));
    let provider = provider_identifier.map(|identifier| RelayedProvider {
        identifier,
        issuing_authority,
        name: provider_name,
        address,
        facility_type,
        home_community_id,
        organisation_type: None,
    });
    let country_code = one(take("professional.country_code"));
    let point_of_care = one(take("point_of_care"));
    let identity_assertion_id = one(take("assertion.id"));
    let trc_assertion_id = one(take("trc.id"));
    let consent = one(take("consent.reference"));
    let organisation = record.code == BindingCode::Organisation;
    let access_class = match agreement.access_class.as_deref() {
        None => None,
        Some("automated") => Some(AccessClass::Automated),
        Some("chained") => Some(AccessClass::Chained),
        Some(_) => return Err(BindingError::AccessClass),
    };
    let parts = RelayedParts {
        via: Via::Federation {
            issuer: issuer.ok_or(BindingError::IssuerMissing)?.to_owned(),
        },
        country_code,
        professional: (!organisation).then_some(professional),
        provider,
        agreement: Some(Agreement {
            name: agreement.name.clone(),
            version: agreement.version.clone(),
            legal_ground: agreement
                .legal_ground
                .iter()
                .cloned()
                .map(uncoded)
                .collect(),
            consent,
            access_class,
        }),
        point_of_care,
        identity_assertion_id,
        trc_assertion_id,
        message_id: None,
        gateways: Gateways::default(),
        asserted: fields,
    };
    Ok(Relayed::try_from(parts)?)
}

#[cfg(test)]
pub(crate) mod tests {
    use base64::Engine as _;
    use secrecy::ExposeSecret as _;

    use super::*;
    use crate::system_log::AuditError;
    use crate::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
    use crate::system_log::message::{AuditContext, to_xml};

    /// A hostile value: a NUL, a terminal escape, a bidi override and 10 000
    /// more characters.
    pub(crate) fn hostile() -> String {
        format!("Qz7\u{0}\u{1b}[2J\u{202e}{}", "x".repeat(10_000))
    }

    /// A system client acting for a provider, whose commit names a ward.
    pub(crate) fn system() -> BindingRecord {
        let mut record = BindingRecord::system(Some(Identified {
            issuer: "urn:oid:2.999.7".to_owned(),
            id: "provider-1".to_owned(),
        }));
        record.facilities.push(Identified {
            issuer: "urn:oid:2.999.8".to_owned(),
            id: "ward-7".to_owned(),
        });
        record
    }

    /// An organisation a federation admits under a `chained` agreement.
    pub(crate) fn organisation() -> BindingRecord {
        let fields = BTreeMap::from([
            (
                "provider.healthcare_provider_identifier".to_owned(),
                vec!["urn:oid:2.999.56".to_owned()],
            ),
            (
                "provider.healthcare_provider_name".to_owned(),
                vec!["Example Clinic".to_owned()],
            ),
            ("nl.ura".to_owned(), vec!["12345678".to_owned()]),
            ("consent.reference".to_owned(), vec!["consent-1".to_owned()]),
        ]);
        let agreement = AgreementRecord {
            name: "Regional exchange".to_owned(),
            version: "1".to_owned(),
            legal_ground: Some("GDPR Art. 9(2)(h)".to_owned()),
            accessor: "organisation".to_owned(),
            access_class: Some("chained".to_owned()),
            correlation_id: Some("corr-1".to_owned()),
            correlation_source: Some("claim:correlation_id".to_owned()),
        };
        BindingRecord::external(true, fields, agreement, None)
    }

    /// A bound representative with two groups, reached through an actor.
    pub(crate) fn representative() -> BindingRecord {
        let mut record = BindingRecord::bound(
            PartyTriple {
                namespace: "demographic".to_owned(),
                class: "PERSON".to_owned(),
                id: "party-1".to_owned(),
            },
            false,
            vec!["group-1".to_owned()],
            None,
            false,
        );
        record.representation = Some(crate::system_log::binding::Representation {
            represented: "subject-9".to_owned(),
            ground: Some("guardian".to_owned()),
            relationship: Some("10".to_owned()),
        });
        record.actor_chain = vec!["https://idp.example.org app-1".to_owned()];
        record
    }

    /// The `ParticipantObjectDetail` pairs of the DICOM object named `name`
    /// in `xml`, each value decoded from base64.
    pub(crate) fn dicom_details(xml: &str, name: &str) -> Vec<(String, String)> {
        let marker = format!("<ParticipantObjectName>{name}</ParticipantObjectName>");
        let Some((_, after)) = xml.split_once(&marker) else {
            return Vec::new();
        };
        let object = after
            .split("</ParticipantObjectIdentification>")
            .next()
            .unwrap_or(after);
        object
            .split("<ParticipantObjectDetail type=\"")
            .skip(1)
            .map(|detail| {
                let (kind, rest) = detail.split_once("\" value=\"").expect("a value");
                let value = rest.split('"').next().expect("a quoted value");
                let decoded = base64::engine::general_purpose::STANDARD
                    .decode(value)
                    .expect("base64");
                (kind.to_owned(), String::from_utf8(decoded).expect("UTF-8"))
            })
            .collect()
    }

    fn pairs(entities: &[(&'static str, Vec<Entry>)], name: &str) -> Vec<(String, String)> {
        entities
            .iter()
            .filter(|(entity, _)| *entity == name)
            .flat_map(|(_, entries)| entries.iter())
            .map(|entry| (entry.name.to_owned(), entry.value.clone()))
            .collect()
    }

    fn pair(name: &str, value: &str) -> (String, String) {
        (name.to_owned(), value.to_owned())
    }

    #[test]
    fn a_system_client_names_its_provider_and_facilities() {
        let members = members(&system(), Some("https://idp.example.org")).expect("members");
        let entities = entities(members, "client-1".to_owned());
        assert_eq!(
            pairs(&entities, "ehds-accessor"),
            [
                pair("binding", "system"),
                pair("acts-for", "urn:oid:2.999.7|provider-1"),
                pair("facility", "urn:oid:2.999.8|ward-7"),
            ]
        );
        assert_eq!(
            pairs(&entities, "ehds-relayed"),
            Vec::<(String, String)>::new()
        );
    }

    #[test]
    fn a_representative_names_the_party_groups_actor_and_representation() {
        let members = members(&representative(), Some("https://idp.example.org")).expect("members");
        assert_eq!(
            pairs(&entities(members, "user-1".to_owned()), "ehds-accessor"),
            [
                pair("binding", "bound"),
                pair("party", "demographic|PERSON|party-1"),
                pair("group", "demographic|GROUP|group-1"),
                pair("actor", "https://idp.example.org|app-1"),
                pair("represents", "subject-9"),
                pair("represents-ground", "guardian"),
                pair("represents-relationship", "10"),
            ]
        );
    }

    #[test]
    fn a_federation_organisation_carries_its_agreement_fields_and_correlation() {
        let members =
            members(&organisation(), Some("https://federation.example.org")).expect("members");
        let entities = entities(members, "org-1".to_owned());
        assert_eq!(
            pairs(&entities, "ehds-accessor"),
            [pair("binding", "organisation")]
        );
        let relayed = pairs(&entities, "ehds-relayed");
        for expected in [
            pair("relayed-via", "federation"),
            pair("relayed-issuer", "https://federation.example.org"),
            pair("provider-identifier", "urn:oid:2.999.56"),
            pair("provider-name", "Example Clinic"),
            pair("agreement-name", "Regional exchange"),
            pair("agreement-legal-ground", "GDPR Art. 9(2)(h)"),
            pair("agreement-consent", "consent-1"),
            pair("agreement-access-class", "chained"),
            pair("asserted-field", "nl.ura=12345678"),
            pair("correlation-id", "corr-1"),
            pair("correlation-source", "token-claim"),
        ] {
            assert!(relayed.contains(&expected), "{expected:?} in {relayed:?}");
        }
    }

    #[test]
    fn every_refusal_maps_to_its_code_and_a_missing_party_or_claim_is_an_error() {
        for code in BindingCode::ALL
            .into_iter()
            .filter(|code| code.as_str().starts_with("refused-"))
        {
            let mut record = BindingRecord::refused(
                code,
                BTreeMap::from([("claim".to_owned(), vec!["oid".to_owned()])]),
            );
            record.party = representative().party;
            let binding = binding(&record).expect("a binding");
            assert_eq!(binding.code(), code.as_str());
        }
        let unnamed = BindingRecord::refused(BindingCode::RefusedInactive, BTreeMap::new());
        assert!(matches!(
            binding(&unnamed),
            Err(BindingError::PartyMissing(_))
        ));
        let no_claim = BindingRecord::refused(BindingCode::RefusedKeyClaimMissing, BTreeMap::new());
        assert!(matches!(
            binding(&no_claim),
            Err(BindingError::DetailMissing { name: "claim", .. })
        ));
    }

    #[test]
    fn an_external_accessor_without_an_issuer_is_an_error() {
        assert!(matches!(
            members(&organisation(), None),
            Err(BindingError::IssuerMissing)
        ));
    }

    fn ctx() -> AuditContext {
        AuditContext {
            source_id: "ferroehr".to_owned(),
            value_if_missing: "UNKNOWN".to_owned(),
            ..AuditContext::default()
        }
    }

    fn event() -> AuditEvent {
        let mut event = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        event.user_id = "user-1".to_owned();
        event
    }

    /// The DICOM message of `event`, as UTF-8.
    fn xml(event: &AuditEvent) -> String {
        let bytes = to_xml(event, &ctx(), Some("patient-42")).expect("xml");
        String::from_utf8(bytes.expose_secret().to_vec()).expect("UTF-8")
    }

    /// EHDS Annex II 3.2(a), (b): the binding fields ride as details of the
    /// `ehds-accessor` and `ehds-relayed` objects, as `ehds-logging` writes
    /// them, with the object type Other (`4`).
    #[test]
    fn the_binding_fields_ride_as_details_of_their_objects() {
        let mut e = event();
        e.binding = Some(system());
        let text = xml(&e);
        assert!(text.contains(r#"ParticipantObjectTypeCode="4""#), "{text}");
        assert_eq!(
            dicom_details(&text, "ehds-accessor"),
            [
                ("binding".to_owned(), "system".to_owned()),
                (
                    "acts-for".to_owned(),
                    "urn:oid:2.999.7|provider-1".to_owned()
                ),
                ("facility".to_owned(), "urn:oid:2.999.8|ward-7".to_owned()),
            ]
        );
        e.binding = Some(organisation());
        e.issuer = Some("https://federation.example.org".to_owned());
        let relayed = dicom_details(&xml(&e), "ehds-relayed");
        for (kind, value) in [
            ("relayed-issuer", "https://federation.example.org"),
            ("agreement-name", "Regional exchange"),
            ("agreement-access-class", "chained"),
            ("asserted-field", "nl.ura=12345678"),
            ("correlation-id", "corr-1"),
        ] {
            assert!(
                relayed.contains(&(kind.to_owned(), value.to_owned())),
                "{kind} in {relayed:?}"
            );
        }
    }

    /// A hostile binding value leaves no raw control or bidi character in the
    /// message, and is base64-encoded as given in its detail (DICOM PS3.15
    /// A.5.1.1 `ValuePair`).
    #[test]
    fn a_hostile_binding_value_is_written_escaped() {
        let mut e = event();
        let mut binding = system();
        binding.facilities[0].id = hostile();
        e.binding = Some(binding);
        let text = xml(&e);
        assert!(
            text.chars()
                .all(|c| ihe_atna::escape::untrusted(&c.to_string()) == c.to_string()),
            "a raw control character in the message"
        );
        assert!(dicom_details(&text, "ehds-accessor").contains(&(
            "facility".to_owned(),
            format!("urn:oid:2.999.8|{}", hostile())
        )));
    }

    /// A binding record that names no party where its code needs one fails
    /// the message, never a message without the binding.
    #[test]
    fn a_binding_without_its_party_fails_the_message() {
        let mut e = event();
        e.binding = Some(BindingRecord::of(BindingCode::Bound));
        assert!(matches!(
            to_xml(&e, &ctx(), Some("patient-42")),
            Err(AuditError::Render(_))
        ));
    }
}
