// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care relationship resources (#6076).
//!
//! They are the care relationships of a subject, addressed by its subject
//! reference or by its `PERSON`, one relationship with its versions, and the
//! reverse reading by carer (draft access control package, ITS-REST
//! Resources, Care Relationships).
//!
//! A relationship is the standalone `under_care_of` `PARTY_RELATIONSHIP`
//! (#6055), so every write here is the commit of
//! `/demographic/party_relationship` in a narrower form, versioned with its
//! `AUDIT_DETAILS` in a `CONTRIBUTION`. Every read is decided per subject it
//! reveals ([`super::care_visible`]): that a person is under a carer is data
//! concerning health (GDPR Art 4(15), Art 9). No openEHR spec defines these
//! resources: our own design over the draft package.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the served relationship bodies are the \
              canonical fragments the version reads produce"
)]

use openehr_base::validate::InvariantViolation;
use openehr_rm::draft::access_control::terminology::{CareRelationship, ForeignCode, Term};
use openehr_rm::prelude::PartyRelationship;
use serde_json::Value;
use uuid::Uuid;

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};
use crate::party_binding::derive::container;
use crate::party_binding::grants::caller::care_of::{CareOf, ProviderKey};
use crate::party_binding::grants::caller::store::{self, FactRow};
use crate::party_binding::grants::caller::{CareTeam, FactKind, FactValue};
use crate::party_binding::policy_mark::carries;
use crate::service::FerroEhrService;
use crate::service::demographic::care_visible::{Listed, reveal};
use crate::service::demographic::precondition::Precondition;
use crate::service::demographic::support::version_response;
use crate::service::response::ServiceResponse;
use crate::service::version_update::WriteRequest;
use crate::storage::access_projection::EhrSubject;
use crate::versioning::tree::{Tree, Typed};

/// Which care relationships of a subject a list holds: those live on `at`
/// (today in UTC when `None`), with the policy's grace after each end, or
/// every one when `all`.
#[derive(Debug, Clone, Copy, Default)]
pub struct CareSpan {
    /// The day the relationships are live on; today when `None`.
    pub at: Option<jiff::civil::Date>,
    /// Whether ended relationships are listed too.
    pub all: bool,
}

/// How a care relationship reaches a carer (draft access control decision
/// procedure, Holding a Care Relationship).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CarePath {
    /// The relationship names the carer's `PERSON` or a `ROLE` it performs.
    Direct,
    /// The relationship names a care team or unit the carer is a member of.
    Team,
}

/// The reverse reading: the relationships that reach `carer`, in the role
/// `role` and by the path `path` when given, live on `at`, the page from
/// `offset` of at most `fetch`.
#[derive(Debug, Clone)]
pub struct CarerQuery {
    /// The carer's party.
    pub carer: Uuid,
    /// The care role the relationships are in; any when `None`.
    pub role: Option<ForeignCode>,
    /// The path they reach the carer by; either when `None`.
    pub path: Option<CarePath>,
    /// The day they are live on; today when `None`.
    pub at: Option<jiff::civil::Date>,
    /// The first relationship of the page.
    pub offset: usize,
    /// The largest page.
    pub fetch: usize,
}

/// Each subject a read revealed, the access-log entries it writes: the EHRs
/// of a subject that has a record, with how many relationships were revealed
/// of it, and the `PERSON` of a subject that has none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Revealed {
    /// The EHRs of the revealed subjects, with the relationships revealed.
    pub ehrs: Vec<(EhrId, u64)>,
    /// The revealed subjects that have no EHR here, by their `PERSON`.
    pub persons: Vec<Uuid>,
}

/// What a read of care relationships answers: the served `body` and the
/// subjects it revealed.
#[derive(Debug, Clone)]
pub struct CareRead<B> {
    /// What the read serves.
    pub body: B,
    /// Each subject it revealed.
    pub revealed: Revealed,
}

impl FerroEhrService {
    /// The `PERSON` the subject reference `subject` names, as the access
    /// evaluator finds a subject's party; `None` when it names none.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when the party store cannot be read.
    pub async fn care_subject_person(
        &self,
        subject: &EhrSubject,
    ) -> Result<Option<Uuid>, ServiceError> {
        Ok(self.grants.person_of(subject).await?)
    }

