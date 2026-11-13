// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cost of a large SCIM Group (#5015): a Group built to its cap one member
//! per change, then members removed and added again at the cap, each change
//! timed and weighed against its budget.
//!
//! "A Version of a `PARTY` includes all the compositional parts, such as
//! identities, contacts, Party relationships of which it is the source" (RM
//! demographic `master02-demographic_package.adoc` §Versioning Semantics), and
//! a member is the source of its membership (#5220), so a membership change
//! stores one version of the member's `PERSON`, whatever the Group's size. The run
//! provisions the Users and writes the Group through the platform library's
//! own SCIM service ([`FerroEhrService::provision_user`],
//! [`FerroEhrService::update_group`]), the path a `PATCH /scim/v2/Groups/{id}`
//! takes after its body is parsed, against synthetic ADL 2 definitions of its
//! own. It measures:
//!
//! - the p99 of one member removed and one added at the cap, against
//!   `--change-p99-ms`;
//! - the bytes one change stores at the cap, the `version` and `node` rows of
//!   the member's version as `pg_column_size` counts them, against
//!   `--change-bytes`;
//! - what building the Group stored, as the database's growth, against
//!   `--build-bytes`;
//! - the side-table rows one change writes, which must be one.
//!
//! A missed budget fails the run. No openEHR specification governs load
//! generation or these budgets: our own design.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context as _, bail};
use ferroehr::config::binding::PartyBindingConfig;
use ferroehr::config::binding::person::{
    GroupSection, MembershipSection, PersonSection, PersonTemplate, RoleSection, Selector,
};
use ferroehr::ids::VoId;
use ferroehr::party_binding::provisioned::{GroupClient, UserFilter};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::group_provision::{GroupState, GroupWriter};
use ferroehr::service::demographic::provision::NewUser;
use sqlx::PgPool;

/// The identifier issuer of the provisioned Users' binding keys.
const KEY_ISSUER: &str = "urn:ferroehr:scaleseed:scim-login";
/// The issuer of the Group's `externalId`.
const GROUP_ISSUER: &str = "urn:ferroehr:scaleseed:scim-groups";
/// The `PARTY_REF.namespace`.
const NAMESPACE: &str = "scaleseed";
/// The synthetic definitions: the professional `PERSON`, its `ROLE`, the
/// `GROUP` and the membership, and the interface id a membership carries.
const PERSON: &str = "openEHR-DEMOGRAPHIC-PERSON.scaleseed_professional.v1.0.0";
const ROLE: &str = "openEHR-DEMOGRAPHIC-ROLE.scaleseed_professional.v1.0.0";
const GROUP: &str = "openEHR-DEMOGRAPHIC-GROUP.scaleseed_team.v1.0.0";
const MEMBERSHIP_HRID: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.scaleseed_membership.v1.0.0";
const MEMBERSHIP: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.scaleseed_membership.v1";
/// The identifier element and the name element of each definition.
const ID_PATH: &str = "/details[id2]/items[id3]";
const NAME_PATH: &str = "/identities[id4]/details[id5]/items[id6]";

/// What one `groups` run builds and its budgets.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Target {
    /// The members the Group is built to, `[scim] max_group_members`.
    pub(crate) members: usize,
    /// The member removals and additions timed at the cap, each pair two
    /// changes.
    pub(crate) cycles: usize,
    /// The p99 budget of one change at the cap, in milliseconds.
    pub(crate) change_p99_ms: f64,
    /// The budget of the bytes one change stores at the cap.
    pub(crate) change_bytes: i64,
    /// The budget of the bytes building the Group stores.
    pub(crate) build_bytes: i64,
}

