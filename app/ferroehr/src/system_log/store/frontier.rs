// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One frontier round of the access log (#6242).
//!
//! The round moves the frontier ([`crate::access_log::frontier`]), owes the
//! entries below it to the sinks that deliver from the store, and, when due,
//! seals them in windows ([`crate::access_log::seal`]). No specification
//! governs it: our own design.

use crate::access_log::frontier::Frontier;
use crate::access_log::seal::SealRound;
use crate::db::acquire::acquire;
use crate::system_log::AuditError;

use super::AuditStore;

/// The sinks that deliver from the store, owed each record.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Owed {
    /// The ITI-20 ATX: FHIR Feed.
    pub fhir_feed: bool,
    /// The ITI-20 syslog route.
    pub syslog: bool,
}

impl AuditStore {
    /// Move `frontier`, owe the entries below it to the configured sinks,
    /// and seal them when `seal`, in as many windows as they fill; returns how many entries were owed and
    /// whether a window was sealed.
    ///
    /// # Errors
    /// [`AuditError::Store`] when a statement fails, and
    /// [`AuditError::AccessLog`] when the seal fails.
    pub async fn frontier_round(
        &self,
        frontier: &mut Frontier,
        seal: bool,
    ) -> Result<(u64, bool), AuditError> {
        let reached = {
            let mut conn = acquire(&self.pool).await?;
            frontier.round(&mut conn).await?
        };
        let owed = if (self.owed.fhir_feed || self.owed.syslog) && frontier.owe_due() {
            let (owed, through): (i64, i64) = sqlx::query_as(
                "SELECT owed_count, owed_through FROM audit.access_log_owe($1, $2, $3)",
            )
            .bind(reached)
            .bind(self.owed.fhir_feed)
            .bind(self.owed.syslog)
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
            // NOTE: our own design (#6242), entries a write holds stay owed until released.
            if through >= reached {
                frontier.owed();
            }
            u64::try_from(owed).unwrap_or(0)
        } else {
            0
        };
        let mut sealed = false;
        // NOTE: our own design (#6242), a window holds a bounded span of ids, so a round seals
        // window after window until the frontier is reached and no backlog outgrows the rate.
        if seal && frontier.seal_due() {
            loop {
                match self.chaining_store().seal_to(reached).await? {
                    SealRound::Sealed { entries, .. } => {
                        sealed = true;
                        crate::telemetry::access_path::metrics().log_chained.add(
                            u64::try_from(entries).unwrap_or(u64::MAX),
                            &[opentelemetry::KeyValue::new("chain", "access_log")],
                        );
                    }
                    SealRound::Nothing { through } => {
                        // NOTE: our own design (#6242), entries a write holds stay due until released.
                        if through >= reached {
                            frontier.sealed();
                        }
                        break;
                    }
                    SealRound::Busy => break,
                }
            }
        }
        Ok((owed, sealed))
    }
}

impl AuditStore {
    /// This store's gathered write waiting up to `window` after the first
    /// entry for more (`[audit.store] gather_window`); set before the first
    /// write.
    #[must_use]
    #[cfg_attr(
        not(feature = "fhir"),
        expect(
            unused_mut,
            unused_variables,
            reason = "a slim build writes no entry through the gathered writer"
        )
    )]
    pub fn with_gather_window(mut self, window: std::time::Duration) -> Self {
        #[cfg(feature = "fhir")]
        {
            self.group = std::sync::Arc::new(crate::access_log::intake::Gathered::new(window));
        }
        self
    }
}
