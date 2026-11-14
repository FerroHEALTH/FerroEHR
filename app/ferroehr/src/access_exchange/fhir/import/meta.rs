// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What every directive of one `Consent` shares: its kind, who instructed it
//! and when, its identifiers, evidence and legal grounds, and the
//! elements `details` keeps; and the parties its references name.

use openehr_base::v1_3::base_types::identification::generic_id::GenericId;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{
    DirectiveKind, TERMINOLOGY_ID_ACCESS_CONTROL, Term,
};
use openehr_rm::v1_2::common::generic::party_identified::{PartyIdentified, PartyIdentifiedData};
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_rm::v1_2::data_types::uri::dv_ehr_uri::DvEhrUri;

use super::Reader;
use crate::access_exchange::fhir::model::{Code, Ref, Rule};
use crate::access_exchange::fhir::{
    CONSENT_SCOPE, EXT_ARCHETYPE, EXT_BASE_UNDECIDED, EXT_CALLER_KIND, EXT_DIRECTIVE,
    EXT_EMERGENCY_OVERRIDE, EXT_OBJECT_REF, EXT_ORIGIN, EXT_RELATIONSHIP, EXT_ROLE,
    EXT_TIME_VALIDITY, LOINC, Release, URI_SYSTEM,
};
use crate::access_exchange::rm::{details, list, members};
use crate::access_exchange::{Loss, Refused, catalogue_entry};
use crate::versioning::audit::dv_date_time_text;

/// Every FerroEHR extension the mapping reads.
pub(super) const KNOWN: [&str; 10] = [
    EXT_DIRECTIVE,
    EXT_EMERGENCY_OVERRIDE,
    EXT_BASE_UNDECIDED,
    EXT_TIME_VALIDITY,
    EXT_ROLE,
    EXT_RELATIONSHIP,
    EXT_CALLER_KIND,
    EXT_ARCHETYPE,
    EXT_ORIGIN,
    EXT_OBJECT_REF,
];

/// LOINC 59284-0 Patient Consent and 57016-8 Privacy policy acknowledgement
/// Document, the categories R4 `valueset-consent-category.html` holds for a
/// privacy consent.
const PRIVACY_CATEGORIES: [&str; 2] = ["59284-0", "57016-8"];

/// What the directives of one `Consent` share.
#[derive(Debug)]
pub(super) struct Shared {
    /// The kind every directive takes, from `category`.
    pub(super) kind: DirectiveKind,
    identifiers: Vec<DvIdentifier>,
    instructed_by: Option<PartyProxy>,
    instructed: Option<DvDateTime>,
    legal_grounds: Vec<DvCodedText>,
    evidence: Vec<DvEhrUri>,
    kept: Vec<(String, String)>,
}

impl Shared {
    /// Sets on `target` what the `Consent` states of it: from `own`, the
    /// FerroEHR extension the directive was written with, when it carries
    /// one, else from the `Consent`'s core elements.
    pub(super) fn stamp(&self, target: &mut AccessDirective, own: Option<AccessDirective>) {
        if own.is_some() {
            plumb(target, own);
            return;
        }
        target.identifiers = list(self.identifiers.clone());
        target.instructed_by.clone_from(&self.instructed_by);
        target.instructed.clone_from(&self.instructed);
        target.legal_grounds = list(self.legal_grounds.clone());
        target.evidence = list(self.evidence.clone());
        target.details = details(&self.kept);
    }

    /// Adds to `copy`, a catalogue entry copied into the EHR, what the
    /// `Consent` states of the acknowledgement.
    pub(super) fn stamp_copy(&self, copy: &mut AccessDirective) {
        let mut identifiers = members(copy.identifiers.as_ref()).to_vec();
        identifiers.extend(self.identifiers.iter().cloned());
        copy.identifiers = list(identifiers);
        let mut evidence = members(copy.evidence.as_ref()).to_vec();
        evidence.extend(self.evidence.iter().cloned());
        copy.evidence = list(evidence);
        copy.instructed_by.clone_from(&self.instructed_by);
        copy.instructed.clone_from(&self.instructed);
        if !self.kept.is_empty() {
            copy.details = details(&self.kept);
        }
    }
}

/// Copies onto `target` the attributes `own`, the FerroEHR extension a
/// directive was written with, carries: every attribute no evaluator reads.
pub(super) fn plumb(target: &mut AccessDirective, own: Option<AccessDirective>) {
    let Some(own) = own else {
        return;
    };
    target.name = own.name;
    target.archetype_node_id = own.archetype_node_id;
    target.uid = own.uid;
    target.links = own.links;
    target.archetype_details = own.archetype_details;
    target.feeder_audit = own.feeder_audit;
    target.instructed_by = own.instructed_by;
    target.instructed = own.instructed;
    target.legal_grounds = own.legal_grounds;
    target.evidence = own.evidence;
    target.identifiers = own.identifiers;
    target.details = own.details;
}

