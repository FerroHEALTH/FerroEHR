// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The exchange of access directives with the documents other systems hold
//! them in.
//!
//! The formats are HL7 FHIR `Consent` in R4 and R5 ([`fhir`]), the IHE BPPC
//! Patient Privacy Acknowledgement Document ([`bppc`]) and the IHE APPC
//! Privacy Consent Document ([`appc`]).
//!
//! Each format maps to and from the `ACCESS_DIRECTIVE`s of the draft access
//! control package, the content the evaluator decides on
//! ([`crate::access_eval`]). A round trip keeps every attribute the evaluator
//! reads: the kind and effect, `applies_to`, `operations`, `purposes`, the
//! data scope, `time_validity`, `emergency_override` and one level of
//! `exceptions`. Whatever one side holds and the other cannot carry is
//! refused whole with the element named ([`Refused`]), never dropped.
//!
//! The mappings are checked against the vendored texts:
//! `docs/specs/fhir-consent/` (FHIR R4 4.0.1 and R5 5.0.0 `Consent`),
//! `docs/specs/ihe-consents/` (BPPC, ITI TF-1 chapter 19 and ITI TF-3
//! section 5.1; APPC, the supplement Rev. 1.4) and `docs/specs/xacml/` (XACML
//! 2.0). Where those texts leave a choice open, the choice is FerroEHR's own
//! design and says so where it is made. No openEHR specification governs the
//! exchange.

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;

pub mod appc;
pub mod bppc;
#[cfg(feature = "fhir")]
pub mod fhir;
mod rm;
mod xml;

/// The format a refusal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// HL7 FHIR R4 (4.0.1) `Consent`.
    FhirR4,
    /// HL7 FHIR R5 (5.0.0) `Consent`.
    FhirR5,
    /// The IHE BPPC acknowledgement document (ITI TF-3 5.1).
    Bppc,
    /// The IHE APPC Privacy Consent Document (APPC TF-3 5.6).
    Appc,
    /// The FerroEHR `ACCESS_DIRECTIVE` a document is written from.
    Directive,
}

impl std::fmt::Display for Format {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::FhirR4 => "FHIR R4",
            Self::FhirR5 => "FHIR R5",
            Self::Bppc => "IHE BPPC",
            Self::Appc => "IHE APPC",
            Self::Directive => "the access directive's",
        })
    }
}

/// Why an element or an attribute has no form on the other side.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Loss {
    /// The element has no form in an `ACCESS_DIRECTIVE`.
    #[error("has no form in an openEHR access directive")]
    NoDirectiveForm,
    /// The attribute has no form in the target format.
    #[error("has no form in {0}")]
    NoFormatForm(Format),
    /// The element nests deeper than a directive's one level of exceptions.
    #[error("nests deeper than the one level of exceptions an access directive holds")]
    TooDeep,
    /// The element carries a code the mapping does not read.
    #[error("carries {0}, which this mapping does not read")]
    Code(String),
    /// A nested rule states its parent's effect, so it is no exception.
    #[error("states the effect of the rule it sits in, so it is no exception to it")]
    NotException,
    /// The document has rules and no base policy they are exceptions to.
    #[error("holds rules and no base policy they are exceptions to")]
    NoBase,
    /// A whole-EHR deny would override the scoped permits beside it.
    #[error(
        "denies outside its rules on an EHR whose default_access is open, and a whole-EHR deny \
         directive would override every permit the rules state"
    )]
    DenyOverOpen,
    /// The policy is not in the `ACCESS_POLICY` catalogue.
    #[error("names the policy {0}, which the ACCESS_POLICY catalogue does not hold")]
    NotInCatalogue(String),
    /// The reference names nothing FerroEHR can resolve.
    #[error("names no party or object FerroEHR holds")]
    Unresolved,
    /// The element is not in the form its specification gives it.
    #[error("cannot be read: {0}")]
    Malformed(String),
    /// The code is no FerroEHR operation class and no translation names one.
    #[error(
        "codes {0}, which is no operation class of the access control terminology, and the \
         terminology server translates it to none"
    )]
    Untranslated(String),
    /// More than one value where the other side holds one.
    #[error("holds more than one value where the other side holds one")]
    Several,
}

/// A document or a directive refused whole, naming the element.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{format} {element} {loss}")]
pub struct Refused {
    /// The format the element belongs to.
    pub format: Format,
    /// The element or attribute, as its specification names it.
    pub element: String,
    /// Why it was refused.
    pub loss: Loss,
}

impl Refused {
    /// A refusal of `element` of `format`.
    #[must_use]
    pub fn new(format: Format, element: impl Into<String>, loss: Loss) -> Self {
        Self {
            format,
            element: element.into(),
            loss,
        }
    }
}

/// The `ACCESS_POLICY.catalogue` entry identified by `code`: an identifier
/// whose `id` is `code` and whose `issuer`, when both name one, is `system`.
#[must_use]
pub fn catalogue_entry<'a>(
    catalogue: &'a [AccessDirective],
    system: Option<&str>,
    code: &str,
) -> Option<&'a AccessDirective> {
    catalogue.iter().find(|entry| {
        rm::members(entry.identifiers.as_ref())
            .iter()
            .any(|identifier| {
                identifier.id == code
                    && match (system, identifier.issuer.as_deref()) {
                        (Some(system), Some(issuer)) => system == issuer,
                        _ => true,
                    }
            })
    })
}
