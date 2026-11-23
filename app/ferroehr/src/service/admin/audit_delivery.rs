// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operator's view and acts on the delivery of the audit records (#6242).
//!
//! The request that acts is recorded in the system log as an application
//! activity naming its operator, as every admin request is. No openEHR spec
//! governs audit delivery: our own design.

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::service::FerroEhrService;
use crate::system_log::store::chains::AuditChain;
use crate::system_log::store::delivery::{DeliveryView, ParkedAct, Sink};

/// What a failed delivery read or act names as the failing store.
const DELIVERY_STORE: &str = "the audit delivery state";

/// The sink `name` names.
///
/// # Errors
/// `400` for a name of no sink.
fn sink(name: &str) -> Result<Sink, ServiceError> {
    Sink::named(name).ok_or_else(|| {
        ServiceError::precondition(format!("`{name}` names no sink: fhir_feed or syslog"))
    })
}

/// The log `name` names.
///
/// # Errors
/// `400` for a name of no log.
fn chain(name: &str) -> Result<AuditChain, ServiceError> {
    AuditChain::named(name).ok_or_else(|| {
        ServiceError::precondition(format!("`{name}` names no log: system or access_log"))
    })
}

impl FerroEhrService {
    /// The local audit store, or the refusal when it is off.
    fn delivering_store(&self) -> Result<&crate::system_log::store::AuditStore, ServiceError> {
        self.audit_store.as_ref().ok_or_else(|| {
            ServiceError::conflict("the local audit store is off, so no sink delivers from it")
        })
    }

    /// Every sink's cursors, how far each is behind, and the records parked.
    ///
    /// # Errors
    /// - `conflict` (`409`): the local audit store is off.
    /// - `exception`: the store failed.
    pub async fn audit_delivery(&self) -> Result<DeliveryView, ServiceError> {
        self.delivering_store()?
            .delivery_view()
            .await
            .map_err(|error| ServiceError::internal(DELIVERY_STORE, error))
    }

    /// Does `act` to `sink_name`'s parked record `id` of `chain_name`: a
    /// retry offers it again before the next record past the cursor, a
    /// dismissal removes it, which the operator decided the sink will not
    /// deliver.
    ///
    /// # Errors
    /// - `400`: a name of no sink or no log.
    /// - `404`: the sink parked no such record.
    /// - `conflict` (`409`): the local audit store is off.
    /// - `exception`: the store failed.
    pub async fn act_on_parked_audit_record(
        &self,
        act: ParkedAct,
        sink_name: &str,
        chain_name: &str,
        id: i64,
    ) -> Result<(), ServiceError> {
        let (sink, chain) = (sink(sink_name)?, chain(chain_name)?);
        let found = self
            .delivering_store()?
            .act_on_parked(act, sink, chain, id)
            .await
            .map_err(|error| ServiceError::internal(DELIVERY_STORE, error))?;
        if found && act == ParkedAct::Dismiss {
            tracing::warn!(
                sink = sink_name,
                chain = chain_name,
                record = id,
                "an operator dismissed a parked audit record: the sink will not deliver it"
            );
        }
        parked_or_missing(found, sink_name, chain_name, id)
    }

    /// Removes `sink_name`'s cursors and parked records; returns how many of
    /// each went.
    ///
    /// # Errors
    /// - `400`: a name of no sink.
    /// - `conflict` (`409`): the local audit store is off.
    /// - `exception`: the store failed.
    pub async fn forget_audit_sink(&self, sink_name: &str) -> Result<(u64, u64), ServiceError> {
        let forgotten = self
            .delivering_store()?
            .forget_sink(sink(sink_name)?)
            .await
            .map_err(|error| ServiceError::internal(DELIVERY_STORE, error))?;
        tracing::warn!(
            sink = sink_name,
            cursors = forgotten.0,
            parked = forgotten.1,
            "an operator forgot an audit sink: its delivery cursors and parked records are gone"
        );
        Ok(forgotten)
    }
}

/// `Ok` when a parked record was found, the `404` otherwise.
fn parked_or_missing(found: bool, sink: &str, chain: &str, id: i64) -> Result<(), ServiceError> {
    if found {
        return Ok(());
    }
    Err(ServiceError::not_found(
        Missing::VersionedObject,
        format!("the {sink} sink parked no record {id} of the {chain} log"),
    ))
}
