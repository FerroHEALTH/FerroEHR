// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The render policy of the audit store, and the FHIR `AuditEvent` of a
//! stored record.
//!
//! The store keeps a record's facts in columns and renders its FHIR R4
//! `AuditEvent` when the record is read or fed. The rendering also depends on
//! configuration: the audit source, the store horizon a record states as its
//! retention, the category map an access is classified through, and the
//! emergency purposes. [`RenderPolicy`] is that configuration. The store keeps
//! each distinct policy once (`audit.audit_render_policy`, keyed by the
//! SHA-256 of its text) and every record names the policy it was written
//! under, so a record renders the same document however the configuration
//! changes after it was written.
//!
//! The sender builds its own [`AuditContext`] and classifier from the same
//! policy ([`RenderPolicy::compile`]), so the document the drain would have
//! rendered and the one the store renders on read come from one source.
//!
//! NOTE: IHE BALP and ITI-20 define the content of the `AuditEvent`, not how
//! a repository stores it: storing columns and rendering on read is our own
//! design.

use serde::{Deserialize, Serialize};

use crate::system_log::AuditError;
use crate::system_log::categories::{CategoryMapConfig, Classifier};
#[cfg(feature = "fhir")]
use crate::system_log::categories::{ClassifiedContent, MapDigest};
use crate::system_log::config::{AuditConfig, Retention};
#[cfg(feature = "fhir")]
use crate::system_log::event::AuditEvent;
use crate::system_log::message::AuditContext;

/// What the FHIR `AuditEvent` of a record is rendered under beside the
/// record itself, as the store keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderPolicy {
    /// `[audit] source_id`: the audit source and FerroEHR's own agent.
    pub source_id: String,
    /// `[audit] enterprise_site_id`, empty when unset.
    pub enterprise_site_id: String,
    /// `[audit] server_host`, empty when unset.
    pub server_host: String,
    /// `[audit] value_if_missing`.
    pub value_if_missing: String,
    /// The store horizon, which a record outside the EHDS access record
    /// states as its retention.
    pub horizon: Retention,
    /// `[audit.categories]`: the map every access is classified through.
    pub categories: CategoryMapConfig,
    /// `[audit] emergency_purpose_codes`.
    pub emergency_purpose_codes: Vec<String>,
}

impl RenderPolicy {
    /// The policy `config` states.
    #[must_use]
    pub fn of(config: &AuditConfig) -> Self {
        Self {
            source_id: config.source_id.clone(),
            enterprise_site_id: config.enterprise_site_id.clone().unwrap_or_default(),
            server_host: config.server_host.clone().unwrap_or_default(),
            value_if_missing: config.value_if_missing.clone(),
            horizon: config.store.retention(),
            categories: config.categories.clone(),
            emergency_purpose_codes: config.emergency_purpose_codes.clone(),
        }
    }

    /// The context, classifier and emergency purposes this policy renders
    /// with.
    ///
    /// # Errors
    /// [`AuditError::CategoryMap`] when the category map does not compile,
    /// [`AuditError::Emergency`] for an empty emergency code, and
    /// [`AuditError::Policy`] when `ehds-logging` refuses the retention or the
    /// label policy; the boot validation refuses each of them first.
    pub fn compile(&self) -> Result<CompiledPolicy, AuditError> {
        let categories = self.categories.compile().map_err(AuditError::CategoryMap)?;
        let emergency = emergency_purposes(&self.emergency_purpose_codes)?;
        let context = AuditContext {
            source_id: self.source_id.clone(),
            enterprise_site_id: self.enterprise_site_id.clone(),
            server_ip: self.server_host.clone(),
            value_if_missing: self.value_if_missing.clone(),
            #[cfg(feature = "fhir")]
            recording: recording(self.horizon, &categories, &emergency)?,
        };
        Ok(CompiledPolicy {
            context,
            categories,
            emergency,
        })
    }
}

impl Default for RenderPolicy {
    fn default() -> Self {
        Self::of(&AuditConfig::default())
    }
}

/// A [`RenderPolicy`] ready to render with.
#[derive(Debug, Clone)]
pub struct CompiledPolicy {
    /// The context the FHIR and DICOM renderings take.
    pub context: AuditContext,
    /// The compiled category map.
    pub categories: Classifier,
    /// The purposes of use that mark an emergency access.
    pub emergency: ehds_logging::emergency::EmergencyPurposes,
}

/// The purposes of use `codes` declares, held by `ehds-logging`.
///
/// # Errors
/// [`AuditError::Emergency`] for an empty code; boot validation refuses one
/// first.
fn emergency_purposes(
    codes: &[String],
) -> Result<ehds_logging::emergency::EmergencyPurposes, AuditError> {
    let purposes: Vec<ehds_logging::record::Purpose> = codes
        .iter()
        .map(|code| ehds_logging::record::Purpose {
            system: None,
            code: code.clone(),
        })
        .collect();
    ehds_logging::emergency::EmergencyPurposes::declare(&purposes).map_err(AuditError::Emergency)
}

