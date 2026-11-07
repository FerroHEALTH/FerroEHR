// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`LocalPartySource`]: the `local` party source of `ferrohealth-smart`, read
//! from the binding side table on the party pool, on every request, with no
//! cache.
//!
//! It reports every `PERSON` whose live key row carries the query's issuer,
//! type and value, with the state of its head, its markers (`professional`
//! for a professional `ROLE` valid today, `representative` for a recorded
//! representation relationship), the groups it belongs to under the query's
//! group issuer, and the subjects of its live representation relationships.
//! A key that only a deleted `PERSON` carried is [`Lookup::Deprovisioned`].
//! The read runs on the party pool and its database role, so the clinical
//! side gains no read of the party schema. No openEHR spec governs resolving
//! a login to a party: our own design.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use ferrohealth_smart::party::source::{Lookup, PartyRecord, PartySource, Query, SourceError};
use ferrohealth_smart::party::{Markers, Membership, PartyRef, PartyState, Represented};
use sqlx::PgPool;
use uuid::Uuid;

use crate::party_binding::lookup::{self, KeyRow};
use crate::storage::error::StorageError;

/// The RM class of a bound party.
const PERSON: &str = "PERSON";
/// The RM class of a group a party belongs to.
const GROUP: &str = "GROUP";

/// The party source over the binding side table.
#[derive(Debug, Clone)]
pub struct LocalPartySource {
    /// The party pool, whose role is the only one that reads the side table.
    pool: PgPool,
    /// The `PARTY_REF.namespace` a reported party is named under.
    namespace: String,
}

impl LocalPartySource {
    /// Returns the source reading the side table on the party pool `pool`,
    /// naming each party under `namespace`.
    #[must_use]
    pub fn new(pool: PgPool, namespace: impl Into<String>) -> Self {
        Self {
            pool,
            namespace: namespace.into(),
        }
    }

    /// Returns what the side table holds for the key `value` of
    /// `identifier_type` under `issuer`, with the memberships under
    /// `group_issuer`, on `today` (`YYYY-MM-DD`), the date a `time_validity`
    /// is judged on.
    ///
    /// [`PartySource::lookup`] answers its [`Query`] through it on today's
    /// date.
    ///
    /// # Errors
    /// [`StorageError`] when the party pool cannot answer; never an empty
    /// lookup.
    pub async fn read(
        &self,
        issuer: &str,
        identifier_type: &str,
        value: &str,
        group_issuer: Option<&str>,
        today: &str,
    ) -> Result<Lookup, StorageError> {
        let mut conn = self.pool.acquire().await?;
        let rows = lookup::key_rows(&mut conn, issuer, identifier_type, value, today).await?;
        let (live, tombstones): (Vec<KeyRow>, Vec<KeyRow>) =
            rows.into_iter().partition(|row| row.live);
        let live: Vec<KeyRow> = live
            .into_iter()
            .filter(|row| row.professional || row.representative)
            .collect();
        if live.is_empty() {
            return Ok(if tombstones.is_empty() {
                Lookup::Parties(Vec::new())
            } else {
                Lookup::Deprovisioned
            });
        }
        let ids: Vec<Uuid> = live.iter().map(|row| row.party_id).collect();
        let mut groups: BTreeMap<Uuid, Vec<Membership>> = BTreeMap::new();
        if let Some(issuer) = group_issuer {
            for row in lookup::memberships(&mut conn, &ids, issuer).await? {
                groups.entry(row.member).or_default().push(Membership::new(
                    self.party(GROUP, row.group),
                    row.external_id,
                ));
            }
        }
        let representatives: Vec<Uuid> = live
            .iter()
            .filter(|row| row.representative)
            .map(|row| row.party_id)
            .collect();
        let mut subjects: BTreeMap<Uuid, Vec<Represented>> = BTreeMap::new();
        if !representatives.is_empty() {
            for row in lookup::representations(&mut conn, &representatives, today).await? {
                let subject = Represented::new(self.party(PERSON, row.subject_id));
                subjects
                    .entry(row.representative_id)
                    .or_default()
                    .push(match row.relationship {
                        Some(code) => subject.with_relationship(code),
                        None => subject,
                    });
            }
        }
        let records = live
            .into_iter()
            .map(|row| {
                let markers = Markers {
                    professional: row.professional,
                    representative: row.representative,
                };
                PartyRecord::new(
                    self.party(PERSON, row.party_id),
                    state(&row.lifecycle_state),
                    markers,
                )
                .with_memberships(groups.remove(&row.party_id).unwrap_or_default())
                .with_represents(subjects.remove(&row.party_id).unwrap_or_default())
            })
            .collect();
        Ok(Lookup::Parties(records))
    }

    /// The reference to the party `id` of `class`.
    fn party(&self, class: &str, id: Uuid) -> PartyRef {
        PartyRef::new(self.namespace.clone(), class, id.to_string())
    }
}

/// The state a head's `version_lifecycle_state` code reports (RM common
/// `master06-change_control_package.adoc` §Version Lifecycle, §Abandoned and
/// Inactive States).
fn state(code: &str) -> PartyState {
    match code {
        "553" => PartyState::Incomplete,
        "800" => PartyState::Inactive,
        "801" => PartyState::Abandoned,
        // NOTE: ck_binding_identifier_lifecycle_state admits only 532, 553, 800
        // and 801, so every other code is `532|complete|`.
        _ => PartyState::Active,
    }
}

/// Today's date in UTC, the date a `time_validity` is judged on.
///
/// `DV_DATE` carries no zone (RM `data_types` `DV_DATE`), so which zone a
/// date is read in is our own design: the server reads every date in UTC.
fn today() -> String {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
        .to_string()
}

impl PartySource for LocalPartySource {
    fn lookup<'a>(
        &'a self,
        query: &'a Query<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<Lookup, SourceError>> + Send + 'a>> {
        Box::pin(async move {
            self.read(
                query.identifier_issuer,
                query.identifier_type,
                query.value,
                query.group_identifier_issuer,
                &today(),
            )
            .await
            .map_err(SourceError::new)
        })
    }
}
