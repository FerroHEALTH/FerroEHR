// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A CONTRIBUTION of COMPOSITION members committed in one guarded statement
//! (#6256): one gate read of the EHR and every target, the members judged on
//! it, and one statement writing the CONTRIBUTION, every version and the
//! outbox row, judged again when what the gate read judged moved before it
//! ran. A set the statement does not cover (another kind, an attestation, a
//! target off its trunk tip or created elsewhere, more members than
//! [`SET_LIMIT`]) commits on the per-member path. No openEHR spec governs how
//! a commit is run against the database: our own design.

use openehr_base::v1_3::base_types::identification::lexical::composite_ids_equal;
use uuid::Uuid;

use crate::error::ServiceError;
use crate::ids::{EhrId, VoId};
use crate::service::FerroEhrService;
use crate::service::write::{ATTEMPTS, Contended, MOVED, Materials};
use crate::storage::version_repo::gate::{self, Ask, Gate, Shape};
use crate::storage::version_repo::guarded::Guard;
use crate::system_log::categories::{AccessedContent, ContentIds};
use crate::system_log::recorder::WriteFacts;
use crate::versioning::Kind;
use crate::versioning::audit::AuditInput;
use crate::versioning::change::set::{Member, Set, prepare_guarded_set};
use crate::versioning::change::{Change, Committed, CommittedContribution};
use crate::versioning::object_version_id::TreeId;
use crate::versioning::restriction;

/// The most members a CONTRIBUTION commits in one statement; a larger set
/// commits on the per-member path.
///
/// Past 8 members the statement's planning outgrows the round trips it saves
/// (#6256).
// NOTE: our own design (#6256), the bound where the measurement stops favouring one statement.
pub(crate) const SET_LIMIT: usize = 8;

/// What a guarded set commit answered.
pub(crate) enum Outcome {
    /// The set committed.
    Committed(CommittedContribution),
    /// The set is one the statement does not cover, judged on its first
    /// gate read before anything was taken: the changes, for the per-member
    /// path.
    PerMember(Vec<(AuditInput, Change)>),
}

/// Whether `changes` is a set the one statement can cover by its shape alone:
/// at most [`SET_LIMIT`] COMPOSITION members, none with an attestation, every
/// modify and delete naming the version it continues.
pub(crate) fn covers(changes: &[(AuditInput, Change)]) -> bool {
    changes.len() <= SET_LIMIT
        && changes.iter().all(|(_, change)| match change {
            Change::Create {
                kind, attestations, ..
            } => *kind == Kind::Composition && attestations.is_empty(),
            Change::Modify {
                kind,
                attestations,
                expected,
                ..
            } => *kind == Kind::Composition && attestations.is_empty() && expected.is_some(),
            Change::Delete { kind, expected, .. } => {
                *kind == Kind::Composition && expected.is_some()
            }
        })
}

/// The object a change continues, and the version it names.
fn target(change: &Change) -> Option<(VoId, Option<TreeId>)> {
    match change {
        Change::Create { .. } => None,
        Change::Modify {
            vo_id, expected, ..
        }
        | Change::Delete {
            vo_id, expected, ..
        } => Some((*vo_id, *expected)),
    }
}

