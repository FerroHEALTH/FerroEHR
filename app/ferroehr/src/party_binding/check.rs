// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pre-flight of a migration to party binding: what
//! `ferroehr party-binding check` reports before the `persons` issuers are
//! added.
//!
//! It reads the party versions alone, so it runs before any boot fill. The
//! categories that need the side table are read after [`store::refill`] runs
//! inside a transaction that is then rolled back, so the check writes
//! nothing. For each binding key a live `PERSON` carries it reports the
//! committer of the version that introduced the key and judges it: a SCIM
//! client of `[[scim.client]]`, an administrator the operator names, a
//! migration the operator declares, or just-in-time creation on an issuer
//! with `jit = true`. A stored version records no role its
//! committer held, so the administrators are operator input, named in the
//! report. No openEHR spec governs a deployment's migration: our own design.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

mod provenance;

use crate::party_binding::BindingMapping;
use crate::party_binding::check::provenance::provenance_of;
use crate::party_binding::derive::Identifier;
use crate::party_binding::fill::{self, Duplicate};
use crate::party_binding::provisioned;
use crate::party_binding::store::{self, HeldSelector};
use crate::storage::error::StorageError;

/// Why the pre-flight cannot run.
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    /// No party mapping is configured, so no key binds.
    #[error("no [[authz.party_binding.person.template]] is configured, so no PERSON binds a login")]
    Unmapped,
    /// An administrator named by a binding key no live `PERSON` carries.
    #[error(
        "the administrator key {key:?} of the issuer {issuer:?} is carried by no live PERSON of \
         a mapped template: name the administrator by its party id, or by a key its PERSON carries"
    )]
    UnknownAdministrator {
        /// The key's issuer.
        issuer: String,
        /// The key as given.
        key: String,
    },
    /// Two `PERSON`s carry one selector value, which a fill refuses.
    #[error(
        "{}: resolve them first, as `ferroehr party-binding check` reports them",
        duplicates.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
    )]
    Duplicate {
        /// Each value two or more `PERSON`s carry.
        duplicates: Vec<Duplicate>,
    },
    /// The provisional cut-off is out of the time range.
    #[error("provisional_report_days puts the report cut-off out of range")]
    Horizon(#[source] jiff::Error),
    /// A stored committer is no canonical `PARTY_PROXY`.
    #[error("the committer of a version of PERSON {party} is no canonical PARTY_PROXY")]
    Committer {
        /// The `PERSON`'s container.
        party: Uuid,
        /// The decode failure.
        #[source]
        source: openehr_its::json::JsonParseError,
    },
    /// The party store failed.
    #[error("the party versions could not be read")]
    Storage(#[from] StorageError),
}

impl From<sqlx::Error> for CheckError {
    fn from(e: sqlx::Error) -> Self {
        Self::Storage(StorageError::Database(e))
    }
}

/// An administrator the operator names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Administrator {
    /// By the container of its `PERSON`.
    Party(Uuid),
    /// By a binding key its `PERSON` carries.
    Key {
        /// The `identifier_issuer` of the key's `[[auth.party]]` table.
        issuer: String,
        /// The key.
        key: String,
    },
}

/// A mapped Basic user: `[[authz.party_binding.basic_user]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicUser {
    /// The Basic user.
    pub user: String,
    /// The `identifier_issuer` of the key.
    pub key_issuer: String,
    /// The binding key.
    pub key: String,
}

/// The logins the identity provider assigns, one issuer's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    /// The `identifier_issuer` the logins are keys of.
    pub issuer: String,
    /// The login identifiers.
    pub logins: Vec<String>,
}