    /// The care relationships of the subject `person` that `span` holds and
    /// the running request may read, with each subject they reveal.
    ///
    /// # Errors
    /// `service_overloaded` (`503`) when a store cannot be read, and the
    /// fail-closed fault of settings that do not read.
    pub async fn care_relationships_of(
        &self,
        person: Uuid,
        span: CareSpan,
    ) -> Result<CareRead<Vec<Value>>, ServiceError> {
        let rows = {
            let mut conn = crate::db::acquire::acquire(&self.demographic_pool).await?;
            store::care_of_subject(&mut conn, person).await?
        };
        let (day, grace) = self.care_day(span.at);
        let listed: Vec<Listed> = rows
            .iter()
            .filter(|row| span.all || live(row, day, grace))
            .filter_map(|row| Listed::of(person, row))
            .collect();
        let (visible, records) = self.care_visible(listed).await?;
        Ok(CareRead {
            body: self.care_bodies(&visible).await?,
            revealed: reveal(&visible, &records),
        })
    }

    /// The care relationships that reach the carer `query` names, one page
    /// of those the running request may read, with each subject they reveal.
    ///
    /// The carer's principals are read from the caller facts copy once it
    /// holds the store's revisions, so the page agrees with the decision.
    ///
    /// # Errors
    /// As [`Self::care_relationships_of`].
    pub async fn care_relationships_reaching(
        &self,
        query: &CarerQuery,
    ) -> Result<CareRead<Vec<Value>>, ServiceError> {
        self.grants.refresh().await?;
        let facts = self.grants.facts_of(query.carer, jiff::Timestamp::now());
        let direct: Vec<Uuid> = std::iter::once(query.carer)
            .chain(facts.performed().iter().copied())
            .collect();
        let teams: Vec<Uuid> = facts.care_teams().iter().map(CareTeam::team).collect();
        let principals: Vec<Uuid> = match query.path {
            Some(CarePath::Direct) => direct.clone(),
            Some(CarePath::Team) => teams.clone(),
            None => direct.iter().chain(&teams).copied().collect(),
        };
        let subjects = {
            let mut conn = crate::db::acquire::acquire(&self.demographic_pool).await?;
            store::care_of_carers(&mut conn, &principals).await?
        };
        let (day, grace) = self.care_day(query.at);
        let listed: Vec<Listed> = subjects
            .iter()
            .flat_map(|(person, rows)| rows.iter().map(move |row| (*person, row)))
            .filter(|(_, row)| live(row, day, grace) && in_role(row, query.role.as_ref()))
            .filter_map(|(person, row)| Listed::of(person, row))
            .collect();
        let (visible, records) = self.care_visible(listed).await?;
        let page: Vec<Listed> = visible
            .into_iter()
            .skip(query.offset)
            .take(query.fetch)
            .collect();
        Ok(CareRead {
            body: self.care_bodies(&page).await?,
            revealed: reveal(&page, &records),
        })
    }

    /// Create a care relationship of the subject `person`: the commit of
    /// [`Self::party_relationship_create`], once the body is an
    /// `under_care_of` relationship whose `source` is `person`.
    ///
    /// # Errors
    /// [`ServiceError::ValidationFailed`] (`422`) naming the element when
    /// the body is no care relationship of `person`; the commit's refusals.
    pub async fn care_relationship_create(
        &self,
        person: Uuid,
        request: WriteRequest<PartyRelationship>,
    ) -> Result<ServiceResponse, ServiceError> {
        care_body_of(person, &request.data)?;
        self.party_relationship_create(request).await
    }

    /// Change the care relationship `vo_id` under `precondition`: the commit
    /// of [`Self::party_relationship_update`], once its head is a care
    /// relationship and the body one of the same subject.
    ///
    /// # Errors
    /// `404` when `vo_id` holds no care relationship; as
    /// [`Self::care_relationship_create`].
    pub async fn care_relationship_update(
        &self,
        vo_id: VoId,
        precondition: Precondition,
        request: WriteRequest<PartyRelationship>,
    ) -> Result<ServiceResponse, ServiceError> {
        let (person, _) = self.care_held(vo_id).await?;
        care_body_of(person, &request.data)?;
        self.party_relationship_update(vo_id, precondition, request)
            .await
    }

    /// Remove the care relationship `vo_id` written in error under
    /// `precondition`: the logical deletion of
    /// [`Self::party_relationship_delete`]; its earlier versions keep it.
    ///
    /// # Errors
    /// `404` when `vo_id` holds no care relationship; the deletion's
    /// refusals.
    pub async fn care_relationship_delete(
        &self,
        vo_id: VoId,
        precondition: Precondition,
        request: WriteRequest,
    ) -> Result<ServiceResponse, ServiceError> {
        self.care_held(vo_id).await?;
        self.party_relationship_delete(vo_id, precondition, request)
            .await
    }

