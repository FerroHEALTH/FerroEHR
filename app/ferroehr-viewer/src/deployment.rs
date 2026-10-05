// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The deployment profile the connected CDR declares (#3264).
//!
//! `GET {rest root}/status` carries `deployment: {profile, gaps, accepted}`:
//! the profile the server was configured with (`sandbox` or `production`) and
//! the production separations it has not made, as `snake_case` codes. A browser
//! tab is where one deployment gets mistaken for another, so the shell states
//! the profile it is connected to, and under `sandbox` says plainly what that
//! means. Plain types here, the component stays thin. No openEHR spec governs
//! the status document's shape beyond `status` and the versions — our own
//! design/extension.

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

impl SandboxNotice {
    /// The rule this deployment is under, stated as the rule rather than as a
    /// category: it has not made the production separations, and it must not
    /// hold real patient data.
    pub const RULE: &'static str = "Sandbox deployment: this deployment has not made the \
                                    production separations and must not hold real patient \
                                    data.";

    /// The gap list as one sentence, or an empty string when the server
    /// reported none.
    #[must_use]
    pub fn gaps_sentence(&self) -> String {
        if self.gaps.is_empty() {
            return String::new();
        }
        format!("Open: {}.", self.gaps.join("; "))
    }
}

/// One gap code in plain words; an unknown code is shown as itself rather
/// than dropped, so a newer server's gap is still named.
#[must_use]
pub fn gap_text(code: &str) -> String {
    match code {
        "shared_credential" => {
            "the clinical, demographic and linkage domains share one database credential"
        }
        "shared_cluster" => "two domains reach the same PostgreSQL cluster",
        "open_subject_namespace" => {
            "no subject pseudonym namespace is declared, so an EHR subject may carry a real \
             identifier"
        }
        "audit_off" => "access is not recorded to a durable audit trail",
        "migrate_on_runtime_credential" => {
            "schema preparation runs on the clinical runtime credential"
        }
        other => return other.to_owned(),
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{SandboxNotice, StatusDocument, gap_text};

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
                "the clinical, demographic and linkage domains share one database credential",
                "access is not recorded to a durable audit trail",
            ]
        );
        assert_eq!(
            notice.gaps_sentence(),
            "Open: the clinical, demographic and linkage domains share one database \
             credential; access is not recorded to a durable audit trail."
        );
        assert!(SandboxNotice::RULE.contains("must not hold real patient data"));

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
        assert_eq!(
            status("sandbox", &[])
                .sandbox_notice()
                .map(|n| n.gaps_sentence()),
            Some(String::new())
        );
        assert_eq!(StatusDocument::parse("not json"), None);
    }
}
