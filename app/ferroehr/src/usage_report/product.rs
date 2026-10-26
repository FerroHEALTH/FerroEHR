// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What FerroEHR contributes to the report: its `status`, the request groups
//! of its API tree, the identity of the build, the boot notice, and the
//! preview `ferroehr usage-report --print` renders.
//!
//! No openEHR spec governs this: our own design. The envelope is `FerroPULSE`'s
//! report v2, which `ferrohealth-usage-report` assembles and sends.

use std::sync::Arc;
use std::time::Instant;

use ferrohealth_health::document::State;
use ferrohealth_health::registry::HealthIndicator as _;
use ferrohealth_usage_report::envelope::{Key, LicenceGrant, Product, Report, report_git_sha};
use ferrohealth_usage_report::reporter::{Identity, Notice, ProductSection, Reporter};
use ferrohealth_usage_report::section::UsageReportSection;
use ferrohealth_usage_report::window::Window;
use http::Method;
use serde::Serialize;
use sqlx::Connection as _;
use sqlx::PgPool;

use crate::db::acquire::acquire;
use crate::licence::document::Use;
use crate::licence::state::LicenceState;
use crate::telemetry::build_info::BuildInfo;
use crate::telemetry::indicators::{DbHealth, MigrationsHealth};
use crate::usage_report::store::{self, PgStore, StoreError};

/// The book page that carries the full notice.
pub const NOTICE_URL: &str = "https://ferroehr.eu/docs/latest/usage-report.html";

/// The boot line's facts about the report.
pub const NOTICE: Notice = Notice {
    contents: "instance id, version, licence type, deployment, uptime, host size and coarse \
               request latency per route group",
    switch: "FERROEHR__USAGE_REPORT__ENABLED=false",
    state_dir_variable: "FERROEHR__USAGE_REPORT__STATE_DIR",
    url: NOTICE_URL,
};

/// FerroEHR's `status` object, carried by both events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Status {
    /// Whether the database answered.
    pub db_ok: bool,
    /// Whether the schema carries the migrations.
    pub migrations_ok: bool,
    /// The `PostgreSQL` major version, absent when the database did not
    /// answer the question.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub postgres_major: Option<u16>,
}

/// FerroEHR as a reporting product, reading its state from the clinical pool.
#[derive(Debug, Clone)]
pub struct FerroEhr {
    pool: PgPool,
}

impl FerroEhr {
    /// The product over the clinical pool `pool`.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl ProductSection for FerroEhr {
    type Status = Status;

    const PRODUCT: Product = Product::Ehr;

    async fn status(&self) -> Status {
        let db_ok = DbHealth::new(self.pool.clone()).check().await.state() == State::Up;
        let migrations_ok = MigrationsHealth::new(self.pool.clone())
            .check()
            .await
            .state()
            == State::Up;
        // NOTE: no specification governs this: our own design; `db_ok` already
        // reports the failure, and the major is left out rather than guessed.
        let postgres_major = store::postgres_major(&self.pool).await.ok();
        Status {
            db_ok,
            migrations_ok,
            postgres_major,
        }
    }
}

/// A route-template group of the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RouteGroup {
    /// EHR and `EHR_STATUS` endpoints.
    Ehr,
    /// COMPOSITION endpoints.
    Composition,
    /// CONTRIBUTION endpoints.
    Contribution,
    /// Query execution endpoints.
    Query,
    /// Definition endpoints.
    Definition,
    /// DIRECTORY endpoints.
    Directory,
    /// Administrative endpoints.
    Admin,
    /// The demographic, message, terminology and FHIR families.
    Other,
}

