// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The LINKAGE service module: the crossings between the demographic and the
//! clinical domain, and the access record each one owes.
//!
//! **No openEHR spec governs this — our own design/extension.** Which EHR
//! belongs to which subject has one source: the EHR's own
//! `EHR_STATUS.subject.external_ref` (RM ehr master04 §EHR Status), promoted
//! to the `ehr` table's subject columns on every EHR create and `EHR_STATUS`
//! update. A cohort ([`cohort`]) and the AQL demographic join resolve a party
//! or a named subject to its EHRs through those columns, and the EHR index
//! providers ([`crate::service::ehr_index`]) resolve a subject the same way.
//!
//! FerroEHR does no identity work here: it matches no patient, cross-references
//! no identifier domain and merges nothing. That work belongs to an IHE
//! patient identity source, which the default provider asks over PIXm.
//!
//! ## Every crossing is recorded
//!
//! Each crossing emits one `linkage`-domain access event
//! ([`crate::system_log::event::AccessDomain::Linkage`]), naming the actor,
//! the declared purpose of use and how many EHRs it resolved. A boundary
//! crossing nobody can reconstruct afterwards is exactly what the access log
//! exists to prevent, and a MISS is recorded for the same reason a hit is: it
//! says someone asked.

pub mod cohort;

use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::status::StatusError;
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};

/// A linkage-domain crossing whose access record could not be taken.
#[derive(Debug, thiserror::Error)]
pub enum LinkageError {
    /// The crossing ran but its access record could not be taken, and the
    /// deployment fails closed, so the result is withheld.
    ///
    /// A crossing of the pseudonymisation boundary that nobody can reconstruct
    /// afterwards is what the access log exists to prevent; under
    /// `[audit] fail_mode = "closed"` an unrecorded crossing is refused rather
    /// than served.
    #[error("the access record could not be taken and the deployment fails closed")]
    Unrecorded(#[source] StatusError),
}

impl FerroEhrService {
    /// Record one linkage-domain access.
    ///
    /// The record names what was crossed (`party-ehr:query` for a join,
    /// `subject:<namespace>` for a subject resolution), the EHR it resolved to
    /// when it resolved to exactly one, and how many EHRs it resolved — never an identifier value: a trail carrying the identity
    /// would hold the very data the sealing keeps out of readable storage. The
    /// actor is the request's authenticated committer and the purpose is the
    /// code the deployment's own vocabulary accepted
    /// ([`crate::system_log::access_context`]); outside a request scope both
    /// are absent, which the record states rather than guesses.
    ///
    /// `pub(crate)`: the AQL demographic join crosses the same boundary, and
    /// a crossing recorded from one module and not the other would make the
    /// trail depend on which door was used.
    ///
    /// # Errors
    /// [`LinkageError::Unrecorded`] when the sender rejected the record under
    /// `fail_mode = "closed"`; the caller withholds its result.
    pub(crate) fn emit_linkage_access(
        &self,
        action: EventActionCode,
        object_id: String,
        ehr_id: Option<EhrId>,
        result_count: u64,
        succeeded: bool,
    ) -> Result<(), LinkageError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let outcome = if succeeded {
            EventOutcome::Success
        } else {
            EventOutcome::MinorFailure
        };
        let mut event = AuditEvent::new(action, ObjectClass::Demographic, outcome);
        event.domain = AccessDomain::Linkage;
        event.object_id = Some(object_id);
        event.ehr_id = ehr_id.map(|id| id.to_string());
        event.result_count = Some(result_count);
        stamp_requester(&mut event);
        event.legal_basis = self.audit_legal_basis().map(str::to_owned);
        self.record_access(event)
            .map_err(|e| LinkageError::Unrecorded(StatusError::from(e)))
    }
}

/// Put the request's actor, declared purpose and data permit on a
/// service-emitted record.
///
/// The protocol adapter's own records get these from the request it still
/// holds; a record built down here gets them from the task-locals the adapter
/// published for the request's scope
/// ([`crate::service::committer`], [`crate::system_log::access_context`]).
pub(crate) fn stamp_requester(event: &mut AuditEvent) {
    if let Some(committer) = crate::service::committer::current_committer() {
        event.user_id = committer.subject;
    }
    event.purpose = crate::system_log::access_context::current_purpose();
    event.permit = crate::system_log::access_context::current_permit();
}