/// What the pre-flight reads besides the party versions.
#[derive(Debug, Clone, Default)]
pub struct CheckInput {
    /// The `identifier_type` of each `[[auth.party]]` table, by its
    /// `identifier_issuer`, which says how a given key is compared.
    pub key_types: BTreeMap<String, String>,
    /// The mapped Basic users.
    pub basic_users: Vec<BasicUser>,
    /// Each `[[scim.client]]` as its token `issuer` and `client_id`.
    pub scim_clients: Vec<(String, String)>,
    /// The administrators the operator names.
    pub administrators: Vec<Administrator>,
    /// The committer names of the migrations the operator declares.
    pub migrations: Vec<String>,
    /// The logins the identity provider assigns, when given.
    pub expected: Option<Expected>,
    /// `[authz.party_binding] provisional_report_days`: the age in days past
    /// which a provisional `PERSON` is reported.
    pub provisional_report_days: u32,
    /// The `[[authz.party_binding.issuer]]`s with `jit = true`.
    pub jit_issuers: Vec<String>,
    /// The date a professional `ROLE` is judged on (`YYYY-MM-DD`).
    pub today: String,
}

/// The pre-flight's report.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// The administrators as named, each with its `PERSON`.
    pub administrators: Vec<NamedAdministrator>,
    /// The committer names of the declared migrations.
    pub migrations: Vec<String>,
    /// The age in days past which a provisional `PERSON` is reported.
    pub provisional_report_days: u32,
    /// Each selector value two or more live `PERSON`s carry.
    pub duplicates: Vec<DuplicateValue>,
    /// The `PERSON`s carrying a binding key, per key issuer.
    pub issuers: Vec<IssuerPersons>,
    /// The key-bearing `PERSON`s whose head is `800|inactive|`.
    pub inactive: Vec<Uuid>,
    /// The key-bearing `PERSON`s whose head is `801|abandoned|`.
    pub abandoned: Vec<Uuid>,
    /// What the side table answers, absent while a duplicate stands, since a
    /// fill refuses one.
    pub side_table: Option<SideTable>,
    /// The committer that introduced each binding key, with its judgement.
    pub provenance: Vec<Provenance>,
    /// The expected logins with no live `PERSON`, when they were given.
    pub expected: Option<ExpectedReport>,
}

/// An administrator as the operator named it, with its `PERSON`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NamedAdministrator {
    /// The name as given: a party id, or `issuer|key`.
    pub named: String,
    /// The administrator's `PERSON`.
    pub party: Uuid,
}

/// One selector value two or more `PERSON`s carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateValue {
    /// `key`, `user_name` or `external_id`.
    pub selector: &'static str,
    /// The issuer; empty for a `userName`.
    pub issuer: String,
    /// The `PERSON`s carrying it.
    pub parties: Vec<Uuid>,
}

/// The `PERSON`s carrying a key of one issuer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IssuerPersons {
    /// The key issuer.
    pub issuer: String,
    /// The `PERSON`s.
    pub persons: Vec<Uuid>,
}

/// What the side table answers once refilled.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SideTable {
    /// The key-bearing `PERSON`s that perform no professional `ROLE`, which
    /// `adopt` has not reached.
    pub unadopted: Vec<Uuid>,
    /// The provisional `PERSON`s, `553|incomplete|` with a binding key, created
    /// more than `provisional_report_days` days ago, oldest first.
    pub provisional: Vec<ProvisionalPerson>,
    /// Every mapped Basic user, with the `PERSON` it binds.
    pub basic_users: Vec<BasicUserReport>,
}

/// A provisional `PERSON` and when it was created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProvisionalPerson {
    /// The `PERSON`.
    pub party: Uuid,
    /// When its first version was committed (RFC 3339).
    pub created: String,
}

/// A mapped Basic user and the `PERSON` it binds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BasicUserReport {
    /// The Basic user.
    pub user: String,
    /// The key issuer.
    pub key_issuer: String,
    /// The binding key.
    pub key: String,
    /// The `PERSON` it binds today; absent when none does.
    pub party: Option<Uuid>,
}

/// The committer that introduced a binding key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Provenance {
    /// The `PERSON`.
    pub party: Uuid,
    /// The key issuer.
    pub issuer: String,
    /// The canonical key.
    pub key: String,
    /// When the introducing version was committed (RFC 3339).
    pub committed_at: String,
    /// The CONTRIBUTION of the introducing version.
    pub contribution: Uuid,
    /// The committer of the introducing version.
    pub committer: Committer,
    /// Who the committer is under the rule; absent when it fails.
    pub by: Option<Origin>,
}