impl FerroEhrService {
    /// Commit the COMPOSITION set `changes` into `ehr_id` in one guarded
    /// statement, on `first`, the gate read its planning made, and on a new
    /// one for each later attempt; or hand it back for the per-member path
    /// when a target is not at the trunk tip this system continues.
    ///
    /// # Errors
    /// The EHR's `404` and `409`, `access-refreshing` for a moved policy, the
    /// `412` of a version superseded meanwhile, the per-member refusals, and
    /// `service_overloaded` once the attempts are spent.
    pub(crate) async fn commit_member_set(
        &self,
        ehr_id: EhrId,
        (supplied_uid, audit): (Option<Uuid>, &AuditInput),
        changes: Vec<(AuditInput, Change)>,
        mut first: Option<Gate>,
    ) -> Result<Outcome, ServiceError> {
        let targets: Vec<VoId> = changes
            .iter()
            .filter_map(|(_, change)| target(change).map(|(vo_id, _)| vo_id))
            .collect();
        let named: Vec<_> = changes.iter().map(|(_, change)| target(change)).collect();
        let materials = Materials::once(changes);
        let signing_ctx = self.signing_ctx();
        for attempt in 0..ATTEMPTS {
            let mut conn = self.recorded_conn(&self.pool).await?;
            // The first attempt commits on the gate read the set's planning made.
            let gate = if let Some(gate) = first.take() {
                gate
            } else {
                let ask = Ask {
                    ehr_id,
                    targets: &targets,
                    shape: Shape::Members,
                    persistent: None,
                    addressed: None,
                    items: &[],
                };
                let gate = gate::read(&mut conn, ask).await?;
                self.ensure_gate_policy_held().await?;
                gate
            };
            let placed = match self.placed(&gate, ehr_id, &named)? {
                Placed::Members(placed) => placed,
                // The first gate read hands an uncovered set to the per-member path,
                // which answers it; a later one follows a commit that superseded a
                // named version, which that path answers 412.
                _ if attempt == 0 => return Ok(Outcome::PerMember(materials.take_first()?)),
                Placed::Superseded(tree) => {
                    return Err(ServiceError::version_conflict(format!(
                        "expected version {tree} has been superseded"
                    )));
                }
                Placed::Elsewhere => return Err(ServiceError::overloaded(MOVED, Contended)),
            };
            let members = members(materials.take_first()?, placed, &gate)?;
            let set = Set {
                ehr_id,
                supplied_uid,
                audit,
                now: gate.now,
                guard: Guard {
                    policy: gate.policy,
                    definitions: gate.definitions,
                    ..Guard::default()
                },
            };
            let ready =
                Box::pin(prepare_guarded_set(&mut conn, set, members, &signing_ctx)).await?;
            let mut content: Vec<ContentIds> = ready
                .versions()
                .filter_map(Committed::content_ids)
                .collect();
            content.sort();
            content.dedup();
            let facts = WriteFacts {
                ehr_id: Some(ehr_id.to_string()),
                object_id: Some(ready.id().to_string()),
                content: (!content.is_empty()).then_some(AccessedContent::Objects(content)),
                ..WriteFacts::default()
            };
            // NOTE: our own design (#5747, #5986, #6256), the records are durable before the
            // statement that makes the set visible inserts their reservation beside it.
            let held = self.hold_records(&mut conn, &facts).await?;
            crate::policy_register::observed::wrote();
            let reservation = held.as_ref().map(|held| held.reservation);
            let outcome = if ready.needs_transaction(&signing_ctx) {
                let mut tx = sqlx::Connection::begin(&mut *conn).await?;
                // NOTE: our own design (#5762), the records are held (#6256) before this commit,
                // and its statement inserts their reservation, so they commit with it.
                match Box::pin(ready.commit(&mut tx, &signing_ctx, reservation)).await {
                    Ok(outcome) => tx.commit().await.map(|()| outcome).map_err(Into::into),
                    Err(error) => Err(error),
                }
            } else {
                Box::pin(ready.commit(&mut conn, &signing_ctx, reservation)).await
            };
            if let Some(outcome) = self.settle_held(&mut conn, held, outcome).await? {
                return Ok(Outcome::Committed(outcome));
            }
        }
        Err(ServiceError::overloaded(MOVED, Contended))
    }

