// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Who may change what a party's memberships, care relationship and record
//! subjects say: the `write_care_relationship` grant scoped by the providers
//! and target kinds its elements name, or an administrator (draft access
//! control decision procedure, Writing Relationships). No openEHR spec
//! governs who writes a party: our own design over that procedure.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the stored canonical party body \
              the caller facts are derived from"
)]

use serde_json::Value;
use uuid::Uuid;

use super::{CallerFact, FactValue, care_of, subject};
use crate::error::ServiceError;
use crate::party_binding::BindingMapping;
use crate::party_binding::derive::Facts;
use crate::party_binding::grants::capabilities::{Capabilities, Credentials, Grant};
use crate::party_binding::grants::derive::GrantFacts;
use crate::party_binding::grants::operation::{Operation, PROVIDER, TARGET_KIND};
use crate::party_binding::policy_mark::TERMINOLOGY;
use crate::storage::error::StorageError;
use crate::versioning::Kind;
use openehr_rm::draft::access_control::terminology::{CareRelationship, Term};

/// Refuses the commit `committed` of a `PERSON`, an `ORGANISATION`, a
/// `GROUP` or a `PARTY_RELATIONSHIP` when its version changes what the
/// memberships or the care relationship say, or the record subjects it
/// declares, and its caller may not ([`check_care_writer`]).
///
/// The memberships, care relationships and record subjects of `facts`, what
/// the committed body holds, are compared with those of the last earlier
/// version that holds a body: a relationship by kind, what it states and
/// `time_validity`, a subject by namespace and id, never by position
/// (#5640). A logical deletion of a party holds no body and changes none:
/// its earlier versions keep them (RM common
/// `master06-change_control_package.adoc` §Logical Deletion). A logical
/// deletion of a care relationship removes it from the current set, so it
/// needs the grant its writing does. A lifecycle state is no relationship
/// content, so a deactivation that keeps them needs no grant (#5628). Our own
/// design.
///
/// Whether the commit changed them.
///
/// # Errors
/// The `403` of [`check_care_writer`], and a storage failure.
pub(crate) async fn check_care(
    tx: &mut sqlx::PgConnection,
    mapping: &BindingMapping,
    committed: &crate::versioning::change::Committed,
    facts: Option<&Facts>,
    prior: Option<&crate::storage::version_repo::gate::Prior>,
) -> Result<bool, ServiceError> {
    let standalone = committed.kind == Kind::PartyRelationship;
    if !standalone
        && !matches!(
            committed.kind,
            Kind::Person | Kind::Organisation | Kind::Group
        )
    {
        return Ok(false);
    }
    let after = match facts {
        Some(facts) => &facts.grants,
        None if standalone => &GrantFacts::default(),
        None => return Ok(false),
    };
    // The gate read's last body is the newest before the commit, every version it
    // saw being older than the one the commit wrote.
    let before: Option<String> = match prior {
        Some(prior) => prior.last_body.as_ref().map(|(_, _, body)| body.clone()),
        None => sqlx::query_scalar(
            "SELECT v.body FROM version v WHERE v.vo_id = $1 AND v.sys_version < $2 \
             AND v.body IS NOT NULL ORDER BY v.sys_version DESC LIMIT 1",
        )
        .bind(committed.vo_id)
        .bind(committed.sys_version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from)?,
    };
    let before = match before {
        Some(body) => {
            let body: Value = serde_json::from_str(&body).map_err(StorageError::BodyDecode)?;
            GrantFacts::of(committed.kind, &body)
        }
        None => GrantFacts::default(),
    };
    let changed = care_changes(&before.facts, &after.facts);
    let subjects = !same_subjects(&before.subjects, &after.subjects);
    if changed.is_empty() && !subjects {
        return Ok(false);
    }
    check_care_writer(tx, mapping, committed.vo_id.0, &changed).await?;
    Ok(true)
}

/// The care relationships and memberships among `before` and `after` that
/// the other does not hold, matched one to one by kind, what they state and
/// `time_validity`, whatever their order: what a commit removed and added.
pub(super) fn care_changes<'f>(
    before: &'f [CallerFact],
    after: &'f [CallerFact],
) -> Vec<&'f FactValue> {
    let care = |fact: &&CallerFact| {
        matches!(
            fact.value,
            FactValue::Team(_) | FactValue::Care(_) | FactValue::Kin(_)
        )
    };
    let mut added: Vec<&CallerFact> = after.iter().filter(care).collect();
    let mut removed = Vec::new();
    for fact in before.iter().filter(care) {
        let found = added.iter().position(|other| {
            other.kind == fact.kind
                && other.value == fact.value
                && other.validity == fact.validity
                && other.about == fact.about
        });
        match found {
            Some(at) => {
                added.swap_remove(at);
            }
            None => removed.push(&fact.value),
        }
    }
    removed.extend(added.into_iter().map(|fact| &fact.value));
    removed
}

/// Whether `before` and `after` declare the same record subjects, by
/// namespace and id, whatever their order.
pub(super) fn same_subjects(
    before: &[subject::DeclaredSubject],
    after: &[subject::DeclaredSubject],
) -> bool {
    let sorted = |held: &[subject::DeclaredSubject]| {
        let mut subjects: Vec<_> = held
            .iter()
            .map(|declared| declared.subject.clone())
            .collect();
        subjects.sort();
        subjects
    };
    sorted(before) == sorted(after)
}

