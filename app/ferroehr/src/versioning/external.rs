// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a verified external caller of a `federation` issuer commits: an
//! external person only a COMPOSITION of a template its
//! `[[authz.federation.agreement]]` lists in `writes`, naming its healthcare
//! provider as the facility, and an external organisation nothing.
//!
//! EHDS Art. 13(4) (`docs/law/eu/ehds/text.html`) asks the record to identify
//! the professional who registers data, so an organisation access is
//! read-only, and an external person's rights come from the agreement, such as
//! the dispensation of Art. 23(6). What an external caller may commit is our
//! own design.

use serde_json::Value;

use crate::error::{ServiceError, Violation};
use crate::versioning::Kind;
use crate::versioning::committer::ExternalCaller;
use crate::versioning::committer::external::{ExternalProfessional, committer, provider};
use crate::versioning::system::{facility_of, identifiers, template_of};

/// Why a commit by an external organisation is refused.
const ORGANISATION: &str =
    "an external organisation's access names no professional and is read-only (EHDS Art. 13(4))";

/// Admits a create or a modify of `kind`, with the content `canonical` of
/// the template `template_id` or its own, by the request's external caller;
/// any other caller passes.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) for a commit by an external
/// organisation, for an external person's commit of anything but a
/// COMPOSITION of a template its agreement's `writes` lists, and for a token
/// whose fields name no committer or no facility
/// ([`crate::versioning::committer::external::committer`]);
/// [`ServiceError::Unprocessable`] (`422`) for a COMPOSITION whose facility
/// does not carry the provider's identifier.
pub(crate) fn admit_content(
    kind: Kind,
    canonical: &Value,
    template_id: Option<&str>,
) -> Result<(), ServiceError> {
    let Some(external) = external() else {
        return Ok(());
    };
    let template = template_of(canonical, template_id);
    match external {
        ExternalCaller::Person {
            writes,
            professional,
        } if kind == Kind::Composition
            && template.is_some_and(|template| writes.iter().any(|w| w == template)) =>
        {
            if let Some(identity) = crate::versioning::committer::current_committer() {
                committer(&identity, &professional)?;
            }
            admit_facility(&professional, canonical)
        }
        ExternalCaller::Person { .. } => Err(ServiceError::not_permitted(
            "a verified external person commits only a COMPOSITION of a template its \
             [[authz.federation.agreement]] writes lists",
        )),
        ExternalCaller::Organisation => Err(ServiceError::not_permitted(ORGANISATION)),
    }
}

/// Admits a logical deletion by the request's external caller; any other
/// caller passes.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) for every deletion by an external
/// caller.
// TODO(#5144): admit an external person's discard of a dispensation from its point of care.
pub(crate) fn admit_delete() -> Result<(), ServiceError> {
    match external() {
        None => Ok(()),
        Some(ExternalCaller::Organisation) => Err(ServiceError::not_permitted(ORGANISATION)),
        Some(ExternalCaller::Person { .. }) => Err(ServiceError::not_permitted(
            "a verified external person deletes nothing its agreement does not admit",
        )),
    }
}

/// Refuses a COMPOSITION whose facility does not carry the identifier of the
/// healthcare provider `professional`'s token asserts (EHDS Art. 13(4)).
fn admit_facility(
    professional: &ExternalProfessional,
    canonical: &Value,
) -> Result<(), ServiceError> {
    let (issuer, id) = provider(professional)?;
    let (facility, path) = facility_of(canonical);
    if identifiers(facility).any(|named| named == (issuer.as_str(), id.as_str())) {
        crate::system_log::access_context::record_facility(&issuer, &id);
        return Ok(());
    }
    Err(ServiceError::content_invalid(
        Violation::new(
            "carries no identifier of the healthcare provider the federation token asserts, \
             its healthcare_provider_identifier under its issuing_authority_name; the server \
             writes no facility (EHDS Art. 13(4))",
        )
        .with_path(path),
    ))
}

/// The request's external caller, when there is one.
fn external() -> Option<ExternalCaller> {
    crate::versioning::committer::current_committer().and_then(|identity| identity.external)
}