/// The RM type of the party a FHIR resource type names: our own design over
/// the FHIR resources that stand for a person, an organisation, a group, a
/// device and a role.
fn rm_type(fhir_type: Option<&str>) -> &'static str {
    match fhir_type {
        Some("Patient" | "Practitioner" | "RelatedPerson" | "Person") => "PERSON",
        Some("Organization") => "ORGANISATION",
        Some("Group" | "CareTeam") => "GROUP",
        Some("Device") => "AGENT",
        Some("PractitionerRole") => "ROLE",
        _ => "PARTY",
    }
}

/// The party `reference` names: by an identifier, a `HIER_OBJECT_ID` when its
/// system is the URI system (`urn:uuid:`, `urn:oid:`) or absent, else a
/// `GENERIC_ID` of that system; by a literal reference, a `GENERIC_ID` of
/// scheme `fhir`. `None` when it names nothing, such as a display alone.
pub(super) fn party_of(reference: &Ref) -> Option<PartyRef> {
    let literal_type = reference
        .reference
        .as_deref()
        .and_then(|literal| literal.rsplit('/').nth(1));
    let rm = rm_type(reference.kind.as_deref().or(literal_type));
    let id = if let Some(identifier) = &reference.identifier {
        let value = identifier.value.clone()?;
        match identifier.system.as_deref() {
            Some(URI_SYSTEM) => {
                let bare = value
                    .strip_prefix("urn:uuid:")
                    .or_else(|| value.strip_prefix("urn:oid:"))?;
                ObjectId::HierObjectId(HierObjectId::new(bare).ok()?)
            }
            None => ObjectId::HierObjectId(HierObjectId::new(value).ok()?),
            Some(scheme) => ObjectId::GenericId(GenericId {
                value,
                scheme: scheme.to_owned(),
            }),
        }
    } else {
        ObjectId::GenericId(GenericId {
            value: reference.reference.clone()?,
            scheme: "fhir".to_owned(),
        })
    };
    Some(PartyRef {
        namespace: "demographic".to_owned(),
        r#type: rm.to_owned(),
        id,
    })
}

