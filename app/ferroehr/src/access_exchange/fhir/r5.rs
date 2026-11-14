// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FHIR R5 (5.0.0) `Consent` read into and written from
//! [`super::model::Document`] (`docs/specs/fhir-consent/R5/`).
//!
//! R5 alone has `decision`, `period`, `grantee`, `manager`, `regulatoryBasis`,
//! `policyBasis`, `policyText`, a list of root provisions, and the provision
//! `documentType`, `resourceType` and `expression`; the actor's `role` and
//! `reference` are optional (`consent-definitions.html`).

use fhir_types::codec::{Json as _, Object, Path};
use fhir_types::r5::consent::{Consent, ConsentProvisionActor, ConsentVerification};

use crate::access_exchange::Format;
use crate::access_exchange::fhir::model::{Actor, Document, line};

super::codec::release_codec!(r5, Format::FhirR5);

/// The format refusals name.
const FORMAT: Format = Format::FhirR5;

/// Reads `object` as an R5 `Consent`.
///
/// # Errors
/// [`Refused`] for a body that is no R5 `Consent`, an element with no field in
/// [`Document`], or one a reader must not ignore.
pub(super) fn read(object: &Object) -> Result<Document, Refused> {
    let consent = Consent::from_json(object, &mut Path::root("Consent"))
        .map_err(|error| Refused::new(FORMAT, "Consent", Loss::Malformed(error.to_string())))?;
    unmodified(&consent.modifier_extension, "Consent")?;
    if consent.implicit_rules.is_some() {
        return Err(Refused::new(
            FORMAT,
            "Consent.implicitRules",
            Loss::NoDirectiveForm,
        ));
    }
    if !consent.contained.is_empty() {
        return Err(Refused::new(
            FORMAT,
            "Consent.contained",
            Loss::NoDirectiveForm,
        ));
    }
    let mut kept = Vec::new();
    for attachment in &consent.source_attachment {
        kept.push(attachment_kept(attachment, "Consent.sourceAttachment")?);
    }
    for manager in &consent.manager {
        kept.push(("Consent.manager".to_owned(), ref_in(manager).render()));
    }
    for text in &consent.policy_text {
        kept.push(("Consent.policyText".to_owned(), ref_in(text).render()));
    }
    for verification in &consent.verification {
        kept.push(verification_kept(verification)?);
    }
    let policy_basis = match &consent.policy_basis {
        Some(basis) => {
            unmodified(&basis.modifier_extension, "Consent.policyBasis")?;
            if basis.reference.is_some() {
                return Err(Refused::new(
                    FORMAT,
                    "Consent.policyBasis.reference",
                    Loss::NoDirectiveForm,
                ));
            }
            text!(basis.url)
        }
        None => None,
    };
    Ok(Document {
        status: text!(Some(&consent.status)).unwrap_or_default(),
        categories: consent.category.iter().map(concept_in).collect(),
        subject: consent.subject.as_ref().map(ref_in),
        stated: text!(consent.date),
        period: consent.period.as_ref().map(period_in),
        instructed_by: consent.grantor.iter().map(ref_in).collect(),
        grantees: consent.grantee.iter().map(ref_in).collect(),
        controllers: consent.controller.iter().map(ref_in).collect(),
        evidence: consent.source_reference.iter().map(ref_in).collect(),
        identifiers: consent.identifier.iter().map(ident_in).collect(),
        legal_grounds: consent.regulatory_basis.iter().map(concept_in).collect(),
        decision: text!(consent.decision),
        policy_basis,
        rules: consent
            .provision
            .iter()
            .map(|provision| rule_in(provision, "Consent.provision"))
            .collect::<Result<_, _>>()?,
        extensions: consent.extension.iter().map(ext_in).collect(),
        kept,
        ..Document::default()
    })
}

/// Writes `document` as an R5 `Consent`.
///
/// # Errors
/// [`Refused`] when the resource does not encode.
pub(super) fn write(document: &Document) -> Result<Object, Refused> {
    let consent = Consent {
        status: primitives::Code {
            value: Some(document.status.clone()),
            ..Default::default()
        },
        category: document.categories.iter().map(concept_out).collect(),
        subject: document.subject.as_ref().map(ref_out),
        date: prim!(Date, document.stated.clone()),
        period: document.period.as_ref().map(period_out),
        grantor: document.instructed_by.iter().map(ref_out).collect(),
        grantee: document.grantees.iter().map(ref_out).collect(),
        controller: document.controllers.iter().map(ref_out).collect(),
        source_reference: document.evidence.iter().map(ref_out).collect(),
        identifier: document.identifiers.iter().map(ident_out).collect(),
        regulatory_basis: document.legal_grounds.iter().map(concept_out).collect(),
        decision: prim!(Code, document.decision.clone()),
        provision: document.rules.iter().map(rule_out).collect(),
        extension: document.extensions.iter().map(ext_out).collect(),
        ..Default::default()
    };
    consent
        .to_json()
        .map_err(|error| Refused::new(FORMAT, "Consent", Loss::Malformed(error.to_string())))
}

fn actor_in(actor: &ConsentProvisionActor, element: &str) -> Result<Actor, Refused> {
    unmodified(&actor.modifier_extension, element)?;
    Ok(Actor {
        role: actor.role.as_ref().map(concept_in),
        reference: actor.reference.as_ref().map(ref_in),
        extensions: actor.extension.iter().map(ext_in).collect(),
    })
}

fn actor_out(actor: &Actor) -> ConsentProvisionActor {
    ConsentProvisionActor {
        role: actor.role.as_ref().map(concept_out),
        reference: actor.reference.as_ref().map(ref_out),
        extension: actor.extensions.iter().map(ext_out).collect(),
        ..Default::default()
    }
}

fn rule_extra_in(provision: &ConsentProvision, rule: &mut Rule) {
    rule.templates = provision.document_type.iter().map(code_in).collect();
    rule.resource_types = provision.resource_type.iter().map(code_in).collect();
    rule.expression = provision.expression.is_some();
}

fn rule_extra_out(rule: &Rule, provision: &mut ConsentProvision) {
    provision.document_type = rule.templates.iter().map(code_out).collect();
}

fn verification_kept(verification: &ConsentVerification) -> Result<(String, String), Refused> {
    unmodified(&verification.modifier_extension, "Consent.verification")?;
    let rendering = line([
        verification.verified.value.map(|v| format!("verified {v}")),
        verification
            .verification_type
            .as_ref()
            .and_then(|t| t.coding.first().map(|c| code_in(c).token())),
        verification
            .verified_by
            .as_ref()
            .map(|r| ref_in(r).render()),
        verification
            .verified_with
            .as_ref()
            .map(|r| ref_in(r).render()),
        Some(
            verification
                .verification_date
                .iter()
                .filter_map(|date| date.value.clone())
                .collect::<Vec<_>>()
                .join(" "),
        )
        .filter(|dates| !dates.is_empty()),
    ]);
    Ok(("Consent.verification".to_owned(), rendering))
}