/// What a run measured.
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct Figures {
    /// The members the Group was built to.
    members: usize,
    /// The changes timed at the cap.
    timed_changes: usize,
    /// The p99 of a change at the cap, in milliseconds.
    change_p99_ms: f64,
    /// The p50 of a change at the cap, in milliseconds.
    change_p50_ms: f64,
    /// The p99 of the build's changes, in milliseconds.
    build_p99_ms: f64,
    /// The bytes the last change at the cap stored in its rows.
    change_bytes: i64,
    /// The database's growth while the Group was built.
    build_bytes: i64,
    /// The `version` and `node` row bytes every version of the Group holds.
    build_row_bytes: i64,
    /// The side-table rows one change at the cap wrote.
    side_table_rows_per_change: i64,
    /// How long the build took, in seconds.
    build_seconds: f64,
    /// The budgets.
    budget_change_p99_ms: f64,
    budget_change_bytes: i64,
    budget_build_bytes: i64,
}

impl Figures {
    /// The budgets this run missed, each named.
    fn misses(&self) -> Vec<String> {
        let mut misses = Vec::new();
        if self.change_p99_ms > self.budget_change_p99_ms {
            misses.push(format!(
                "a change at the cap takes {:.1} ms at p99, over {} ms",
                self.change_p99_ms, self.budget_change_p99_ms
            ));
        }
        if self.change_bytes > self.budget_change_bytes {
            misses.push(format!(
                "a change at the cap stores {} bytes, over {}",
                self.change_bytes, self.budget_change_bytes
            ));
        }
        if self.build_bytes > self.budget_build_bytes {
            misses.push(format!(
                "building the Group stores {} bytes, over {}",
                self.build_bytes, self.budget_build_bytes
            ));
        }
        if self.side_table_rows_per_change != 1 {
            misses.push(format!(
                "a change writes {} side-table rows, not one",
                self.side_table_rows_per_change
            ));
        }
        misses
    }
}

/// Runs [`run`] and reports where its report is and whether every budget
/// holds.
///
/// # Errors
/// The [`run`] errors.
pub(crate) async fn main(pool: &PgPool, target: Target, out: &Path) -> anyhow::Result<()> {
    let result = Box::pin(run(pool, target, out)).await;
    eprintln!(
        "scaleseed: the Group report is written to {}",
        out.join("report.md").display()
    );
    let figures = result?;
    eprintln!(
        "scaleseed: every budget holds, a change at the cap at {:.1} ms p99",
        figures.change_p99_ms
    );
    Ok(())
}

/// Builds the Group, measures it, writes `report.json` and `report.md` to
/// `out`, and fails when a budget is missed.
///
/// # Errors
/// When the store already holds this run's Users, a write or a statement
/// fails, the report cannot be written, or a budget is missed.
async fn run(pool: &PgPool, target: Target, out: &Path) -> anyhow::Result<Figures> {
    let svc = service(pool).await?;
    let client = GroupClient {
        binding_issuer: KEY_ISSUER,
        group_issuer: GROUP_ISSUER,
    };
    let held = svc
        .provisioned_users(KEY_ISSUER, &UserFilter::default(), 0, 1)
        .await
        .context("count the provisioned Users")?;
    if held.total > 0 {
        bail!("the store already holds a groups run; run it on a fresh database");
    }
    let members = provision(&svc, target.members).await?;
    let writer = GroupWriter {
        client,
        namespace: NAMESPACE,
        max_members: target.members,
    };
    let state = |members: &[VoId]| GroupState {
        display_name: "scaleseed team".to_owned(),
        external_id: Some("scaleseed-team".to_owned()),
        members: members.to_vec(),
    };
    let before = database_bytes(pool).await?;
    let started = Instant::now();
    let group = svc
        .provision_group(writer, state(&[]))
        .await
        .context("create the Group")?;
    let id = group.row.party;
    let mut build = Vec::with_capacity(target.members);
    let mut joined = Vec::with_capacity(target.members);
    for (held, member) in members.iter().enumerate() {
        joined.push(*member);
        let held = held + 1;
        let timed = Instant::now();
        changed(svc.update_group(writer, id, state(&joined), None).await)?;
        build.push(timed.elapsed().as_secs_f64() * 1000.0);
        if held % 500 == 0 {
            eprintln!("scaleseed: the Group holds {held} members");
        }
    }
    let build_seconds = started.elapsed().as_secs_f64();
    let build_bytes = database_bytes(pool).await? - before;
    let build_row_bytes = row_bytes(pool, id, None).await?;

    let mut changes = Vec::with_capacity(target.cycles * 2);
    let mut side_table_rows_per_change = 0;
    let mut last = None;
    for member in members.iter().copied().cycle().take(target.cycles) {
        last = Some(member);
        let without: Vec<VoId> = members.iter().copied().filter(|m| *m != member).collect();
        let timed = Instant::now();
        changed(svc.update_group(writer, id, state(&without), None).await)?;
        changes.push(timed.elapsed().as_secs_f64() * 1000.0);
        let rows = membership_rows(pool, id).await?;
        let timed = Instant::now();
        changed(svc.update_group(writer, id, state(&members), None).await)?;
        changes.push(timed.elapsed().as_secs_f64() * 1000.0);
        side_table_rows_per_change = membership_rows(pool, id).await? - rows;
    }
    let change_bytes = match last {
        Some(member) => row_bytes(pool, member, Some(head(pool, member).await?)).await?,
        None => 0,
    };
    let figures = Figures {
        members: target.members,
        timed_changes: changes.len(),
        change_p99_ms: percentile(&mut changes, 99),
        change_p50_ms: percentile(&mut changes, 50),
        build_p99_ms: percentile(&mut build, 99),
        change_bytes,
        build_bytes,
        build_row_bytes,
        side_table_rows_per_change,
        build_seconds,
        budget_change_p99_ms: target.change_p99_ms,
        budget_change_bytes: target.change_bytes,
        budget_build_bytes: target.build_bytes,
    };
    report(&figures, out)?;
    let misses = figures.misses();
    if !misses.is_empty() {
        bail!("the Group missed its budgets: {}", misses.join("; "));
    }
    Ok(figures)
}