/// A committer as the report names it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Committer {
    /// `PARTY_IDENTIFIED.name`.
    pub name: Option<String>,
    /// The `external_ref` id.
    pub external_ref: Option<String>,
    /// Each identifier as `issuer|type|id`.
    pub identifiers: Vec<String>,
}

/// Who introduced a binding key, under the provenance rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// A `[[scim.client]]`.
    Scim,
    /// An administrator the operator named.
    Administrator,
    /// A migration the operator declared.
    Migration,
    /// Just-in-time creation at a first login of an issuer with `jit = true`.
    Jit,
}

/// The expected logins with no live `PERSON`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExpectedReport {
    /// The key issuer.
    pub issuer: String,
    /// The logins no live `PERSON` carries.
    pub missing: Vec<String>,
}

impl Report {
    /// Why the pre-flight fails, one line each: a duplicate, a mapped Basic
    /// user or an expected login with no `PERSON`, and a key whose committer
    /// fails the provenance rule. Empty when it passes.
    #[must_use]
    pub fn failures(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .duplicates
            .iter()
            .map(|d| {
                Duplicate {
                    selector: d.selector,
                    issuer: d.issuer.clone(),
                    parties: d.parties.clone(),
                }
                .to_string()
            })
            .collect();
        if let Some(table) = &self.side_table {
            out.extend(
                table
                    .basic_users
                    .iter()
                    .filter(|u| u.party.is_none())
                    .map(|u| format!("the mapped Basic user {:?} binds no PERSON", u.user)),
            );
        }
        if let Some(expected) = &self.expected {
            out.extend(expected.missing.iter().map(|login| {
                format!(
                    "the expected login {login:?} of the issuer {:?} has no PERSON",
                    expected.issuer
                )
            }));
        }
        out.extend(self.provenance.iter().filter(|p| p.by.is_none()).map(|p| {
            format!(
                "the binding key of PERSON {} was introduced by a committer that is no SCIM \
                 client, named administrator, declared migration or just-in-time creation",
                p.party
            )
        }));
        out
    }
}

/// Runs the pre-flight over the party versions of `pool` under `mapping`,
/// writing nothing.
///
/// # Errors
/// [`CheckError::UnknownAdministrator`] for an administrator key no live
/// `PERSON` carries, [`CheckError::Committer`] for a stored committer that
/// does not decode, [`CheckError::Storage`] when the store fails.
pub async fn check(
    pool: &PgPool,
    mapping: &BindingMapping,
    input: &CheckInput,
) -> Result<Report, CheckError> {
    let mut tx = pool.begin().await?;
    let report = run(&mut tx, mapping, input).await;
    tx.rollback().await?;
    report
}

/// A `PERSON` an adoption names, as the pre-flight reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// The `PERSON`.
    pub(crate) party: Uuid,
    /// Whether it is a live `PERSON` of a mapped template carrying a key.
    pub(crate) keyed: bool,
    /// Whether it performs a professional `ROLE` already.
    pub(crate) professional: bool,
    /// Whether the committer of each of its keys passes the provenance rule.
    pub(crate) passes: bool,
}

/// The named administrators and what the pre-flight reads of each of
/// `parties`, writing nothing.
///
/// # Errors
/// [`CheckError::Duplicate`] while a selector value two `PERSON`s carry
/// stands, and the [`check`] refusals.
pub(crate) async fn candidates(
    pool: &PgPool,
    mapping: &BindingMapping,
    input: &CheckInput,
    parties: &[Uuid],
) -> Result<(Vec<NamedAdministrator>, Vec<Candidate>), CheckError> {
    let mut tx = pool.begin().await?;
    let read = read_candidates(&mut tx, mapping, input, parties).await;
    tx.rollback().await?;
    read
}

