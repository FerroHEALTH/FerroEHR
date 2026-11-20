// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Which care relationships a read may reveal, decided per subject (#6076).
//!
//! The reads of one relationship and its versions are here too.
//!
//! Draft access control package, ITS-REST Resources, Care Relationships:
//! "Every read of these resources is decided per subject it reveals: a
//! relationship of a subject whose record the caller may not read is left
//! out of a list, and its role and context are returned only to a caller
//! that may read the record or to the writer for its own provider." A
//! subject's record is each EHR whose subject reference names its `PERSON`,
//! judged by the access evaluator as a read of that EHR; a subject with no
//! EHR has no record a caller may read. The writer for its own provider is
//! an administrator or a caller holding `write_care_relationship` for the
//! relationship's provider. A relationship left out is never named: a list
//! omits it and a read of it answers `404`. No openEHR spec governs this:
//! our own design over the draft package.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the relationship body a read serves is the \
              canonical fragment the version read produced"
)]

use std::collections::{BTreeMap, BTreeSet};

use openehr_rm::draft::access_control::terminology::{CareRelationship, Term};
use serde_json::Value;
use uuid::Uuid;

use crate::access_eval::condition::current;
use crate::access_eval::procedure::Outcome;
use crate::access_eval::scope::Item;
use crate::error::ServiceError;
use crate::ids::{EhrId, VoId};
use crate::party_binding::derive::container;
use crate::party_binding::grants::caller::care_of::{self, ProviderKey};
use crate::party_binding::grants::caller::subject::declared_by;
use crate::party_binding::grants::capabilities::Capabilities;
use crate::party_binding::grants::operation::{Operation, PROVIDER};
use crate::party_binding::policy_mark::carries;
use crate::service::FerroEhrService;
use crate::service::demographic::care_relationship::{CareRead, Revealed, not_care};
use crate::service::response::{RawServiceResponse, ReadBody, ServiceResponse};
use crate::storage::access_projection::EhrSubject;
use crate::versioning::object_version_id::{TreeId, UidAddress, parse_uid_based_id};
use crate::versioning::tree::Tree;

/// One care relationship a read may reveal: its versioned object, its
/// subject's `PERSON` and the provider in whose care it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listed {
    /// The relationship's version container.
    pub(crate) holder: Uuid,
    /// The subject's `PERSON`.
    pub(crate) person: Uuid,
    /// The provider in whose care it is.
    pub(crate) provider: ProviderKey,
}

/// The EHRs of each subject, by its `PERSON`; a subject with none is absent.
pub(crate) type Records = BTreeMap<Uuid, Vec<EhrId>>;

impl FerroEhrService {
    /// Of `rows`, those the running request may read, in their order, and the
    /// EHRs of each of their subjects.
    ///
    /// # Errors
    /// `503` when a store cannot be read, and the fail-closed fault of EHR
    /// settings that do not read.
    pub(super) async fn care_visible(
        &self,
        rows: Vec<Listed>,
    ) -> Result<(Vec<Listed>, Records), ServiceError> {
        let mut persons: Vec<Uuid> = rows.iter().map(|row| row.person).collect();
        persons.sort_unstable();
        persons.dedup();
        let records = self.subject_records(&persons).await?;
        let readable = self.readable_subjects(&persons, &records).await?;
        let visible = rows
            .into_iter()
            .filter(|row| readable.contains(&row.person) || self.writes_care_for(&row.provider))
            .collect();
        Ok((visible, records))
    }

