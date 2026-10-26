// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The LINKAGE service module: the in-instance relation that says which
//! demographic party, and which subject identifier, name which EHR.
//!
//! **No openEHR spec governs the storage — our own design/extension.** The
//! relation (`linkage.subject_ehr`, written and read through this module's
//! `store`) holds the SM's EHR Index associations when a FerroEHR run on its
//! own opts in to the local map (`[ehr_index] provider = "local"`), because
//! master07 §Overview describes exactly this schema's purpose — "In a
//! privacy-supporting environment, this enables EHRs to be persisted with only
//! an EHR id; the EHR Index has to be used to obtain the subject identifier".
//! The `I_EHR_INDEX` operations themselves live in
//! [`crate::service::ehr_index`], which is the SM chapter and the provider
//! seam; this module owns the relation.
//!
//! FerroEHR does no identity work here: it matches no patient, cross-references
//! no identifier domain and merges nothing. That work belongs to an IHE
//! patient identity source, which the default provider asks over PIXm.
//!
//! `ehr_get_by_subject` does NOT come here. It obliges a match against the
//! EHR's own `EHR_STATUS.subject.external_ref.id.value` and `.namespace`
//! (ITS-REST `specifications/operations/ehr_get_by_subject.yaml`), which the
//! clinical schema serves from its own promoted columns — and which
//! [`crate::privacy::PrivacyPolicy`] constrains to an opaque pseudonym.
//!
//! GDPR Art. 4(5) defines pseudonymisation as processing where attribution to
//! a person needs "additional information" that is "kept separately and
//! subject to technical and organisational measures"
//! (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>); this relation is such
//! additional information, so it lives in its own schema under its own role,
//! each runtime role revoked from the other domains' schemas, and
//! `crate::db::verify_domain_isolation` refuses to boot a database where one
//! is not.
//!
//! ## Temporal, never destructive
//!
//! Nothing here deletes a row, and the linkage role holds no `DELETE`
//! privilege to do it with; an association ends by closing its period. The
//! temporal key (`UNIQUE (party_id, sys_period WITHOUT OVERLAPS)`) enforces one
//! mapping in force per party. The one destructive path is erasure
//! ([`FerroEhrService::erase_ehr_linkage`]), which the role cannot perform
//! itself and reaches only through a definer function.
//!
//! ## Every crossing is recorded
//!
//! Each operation emits one `linkage`-domain access event
//! ([`crate::system_log::event::AccessDomain::Linkage`]), naming the actor,
//! the declared purpose of use and how many mappings it resolved. A boundary
//! crossing nobody can reconstruct afterwards is exactly what the access log
//! exists to prevent, and a MISS is recorded for the same reason a hit is: it
//! says someone asked.

pub mod cohort;
pub(crate) mod store;

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::status::SmError;
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};

