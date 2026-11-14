// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The projection of each EHR's `EHR_ACCESS` settings: the facts the access
//! gate and the access evaluator read, written in the transaction that
//! commits the version and rebuilt by a refill.
//!
//! `EHR_ACCESS` is the access-decision authority of an EHR (RM ehr
//! `org.openehr.rm.ehr.ehr_access.adoc` §`EHR_ACCESS` Class), and every change
//! to it is a version in a contribution (RM common
//! `master06-change_control_package.adoc` §Committal and Audits). The commit
//! of a version therefore replaces the EHR's row
//! ([`crate::storage::access_projection`]): its settings, their class and
//! their facts ([`crate::access_eval::class`]). Settings that do not read as
//! the `access_directives` scheme project as held closed, with no class. A
//! change to an EHR's settings
//! notifies every replica when it commits, so each drops its copy
//! (`service::ehr::access::EhrAccessCache`). No openEHR spec governs
//! the projection: our own design (#5509).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use openehr_rm::draft::access_control::terminology::Term as _;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::quantity::dv_interval::DvInterval;
use serde_json::Value;
use sqlx::PgConnection;

use crate::access_eval::class::classify;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::storage::access_projection::{self as store, Classed, Rows, Source};
use crate::storage::version_repo::ehr_access::{self as heads, CurrentAccess};
use crate::versioning::Kind;
use crate::versioning::change::{Change, Committed};

/// One bound of a [`Validity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Bound {
    /// The instant.
    pub at: jiff::Timestamp,
    /// Whether the instant itself is inside.
    pub included: bool,
}

/// The instants a directive is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validity {
    /// From and until the bounds; an absent bound is unbounded.
    Within {
        /// The lower bound.
        from: Option<Bound>,
        /// The upper bound.
        until: Option<Bound>,
    },
    /// A bound is not a full date and time with an offset, so no instant
    /// can be judged against it.
    Unreadable,
}

/// The instants `interval` holds.
pub(crate) fn validity(interval: &DvInterval<DvDateTime>) -> Validity {
    // NOTE: our own design, a bound without a full date, time and offset names no instant.
    let bound = |unbounded: bool, at: Option<&DvDateTime>, included: bool| match at {
        Some(at) if !unbounded => at
            .value
            .parse::<jiff::Timestamp>()
            .ok()
            .map(|at| Some(Bound { at, included })),
        _ => Some(None),
    };
    match (
        bound(
            interval.lower_unbounded,
            interval.lower.as_ref(),
            interval.lower_included,
        ),
        bound(
            interval.upper_unbounded,
            interval.upper.as_ref(),
            interval.upper_included,
        ),
    ) {
        (Some(from), Some(until)) => Validity::Within { from, until },
        _ => Validity::Unreadable,
    }
}

/// The rows the settings of `access`, an `EHR_ACCESS` in canonical JSON,
/// project as.
fn rows_of(access: Option<&Value>) -> Rows {
    let Some((access, settings)) = access.and_then(|access| {
        access
            .get("settings")
            .filter(|settings| !settings.is_null())
            .map(|settings| (access, settings))
    }) else {
        return Rows::None;
    };
    let read = match EhrAccessSettings::from_ehr_access(access) {
        Ok(Some(EhrAccessSettings::Directives(read))) => *read,
        Ok(_) => return held_closed(),
        Err(error) => return unprojected(&error),
    };
    let classified = match classify(&read) {
        Ok(classified) => classified,
        Err(error) => return unprojected(&error),
    };
    Rows::Settings(store::SettingsRow {
        default_access: EhrAccessSettings::Directives(Box::new(read))
            .default_access()
            .code(),
        settings: Some(Classed {
            settings: settings.to_string(),
            digest: classified.digest,
            class: classified.class.to_string(),
            facts: Value::Array(classified.facts).to_string(),
        }),
    })
}