/// Refuses a change that commits nothing: every timed change moves a member.
fn changed<T>(outcome: Result<Option<T>, ferroehr::error::ServiceError>) -> anyhow::Result<()> {
    match outcome.context("change the Group")? {
        Some(_) => Ok(()),
        None => bail!("the Group is no longer found"),
    }
}

/// A service over `pool` holding the synthetic definitions, with the party
/// mapping installed.
async fn service(pool: &PgPool) -> anyhow::Result<FerroEhrService> {
    let selector = |kind: &str| Selector {
        path: ID_PATH.to_owned(),
        identifier_type: kind.to_owned(),
    };
    let binding = PartyBindingConfig {
        namespace: NAMESPACE.to_owned(),
        person: Some(PersonSection {
            template: vec![PersonTemplate {
                template_id: PERSON.to_owned(),
                key: selector("guid"),
                user_name: Some(selector("scim_user_name")),
                external_id: None,
                default: true,
            }],
            attribute: Vec::new(),
        }),
        professional_role: Some(RoleSection {
            template_id: ROLE.to_owned(),
        }),
        group: Some(GroupSection {
            template_id: GROUP.to_owned(),
            external_id: selector("group_external_id"),
            display_name: Some(NAME_PATH.to_owned()),
        }),
        membership: Some(MembershipSection {
            archetype_id: MEMBERSHIP.to_owned(),
        }),
        ..PartyBindingConfig::default()
    };
    let sections = binding.sections();
    let mapping = sections
        .binding_mapping(&[KEY_ISSUER.to_owned()], "ADMIN")
        .context("compile the party mapping")?;
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping);
    for adl in [
        archetype(PERSON, "PERSON"),
        archetype(ROLE, "ROLE"),
        archetype(GROUP, "GROUP"),
        membership(),
    ] {
        svc.template_adl2_upload(&adl)
            .await
            .context("upload a synthetic definition")?;
    }
    Ok(svc)
}

