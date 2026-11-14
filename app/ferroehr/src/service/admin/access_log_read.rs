// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads of the access log its ITS-REST resources serve (#5130).
//!
//! One EHR's log is read by its subject, their representative or a
//! reviewer; the whole log and the links of its chain by a reviewer (EHDS
//! Annex II 3.3).
//!
//! The person's view leaves out the entries about data held from them under
//! EHDS Art. 3(3) (`docs/law/eu/ehds/text.html`) until the hold ends, as their
//! ITI-81 trail does. No openEHR spec governs the access log: our own design.

use jiff::Timestamp;

use crate::access_log::search::LogQuery;
use crate::access_log::store::{AccessLogError, AccessLogStore};
use crate::access_log::view::{LogReader, withheld};
use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::system_log::sender::AuditSender;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_link::AccessLogLink;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;

impl FerroEhrService {
    /// The records of `ehr`'s access log `query` selects, newest first, for
    /// `reader`; the person reads no entry about data held from them.
    ///
    /// # Errors
    /// - `not_found` (`404`): no EHR has the id.
    /// - `conflict` (`409`): the local audit store, which holds the access
    ///   log, is off.
    /// - `exception`: the store failed.
    pub async fn ehr_access_log(
        &self,
        ehr: EhrId,
        query: &LogQuery,
        reader: LogReader,
    ) -> Result<Vec<AccessLogRecord>, ServiceError> {
        let log = self.access_log_store()?;
        self.require_logged_ehr(ehr).await?;
        let query = LogQuery {
            ehr_id: Some(ehr.to_string()),
            ..query.clone()
        };
        let records = log.search(&query).await.map_err(log_failure)?;
        if reader == LogReader::Reviewer {
            return Ok(records);
        }
        let now = Timestamp::now();
        let Some(holds) = self.person_holds(&[ehr.to_string()], now).await? else {
            return Ok(records);
        };
        Ok(records
            .into_iter()
            .filter(|record| match record {
                AccessLogRecord::AccessLogEntry(entry) => !withheld(entry, &holds, now),
                AccessLogRecord::AccessLogTombstone(_) => true,
            })
            .collect())
    }

    /// The entry `uid` of `ehr`'s access log, for `reader`; the person reads
    /// no entry about data held from them.
    ///
    /// # Errors
    /// - `not_found` (`404`): no EHR has the id, or its log holds no entry
    ///   with the uid that `reader` may read.
    /// - `conflict` (`409`): the local audit store is off.
    /// - `exception`: the store failed.
    pub async fn ehr_access_log_entry(
        &self,
        ehr: EhrId,
        uid: &str,
        reader: LogReader,
    ) -> Result<AccessLogEntry, ServiceError> {
        let log = self.access_log_store()?;
        self.require_logged_ehr(ehr).await?;
        let unknown = || {
            ServiceError::not_found(
                Missing::VersionedObject,
                format!("the access log of the EHR {ehr} holds no entry {uid}"),
            )
        };
        let Some(AccessLogRecord::AccessLogEntry(entry)) =
            log.read(uid).await.map_err(log_failure)?
        else {
            return Err(unknown());
        };
        let ehr_id = ehr.to_string();
        if entry.ehr_id.as_ref().map(HierObjectId::value) != Some(ehr_id.as_str()) {
            return Err(unknown());
        }
        if reader == LogReader::Reviewer {
            return Ok(entry);
        }
        let now = Timestamp::now();
        match self.person_holds(&[ehr_id], now).await? {
            Some(holds) if withheld(&entry, &holds, now) => Err(unknown()),
            _ => Ok(entry),
        }
    }

    /// The records of the whole access log `query` selects, newest first,
    /// for review.
    ///
    /// # Errors
    /// - `conflict` (`409`): the local audit store is off.
    /// - `exception`: the store failed.
    pub async fn access_log_search(
        &self,
        query: &LogQuery,
    ) -> Result<Vec<AccessLogRecord>, ServiceError> {
        let log = self.access_log_store()?;
        log.search(query).await.map_err(log_failure)
    }

    /// The uid and link of every access-log record from `from` to `to`, both
    /// included, for a verifier.
    ///
    /// # Errors
    /// - `conflict` (`409`): the local audit store is off.
    /// - `precondition` (`400`): a position between holds a record of the
    ///   `audit_event` form, which only this database verifies.
    /// - `exception`: the store failed.
    pub async fn access_log_chain(
        &self,
        from: i64,
        to: i64,
    ) -> Result<Vec<(String, AccessLogLink)>, ServiceError> {
        let log = self.access_log_store()?;
        log.chain(from, to).await.map_err(log_failure)
    }

    /// `Ok` when an EHR has the id `ehr`.
    ///
    /// # Errors
    /// `not_found` when none has, and the store errors.
    async fn require_logged_ehr(&self, ehr: EhrId) -> Result<(), ServiceError> {
        let exists =
            crate::storage::ehr_repo::ehr_exists(&mut *acquire(&self.pool).await?, ehr).await?;
        if exists {
            return Ok(());
        }
        Err(ServiceError::not_found(
            Missing::Ehr,
            format!("no EHR has the id {ehr}"),
        ))
    }

    /// The access-log store the server appends through, or one over the
    /// audit store's pool.
    ///
    /// # Errors
    /// `conflict` when the local audit store, which holds the access log, is
    /// off.
    pub(in crate::service) fn access_log_store(&self) -> Result<AccessLogStore, ServiceError> {
        let Some(audit_store) = &self.audit_store else {
            return Err(ServiceError::conflict(
                "the local audit store is off, so this server keeps no access log",
            ));
        };
        Ok(self
            .audit
            .as_ref()
            .and_then(AuditSender::access_log)
            .unwrap_or_else(|| AccessLogStore::new(audit_store.pool.clone())))
    }
}

/// The service error of a failed access-log read.
pub(in crate::service) fn log_failure(error: AccessLogError) -> ServiceError {
    match error {
        AccessLogError::Interleaved { .. } => ServiceError::precondition(error.to_string()),
        other => ServiceError::internal("the access-log store", other),
    }
}