/// The rows of settings that do not project, logged with `error`.
fn unprojected(error: &(dyn std::error::Error + 'static)) -> Rows {
    tracing::error!(
        error,
        "EHR_ACCESS settings do not project; the EHR is held closed"
    );
    held_closed()
}

/// The rows of an EHR held closed.
fn held_closed() -> Rows {
    Rows::Settings(store::SettingsRow {
        default_access: EhrAccessSettings::HeldClosed.default_access().code(),
        settings: None,
    })
}

/// What a commit of an `EHR_ACCESS` version projects, captured from its
/// change before the change is written.
#[derive(Debug)]
pub(crate) enum AccessCommit {
    /// The version's `EHR_ACCESS` body; `None` for a logical deletion.
    Body(Option<Value>),
}

impl AccessCommit {
    /// What `change` projects; `None` for a change of another kind and for
    /// the first version of an `EHR_ACCESS` with no settings, which replaces
    /// no row.
    pub(crate) fn of(change: &Change) -> Option<Self> {
        let has_settings =
            |canonical: &Value| canonical.get("settings").is_some_and(|s| !s.is_null());
        match change {
            Change::Create {
                kind: Kind::EhrAccess,
                canonical,
                ..
            } => has_settings(canonical).then(|| Self::Body(Some(canonical.clone()))),
            Change::Modify {
                kind: Kind::EhrAccess,
                canonical,
                ..
            } => Some(Self::Body(Some(canonical.clone()))),
            Change::Delete {
                kind: Kind::EhrAccess,
                ..
            } => Some(Self::Body(None)),
            _ => None,
        }
    }

    /// Replaces the rows of `ehr_id` with the projection of the committed
    /// version, on the commit's transaction.
    ///
    /// # Errors
    /// The storage error of the write.
    pub(crate) async fn project(
        self,
        tx: &mut PgConnection,
        ehr_id: EhrId,
        committed: &Committed,
    ) -> Result<(), ServiceError> {
        let Self::Body(body) = self;
        let source = Source {
            ehr_id,
            vo_id: committed.vo_id,
            sys_version: committed.sys_version,
        };
        store::replace(tx, source, &rows_of(body.as_ref())).await?;
        Ok(())
    }
}

/// Replaces the rows of `access` with the projection of its body.
///
/// # Errors
/// [`ServiceError`] when the stored body is not JSON, and the storage error
/// of the write.
async fn project_stored(tx: &mut PgConnection, access: CurrentAccess) -> Result<(), ServiceError> {
    let body = access
        .body
        .as_deref()
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|e| ServiceError::internal("read a stored EHR_ACCESS body", e))?;
    let source = Source {
        ehr_id: access.ehr_id,
        vo_id: access.vo_id,
        sys_version: access.sys_version,
    };
    store::replace(tx, source, &rows_of(body.as_ref())).await?;
    Ok(())
}

/// Replaces the rows of `ehr_id` with the projection of its current
/// `EHR_ACCESS` version, on `tx`: the path of a write that lands versions
/// without the commit's projection, such as an import.
///
/// # Errors
/// The storage errors of the read and the write.
pub(crate) async fn project_current(
    tx: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<(), ServiceError> {
    match heads::current_ehr_access(tx, ehr_id).await? {
        Some(access) => project_stored(tx, access).await,
        None => Ok(()),
    }
}

/// Which rebuild [`FerroEhrService::rebuild_ehr_access_projection`] makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rebuild {
    /// Fill the projection once: nothing when it was filled before.
    Fill,
    /// Fill it again from the stored versions.
    Refill,
}