/// [`candidates`] on `tx`, which the caller rolls back.
async fn read_candidates(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    input: &CheckInput,
    parties: &[Uuid],
) -> Result<(Vec<NamedAdministrator>, Vec<Candidate>), CheckError> {
    let held = store::held_selectors(tx, mapping).await?;
    let duplicates = fill::duplicates(held.clone());
    if !duplicates.is_empty() {
        return Err(CheckError::Duplicate { duplicates });
    }
    let keys: Vec<&HeldSelector> = held.iter().filter(|h| h.selector == "key").collect();
    let administrators = named_administrators(input, &keys)?;
    let admin_parties: BTreeSet<Uuid> = administrators.iter().map(|a| a.party).collect();
    store::refill(tx, mapping).await?;
    let unadopted = side_table(tx, parties, input, &|issuer: &str, key: &str| {
        canonical(input, issuer, key)
    })
    .await?
    .unadopted;
    let mut out = Vec::with_capacity(parties.len());
    for party in parties {
        let own: Vec<&&HeldSelector> = keys.iter().filter(|k| k.party_id == *party).collect();
        let mut passes = !own.is_empty();
        for key in &own {
            let provenance = provenance_of(tx, mapping, key, input, &admin_parties).await?;
            passes &= provenance.is_some_and(|p| p.by.is_some());
        }
        out.push(Candidate {
            party: *party,
            keyed: !own.is_empty(),
            professional: !own.is_empty() && !unadopted.contains(party),
            passes,
        });
    }
    Ok((administrators, out))
}

/// The pre-flight on `tx`, which the caller rolls back.
async fn run(
    tx: &mut PgConnection,
    mapping: &BindingMapping,
    input: &CheckInput,
) -> Result<Report, CheckError> {
    let held = store::held_selectors(tx, mapping).await?;
    let keys: Vec<&HeldSelector> = held.iter().filter(|h| h.selector == "key").collect();
    let canonical_of = |issuer: &str, key: &str| canonical(input, issuer, key);
    let administrators = named_administrators(input, &keys)?;
    let duplicates: Vec<DuplicateValue> = fill::duplicates(held.clone())
        .into_iter()
        .map(|d| DuplicateValue {
            selector: d.selector,
            issuer: d.issuer,
            parties: d.parties,
        })
        .collect();
    let mut by_issuer: BTreeMap<&str, BTreeSet<Uuid>> = BTreeMap::new();
    for key in &keys {
        by_issuer
            .entry(key.issuer.as_str())
            .or_default()
            .insert(key.party_id);
    }
    let bearers: Vec<Uuid> = by_issuer
        .values()
        .flatten()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let heads = head_states(tx, &bearers).await?;
    let in_state = |state: &str| -> Vec<Uuid> {
        heads
            .iter()
            .filter(|(_, s)| s.as_str() == state)
            .map(|(party, _)| *party)
            .collect()
    };
    let side_table = if duplicates.is_empty() {
        store::refill(tx, mapping).await?;
        Some(side_table(tx, &bearers, input, &canonical_of).await?)
    } else {
        None
    };
    let admin_parties: BTreeSet<Uuid> = administrators.iter().map(|a| a.party).collect();
    let mut provenance = Vec::with_capacity(keys.len());
    for key in &keys {
        provenance.extend(provenance_of(tx, mapping, key, input, &admin_parties).await?);
    }
    let expected = input.expected.as_ref().map(|expected| ExpectedReport {
        issuer: expected.issuer.clone(),
        missing: expected
            .logins
            .iter()
            .filter(|login| carrier(input, &keys, &expected.issuer, login).is_none())
            .cloned()
            .collect(),
    });
    Ok(Report {
        administrators,
        migrations: input.migrations.clone(),
        provisional_report_days: input.provisional_report_days,
        duplicates,
        issuers: by_issuer
            .into_iter()
            .map(|(issuer, persons)| IssuerPersons {
                issuer: issuer.to_owned(),
                persons: persons.into_iter().collect(),
            })
            .collect(),
        inactive: in_state("800"),
        abandoned: in_state("801"),
        side_table,
        provenance,
        expected,
    })
}

