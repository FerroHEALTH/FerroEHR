// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The deployment profile the connected CDR declares (#3264).
//!
//! `GET {rest root}/status` carries `deployment: {profile}`, the profile the
//! server was configured with (`sandbox` or `production`); the production
//! separations it has not made (`gaps`, `accepted`, as `snake_case` codes) are
//! served only by the authenticated `GET /management/status`, so a public
//! document lists none and the notice then states the rule alone. A browser
//! tab is where one deployment gets mistaken for another, so the shell states
//! the profile it is connected to, and under `sandbox` says plainly what that
//! means. Plain types here, the component stays thin. No openEHR spec governs
//! the status document's shape beyond `status` and the versions — our own
//! design/extension.

use crate::locale::server::ServerMsg;
use crate::locale::tr;
use serde::Deserialize;

/// The CDR status document, typed over the fields the shell reads.
///
/// Unknown fields are ignored: the document carries more (the licence, the
/// versions, a timestamp) and a newer server may add to it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StatusDocument {
    /// `UP` when the server answers at all.
    #[serde(default)]
    pub status: Option<String>,
    /// The server's own version.
    #[serde(default)]
    pub server_version: Option<String>,
    /// The declared deployment posture; absent on a server older than the
    /// profile, which claims nothing.
    #[serde(default)]
    pub deployment: Option<Deployment>,
}

/// The `deployment` block of the status document.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Deployment {
    /// `sandbox` or `production`.
    pub profile: String,
    /// The production separations not made, as `snake_case` codes.
    #[serde(default)]
    pub gaps: Vec<String>,
    /// The gaps the operator accepted by name.
    #[serde(default)]
    pub accepted: Vec<String>,
}

impl StatusDocument {
    /// Parse the raw status body; `None` when it is not the document.
    #[must_use]
    pub fn parse(body: &str) -> Option<Self> {
        serde_json::from_str(body).ok()
    }

    /// The declared profile, when the server reports one.
    #[must_use]
    pub fn profile(&self) -> Option<&str> {
        self.deployment.as_ref().map(|d| d.profile.as_str())
    }

    /// The notice a `sandbox` document earns; `None` for `production` and for
    /// a document that declares no profile.
    #[must_use]
    pub fn sandbox_notice(&self) -> Option<SandboxNotice> {
        let deployment = self.deployment.as_ref()?;
        if deployment.profile != "sandbox" {
            return None;
        }
        Some(SandboxNotice {
            gaps: deployment.gaps.iter().map(|code| gap_text(code)).collect(),
        })
    }
}

/// The sandbox notice the shell renders: the rule, and the open gaps by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxNotice {
    /// The separations the server reported as not made, in plain words.
    pub gaps: Vec<String>,
}

/// One gap code in plain words; an unknown code is shown as itself rather
/// than dropped, so a newer server's gap is still named.
#[must_use]
pub fn gap_text(code: &str) -> String {
    match code {
        "shared_credential" => tr(ServerMsg::ClinicalDemographicDomainsShareOne),
        "shared_cluster" => tr(ServerMsg::TwoDomainsReachSamePostgresqlCluster),
        "open_subject_namespace" => tr(ServerMsg::NoSubjectPseudonymNamespaceDeclaredSo),
        "audit_off" => tr(ServerMsg::AccessNotRecordedDurableAuditTrail),
        "audit_fails_open" => tr(ServerMsg::AccessRecordAuditQueueCannotTake),
        "no_category_map" => tr(ServerMsg::AccessRecordedWithoutCategoryData),
        "open_ehr_access_default" => tr(ServerMsg::EhrWithoutOwnAccessSettingsAdmits),
        "migrate_on_runtime_credential" => {
            tr(ServerMsg::SchemaPreparationRunsClinicalRuntimeCredential)
        }
        "auth_off" => tr(ServerMsg::RequestsServedWithoutAuthentication),
        "rbac_off" => tr(ServerMsg::RoleChecksOffSoEverySigned),
        "plaintext_listener" => tr(ServerMsg::ListenerServesPlainHttpAddressReachable),
        "plaintext_broker" => tr(ServerMsg::ChangeEventsPublishedMessageBrokerWithout),
        "plaintext_audit_feed" => tr(ServerMsg::AccessRecordsSentAuditRepositoryWithout),
        "plaintext_object_store" => tr(ServerMsg::MultimediaFilesStoredOverConnectionWithout),
        other => return other.to_owned(),
    }
    .clone()
}

