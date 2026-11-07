// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a verified external caller of a `federation` issuer commits: an
//! external person only a COMPOSITION of a template its
//! `[[authz.federation.agreement]]` lists in `writes`, and an external
//! organisation nothing.
//!
//! EHDS Art. 13(4) (`docs/law/eu/ehds/text.html`) asks the record to identify
//! the professional who registers data, so an organisation access is
//! read-only, and an external person's rights come from the agreement, such as
//! the dispensation of Art. 23(6). What an external caller may commit is our
//! own design.

use crate::error::ServiceError;
use crate::versioning::Kind;
use crate::versioning::committer::ExternalCaller;

/// Why a commit by an external organisation is refused.
const ORGANISATION: &str =
    "an external organisation's access names no professional and is read-only (EHDS Art. 13(4))";

/// Admits a create or a modify of `kind`, of the template `template`, by the
/// request's external caller; any other caller passes.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) for a commit by an external
/// organisation, and for an external person's commit of anything but a
/// COMPOSITION of a template its agreement's `writes` lists.
pub(crate) fn admit_content(kind: Kind, template: Option<&str>) -> Result<(), ServiceError> {
    let Some(external) = external() else {
        return Ok(());
    };
    match external {
        ExternalCaller::Person { writes }
            if kind == Kind::Composition
                && template.is_some_and(|template| writes.iter().any(|w| w == template)) =>
        {
            Ok(())
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
// TODO(#5102): admit an external person's discard of a dispensation from its point of care.
pub(crate) fn admit_delete() -> Result<(), ServiceError> {
    match external() {
        None => Ok(()),
        Some(ExternalCaller::Organisation) => Err(ServiceError::not_permitted(ORGANISATION)),
        Some(ExternalCaller::Person { .. }) => Err(ServiceError::not_permitted(
            "a verified external person deletes nothing its agreement does not admit",
        )),
    }
}

/// The request's external caller, when there is one.
fn external() -> Option<ExternalCaller> {
    crate::versioning::committer::current_committer().and_then(|identity| identity.external)
}
