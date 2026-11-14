// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Access directives as HL7 FHIR `Consent`, R4 (4.0.1) and R5 (5.0.0), both
//! ways (`docs/specs/fhir-consent/`).
//!
//! Both releases state a `Consent` as "a base policy ... followed by a
//! listing of exceptions to that policy" (R4 and R5 `consent.html` §6.2.3):
//! R4 writes the base as `policyRule` and the exceptions as the root
//! `provision` and its nested ones; R5 writes it as `decision` and a list of
//! provisions. One directive is one `Consent`: an unscoped directive is the
//! base and its exceptions the provisions; a scoped one is written as one
//! provision under the opposite base, its exceptions nested in it
//! ([`export`]). What has no core element goes in a FerroEHR extension
//! ([`EXTENSION_BASE`]). How a base maps onto `default_access`, and how a
//! `Consent` with three levels splits into directives, is FerroEHR's own
//! design ([`import`]), since FHIR leaves enforcement out of scope
//! (§6.2.3: "The specification of these details is not in scope for the
//! Consent resource").

mod codec;
pub mod export;
pub mod import;
pub mod model;
mod r4;
mod r5;

use fhir_types::codec::{Object, Value};

use crate::access_exchange::fhir::model::Document;
use crate::access_exchange::{Format, Loss, Refused};

/// Where the FerroEHR extensions are defined.
///
/// Our own design, since no FHIR element carries these directive
/// attributes.
pub const EXTENSION_BASE: &str = "https://ferroehr.eu/fhir/StructureDefinition/";

/// The FerroEHR extension `name`, under [`EXTENSION_BASE`].
macro_rules! extension {
    ($name:literal) => {
        concat!("https://ferroehr.eu/fhir/StructureDefinition/", $name)
    };
}

/// The directive's attributes no evaluator reads, as canonical JSON.
///
/// They are its name, uid, archetype details, `details`, `instructed`,
/// `instructed_by`, `legal_grounds`, `evidence` and `identifiers`, carried as
/// an `ACCESS_DIRECTIVE` holding only those.
pub const EXT_DIRECTIVE: &str = extension!("access-directive");
/// `ACCESS_DIRECTIVE.emergency_override`, `valueBoolean`.
pub const EXT_EMERGENCY_OVERRIDE: &str = extension!("emergency-override");
/// On a `Consent` written from a scoped directive: its base is no directive,
/// FerroEHR decides outside the scope by `default_access` and the other
/// directives, `valueBoolean`.
pub const EXT_BASE_UNDECIDED: &str = extension!("base-undecided");
/// `ACCESS_DIRECTIVE.time_validity` of an unscoped directive in R4, which
/// has no `Consent.period`, `valuePeriod`.
pub const EXT_TIME_VALIDITY: &str = extension!("time-validity");
/// `ACCESS_ACTOR_SCOPE.role`, `valueCoding`.
pub const EXT_ROLE: &str = extension!("role");
/// `ACCESS_ACTOR_SCOPE.relationship`, `valueCoding`.
pub const EXT_RELATIONSHIP: &str = extension!("relationship");
/// `ACCESS_ACTOR_SCOPE.caller_kind`, `valueCoding`.
pub const EXT_CALLER_KIND: &str = extension!("caller-kind");
/// One of `ACCESS_DATA_SCOPE.archetypes`, `valueString`.
pub const EXT_ARCHETYPE: &str = extension!("archetype");
/// One of `ACCESS_DATA_SCOPE.origins`, `valueString`.
pub const EXT_ORIGIN: &str = extension!("origin");
/// One of `ACCESS_DATA_SCOPE.objects`, the canonical JSON of the
/// `OBJECT_REF`, on its `provision.data`, `valueString`.
pub const EXT_OBJECT_REF: &str = extension!("object-ref");

/// The code system of an openEHR template id in R4 `provision.class` and R5
/// `provision.documentType`: our own design.
pub const TEMPLATE_SYSTEM: &str = "https://ferroehr.eu/fhir/CodeSystem/template-id";

/// HL7 `consentscope` (R4 `Consent.scope`, `valueset-consent-scope.html`).
pub const CONSENT_SCOPE: &str = "http://terminology.hl7.org/CodeSystem/consentscope";
/// HL7 v3 `ActCode`, which holds `OPTIN` and `OPTOUT` (R4
/// `valueset-consent-policy.html`).
pub const ACT_CODE: &str = "http://terminology.hl7.org/CodeSystem/v3-ActCode";
/// HL7 v3 `ParticipationType`, which holds the actor role `PRCP` the R4
/// examples use.
pub const PARTICIPATION_TYPE: &str = "http://terminology.hl7.org/CodeSystem/v3-ParticipationType";
/// HL7 v3 `Confidentiality`, whose levels are ordered (R4 and R5
/// `Consent.provision.securityLabel`).
pub const CONFIDENTIALITY: &str = "http://terminology.hl7.org/CodeSystem/v3-Confidentiality";
/// LOINC (R4 `valueset-consent-category.html`).
pub const LOINC: &str = "http://loinc.org";
/// The identifier system of a URI (FHIR R4 `identifier-registry.html`).
pub const URI_SYSTEM: &str = "urn:ietf:rfc:3986";

/// The FHIR release a `Consent` is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// FHIR R4, 4.0.1.
    R4,
    /// FHIR R5, 5.0.0.
    R5,
}

impl Release {
    /// The format refusals name.
    #[must_use]
    pub const fn format(self) -> Format {
        match self {
            Self::R4 => Format::FhirR4,
            Self::R5 => Format::FhirR5,
        }
    }
}

/// Reads `body` as a JSON `Consent` of `release`.
///
/// # Errors
/// [`Refused`] for a body that is no JSON object or no `Consent` of the
/// release, an element the mapping has no field for, or a modifier
/// extension.
pub fn read(release: Release, body: &[u8]) -> Result<Document, Refused> {
    let object: Object = serde_json::from_slice(body).map_err(|error| {
        Refused::new(
            release.format(),
            "Consent",
            Loss::Malformed(error.to_string()),
        )
    })?;
    match release {
        Release::R4 => r4::read(&object),
        Release::R5 => r5::read(&object),
    }
}

/// Writes `documents` as a FHIR `Bundle` of type `collection`, one entry
/// per `Consent` (R4 and R5 `bundle.html`).
///
/// # Errors
/// [`Refused`] when a document does not fit the release, such as an R4
/// document with more than one root provision.
pub fn bundle(release: Release, documents: &[Document]) -> Result<Vec<u8>, Refused> {
    let mut entries = Vec::with_capacity(documents.len());
    for document in documents {
        let resource = match release {
            Release::R4 => r4::write(document)?,
            Release::R5 => r5::write(document)?,
        };
        entries.push(Value::Object(Object::from([(
            "resource".to_owned(),
            Value::Object(resource),
        )])));
    }
    let bundle = Object::from([
        (
            "resourceType".to_owned(),
            Value::String("Bundle".to_owned()),
        ),
        ("type".to_owned(), Value::String("collection".to_owned())),
        ("entry".to_owned(), Value::Array(entries)),
    ]);
    serde_json::to_vec(&bundle).map_err(|error| {
        Refused::new(
            release.format(),
            "Bundle",
            Loss::Malformed(error.to_string()),
        )
    })
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "test fixtures are raw JSON by the testing rule (.claude/rules/testing.md)"
)]
mod tests;