#[cfg(test)]
mod tests {
    use super::{StatusDocument, gap_text};
    use crate::served_vocabulary::DEPLOYMENT_GAPS;

    fn status(profile: &str, gaps: &[&str]) -> StatusDocument {
        let gaps: Vec<String> = gaps.iter().map(|g| format!("\"{g}\"")).collect();
        StatusDocument::parse(&format!(
            "{{\"status\":\"UP\",\"server_version\":\"4.2.0\",\"timestamp\":\"2026-09-11T00:00:00Z\",\
             \"deployment\":{{\"profile\":\"{profile}\",\"gaps\":[{}],\"accepted\":[]}}}}",
            gaps.join(",")
        ))
        .expect("the fixture parses")
    }

    /// A `sandbox` document earns the notice, naming every open gap; a
    /// `production` one earns nothing.
    #[test]
    fn sandbox_earns_the_notice_and_production_does_not() {
        let sandbox = status("sandbox", &["shared_credential", "audit_off"]);
        assert_eq!(sandbox.profile(), Some("sandbox"));
        let notice = sandbox
            .sandbox_notice()
            .expect("sandbox renders the notice");
        assert_eq!(
            notice.gaps,
            [
                "the clinical and demographic domains share one database credential",
                "access is not recorded to a durable audit trail",
            ]
        );

        let production = status("production", &[]);
        assert_eq!(production.profile(), Some("production"));
        assert_eq!(production.sandbox_notice(), None);
    }

    /// A production deployment that accepted a gap by name still renders
    /// quietly: the acceptance is the operator's stated decision.
    #[test]
    fn an_accepted_gap_under_production_stays_quiet() {
        let doc = StatusDocument::parse(
            r#"{"status":"UP","deployment":{"profile":"production","gaps":["shared_cluster"],"accepted":["shared_cluster"]}}"#,
        )
        .expect("parses");
        assert_eq!(doc.sandbox_notice(), None);
    }

    /// Every gap code the server reports has a plain-words text: the list is
    /// the shared `served_vocabulary::DEPLOYMENT_GAPS`, which the server holds
    /// its `DeploymentGap` tokens to, so a server gap added without a text here
    /// fails this test rather than falling through to the raw code.
    #[test]
    fn every_server_gap_has_plain_words() {
        for code in DEPLOYMENT_GAPS {
            let text = gap_text(code);
            assert_ne!(text, code, "`{code}` has no plain-words text");
            assert!(!text.contains('_'), "`{code}` reads as a code: {text}");
        }
    }

    /// A server that reports no `deployment` block claims nothing, and an
    /// unknown gap code is shown as itself.
    #[test]
    fn no_profile_means_no_claim_and_unknown_gaps_are_named_verbatim() {
        let older =
            StatusDocument::parse(r#"{"status":"UP","server_version":"4.1.0"}"#).expect("parses");
        assert_eq!(older.profile(), None);
        assert_eq!(older.sandbox_notice(), None);
        assert_eq!(older.server_version.as_deref(), Some("4.1.0"));
        assert_eq!(gap_text("future_gap"), "future_gap");
        let notice = status("sandbox", &["future_gap"])
            .sandbox_notice()
            .expect("sandbox");
        assert_eq!(notice.gaps, ["future_gap"]);
        assert_eq!(StatusDocument::parse("not json"), None);
    }
}