/// Refuses a commit that changed the care relationships, memberships or
/// record subjects of the party `party` when its caller is no administrator
/// of `mapping` and no [`Operation::WriteCareRelationship`] grant it holds
/// covers each of `changed`, what the commit removed and added, on `tx`.
///
/// A kinship is covered by any such grant. A care relationship is covered by
/// a grant whose [`PROVIDER`] elements
/// name its provider and whose [`TARGET_KIND`] elements name the RM type of
/// its target. A membership is covered by a grant whose [`PROVIDER`]
/// elements name a provider of the `ORGANISATION` it targets, or of the
/// `ORGANISATION` the `GROUP` it targets belongs to
/// ([`membership_providers`]); a membership of a `GROUP` that belongs to no
/// provider is written by an administrator alone. A grant with no such
/// element covers none. A change of the record subjects alone needs the
/// grant. Draft access control decision procedure, Writing Relationships: a
/// relationship is written "for the providers and the kinds of target its
/// authorisation names"; our own design over it.
///
/// # Errors
/// [`ServiceError::Forbidden`] (`403`) naming the grant, or the relationship
/// no grant covers; a storage failure.
async fn check_care_writer(
    tx: &mut sqlx::PgConnection,
    mapping: &BindingMapping,
    party: Uuid,
    changed: &[&FactValue],
) -> Result<(), ServiceError> {
    if crate::party_binding::protect::administrator(mapping) {
        return Ok(());
    }
    let capabilities = Capabilities::current();
    let grants: Vec<Credentials<'_>> = capabilities
        .of(Operation::WriteCareRelationship)
        .map(Grant::credentials)
        .collect();
    if grants.is_empty() {
        return Err(no_grant(party));
    }
    for value in changed {
        let covered = match value {
            FactValue::Care(care) => grants.iter().any(|grant| {
                grant.identifiers(PROVIDER).contains(&care.provider)
                    && grant
                        .words(TARGET_KIND)
                        .iter()
                        .any(|kind| kind.trim() == care.kind.rm_type())
            }),
            FactValue::Team(team) => {
                let providers = membership_providers(tx, *team).await?;
                grants.iter().any(|grant| {
                    grant
                        .identifiers(PROVIDER)
                        .iter()
                        .any(|provider| providers.contains(provider))
                })
            }
            // NOTE: draft decision procedure, Writing Relationships, a kinship names no provider
            // and admits no one, so any write_care_relationship grant covers it.
            FactValue::Code(_) | FactValue::Kin(_) => true,
        };
        if !covered {
            return Err(uncovered(party, value));
        }
    }
    Ok(())
}

/// The providers a membership of `team` is written for: the identifiers
/// `binding_provider` holds for `team`, an `ORGANISATION`, and for each
/// `ORGANISATION` `team`, a `GROUP`, is a member of, read on `tx`.
async fn membership_providers(
    tx: &mut sqlx::PgConnection,
    team: Uuid,
) -> Result<Vec<care_of::ProviderKey>, StorageError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.issuer, p.id FROM binding_provider p WHERE p.party_id = $1 \
         UNION SELECT p.issuer, p.id FROM binding_caller_fact f \
         JOIN binding_provider p ON p.party_id = f.target_id \
         WHERE f.party_id = $1 AND f.kind = 'member_of'",
    )
    .bind(team)
    .fetch_all(&mut *tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(issuer, id)| care_of::ProviderKey::new(issuer, id))
        .collect())
}

/// The `403` of a caller holding no [`Operation::WriteCareRelationship`]
/// grant that changed what the party `party` says.
fn no_grant(party: Uuid) -> ServiceError {
    ServiceError::not_permitted(format!(
        "this version changes the care relationships of party {party} ({}::{} or {}::{}), a \
         kinship ({TERMINOLOGY}::related_to) or the record subjects it declares \
         ({TERMINOLOGY}::{}): only a caller holding the {} \
         grant, such as the care delivery environment's ADT system, or an administrator \
         writes them",
        CareRelationship::MemberOf.terminology(),
        CareRelationship::MemberOf.code(),
        CareRelationship::UnderCareOf.terminology(),
        CareRelationship::UnderCareOf.code(),
        openehr_rm::draft::access_control::terminology::RecordSubject::RecordSubject.code(),
        Operation::WriteCareRelationship
    ))
}

/// The `403` of a change of party `party`, `value`, that no
/// [`Operation::WriteCareRelationship`] grant of the caller covers.
fn uncovered(party: Uuid, value: &FactValue) -> ServiceError {
    let what = match value {
        FactValue::Care(care) => format!(
            "a care relationship in the care of the provider {} to a target of kind {}",
            care.provider,
            care.kind.rm_type()
        ),
        FactValue::Team(team) => format!(
            "a membership of {team}, whose providers no grant names (a GROUP that belongs to \
             no ORGANISATION carrying an identifier is written by an administrator alone)"
        ),
        FactValue::Code(_) | FactValue::Kin(_) => String::new(),
    };
    ServiceError::not_permitted(format!(
        "this version of party {party} writes {what}: no {} grant of the caller names that \
         provider among its {PROVIDER} elements{}",
        Operation::WriteCareRelationship,
        match value {
            FactValue::Care(_) => format!(" with that kind among its {TARGET_KIND} elements"),
            FactValue::Team(_) | FactValue::Code(_) | FactValue::Kin(_) => String::new(),
        }
    ))
}