/// Provisions `count` Users; their `PERSON`s' containers.
async fn provision(svc: &FerroEhrService, count: usize) -> anyhow::Result<Vec<VoId>> {
    let mut members = Vec::with_capacity(count);
    for ordinal in 0..count {
        let key = format!("{ordinal:08x}-5c1e-4000-8000-000000005015");
        let mut values = Vec::with_capacity(2);
        for (text, kind) in [
            (key.clone(), "guid"),
            (format!("user{ordinal}"), "scim_user_name"),
        ] {
            values.push((
                ID_PATH.to_owned(),
                svc.mapped_text_value(PERSON, ID_PATH, &text, (KEY_ISSUER, kind))
                    .await
                    .context("map a User's identifier")?,
            ));
        }
        let user = NewUser {
            template_id: PERSON.to_owned(),
            namespace: NAMESPACE.to_owned(),
            values,
        };
        let created = svc
            .provision_user(KEY_ISSUER, ROLE, user)
            .await
            .context("provision a User")?;
        members.push(created.row.party);
        if (ordinal + 1) % 1000 == 0 {
            eprintln!("scaleseed: {} Users provisioned", ordinal + 1);
        }
    }
    Ok(members)
}

/// The size of the database, in bytes.
async fn database_bytes(pool: &PgPool) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT pg_database_size(current_database())")
        .fetch_one(pool)
        .await
        .context("read the database size")
}

/// The `version` and `node` row bytes of the party `id`, of its version
/// `sys_version` alone when given.
async fn row_bytes(pool: &PgPool, id: VoId, sys_version: Option<i32>) -> anyhow::Result<i64> {
    sqlx::query_scalar(
        "SELECT (SELECT coalesce(sum(pg_column_size(v.*)), 0) FROM party.version v \
                 WHERE v.vo_id = $1 AND ($2::int4 IS NULL OR v.sys_version = $2))::int8 \
              + (SELECT coalesce(sum(pg_column_size(n.*)), 0) FROM party.node n \
                 WHERE n.vo_id = $1 AND ($2::int4 IS NULL OR n.sys_version = $2))::int8",
    )
    .bind(id)
    .bind(sys_version)
    .fetch_one(pool)
    .await
    .context("weigh the party's rows")
}

/// The `sys_version` of the head of the party `id`.
async fn head(pool: &PgPool, id: VoId) -> anyhow::Result<i32> {
    sqlx::query_scalar("SELECT max(sys_version) FROM party.version WHERE vo_id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .context("read the party's head")
}

/// Every `binding_membership` row of the `GROUP` `id`, retired ones
/// included: their growth is the rows a change wrote.
async fn membership_rows(pool: &PgPool, id: VoId) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT count(*) FROM party.binding_membership WHERE group_id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .context("count the Group's side-table rows")
}

/// The `pct` percentile of `samples`, by nearest rank: the smallest value
/// at or above `pct` % of them.
fn percentile(samples: &mut [f64], pct: usize) -> f64 {
    samples.sort_unstable_by(f64::total_cmp);
    let rank = samples.len().saturating_mul(pct).div_ceil(100);
    samples.get(rank.saturating_sub(1)).copied().unwrap_or(0.0)
}

