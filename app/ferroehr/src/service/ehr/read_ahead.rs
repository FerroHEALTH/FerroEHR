// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHR read made before the request's access decision is final (#5959).
//!
//! The REST layer starts the read while the access middleware decides, and
//! hands it to [`crate::service::FerroEhrService::ehr_read_on`] only once the
//! decision admits the request. No openEHR spec governs this: our own design.

use crate::error::ServiceError;
use crate::storage::ehr_repo::EhrSummaryRead;

/// The one statement of a `GET /ehr/{ehr_id}` read, read ahead of its
/// decision. Nothing of it leaves the service until
/// [`crate::service::FerroEhrService::ehr_read_on`] serves it.
pub struct EhrAhead(pub(super) Result<Option<EhrSummaryRead>, ServiceError>);

impl std::fmt::Debug for EhrAhead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // NOTE: our own design (#5959), the read EHR is clinical content and never formatted.
        let state = match &self.0 {
            Ok(Some(_)) => "held",
            Ok(None) => "unheld",
            Err(_) => "unread",
        };
        f.debug_tuple("EhrAhead").field(&state).finish()
    }
}