impl FerroEhrService {
    /// Rebuilds the projection from every EHR's current `EHR_ACCESS`
    /// version and drops every replica's copy of the settings: a
    /// [`Rebuild::Fill`] only when it was never filled (the first boot after
    /// the projection's migration), a [`Rebuild::Refill`] always. It holds the
    /// lock that orders it with every commit's write of the rows.
    ///
    /// # Errors
    /// The storage errors of the reads and the writes.
    pub async fn rebuild_ehr_access_projection(
        &self,
        rebuild: Rebuild,
    ) -> Result<(), ServiceError> {
        let mut conn = crate::db::acquire::acquire(&self.pool).await?;
        let mut tx = sqlx::Connection::begin(&mut *conn).await?;
        store::lock(&mut tx).await?;
        if rebuild == Rebuild::Fill && store::filled(&mut tx).await? {
            return Ok(());
        }
        store::clear(&mut tx).await?;
        let current = heads::every_current_with_settings(&mut tx).await?;
        let count = current.len();
        for access in current {
            project_stored(&mut tx, access).await?;
        }
        store::mark_filled(&mut tx).await?;
        tx.commit().await?;
        self.ehr_access.forget_every();
        tracing::info!(ehrs = count, "the EHR_ACCESS projection is filled");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::party_binding::policy_mark::TERMINOLOGY;

    /// A term of the access control groups.
    fn term(code: &str) -> Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
                    "code_string": code } })
    }

    /// A code of the jurisdiction's terminology `terminology`.
    fn foreign_term(terminology: &str, code: &str) -> Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                    "code_string": code } })
    }

    /// A directive of `kind` and `effect`, with `extra` members.
    fn directive(kind: &str, effect: &str, extra: &Value) -> Value {
        let mut d = json!({
            "_type": "ACCESS_DIRECTIVE",
            "name": { "_type": "DV_TEXT", "value": "Directive" },
            "archetype_node_id": "at0001",
            "kind": term(kind),
            "effect": term(effect),
            "emergency_override": false
        });
        if let (Some(d), Some(extra)) = (d.as_object_mut(), extra.as_object()) {
            d.extend(extra.clone());
        }
        d
    }

    /// Settings of `default` with `directives` and `labels`.
    fn settings(default: &str, directives: Vec<Value>, labels: Vec<Value>) -> Value {
        let mut s =
            json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term(default) });
        if !directives.is_empty() {
            s["directives"] = Value::Array(directives);
        }
        if !labels.is_empty() {
            s["labels"] = Value::Array(labels);
        }
        s
    }

    /// An `EHR_ACCESS` carrying `settings`.
    fn access(settings: Value) -> Value {
        let mut access = json!({ "_type": "EHR_ACCESS" });
        access["settings"] = settings;
        access
    }

    const DOCTOR: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f81";
    const NURSE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f82";

    /// A restriction from `from` with an exception for `party`, and a label
    /// on `party`'s composition.
    fn restricted(from: &str, party: &str) -> Value {
        let exception = directive(
            "consent",
            "permit",
            &json!({ "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                "party": { "_type": "PARTY_REF", "namespace": "local", "type": "PERSON",
                           "id": { "_type": "HIER_OBJECT_ID", "value": party } } }] }),
        );
        let restriction = directive(
            "restriction",
            "deny",
            &json!({
                "name": { "_type": "DV_TEXT", "value": format!("restriction from {from}") },
                "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE", "caller_kind": term("professional") }],
                "operations": [term("read"), term("query")],
                "data": [{ "_type": "ACCESS_DATA_SCOPE",
                           "categories": [foreign_term("EHDS", "mental-health")] }],
                "time_validity": { "_type": "DV_INTERVAL",
                    "lower": { "_type": "DV_DATE_TIME", "value": from },
                    "lower_unbounded": false, "upper_unbounded": true,
                    "lower_included": true, "upper_included": false },
                "exceptions": [exception]
            }),
        );
        let label = json!({ "_type": "ACCESS_LABEL",
            "target": { "_type": "OBJECT_REF", "namespace": "local", "type": "VERSIONED_COMPOSITION",
                        "id": { "_type": "HIER_OBJECT_ID", "value": party } },
            "sensitivities": [foreign_term("v3-ActCode", "HIV")] });
        access(settings("restricted", vec![restriction], vec![label]))
    }

    /// The settings of `access` as the projection stores them.
    fn classed(access: &Value) -> Classed {
        let Rows::Settings(store::SettingsRow {
            default_access: "restricted",
            settings: Some(classed),
        }) = rows_of(Some(access))
        else {
            panic!("the settings project a class");
        };
        classed
    }

    /// Two EHRs whose settings differ only in their instants, parties and
    /// names share one class; the facts hold what differs, depth-first.
    #[test]
    fn settings_that_differ_in_their_facts_share_a_class() {
        let first = classed(&restricted("2026-11-01T00:00:00Z", DOCTOR));
        let second = classed(&restricted("2026-12-24T08:30:00+01:00", NURSE));
        assert_eq!(first.digest, second.digest, "one class");
        assert_eq!(first.class, second.class);
        assert!(
            !first.class.contains(DOCTOR),
            "the class holds no party: {}",
            first.class
        );
        assert!(
            !first.class.contains("restriction from"),
            "the class holds no name"
        );
        let facts: Value = serde_json::from_str(&first.facts).expect("the facts are JSON");
        assert_eq!(facts, json!(["2026-11-01T00:00:00Z", DOCTOR, [DOCTOR]]));
        let facts: Value = serde_json::from_str(&second.facts).expect("the facts are JSON");
        assert_eq!(facts, json!(["2026-12-24T07:30:00Z", NURSE, [NURSE]]));
    }

    /// A bound that names no instant stays in the class, which the evaluator
    /// never reads as permissive, and is no fact.
    #[test]
    fn a_bound_with_no_offset_stays_in_the_class() {
        let deny = |upper: &str| {
            directive(
                "consent",
                "deny",
                &json!({ "time_validity": { "_type": "DV_INTERVAL",
                    "upper": { "_type": "DV_DATE_TIME", "value": upper },
                    "lower_unbounded": true, "upper_unbounded": false,
                    "lower_included": false, "upper_included": true } }),
            )
        };
        let class = |upper: &str| {
            let Rows::Settings(store::SettingsRow {
                settings: Some(classed),
                ..
            }) = rows_of(Some(&access(settings("open", vec![deny(upper)], vec![]))))
            else {
                panic!("the settings project a class");
            };
            classed
        };
        let unreadable = class("2026-11-01T00:00:00");
        assert!(unreadable.class.contains("2026-11-01T00:00:00"));
        assert_eq!(unreadable.facts, "[]");
        assert_ne!(
            unreadable.digest,
            class("2026-11-01T00:00:00Z").digest,
            "a readable bound makes another class"
        );
    }

    /// Settings that do not read hold the EHR closed, and no settings
    /// project no row.
    #[test]
    fn unreadable_settings_project_held_closed() {
        let bogus =
            json!({ "_type": "ACCESS_DIRECTIVE_SETTINGS", "default_access": term("bogus") });
        assert_eq!(
            rows_of(Some(&access(bogus))),
            held_closed(),
            "settings outside the scheme are held closed"
        );
        assert_eq!(rows_of(None), Rows::None);
        assert_eq!(rows_of(Some(&access(Value::Null))), Rows::None);
    }
}