/// What went wrong resolving or rewriting the party-to-EHR map.
///
/// Typed rather than stringly: a caller distinguishing "this party is already
/// linked" from "the store is unavailable" reaches two different outcomes, and
/// a `23P01` read out of an error message is not a decision anyone should have
/// to make twice.
#[derive(Debug, thiserror::Error)]
pub enum LinkageError {
    /// The party already holds a mapping in force, so a second one would
    /// overlap it.
    ///
    /// Raised by the temporal primary key, not by a pre-read: a check-then-act
    /// would race, and the constraint is the enforcement either way.
    #[error("party {party_id} already holds a mapping to an EHR")]
    AlreadyLinked {
        /// The party that already holds one.
        party_id: VoId,
    },
    /// The linkage store refused or failed.
    #[error("the linkage store is unavailable")]
    Database(#[source] sqlx::Error),
    /// No connection to the linkage store could be obtained, so nothing ran.
    #[error("the linkage store is unavailable")]
    Acquire(#[from] crate::db::acquire::AcquireError),
    /// The operation ran but its access record could not be taken, and the
    /// deployment fails closed, so the result is withheld.
    ///
    /// A crossing of the pseudonymisation boundary that nobody can reconstruct
    /// afterwards is what the access log exists to prevent; under
    /// `[audit] fail_mode = "closed"` an unrecorded crossing is refused rather
    /// than served.
    #[error("the access record could not be taken and the deployment fails closed")]
    Unrecorded(#[source] SmError),
}

impl FerroEhrService {
    /// Open a mapping: `party_id` is the subject of `ehr_id`, from now on.
    ///
    /// Who calls this is the consumer's question, not this module's: nothing
    /// populates the map automatically, because a mapping written by a path
    /// that guessed at it would be the one fact in the deployment nobody
    /// could audit.
    ///
    /// # Errors
    /// [`LinkageError::AlreadyLinked`] when the party already holds a mapping
    /// in force, [`LinkageError::Acquire`] when no connection can be obtained,
    /// and [`LinkageError::Database`] when the write fails.
    pub async fn link(&self, party_id: VoId, ehr_id: EhrId) -> Result<(), LinkageError> {
        let mut conn = acquire(&self.linkage_pool).await?;
        let outcome = store::open(&mut conn, party_id, ehr_id)
            .await
            .map_err(|error| classify_for(error, party_id));
        self.emit_linkage_access(
            EventActionCode::Create,
            format!("party-ehr:{party_id}"),
            outcome.is_ok().then_some(ehr_id),
            u64::from(outcome.is_ok()),
            outcome.is_ok(),
        )?;
        outcome
    }

    /// The EHR `party_id` is the subject of right now, or `None`.
    ///
    /// The direct half of the map, for a caller that already holds a party id
    /// — an identity resolution that got there through the demographic domain,
    /// or a demographic read that is already entitled to the party.
    ///
    /// # Errors
    /// [`LinkageError::Acquire`] when no connection can be obtained, and
    /// [`LinkageError::Database`] when the read fails.
    pub async fn resolve_ehr_for_party(
        &self,
        party_id: VoId,
    ) -> Result<Option<EhrId>, LinkageError> {
        let resolved = store::open_mapping(&self.linkage_pool, party_id)
            .await
            .map_err(LinkageError::from)?;
        self.emit_linkage_access(
            EventActionCode::Read,
            format!("party-ehr:{party_id}"),
            resolved,
            u64::from(resolved.is_some()),
            true,
        )?;
        Ok(resolved)
    }

    /// Remove every cross-reference row naming `ehr_id`, returning how many.
    ///
    /// The erasure path, and the only destructive one this domain has. A
    /// physically deleted EHR must leave no row here asserting whose record it
    /// was: that row is the additional information of GDPR Art. 4(5) outliving
    /// the data it was additional to, and Art. 17(1) reaches it
    /// (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>). The role holds no
    /// `DELETE`, so the work is done by the definer function the migration set
    /// defines for exactly this.
    ///
    /// Called after the clinical delete has committed, never before: a map
    /// erased ahead of a delete that then failed would leave a record no one
    /// can attribute, which is the worse of the two half-states.
    ///
    /// # Errors
    /// [`LinkageError::Acquire`] when no connection can be obtained,
    /// [`LinkageError::Database`] when the call fails — including the privilege
    /// refusal of a deployment that granted the role no `EXECUTE` — and
    /// [`LinkageError::Unrecorded`] when the access record is refused under
    /// `fail_mode = "closed"`.
    pub async fn erase_ehr_linkage(&self, ehr_id: EhrId) -> Result<u64, LinkageError> {
        let outcome = store::erase_ehr(&self.linkage_pool, ehr_id)
            .await
            .map_err(LinkageError::from);
        self.emit_linkage_access(
            EventActionCode::Delete,
            format!("ehr:{ehr_id}"),
            Some(ehr_id),
            outcome.as_ref().copied().unwrap_or(0),
            outcome.is_ok(),
        )?;
        outcome
    }

    /// Record one linkage-domain access.
    ///
    /// The record names the row (`party-ehr:<party>`;
    /// `subject:<namespace>` for an index operation), the EHR it
    /// resolved to when it resolved to exactly one, and how many rows it
    /// touched — never an identifier value: a trail carrying the identity
    /// would hold the very data the sealing keeps out of readable storage. The
    /// actor is the request's authenticated committer and the purpose is the
    /// code the deployment's own vocabulary accepted
    /// ([`crate::system_log::access_context`]); outside a request scope both
    /// are absent, which the record states rather than guesses.
    ///
    /// `pub(crate)`: the SM `I_EHR_INDEX` operations
    /// ([`crate::service::ehr_index`]) reach the same relation through the same
    /// pool, and a crossing recorded from one module and not the other would
    /// make the trail depend on which door was used.
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
        self.record_access(event).map_err(LinkageError::Unrecorded)
    }
}

/// Put the request's actor and declared purpose on a service-emitted record.
///
/// The protocol adapter's own records get these from the request it still
/// holds; a record built down here gets them from the two task-locals the
/// adapter published for the request's scope
/// ([`crate::service::committer`], [`crate::system_log::access_context`]).
pub(crate) fn stamp_requester(event: &mut AuditEvent) {
    if let Some(committer) = crate::service::committer::current_committer() {
        event.user_id = committer.subject;
    }
    event.purpose = crate::system_log::access_context::current_purpose();
}

impl From<store::StoreError> for LinkageError {
    fn from(error: store::StoreError) -> Self {
        match error {
            store::StoreError::Acquire(e) => LinkageError::Acquire(e),
            store::StoreError::Database(e) => LinkageError::Database(e),
        }
    }
}

/// A driver error from a write that could be the temporal key refusing an
/// overlap, classified against the party the write named.
fn classify_for(error: sqlx::Error, party_id: VoId) -> LinkageError {
    if store::is_overlap(&error) {
        return LinkageError::AlreadyLinked { party_id };
    }
    LinkageError::Database(error)
}