impl RouteGroup {
    /// Returns the group a matched route template belongs to, or `None` for a
    /// route outside the API families (the health family, the management
    /// surface, the status and discovery documents).
    ///
    /// The first literal segment naming an API family decides (`admin`,
    /// `definition`, `query`, `ehr`, or a family the report folds into
    /// `other`); under `ehr` the resource segment refines it. Parameter
    /// segments (`{ehr_id}`) are never read.
    #[must_use]
    pub fn of_template(template: &str) -> Option<Self> {
        let mut segments = template
            .split('/')
            .filter(|segment| !segment.is_empty() && !segment.starts_with('{'));
        while let Some(segment) = segments.next() {
            match segment {
                "admin" => return Some(Self::Admin),
                "definition" => return Some(Self::Definition),
                "query" => return Some(Self::Query),
                "ehr" => {
                    return Some(
                        segments
                            .find_map(|resource| match resource {
                                "composition" | "versioned_composition" => Some(Self::Composition),
                                "contribution" => Some(Self::Contribution),
                                "directory" => Some(Self::Directory),
                                _ => None,
                            })
                            .unwrap_or(Self::Ehr),
                    );
                }
                "demographic" | "message" | "terminology" | "fhir" => return Some(Self::Other),
                _ => {}
            }
        }
        None
    }

    /// Returns the group's name in the report.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ehr => "ehr",
            Self::Composition => "composition",
            Self::Contribution => "contribution",
            Self::Query => "query",
            Self::Definition => "definition",
            Self::Directory => "directory",
            Self::Admin => "admin",
            Self::Other => "other",
        }
    }
}

/// The window's grouping: the [`RouteGroup`] of a matched route template, and
/// nothing for a request no route matched.
#[must_use]
pub fn grouping(_method: &Method, route: Option<&str>) -> Option<Key> {
    route
        .and_then(RouteGroup::of_template)
        .and_then(|group| Key::new(group.as_str()))
}

/// A latency window over FerroEHR's grouping, for the request log to fill.
#[must_use]
pub fn window() -> Arc<Window> {
    Arc::new(Window::new(grouping))
}

/// The licence grant type a licence state reports.
///
/// A build with no licence in force runs under what BUSL-1.1 grants everyone,
/// which is the non-commercial grant.
#[must_use]
pub fn licence_grant(licence: &LicenceState) -> LicenceGrant {
    match licence {
        LicenceState::Licensed { verified, .. } => match verified.licence.permitted_use {
            Use::Commercial => LicenceGrant::Commercial,
            Use::NonCommercial => LicenceGrant::NonCommercial,
        },
        LicenceState::NoLicence(_) => LicenceGrant::NonCommercial,
    }
}

/// The identity a report names: the build's version and commit, and the
/// licence grant in force.
#[must_use]
pub fn identity(build: &BuildInfo, licence: &LicenceState) -> Identity {
    Identity {
        version: build.version.to_owned(),
        git_sha: report_git_sha(build.git_sha),
        licence: licence_grant(licence),
    }
}

/// Which report a preview renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// The start report.
    Start,
    /// The daily report.
    Daily,
}

/// A report assembled for display, and whether its instance id is the stored
/// one.
#[derive(Debug)]
pub struct Preview {
    /// The report, exactly as a send would render it now.
    pub report: Report<Status>,
    /// False when the database holds no instance id yet, so the one shown is a
    /// sample; the first start creates the real one.
    pub instance_stored: bool,
}

