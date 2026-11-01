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

use serde_json::Value;

use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::service::demographic::identifier::store::StoreError;
use crate::service::error::ServiceError;
use crate::system_log::event::{
    AccessDomain, AuditEvent, EventActionCode, EventOutcome, ObjectClass,
};
use crate::versioning::object_version_id::parse_version_uid;

impl FerroEhrService {
    /// The `ORIGINAL_VERSION` of the party version `version_uid` with its
    /// protected identifiers expanded back to their values, and the scheme of
    /// each value expanded, recorded as an access.
    ///
    /// The audited read of a party's protected national identifiers (#3776).
    /// Any version may be named, so a historical version shows the values it
    /// committed. The expansion is a view for an authorised reader: the
    /// served body no longer matches the stored, signed one, and nothing is
    /// written back. The caller decides that the reader may see the values;
    /// every call writes one access event per scheme expanded (one naming no
    /// scheme when the version holds none), naming the reader, the party and
    /// the scheme and never the value, before the body is returned.
    ///
    /// **No openEHR spec governs this — our own design/extension.**
    ///
    /// # Errors
    /// [`ServiceError::precondition`] when the deployment configures no identifier
    /// protection or `version_uid` is no `OBJECT_VERSION_ID`, the
    /// `object_version_does_not_exist` [`ServiceError`] when no party holds that
    /// version, `exception` when a sealed value cannot be read or does not
    /// authenticate, and `service_overloaded` when the access record is
    /// refused under `fail_mode = "closed"`, which withholds the values.
    pub async fn read_party_identifiers(
        &self,
        version_uid: &str,
    ) -> Result<(Value, Vec<String>), ServiceError> {
        let Some(engine) = self.identifier_protection() else {
            return Err(ServiceError::precondition(
                "this deployment configures no national-identifier protection, so a party \
                 carries no protected identifier to expand",
            ));
        };
        let (party, _) = parse_version_uid(version_uid).map_err(|error| {
            ServiceError::bad_request(
                format!("`{version_uid}` is not an OBJECT_VERSION_ID"),
                error,
            )
        })?;
        let mut version = self.get_party_at_version(version_uid.to_owned()).await?;
        let schemes = engine.expand(&mut version).await.map_err(|error| {
            if let StoreError::Acquire(acquire) = error {
                return ServiceError::Acquire(acquire);
            }
            ServiceError::fault_caused(
                "expanding the protected identifiers of a party version failed",
                error,
            )
        })?;
        self.emit_identifier_expansion(party, &schemes)?;
        Ok((version, schemes))
    }

    /// Record one expansion of a party's protected identifiers: an event per
    /// scheme expanded, or one naming no scheme when the version held none.
    ///
    /// # Errors
    /// The `service_overloaded` [`ServiceError`] when the sender rejected a record
    /// under `fail_mode = "closed"`; the values are then withheld.
    fn emit_identifier_expansion(
        &self,
        party: VoId,
        schemes: &[String],
    ) -> Result<(), ServiceError> {
        if !self.audit_enabled() {
            return Ok(());
        }
        let mut named: Vec<Option<&str>> = schemes
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(Some)
            .collect();
        if named.is_empty() {
            named.push(None);
        }
        for scheme in named {
            let mut event = AuditEvent::new(
                EventActionCode::Read,
                ObjectClass::Demographic,
                EventOutcome::Success,
            );
            event.domain = AccessDomain::Demographic;
            event.object_id = Some(match scheme {
                Some(scheme) => format!("party:{}#national-identifier:{scheme}", party.0),
                None => format!("party:{}#national-identifier", party.0),
            });
            event.result_count = Some(u64::from(scheme.is_some()));
            event.purpose = crate::system_log::access_context::current_purpose();
            event.permit = crate::system_log::access_context::current_permit();
            if let Some(committer) = crate::service::committer::current_committer() {
                event.user_id = committer.subject;
            }
            event.legal_basis = self.audit_legal_basis().map(str::to_owned);
            self.record_access(event)?;
        }
        Ok(())
    }
}