/// The canonical form of `key` under the key type of `issuer`.
fn canonical(input: &CheckInput, issuer: &str, key: &str) -> String {
    let key_type = input.key_types.get(issuer).map_or("", String::as_str);
    Identifier::new(issuer, key_type, key).canonical()
}

/// The live `PERSON` among `keys` that carries `key` of `issuer`.
fn carrier(input: &CheckInput, keys: &[&HeldSelector], issuer: &str, key: &str) -> Option<Uuid> {
    let value = canonical(input, issuer, key);
    keys.iter()
        .find(|h| h.issuer == issuer && h.value == value)
        .map(|h| h.party_id)
}

/// The administrators of `input`, each with its `PERSON`.
fn named_administrators(
    input: &CheckInput,
    keys: &[&HeldSelector],
) -> Result<Vec<NamedAdministrator>, CheckError> {
    input
        .administrators
        .iter()
        .map(|named| match named {
            Administrator::Party(party) => Ok(NamedAdministrator {
                named: party.to_string(),
                party: *party,
            }),
            Administrator::Key { issuer, key } => Ok(NamedAdministrator {
                named: format!("{issuer}|{key}"),
                party: carrier(input, keys, issuer, key).ok_or_else(|| {
                    CheckError::UnknownAdministrator {
                        issuer: issuer.clone(),
                        key: key.clone(),
                    }
                })?,
            }),
        })
        .collect()
}

/// The head state of each of `parties`.
async fn head_states(
    tx: &mut PgConnection,
    parties: &[Uuid],
) -> Result<BTreeMap<Uuid, String>, CheckError> {
    let rows = sqlx::query(
        "SELECT vo_id, lifecycle_state FROM vo_head WHERE vo_id = ANY($1) ORDER BY vo_id",
    )
    .bind(parties)
    .fetch_all(&mut *tx)
    .await?;
    let mut out = BTreeMap::new();
    for row in rows {
        out.insert(row.try_get("vo_id")?, row.try_get("lifecycle_state")?);
    }
    Ok(out)
}

/// What the refilled side table on `tx` answers for the key-bearing
/// `bearers` and the mapped Basic users.
async fn side_table(
    tx: &mut PgConnection,
    bearers: &[Uuid],
    input: &CheckInput,
    canonical: &impl Fn(&str, &str) -> String,
) -> Result<SideTable, CheckError> {
    let professionals: BTreeSet<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT party_id FROM binding_identifier \
         WHERE selector = 'key' AND retired_at IS NULL AND NOT isempty(professional) \
           AND party_id = ANY($1)",
    )
    .bind(bearers)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    let mut basic_users = Vec::with_capacity(input.basic_users.len());
    for user in &input.basic_users {
        let party: Option<Uuid> = sqlx::query_scalar(
            "SELECT party_id FROM binding_identifier \
             WHERE selector = 'key' AND retired_at IS NULL AND issuer = $1 AND value = $2 \
               AND lifecycle_state IN ('532', '553') AND professional @> $3::date \
             ORDER BY party_id LIMIT 1",
        )
        .bind(&user.key_issuer)
        .bind(canonical(&user.key_issuer, &user.key))
        .bind(&input.today)
        .fetch_optional(&mut *tx)
        .await?;
        basic_users.push(BasicUserReport {
            user: user.user.clone(),
            key_issuer: user.key_issuer.clone(),
            key: user.key.clone(),
            party,
        });
    }
    let before = jiff::Timestamp::now()
        .saturating_sub(jiff::SignedDuration::from_hours(
            i64::from(input.provisional_report_days) * 24,
        ))
        .map_err(CheckError::Horizon)?;
    let provisional = provisioned::provisional_before(tx, before)
        .await?
        .into_iter()
        .map(|(party, created)| ProvisionalPerson {
            party: party.0,
            created: created.to_string(),
        })
        .collect();
    Ok(SideTable {
        provisional,
        unadopted: bearers
            .iter()
            .filter(|party| !professionals.contains(party))
            .copied()
            .collect(),
        basic_users,
    })
}