/// Why a preview could not be assembled.
#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    /// The database state could not be read.
    #[error("usage report database state")]
    Store(#[from] StoreError),
    /// The reporter could not be built.
    #[error("usage report client")]
    Report(#[from] ferrohealth_usage_report::reporter::ReportError),
}

/// Assembles the report `kind` exactly as a send would now, writing nothing.
///
/// The instance id is the stored one; when none exists yet it is created
/// inside a transaction that is rolled back, so the one shown is a sample. A
/// daily report holds every replica's stored counts and the counts `live`
/// holds, which stay in `live`.
///
/// # Errors
/// [`PreviewError::Store`] when the database cannot be read, and
/// [`PreviewError::Report`] when the HTTP client cannot be built.
pub async fn preview(
    section: &UsageReportSection,
    identity: Identity,
    pool: &PgPool,
    live: &Window,
    kind: EventKind,
) -> Result<Preview, PreviewError> {
    let instance = {
        let mut conn = acquire(pool).await.map_err(StoreError::from)?;
        let mut tx = conn.begin().await.map_err(StoreError::from)?;
        let instance = store::ensure_instance_on(&mut tx).await?;
        tx.rollback().await.map_err(StoreError::from)?;
        instance
    };
    let shown = window();
    if kind == EventKind::Daily {
        let store = PgStore::new(pool.clone());
        shown.restore(&store.peek().await?);
        let counts = live.take();
        live.restore(&counts);
        shown.restore(&counts);
    }
    let reporter = Reporter::with_store(
        section,
        identity,
        (instance, Arc::new(PgStore::new(pool.clone()))),
        Arc::new(FerroEhr::new(pool.clone())),
        shown,
        Instant::now(),
    )?;
    let report = match kind {
        EventKind::Start => reporter.start_report().await,
        EventKind::Daily => reporter.daily_report().await,
    };
    Ok(Preview {
        report,
        instance_stored: !instance.created,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_map_to_their_group() {
        let base = "/ferroehr/rest/openehr/v1";
        let cases = [
            ("/ehr", RouteGroup::Ehr),
            ("/ehr/{ehr_id}/ehr_status/{version_uid}", RouteGroup::Ehr),
            (
                "/ehr/{ehr_id}/composition/{uid_based_id}",
                RouteGroup::Composition,
            ),
            (
                "/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version",
                RouteGroup::Composition,
            ),
            (
                "/ehr/{ehr_id}/contribution/{contribution_uid}",
                RouteGroup::Contribution,
            ),
            ("/ehr/{ehr_id}/directory", RouteGroup::Directory),
            ("/query/aql", RouteGroup::Query),
            ("/query/{qualified_query_name}", RouteGroup::Query),
            (
                "/definition/query/{qualified_query_name}",
                RouteGroup::Definition,
            ),
            ("/definition/template/adl1.4", RouteGroup::Definition),
            ("/admin/ehr/{ehr_id}", RouteGroup::Admin),
            (
                "/admin/query/{qualified_query_name}/{version}",
                RouteGroup::Admin,
            ),
            ("/demographic/person/{uid_based_id}", RouteGroup::Other),
            ("/demographic/contribution", RouteGroup::Other),
            ("/terminology/{terminology_id}", RouteGroup::Other),
            ("/message/export/{ehr_id}", RouteGroup::Other),
        ];
        for (path, group) in cases {
            assert_eq!(RouteGroup::of_template(path), Some(group), "{path}");
            assert_eq!(
                RouteGroup::of_template(&format!("{base}{path}")),
                Some(group),
                "{base}{path}"
            );
        }
    }

    #[test]
    fn routes_outside_the_api_families_are_not_counted() {
        for template in [
            "/health/readiness",
            "/management/prometheus",
            "/ferroehr/rest/status",
            "/ferroehr/rest/.well-known/smart-configuration",
            "/ferroehr/rest/api-docs/openapi.json",
            "unmatched",
            "",
        ] {
            assert_eq!(RouteGroup::of_template(template), None, "{template}");
        }
        assert_eq!(grouping(&Method::GET, None), None);
    }

    #[test]
    fn a_parameter_segment_never_decides_the_group() {
        assert_eq!(
            RouteGroup::of_template("/ehr/{admin}"),
            Some(RouteGroup::Ehr)
        );
        assert_eq!(RouteGroup::of_template("/{query}/x"), None);
    }

    #[test]
    fn every_group_name_is_an_envelope_key() {
        for group in [
            RouteGroup::Ehr,
            RouteGroup::Composition,
            RouteGroup::Contribution,
            RouteGroup::Query,
            RouteGroup::Definition,
            RouteGroup::Directory,
            RouteGroup::Admin,
            RouteGroup::Other,
        ] {
            assert!(Key::new(group.as_str()).is_some(), "{group:?}");
        }
    }
}