    /// Where each member's target sits in `gate`, judged as the per-member
    /// path would judge it ([`Placed`]).
    ///
    /// # Errors
    /// The EHR's `404` and the `409` of a set writing content into an EHR
    /// that is not modifiable; a modify's `VERSIONED_COMPOSITION` invariant
    /// refusal.
    fn placed(
        &self,
        gate: &Gate,
        ehr_id: EhrId,
        named: &[Option<(VoId, Option<TreeId>)>],
    ) -> Result<Placed, ServiceError> {
        match gate.is_modifiable {
            None => {
                return Err(ServiceError::not_found(
                    crate::error::refusal::Missing::Ehr,
                    format!("EHR {ehr_id}"),
                ));
            }
            Some(false) => {
                return Err(crate::versioning::change::precondition::not_modifiable_error(ehr_id));
            }
            Some(true) => {}
        }
        let system_id = self.effective_system_id();
        let mut placed = Vec::with_capacity(named.len());
        let mut at = gate.targets.iter().enumerate();
        for named in named {
            let Some((vo_id, expected)) = named else {
                placed.push(None);
                continue;
            };
            let Some((index, found)) = at.next().filter(|(_, t)| t.vo_id == *vo_id) else {
                return Ok(Placed::Elsewhere);
            };
            let Some(tip) = found.placement.tip.as_ref().filter(|tip| {
                tip.ehr_id == Some(ehr_id)
                    && tip.kind == Kind::Composition.as_str()
                    && tip.lifecycle_state != crate::versioning::lifecycle::state::DELETED
                    && composite_ids_equal(&tip.creating_system_id, &system_id)
            }) else {
                return Ok(Placed::Elsewhere);
            };
            let tree =
                TreeId::from_columns(tip.trunk_version, tip.branch_number, tip.branch_version);
            match expected {
                Some(expected) if *expected == tree => placed.push(Some(index)),
                Some(expected) => return Ok(Placed::Superseded(*expected)),
                None => return Ok(Placed::Elsewhere),
            }
        }
        Ok(Placed::Members(placed))
    }
}

/// Where a set's targets sit, as its gate read found them.
enum Placed {
    /// Every target is at the trunk tip this system continues, which its
    /// member names: the index of each member's target in the gate read.
    Members(Vec<Option<usize>>),
    /// A member names a version that is not the trunk tip.
    Superseded(TreeId),
    /// A target is gone, deleted, of another EHR or kind, or created
    /// elsewhere.
    Elsewhere,
}

/// The members of `changes`, each with what `gate` found of its target at
/// `placed`, its modify invariants judged on the gate's first root.
///
/// # Errors
/// A modify's `VERSIONED_COMPOSITION` invariant refusal.
fn members(
    changes: Vec<(AuditInput, Change)>,
    placed: Vec<Option<usize>>,
    gate: &Gate,
) -> Result<Vec<Member>, ServiceError> {
    let mut members = Vec::with_capacity(changes.len());
    for ((audit, change), at) in changes.into_iter().zip(placed) {
        let found = at.and_then(|at| gate.targets.get(at));
        if let (Change::Modify { body, .. }, Some(found)) = (&change, found) {
            crate::validation::versioned::check_versioned_composition_first_root(
                found.first_root.clone(),
                body,
            )?;
        }
        let deleted = found
            .and_then(|found| found.content.as_ref())
            .map(|content| {
                ContentIds::of_kind(
                    &content.kind,
                    content.template_id.clone(),
                    content.root_archetype.clone(),
                )
            });
        members.push(Member {
            audit,
            placement: found.map(|found| clone_placement(&found.placement)),
            head: found.and_then(|found| found.head),
            known: restriction::Known {
                now: Some(gate.now),
                restriction: Some(gate.restriction.clone()),
                object_restricted: found.map(|found| found.restricted),
            },
            deleted: matches!(change, Change::Delete { .. })
                .then_some(deleted)
                .flatten(),
            change,
        });
    }
    Ok(members)
}

/// A copy of `placement`.
fn clone_placement(
    placement: &crate::storage::version_repo::placement::Placement,
) -> crate::storage::version_repo::placement::Placement {
    crate::storage::version_repo::placement::Placement {
        tip: placement.tip.clone(),
        next_ordinal: placement.next_ordinal,
        now: placement.now,
    }
}
