// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FHIR R4 (4.0.1) `Consent` read into and written from
//! [`super::model::Document`] (`docs/specs/fhir-consent/R4/`).
//!
//! R4 alone has `scope`, `policyRule`, `policy`, one root `provision` and the
//! provision `type` and `class`; the actor's `role` and `reference` are both
//! required (`consent-definitions.html`, `Consent.provision.actor`).

use fhir_types::codec::{Json as _, Object, Path};
use fhir_types::r4::consent::{
    Consent, ConsentPolicy, ConsentProvisionActor, ConsentSource, ConsentVerification,
};

use crate::access_exchange::Format;
use crate::access_exchange::fhir::model::{Actor, Document, line};

super::codec::release_codec!(r4, Format::FhirR4);

/// The format refusals name.
const FORMAT: Format = Format::FhirR4;

/// Reads `object` as an R4 `Consent`.
///
/// # Errors
/// [`Refused`] for a body that is no R4 `Consent`, an element with no field in
/// [`Document`], or one a reader must not ignore.
pub(super) fn read(object: &Object) -> Result<Document, Refused> {
    let consent = Consent::from_json(object, &mut Path::root("Consent"))
        .map_err(|error| Refused::new(FORMAT, "Consent", Loss::Malformed(error.to_string())))?;
    unmodified(&consent.modifier_extension, "Consent")?;
    if consent.implicit_rules.is_some() {
        return Err(Refused::new(FORMAT, "Consent.implicitRules", Loss::NoDirectiveForm));
    }
    if !consent.contained.is_empty() {
        return Err(Refused::new(FORMAT, "Consent.contained", Loss::NoDirectiveForm));
    }
    let mut kept = Vec::new();
    let mut evidence = Vec::new();
    match &consent.source {
        Some(ConsentSource::Reference(reference)) => evidence.push(ref_in(reference)),
        Some(ConsentSource::Attachment(attachment)) => {
            kept.push(attachment_kept(attachment, "Consent.sourceAttachment")?);
        }
        None => {}
    }
    for policy in &consent.policy {
        kept.push(policy_kept(policy)?);
    }
    for verification in &consent.verification {
        kept.push(verification_kept(verification)?);
    }
    let rules = consent
        .provision
        .as_ref()
        .map(|provision| rule_in(provision, "Consent.provision"))
        .transpose()?
        .into_iter()
        .collect();
    Ok(Document {
        status: text!(Some(&consent.status)).unwrap_or_default(),
        scope: Some(concept_in(&consent.scope)),
        categories: consent.category.iter().map(concept_in).collect(),
        subject: consent.patient.as_ref().map(ref_in),
        stated: text!(consent.date_time),
        instructed_by: consent.performer.iter().map(ref_in).collect(),
        controllers: consent.organization.iter().map(ref_in).collect(),
        evidence,
        identifiers: consent.identifier.iter().map(ident_in).collect(),
        policy_rule: consent.policy_rule.as_ref().map(concept_in),
        rules,
        extensions: consent.extension.iter().map(ext_in).collect(),
        kept,
        ..Document::default()
    })
}

/// Writes `document` as an R4 `Consent`.
///
/// # Errors
/// [`Refused`] when `document` holds more than the one root provision R4
/// has, or the resource does not encode.
pub(super) fn write(document: &Document) -> Result<Object, Refused> {
    if document.rules.len() > 1 {
        return Err(Refused::new(
            Format::Directive,
            "exceptions",
            Loss::NoFormatForm(FORMAT),
        ));
    }
    let consent = Consent {
        status: primitives::Code {
            value: Some(document.status.clone()),
            ..Default::default()
        },
        scope: document.scope.as_ref().map(concept_out).unwrap_or_default(),
        category: document.categories.iter().map(concept_out).collect(),
        patient: document.subject.as_ref().map(ref_out),
        date_time: prim!(DateTime, document.stated.clone()),
        performer: document.instructed_by.iter().map(ref_out).collect(),
        organization: document.controllers.iter().map(ref_out).collect(),
        source: document
            .evidence
            .first()
            .map(|evidence| ConsentSource::Reference(Box::new(ref_out(evidence)))),
        identifier: document.identifiers.iter().map(ident_out).collect(),
        policy_rule: document.policy_rule.as_ref().map(concept_out),
        provision: document.rules.first().map(rule_out),
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
        role: Some(concept_in(&actor.role)),
        reference: Some(ref_in(&actor.reference)),
        extensions: actor.extension.iter().map(ext_in).collect(),
    })
}

fn actor_out(actor: &Actor) -> ConsentProvisionActor {
    ConsentProvisionActor {
        role: actor.role.as_ref().map(concept_out).unwrap_or_default(),
        reference: actor.reference.as_ref().map(ref_out).unwrap_or_default(),
        extension: actor.extensions.iter().map(ext_out).collect(),
        ..Default::default()
    }
}

fn rule_extra_in(provision: &ConsentProvision, rule: &mut Rule) {
    rule.effect = text!(provision.r#type);
    rule.templates = provision.class.iter().map(code_in).collect();
}

fn rule_extra_out(rule: &Rule, provision: &mut ConsentProvision) {
    provision.r#type = prim!(Code, rule.effect.clone());
    provision.class = rule.templates.iter().map(code_out).collect();
}

fn policy_kept(policy: &ConsentPolicy) -> Result<(String, String), Refused> {
    unmodified(&policy.modifier_extension, "Consent.policy")?;
    let rendering = line([text!(policy.authority), text!(policy.uri)]);
    Ok(("Consent.policy".to_owned(), rendering))
}

fn verification_kept(verification: &ConsentVerification) -> Result<(String, String), Refused> {
    unmodified(&verification.modifier_extension, "Consent.verification")?;
    let rendering = line([
        verification.verified.value.map(|v| format!("verified {v}")),
        verification.verified_with.as_ref().map(|r| ref_in(r).render()),
        text!(verification.verification_date),
    ]);
    Ok(("Consent.verification".to_owned(), rendering))
}