    /// The subject and the provider of the care relationship the head of
    /// `vo_id` holds, read in one indexed statement.
    ///
    /// # Errors
    /// `404` when the head holds none; `503` when the store cannot be read.
    pub(super) async fn care_held(&self, vo_id: VoId) -> Result<(Uuid, ProviderKey), ServiceError> {
        let held = {
            let mut conn = crate::db::acquire::acquire(&self.demographic_pool).await?;
            store::care_of_holder(&mut conn, vo_id.0).await?
        };
        held.and_then(|(person, row)| Listed::of(person, &row))
            .map(|listed| (listed.person, listed.provider))
            .ok_or_else(|| not_care(vo_id))
    }

    /// The day relationships are judged live on, `at` or today in UTC, as
    /// the decision judges them, and the grace the policy in force adds.
    fn care_day(&self, at: Option<jiff::civil::Date>) -> (jiff::civil::Date, jiff::Span) {
        let in_force = self.access_policy().in_force();
        let grace = crate::access_eval::procedure::Policy::of(in_force.policy.as_deref()).grace;
        let day = at.unwrap_or_else(|| {
            jiff::Timestamp::now()
                .to_zoned(jiff::tz::TimeZone::UTC)
                .date()
        });
        (day, grace)
    }

    /// The current bodies of `listed`, each with its `uid`, in their order,
    /// read in one statement.
    async fn care_bodies(&self, listed: &[Listed]) -> Result<Vec<Value>, ServiceError> {
        let ids: Vec<VoId> = listed.iter().map(|row| VoId(row.holder)).collect();
        let mut read =
            crate::versioning::read::read_currents(&self.demographic_pool, self.spec_profile, &ids)
                .await?;
        let mut bodies = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(version) = read.remove(&id) else {
                continue;
            };
            if version.deleted() {
                continue;
            }
            bodies.push(version_response(id, version)?.body);
        }
        Ok(bodies)
    }
}

impl Listed {
    /// The care relationship `row` of the subject `person`; `None` for a
    /// fact of another kind.
    pub(super) fn of(person: Uuid, row: &FactRow) -> Option<Self> {
        match (&row.kind, &row.value) {
            (FactKind::UnderCareOf, FactValue::Care(care)) => Some(Self {
                holder: row.holder,
                person,
                provider: care.provider.clone(),
            }),
            _ => None,
        }
    }
}

/// Whether the care relationship `row` is live on `day`, with `grace` after
/// its end, of a head that is `complete` or `incomplete`.
fn live(row: &FactRow, day: jiff::civil::Date, grace: jiff::Span) -> bool {
    let FactValue::Care(care) = &row.value else {
        return false;
    };
    let held = matches!(row.term.lifecycle_state.as_str(), "532" | "553");
    held && row.term.dates.is_some_and(|dates| {
        CareOf::spanning(
            care.carer,
            (dates.lower, dates.upper),
            care.role.clone(),
            care.provider.clone(),
        )
        .live_on(day, grace)
    })
}

/// Whether the care relationship `row` is in the role `role`, compared by
/// terminology id and code; any role when `None`.
fn in_role(row: &FactRow, role: Option<&ForeignCode>) -> bool {
    match (&row.value, role) {
        (_, None) => true,
        (FactValue::Care(care), Some(role)) => care.role == *role,
        _ => false,
    }
}

/// Refuses `body` unless it is an `under_care_of` relationship whose
/// `source` is the `PERSON` `person`.
///
/// # Errors
/// [`ServiceError::ValidationFailed`] (`422`) naming `/name` or `/source`.
fn care_body_of(person: Uuid, body: &PartyRelationship) -> Result<(), ServiceError> {
    let tree = Typed::Object(body);
    let care = CareRelationship::UnderCareOf;
    if !carries(tree, care.terminology(), care.code()) {
        return Err(ServiceError::ValidationFailed(vec![
            InvariantViolation::at(
                "/name",
                format!(
                    "a care relationship is a PARTY_RELATIONSHIP named {}::{}",
                    care.terminology(),
                    care.code()
                ),
            ),
        ]));
    }
    let source = tree
        .pointer_text("/source/id/value")
        .and_then(|id| container(&id));
    if source != Some(person) {
        return Err(ServiceError::ValidationFailed(vec![
            InvariantViolation::at(
                "/source",
                format!("the source of a care relationship of this subject is its PERSON {person}"),
            ),
        ]));
    }
    Ok(())
}

/// The `404` of `vo_id`, which holds no care relationship.
pub(super) fn not_care(vo_id: VoId) -> ServiceError {
    ServiceError::not_found(
        Missing::VersionedObject,
        format!("care relationship {vo_id}"),
    )
}
