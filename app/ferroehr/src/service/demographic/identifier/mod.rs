// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Protected national identifiers in the demographic domain.
//!
//! **No openEHR spec governs this — our own design/extension.** The RM models
//! identifiers generically (`PARTY_IDENTITY`, RM demographic
//! `UML/classes/org.openehr.rm.demographic.party_identity.adoc`; `DV_IDENTIFIER`,
//! RM `data_types` `UML/classes/org.openehr.rm.data_types.dv_identifier.adoc`)
//! and says nothing about protecting the value. A national identifier needs
//! more than that generic slot: GDPR Art. 32(1)(a) names encryption as an
//! appropriate measure, and UAVG Art. 46 with the Wabvpz permit BSN processing
//! in care only for identification and under the act's conditions.
//!
//! The value therefore leaves the versioned body and lives in
//! `party.national_identifier`, sealed under a per-domain key, with a
//! keyed digest beside it for equality lookup without decryption. The body
//! keeps a reference in its place.

pub mod body;
pub mod config;
pub mod crypto;
pub mod engine;
pub mod store;

use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::status::SmError;
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};

impl FerroEhrService {
    /// The party holding `value` in `scheme`, resolved without decrypting, and
    /// recorded as an access.
    ///
    /// The sole identifier-to-party path above the store. The resolution goes
    /// through the keyed digest, so the plaintext never reaches the database,
    /// and every call — hit or miss — writes an access event naming the actor,
    /// the scheme and the purpose the caller declared. A resolution nobody can
    /// reconstruct afterwards is the boundary crossing the access log exists to
    /// prevent, and a MISS is worth recording for the same reason a hit is: it
    /// says someone asked whether this deployment holds that identifier.
    ///
    /// **No openEHR spec governs this — our own design/extension.**
    ///
    /// # Errors
    /// [`SmError::precondition`] when the deployment configures no
    /// identifier protection, and `exception` when the resolution query fails.
    pub async fn resolve_party_by_identifier(
        &self,
        scheme: &str,
        value: &str,
    ) -> Result<Option<VoId>, SmError> {
        let Some(engine) = self.identifier_protection() else {
            return Err(SmError::precondition(
                "this deployment configures no national-identifier protection, so there is \
                 no protected identifier to resolve",
            ));
        };
        let resolved = engine.resolve(scheme, value).await.map_err(|error| {
            // The client is told the scheme and nothing else; the cause rides
            // as a source so an operator can walk the chain without a database
            // error text reaching a caller.
            SmError::exception(format!("resolving a `{scheme}` identifier failed"))
                .with_source(error)
        })?;
        self.emit_identifier_resolution(scheme, resolved.is_some())?;
        Ok(resolved.map(VoId))
    }

    /// Record one identifier resolution on the access log.
    ///
    /// The record names the scheme and whether it matched, never the value:
    /// an audit trail that carried the identifier would hold the very data the
    /// sealing exists to keep out of readable storage.
    ///
    /// # Errors
    /// The `service_overloaded` [`SmError`] when the sender rejected the record
    /// under `fail_mode = "closed"`; the resolution is then withheld.
    fn emit_identifier_resolution(&self, scheme: &str, matched: bool) -> Result<(), SmError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let mut event = AuditEvent::new(
            EventActionCode::Execute,
            ObjectClass::Demographic,
            EventOutcome::Success,
        );
        event.domain = AccessDomain::Linkage;
        event.object_id = Some(format!("national-identifier:{scheme}"));
        event.result_count = Some(u64::from(matched));
        self.record_access(event)
    }
}