    /// The EHRs whose subject reference names each of `persons`: its own
    /// reference in this CDR's namespace, in either case, and each record
    /// subject it declares, read on the party pool and then on the clinical
    /// pool, never both at once.
    async fn subject_records(&self, persons: &[Uuid]) -> Result<Records, ServiceError> {
        let declared = {
            let mut conn = crate::db::acquire::acquire(&self.demographic_pool).await?;
            declared_by(&mut conn, persons).await?
        };
        let namespace = self.grants.namespace();
        let mut owners: BTreeMap<EhrSubject, Uuid> = BTreeMap::new();
        for person in persons {
            let id = person.to_string();
            owners.insert(EhrSubject::new(&*namespace, id.to_uppercase()), *person);
            owners.insert(EhrSubject::new(&*namespace, id), *person);
        }
        owners.extend(
            declared
                .into_iter()
                .map(|(person, subject)| (subject, person)),
        );
        let references: Vec<EhrSubject> = owners.keys().cloned().collect();
        let mut records = Records::new();
        for (subject, ehr) in
            crate::storage::subject_ehrs::ehrs_of_subjects(&self.pool, &references).await?
        {
            if let Some(person) = owners.get(&subject) {
                records.entry(*person).or_default().push(ehr);
            }
        }
        for ehrs in records.values_mut() {
            ehrs.sort_unstable();
            ehrs.dedup();
        }
        Ok(records)
    }

    /// The subjects of `persons` whose record the running request may read:
    /// each of their EHRs is admitted, as a read of that EHR is judged; every
    /// subject outside a request.
    async fn readable_subjects(
        &self,
        persons: &[Uuid],
        records: &Records,
    ) -> Result<BTreeSet<Uuid>, ServiceError> {
        let Some(access) = current() else {
            return Ok(persons.iter().copied().collect());
        };
        let mut readable = BTreeSet::new();
        for person in persons {
            let Some(ehrs) = records.get(person).filter(|ehrs| !ehrs.is_empty()) else {
                continue;
            };
            let mut admitted = true;
            for ehr in ehrs {
                let verdict = self.ehr_verdict(&access, *ehr).await?;
                admitted &= matches!(verdict.outcome(&Item::Ehr), Outcome::Admit);
            }
            if admitted {
                readable.insert(*person);
            }
        }
        Ok(readable)
    }

    /// Whether the running request writes care relationships for `provider`:
    /// an administrator, or a `write_care_relationship` grant naming it.
    fn writes_care_for(&self, provider: &ProviderKey) -> bool {
        crate::party_binding::protect::administrator(&self.binding)
            || Capabilities::current()
                .of(Operation::WriteCareRelationship)
                .any(|grant| grant.credentials().identifiers(PROVIDER).contains(provider))
    }

    /// The care relationship `address` names, its current version or the
    /// one it names, when the running request may read it; a body of `null`
    /// for a deleted one.
    ///
    /// # Errors
    /// `404` when it names no care relationship or one the request may not
    /// read; the refusals of [`Self::party_relationship_get_raw`].
    pub async fn care_relationship_get(
        &self,
        address: UidAddress,
    ) -> Result<CareRead<RawServiceResponse>, ServiceError> {
        let vo_id = address.vo_id;
        let read = self.party_relationship_get_raw(address, None).await?;
        let listed = match &read.body {
            ReadBody::Value(Value::Null) => self.care_of_history(vo_id).await?,
            body => care_of_body(
                vo_id,
                &body.clone().into_value().map_err(|error| {
                    ServiceError::exception(format!("the stored relationship {vo_id}: {error}"))
                })?,
            )?,
        };
        Ok(CareRead {
            body: read,
            revealed: self.care_admitted(listed).await?,
        })
    }

    /// The `VERSIONED_OBJECT` of the care relationship `vo_id`, when the
    /// running request may read it.
    ///
    /// # Errors
    /// As [`Self::care_relationship_get`].
    pub async fn versioned_care_relationship_get(
        &self,
        vo_id: VoId,
    ) -> Result<CareRead<ServiceResponse>, ServiceError> {
        let revealed = self.care_subject_admitted(vo_id).await?;
        Ok(CareRead {
            body: self.versioned_party_relationship_get(vo_id).await?,
            revealed,
        })
    }

    /// The `REVISION_HISTORY` of the care relationship `vo_id`, when the
    /// running request may read it.
    ///
    /// # Errors
    /// As [`Self::care_relationship_get`].
    pub async fn care_relationship_revision_history(
        &self,
        vo_id: VoId,
    ) -> Result<CareRead<ServiceResponse>, ServiceError> {
        let revealed = self.care_subject_admitted(vo_id).await?;
        Ok(CareRead {
            body: self.party_relationship_revision_history(vo_id).await?,
            revealed,
        })
    }

