// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a verified external caller of a `federation` issuer commits: an
//! external person only a COMPOSITION of a template its federation
//! agreement lists (`template`), naming its healthcare
//! provider as the facility, and an external organisation nothing.
//!
//! EHDS Art. 13(4) (`docs/law/eu/ehds/text.html`) asks the record to identify
//! the professional who registers data, so an organisation access is
//! read-only, and an external person's rights come from the agreement, such as
//! the dispensation of Art. 23(6). What an external caller may commit is our
//! own design.
//!
//! An agreement that holds a `discard_window` lets the person discard, as a
//! logical delete, a dispensation of a template it lists, committed within
//! that window at the same point of care: every identifier the
//! person's own write names its point of care by is in the dispensation's
//! `health_care_facility.identifiers` (MyHealth@EU Requirements Catalogue
//! 10.0.0, requirement 07.04; the eHDSI point of care is a name only, so the
//! match on identifiers is our own design).

use serde_json::Value;
use sqlx::PgConnection;

use crate::error::{ServiceError, Violation};
use crate::ids::VoId;
use crate::versioning::Kind;
use crate::versioning::committer::ExternalCaller;
use crate::versioning::committer::external::{
    ExternalProfessional, ORGANIZATION_ID_ISSUER, committer, may_dispense, point_of_care, provider,
};
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
/// does not carry the provider's identifier; [`ServiceError::Forbidden`]
/// for a dispensation whose token carries permissions without PPD-046.
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
            ..
        } if kind == Kind::Composition
            && template.is_some_and(|template| writes.iter().any(|w| w == template)) =>
        {
            if let Some(identity) = crate::versioning::committer::current_committer() {
                committer(&identity, &professional)?;
            }
            admit_dispensing(&professional)?;
            admit_facility(&professional, canonical)
        }
        ExternalCaller::Person { .. } => Err(ServiceError::not_permitted(
            "a verified external person commits only a COMPOSITION of a template its \
             federation agreement lists",
        )),
        ExternalCaller::Organisation => Err(ServiceError::not_permitted(ORGANISATION)),
    }
}

/// Admits a logical deletion of `vo_id`, of `kind`, by the request's
/// external caller: a person's discard of a dispensation its agreement
/// admits; any other caller passes.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) for every deletion by an external
/// organisation, and for a person's deletion that is no discard of a
/// dispensation of a listed template, committed within the window at the
/// person's point of care by a token holding PPD-046 where it carries
/// permissions; the storage error of the dispensation read.
pub(crate) async fn admit_delete(
    tx: &mut PgConnection,
    kind: Kind,
    vo_id: VoId,
) -> Result<(), ServiceError> {
    let (writes, professional, hours) = match external() {
        None => return Ok(()),
        Some(ExternalCaller::Organisation) => {
            return Err(ServiceError::not_permitted(ORGANISATION));
        }
        Some(ExternalCaller::Person {
            writes,
            professional,
            discard_window_hours: Some(hours),
        }) if kind == Kind::Composition => (writes, professional, hours),
        Some(ExternalCaller::Person { .. }) => {
            return Err(ServiceError::not_permitted(
                "a verified external person deletes nothing but a dispensation its \
                 federation agreement lets it discard",
            ));
        }
    };
    admit_dispensing(&professional)?;
    let dispensed = crate::storage::version_repo::placement::dispensed(tx, vo_id).await?;
    let now = crate::storage::version_repo::placement::tx_now(tx).await?;
    let Some(dispensed) = dispensed.filter(|d| {
        d.template
            .as_deref()
            .is_some_and(|template| writes.iter().any(|listed| listed == template))
    }) else {
        return Err(discard_refused(
            "it is no dispensation of a template the agreement writes",
        ));
    };
    let window = jiff::SignedDuration::from_hours(i64::from(hours));
    if now.duration_since(dispensed.first_committed) > window {
        return Err(discard_refused("its discard window has passed"));
    }
    let Some(text) = dispensed.body.as_deref() else {
        return Err(discard_refused("it is already deleted"));
    };
    let body: Value = serde_json::from_str(text)
        .map_err(|e| ServiceError::fault_caused("read the stored dispensation", e))?;
    let named: Vec<(&str, &str)> = identifiers(facility_of(&body).0).collect();
    let point = point_of_care(&professional)?;
    if point
        .iter()
        .all(|(issuer, id)| named.contains(&(issuer.as_str(), id.as_str())))
    {
        return Ok(());
    }
    Err(discard_refused(
        "it was recorded at another point of care: its health_care_facility does not carry \
         every identifier the discarding token names its point of care by",
    ))
}

/// Refuses a dispensation or its discard by a token that carries permissions
/// without PPD-046 (eHDSI SAML Profile 10.0.0 §2.5).
fn admit_dispensing(professional: &ExternalProfessional) -> Result<(), ServiceError> {
    if may_dispense(professional) {
        return Ok(());
    }
    Err(ServiceError::not_permitted(
        "the token carries permissions without PPD-046, which a dispensation and its \
         discard need (eHDSI SAML Profile 10.0.0 §2.5)",
    ))
}

/// The `403` refusing a discard, saying `why`.
fn discard_refused(why: &str) -> ServiceError {
    ServiceError::not_permitted(format!(
        "a verified external person discards only a dispensation of its own point of care \
         within the agreement's discard_window_hours (MyHealth@EU Requirements Catalogue \
         10.0.0, requirement 07.04): {why}"
    ))
}

/// Refuses a COMPOSITION whose facility does not carry the identifier of the
/// healthcare provider `professional`'s token asserts (EHDS Art. 13(4)) and,
/// on the MyHealth@EU route, the HCPO's `organization-id` beside it (eHDSI
/// SAML Profile 10.0.0 §2.3), so a later discard is matched to its point of
/// care from the version alone.
fn admit_facility(
    professional: &ExternalProfessional,
    canonical: &Value,
) -> Result<(), ServiceError> {
    let (issuer, id) = provider(professional)?;
    let (facility, path) = facility_of(canonical);
    let named: Vec<(&str, &str)> = identifiers(facility).collect();
    if !named.contains(&(issuer.as_str(), id.as_str())) {
        return Err(ServiceError::content_invalid(
            Violation::new(
                "carries no identifier of the healthcare provider the federation token \
                 asserts, its healthcare_provider_identifier under its issuing_authority_name; \
                 the server writes no facility (EHDS Art. 13(4))",
            )
            .with_path(path),
        ));
    }
    if point_of_care(professional)?
        .iter()
        .any(|(issuer, id)| !named.contains(&(issuer.as_str(), id.as_str())))
    {
        return Err(ServiceError::content_invalid(
            Violation::new(format!(
                "carries no identifier of the healthcare provider organisation's \
                 organization-id under the issuer {ORGANIZATION_ID_ISSUER} beside the \
                 provider's (eHDSI SAML Profile 10.0.0 §2.3); the server writes no facility"
            ))
            .with_path(path),
        ));
    }
    crate::system_log::access_context::record_facility(&issuer, &id);
    Ok(())
}

/// The request's external caller, when there is one.
fn external() -> Option<ExternalCaller> {
    crate::versioning::committer::current_committer().and_then(|identity| identity.external)
}