/// The policies the EHDS access record is written under: the store's horizon
/// as the record's retention, the categories' and origins' labels, and the
/// emergency purposes.
///
/// The record states the horizon as configured: years, days, or without end
/// for `retention_days = 0`. A horizon under the three years of EHDS Art. 9(2)
/// passes the boot validation only where no Union jurisdiction is in force,
/// so its record names the deployment's own retention as the ground of its
/// floor, never Art. 9(2).
///
/// # Errors
/// [`AuditError::Policy`] when `ehds-logging` refuses the retention or the
/// label policy, which a validated configuration does not cause.
#[cfg(feature = "fhir")]
fn recording(
    horizon: Retention,
    categories: &Classifier,
    emergency: &ehds_logging::emergency::EmergencyPurposes,
) -> Result<crate::system_log::fhir::Recording, AuditError> {
    use ehds_logging::retention::{Bounds, FLOOR_YEARS, Period, RetentionPolicy};
    let none = std::collections::BTreeMap::new();
    let period = match horizon {
        Retention::Forever => Period::Indefinite,
        Retention::Days(days) => Period::Days(days),
        Retention::Years(years) => Period::Years(u16::try_from(years).unwrap_or(u16::MAX)),
    };
    let bounds = if matches!(period, Period::Years(years) if years < FLOOR_YEARS)
        || matches!(period, Period::Days(days) if days < 1096)
    {
        // NOTE: EHDS Art. 9(2) binds the Member States; the boot validation refuses a horizon
        // under a Union floor, so one under three years runs under no Union jurisdiction.
        Bounds::outside_union(
            period,
            format!(
                "the audit.store horizon ({}), under no Union jurisdiction",
                horizon.describe()
            ),
        )
        .map_err(|e| AuditError::Policy(Box::new(e)))?
    } else {
        Bounds::union()
    };
    let retention =
        RetentionPolicy::declare_within(&bounds, period, &none, &none, categories.map())
            .map_err(|e| AuditError::Policy(Box::new(e)))?;
    let unlabelled = std::collections::BTreeMap::new();
    let labels =
        ehds_logging::label::LabelPolicy::declare(None, &unlabelled, &unlabelled, categories.map())
            .map_err(|e| AuditError::Policy(Box::new(e)))?;
    Ok(crate::system_log::fhir::Recording {
        retention,
        labels,
        emergency: emergency.clone(),
        horizon,
    })
}

/// One record as the store read it back: the event its columns hold, the
/// patient its subject resolver found, and what its classification read.
#[cfg(feature = "fhir")]
#[derive(Debug, Clone)]
pub(crate) struct StoredRecord {
    /// The record's chain position, which names it in an error.
    pub(crate) chain_seq: i64,
    /// The event, without its classification.
    pub(crate) event: AuditEvent,
    /// The patient behind the record's EHR (`patient_id`).
    pub(crate) subject: Option<String>,
    /// What the classification read (`category_content`).
    pub(crate) content: Option<ClassifiedContent>,
    /// The digest of the map that classified it (`category_map_digest`).
    pub(crate) map_digest: Option<MapDigest>,
}

/// The FHIR R4 `AuditEvent` of `stored` under the policy it was written under.
///
/// The classification is computed again from what it read, through the
/// policy's category map; a map whose digest is not the one the record names
/// would classify it differently, so the record is refused rather than
/// rendered with categories it never had.
///
/// # Errors
/// [`AuditError::Unreadable`] when the policy's map is not the one that
/// classified the record, and [`AuditError::Render`] when the document cannot
/// be written.
#[cfg(feature = "fhir")]
pub(crate) fn render(
    stored: StoredRecord,
    policy: &CompiledPolicy,
) -> Result<serde_json::Value, AuditError> {
    let (event, subject) = classified(stored, policy)?;
    crate::system_log::fhir::to_fhir(&event, &policy.context, subject.as_deref())
}

/// The event of `stored` with its classification computed again under
/// `policy`, and the patient behind its EHR.
///
/// # Errors
/// [`AuditError::Unreadable`] when the policy's map is not the one that
/// classified the record.
#[cfg(feature = "fhir")]
pub(crate) fn classified(
    stored: StoredRecord,
    policy: &CompiledPolicy,
) -> Result<(AuditEvent, Option<String>), AuditError> {
    let StoredRecord {
        chain_seq,
        mut event,
        subject,
        content,
        map_digest,
    } = stored;
    if let Some(content) = content {
        if map_digest != Some(policy.categories.digest()) {
            return Err(AuditError::Unreadable {
                chain_seq,
                reason: "its render policy holds a category map other than the one that \
                         classified it",
            });
        }
        event.category = Some(
            policy
                .categories
                .classify(&content.content, content.written),
        );
    }
    Ok((event, subject))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The policy reads back from the JSON the store keeps it as, so a record
    /// renders under the policy it was written under.
    #[test]
    fn the_policy_round_trips_through_its_stored_form() {
        let config: AuditConfig = toml::from_str(
            r#"
            source_id = "cdr-1"
            server_host = "cdr.example.org"
            purpose_codes = ["TREAT", "ETREAT"]
            emergency_purpose_codes = ["ETREAT"]
            [store]
            retention_years = 10
            [categories.templates]
            "Lab Report" = ["Laboratory-Reports"]
            "#,
        )
        .expect("parse");
        let policy = RenderPolicy::of(&config);
        let stored = serde_json::to_value(&policy).expect("serialize");
        let again: RenderPolicy = serde_json::from_value(stored).expect("read back");
        assert_eq!(policy, again);
        assert_eq!(again.horizon, Retention::Years(10));
        policy.compile().expect("compile");
    }
}