    /// The `ORIGINAL_VERSION` `version` of the care relationship `vo_id`,
    /// when the running request may read it.
    ///
    /// # Errors
    /// As [`Self::care_relationship_get`].
    pub async fn care_relationship_version_get_by_id(
        &self,
        vo_id: VoId,
        version: TreeId,
    ) -> Result<CareRead<ServiceResponse>, ServiceError> {
        let read = self
            .party_relationship_version_get_by_id(vo_id, version)
            .await?;
        let listed = match read.body.get("data").filter(|data| !data.is_null()) {
            Some(data) => care_of_body(vo_id, data)?,
            None => self.care_of_history(vo_id).await?,
        };
        Ok(CareRead {
            body: read,
            revealed: self.care_admitted(listed).await?,
        })
    }

    /// What the request may reveal of the care relationship `vo_id`, decided
    /// on its head's fact or, for a deleted one, on its last version that
    /// holds data.
    async fn care_subject_admitted(&self, vo_id: VoId) -> Result<Revealed, ServiceError> {
        let listed = match self.care_held(vo_id).await {
            Ok((person, provider)) => Listed {
                holder: vo_id.0,
                person,
                provider,
            },
            Err(ServiceError::NotFound(_)) => self.care_of_history(vo_id).await?,
            Err(other) => return Err(other),
        };
        self.care_admitted(listed).await
    }

    /// The subjects `listed` reveals, or the `404` of a relationship the
    /// running request may not read.
    async fn care_admitted(&self, listed: Listed) -> Result<Revealed, ServiceError> {
        let holder = VoId(listed.holder);
        let (visible, records) = self.care_visible(vec![listed]).await?;
        if visible.is_empty() {
            return Err(not_care(holder));
        }
        Ok(reveal(&visible, &records))
    }

    /// The care relationship the last version of `vo_id` that holds data
    /// states, read from its revision history newest first.
    async fn care_of_history(&self, vo_id: VoId) -> Result<Listed, ServiceError> {
        let history = self.relationship_revision_history(vo_id).await?;
        let items = history
            .get("items")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for item in items.iter().rev() {
            let Some(uid) = item.pointer("/version_id/value").and_then(Value::as_str) else {
                continue;
            };
            let Some(tree) = parse_uid_based_id(uid)?.tree() else {
                continue;
            };
            let version = self.relationship_version(vo_id, tree).await?;
            if let Some(data) = version.get("data").filter(|data| !data.is_null()) {
                return care_of_body(vo_id, data);
            }
        }
        Err(not_care(vo_id))
    }
}

/// Each subject `visible` reveals, by the EHRs `records` holds for it, with
/// how many of its relationships were revealed, or by its `PERSON` when it
/// has none.
pub(crate) fn reveal(visible: &[Listed], records: &Records) -> Revealed {
    let mut counts: BTreeMap<Uuid, u64> = BTreeMap::new();
    for row in visible {
        let count = counts.entry(row.person).or_insert(0);
        *count = count.saturating_add(1);
    }
    let mut revealed = Revealed::default();
    for (person, count) in counts {
        match records.get(&person).filter(|ehrs| !ehrs.is_empty()) {
            Some(ehrs) => revealed.ehrs.extend(ehrs.iter().map(|ehr| (*ehr, count))),
            None => revealed.persons.push(person),
        }
    }
    revealed
}

/// The care relationship the stored body `data` of `vo_id` states.
///
/// # Errors
/// The `404` of a body that is no care relationship.
fn care_of_body(vo_id: VoId, data: &Value) -> Result<Listed, ServiceError> {
    let care = CareRelationship::UnderCareOf;
    if !carries(data, care.terminology(), care.code()) {
        return Err(not_care(vo_id));
    }
    let person = data
        .pointer_text("/source/id/value")
        .and_then(|id| container(&id));
    match (person, care_of::provider(data)) {
        (Some(person), Ok(provider)) => Ok(Listed {
            holder: vo_id.0,
            person,
            provider,
        }),
        _ => Err(not_care(vo_id)),
    }
}
