// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access gates a query of each domain takes from the access evaluator.
//!
//! The access-control decision procedure decides "every access to an EHR"
//! (the FerroEHR access control draft, its decision procedure §Overview), so
//! a clinical query carries the evaluator's condition and the `EHR_ACCESS`
//! readers, and a demographic query, whose parties belong to no EHR, carries
//! neither and reads each party under the Demographic API's rules
//! (`crate::aql::sql`'s gates, #6087). Our own design.

use std::borrow::Cow;

use crate::access_eval::condition::AccessCondition;
use crate::aql::ir::QueryDomain;
use crate::aql::sql::EhrAccessReaders;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::query::request::{AqlQueryRequest, QueryAccess};

impl FerroEhrService {
    /// The access evaluator's condition a query of `domain` applies: the
    /// request's, resolved for the EHRs `pinned` names, on a clinical query;
    /// none on a demographic one, which never judges the request's (#6110).
    ///
    /// # Errors
    /// As [`Self::unresolved_condition`] and [`Self::query_access_condition`].
    pub(super) async fn domain_condition(
        &self,
        domain: QueryDomain,
        request: &AqlQueryRequest,
        pinned: Option<&[EhrId]>,
    ) -> Result<AccessCondition, ServiceError> {
        match domain {
            QueryDomain::Clinical => {
                let unresolved = Box::pin(self.unresolved_condition(&request.ehr_access)).await?;
                Box::pin(self.query_access_condition(&unresolved, pinned)).await
            }
            QueryDomain::Demographic => Ok(AccessCondition::default()),
        }
    }

    /// The condition `access` names before its care is resolved: the running
    /// request's, judged now, or the given one.
    ///
    /// # Errors
    /// As [`Self::request_access_condition`].
    pub(super) async fn unresolved_condition<'a>(
        &self,
        access: &'a QueryAccess,
    ) -> Result<Cow<'a, AccessCondition>, ServiceError> {
        match access {
            QueryAccess::OfRequest => {
                Ok(Cow::Owned(Box::pin(self.request_access_condition()).await?))
            }
            QueryAccess::Given(given) => Ok(Cow::Borrowed(given)),
        }
    }

    /// The EHRs whose `EHR_ACCESS` objects a query of `domain` reads rows
    /// of: those `condition` leaves the caller on a clinical query, and
    /// every EHR's on a demographic one, which reads no `EHR_ACCESS`.
    ///
    /// # Errors
    /// As [`Self::query_access_readers`].
    pub(super) async fn domain_readers(
        &self,
        domain: QueryDomain,
        condition: &AccessCondition,
    ) -> Result<EhrAccessReaders, ServiceError> {
        match domain {
            QueryDomain::Clinical => Box::pin(self.query_access_readers(condition)).await,
            QueryDomain::Demographic => Ok(EhrAccessReaders::Every),
        }
    }
}