/// Writes `report.json` and `report.md` to `out`.
fn report(figures: &Figures, out: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out).context("create the report directory")?;
    std::fs::write(
        out.join("report.json"),
        serde_json::to_string_pretty(figures).context("serialise the figures")?,
    )
    .context("write report.json")?;
    let verdict = |within: bool| if within { "within" } else { "**over**" };
    let mib = |bytes: i64| {
        let kib = f64::from(u32::try_from(bytes.max(0) >> 10).unwrap_or(u32::MAX));
        format!("{bytes} bytes ({:.1} MiB)", kib / 1024.0)
    };
    let markdown = format!(
        "## A {members}-member SCIM Group (#5015)\n\n\
         Built one member per change in {build:.0} s, then {timed} changes timed at the cap.\n\n\
         | Measure | Value | Budget | |\n|---|---|---|---|\n\
         | A change at the cap, p99 | {p99:.1} ms (p50 {p50:.1} ms) | {bp99} ms | {v1} |\n\
         | Bytes one change stores at the cap | {cb} | {bcb} | {v2} |\n\
         | Database growth while building | {bb} | {bbb} | {v3} |\n\
         | Row bytes of the GROUP's versions | {rb} | | |\n\
         | Side-table rows per change | {rows} | 1 | {v4} |\n\
         | A change while building, p99 | {build_p99:.1} ms | | |\n",
        members = figures.members,
        build = figures.build_seconds,
        timed = figures.timed_changes,
        p99 = figures.change_p99_ms,
        p50 = figures.change_p50_ms,
        bp99 = figures.budget_change_p99_ms,
        v1 = verdict(figures.change_p99_ms <= figures.budget_change_p99_ms),
        cb = mib(figures.change_bytes),
        bcb = mib(figures.budget_change_bytes),
        v2 = verdict(figures.change_bytes <= figures.budget_change_bytes),
        bb = mib(figures.build_bytes),
        bbb = mib(figures.budget_build_bytes),
        v3 = verdict(figures.build_bytes <= figures.budget_build_bytes),
        rb = mib(figures.build_row_bytes),
        rows = figures.side_table_rows_per_change,
        v4 = verdict(figures.side_table_rows_per_change == 1),
        build_p99 = figures.build_p99_ms,
    );
    std::fs::write(out.join("report.md"), markdown).context("write report.md")?;
    Ok(())
}

/// A synthetic ADL 2 archetype `hrid` rooted at `root`, with a name in
/// `identities` and repeatable identifiers in `details`.
fn archetype(hrid: &str, root: &str) -> String {
    format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)
    {hrid}

language
    original_language = <[ISO_639-1::en]>

description
    original_author = <[\"name\"] = <\"FerroEHR scaleseed\">>
    lifecycle_state = <\"published\">
    details = <
        [\"en\"] = <
            language = <[ISO_639-1::en]>
            purpose = <\"A party the SCIM group measurement writes.\">
        >
    >

definition
    {root}[id1] matches {{
        identities matches {{
            PARTY_IDENTITY[id4] matches {{
                details matches {{
                    ITEM_TREE[id5] matches {{
                        items matches {{
                            ELEMENT[id6] occurrences matches {{0..1}} matches {{
                                value matches {{ DV_TEXT[id8] }}
                            }}
                        }}
                    }}
                }}
            }}
        }}
        details matches {{
            ITEM_TREE[id2] matches {{
                items matches {{
                    ELEMENT[id3] occurrences matches {{0..*}} matches {{
                        value matches {{ DV_IDENTIFIER[id9] }}
                    }}
                }}
            }}
        }}
    }}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Root\"> description = <\"The root.\">>
            [\"id2\"] = <text = <\"Details\"> description = <\"The details.\">>
            [\"id3\"] = <text = <\"Identifier\"> description = <\"An identifier.\">>
            [\"id4\"] = <text = <\"Identity\"> description = <\"The identity.\">>
            [\"id5\"] = <text = <\"Name\"> description = <\"The name structure.\">>
            [\"id6\"] = <text = <\"Name text\"> description = <\"The name.\">>
        >
    >
"
    )
}

/// The synthetic membership archetype: a `PARTY_RELATIONSHIP` with optional
/// details.
fn membership() -> String {
    format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)
    {MEMBERSHIP_HRID}

language
    original_language = <[ISO_639-1::en]>

description
    original_author = <[\"name\"] = <\"FerroEHR scaleseed\">>
    lifecycle_state = <\"published\">
    details = <
        [\"en\"] = <
            language = <[ISO_639-1::en]>
            purpose = <\"A member of a group.\">
        >
    >

definition
    PARTY_RELATIONSHIP[id1] matches {{
        details matches {{
            ITEM_TREE[id2] occurrences matches {{0..1}}
        }}
    }}

terminology
    term_definitions = <
        [\"en\"] = <
            [\"id1\"] = <text = <\"Membership\"> description = <\"A membership.\">>
            [\"id2\"] = <text = <\"Details\"> description = <\"The details.\">>
        >
    >
"
    )
}