impl Reader<'_> {
    /// What the directives of the `Consent` share, with the checks that hold
    /// for the whole `Consent`.
    pub(super) fn shared(&self) -> Result<Shared, Refused> {
        let document = self.document;
        if document.status != "active" {
            return Err(self.refused("Consent.status", Loss::Code(document.status.clone())));
        }
        if self.release == Release::R4
            && !document
                .scope
                .as_ref()
                .is_some_and(|s| s.codings.iter().any(|c| c.is(CONSENT_SCOPE, "patient-privacy")))
        {
            let named = document
                .scope
                .iter()
                .flat_map(|s| s.codings.iter().map(Code::token))
                .collect::<Vec<_>>();
            return Err(self.refused("Consent.scope", Loss::Code(named.join(", "))));
        }
        let mut shared = Shared {
            kind: DirectiveKind::Consent,
            identifiers: Vec::new(),
            instructed_by: None,
            instructed: None,
            legal_grounds: Vec::new(),
            evidence: Vec::new(),
            kept: document.kept.clone(),
        };
        self.categories(&mut shared)?;
        let controller = match self.release {
            Release::R4 => "Consent.organization",
            Release::R5 => "Consent.controller",
        };
        for reference in &document.controllers {
            shared.kept.push((controller.to_owned(), reference.render()));
        }
        for extension in &document.extensions {
            if !KNOWN.contains(&extension.url.as_str()) {
                shared
                    .kept
                    .push(("Consent.extension".to_owned(), extension.url.clone()));
            }
        }
        self.grantees()?;
        if let Some(url) = &document.policy_basis {
            if catalogue_entry(self.context.catalogue, None, url).is_none() {
                return Err(self.refused(
                    "Consent.policyBasis.url",
                    Loss::NotInCatalogue(url.clone()),
                ));
            }
            shared
                .kept
                .push(("Consent.policyBasis".to_owned(), url.clone()));
        }
        shared.identifiers = document
            .identifiers
            .iter()
            .map(|identifier| {
                Ok(DvIdentifier {
                    issuer: identifier.system.clone(),
                    assigner: identifier.assigner.clone(),
                    id: identifier.value.clone().ok_or_else(|| {
                        self.refused(
                            "Consent.identifier.value",
                            Loss::Malformed("an identifier needs a value".to_owned()),
                        )
                    })?,
                    r#type: identifier.kind.clone(),
                })
            })
            .collect::<Result<_, Refused>>()?;
        shared.instructed_by = self.instructed_by()?;
        shared.instructed = document.stated.clone().map(dv_date_time_text);
        for evidence in &document.evidence {
            match evidence.reference.as_deref() {
                Some(uri) if uri.starts_with("ehr:") => shared.evidence.push(DvEhrUri {
                    value: uri.to_owned(),
                }),
                _ => shared
                    .kept
                    .push(("Consent.sourceReference".to_owned(), evidence.render())),
            }
        }
        shared.legal_grounds = document
            .legal_grounds
            .iter()
            .map(|concept| match concept.codings.as_slice() {
                [one] => self.verbatim(one, "Consent.regulatoryBasis"),
                _ => Err(self.refused("Consent.regulatoryBasis", Loss::Several)),
            })
            .collect::<Result<_, _>>()?;
        Ok(shared)
    }

    /// The kind `category` states, into `shared`; a category of no kind and
    /// no privacy consent is kept in `details`.
    fn categories(&self, shared: &mut Shared) -> Result<(), Refused> {
        for category in &self.document.categories {
            for code in &category.codings {
                if code.system.as_deref() == Some(TERMINOLOGY_ID_ACCESS_CONTROL) {
                    shared.kind = DirectiveKind::read(
                        TERMINOLOGY_ID_ACCESS_CONTROL,
                        code.code.as_deref().unwrap_or_default(),
                    )
                    .map_err(|error| {
                        self.refused("Consent.category", Loss::Code(error.to_string()))
                    })?;
                } else if !PRIVACY_CATEGORIES.iter().any(|c| code.is(LOINC, c)) {
                    shared.kept.push(("Consent.category".to_owned(), code.token()));
                }
            }
        }
        Ok(())
    }

    /// `instructed_by`, from R4 `performer` or R5 `grantor`.
    fn instructed_by(&self) -> Result<Option<PartyProxy>, Refused> {
        let element = match self.release {
            Release::R4 => "Consent.performer",
            Release::R5 => "Consent.grantor",
        };
        match self.document.instructed_by.as_slice() {
            [] => Ok(None),
            [one] => Ok(Some(PartyProxy::PartyIdentified(
                PartyIdentified::PartyIdentified(PartyIdentifiedData {
                    external_ref: party_of(one),
                    name: one.display.clone(),
                    identifiers: None,
                }),
            ))),
            _ => Err(self.refused(element, Loss::Several)),
        }
    }

    /// Refuses an R5 `grantee` that is no actor of a provision: `grantee`
    /// repeats the actors "for ease of search only" (R5
    /// `Consent.grantee`), so one that is not among them states what no
    /// directive holds.
    fn grantees(&self) -> Result<(), Refused> {
        fn actors<'r>(rules: &'r [Rule], out: &mut Vec<&'r Ref>) {
            for rule in rules {
                out.extend(rule.actors.iter().filter_map(|a| a.reference.as_ref()));
                actors(&rule.rules, out);
            }
        }
        let mut named = Vec::new();
        actors(&self.document.rules, &mut named);
        match self
            .document
            .grantees
            .iter()
            .find(|grantee| !named.contains(grantee))
        {
            Some(_) => Err(self.refused("Consent.grantee", Loss::NoDirectiveForm)),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uuid_identifier_is_a_hier_object_id_and_a_literal_a_generic_id() {
        let uuid = Ref {
            kind: Some("Organization".to_owned()),
            identifier: Some(crate::access_exchange::fhir::model::Ident {
                system: Some(URI_SYSTEM.to_owned()),
                value: Some("urn:uuid:0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f80".to_owned()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let party = party_of(&uuid).expect("a party");
        assert_eq!(party.r#type, "ORGANISATION");
        assert!(matches!(party.id, ObjectId::HierObjectId(_)));
        let literal = Ref {
            reference: Some("Practitioner/f204".to_owned()),
            ..Default::default()
        };
        let party = party_of(&literal).expect("a party");
        assert_eq!(party.r#type, "PERSON");
        assert_eq!(
            party.id,
            ObjectId::GenericId(GenericId {
                value: "Practitioner/f204".to_owned(),
                scheme: "fhir".to_owned()
            })
        );
        let display = Ref {
            display: Some("someone".to_owned()),
            ..Default::default()
        };
        assert_eq!(party_of(&display), None);
    }
}
