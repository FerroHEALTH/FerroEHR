// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caseload a query request may be narrowed to: the EHRs whose subjects
//! the caller holds a live care relationship to, by a path and in roles the
//! request names.
//!
//! The draft access control package lets a query carry the members
//! `caseload` (`direct`, `team`, `all`, `none`) and `caseload_roles` beside
//! the AQL text, which stays portable (draft ITS-REST page, Queries and the
//! Caseload). The caseload only narrows: the decision procedure still
//! decides every row, and the narrowing is a condition of the query's one
//! statement, applied before it is counted, limited or paged (The Decision
//! Procedure, Queries). Liveness is the decision's own, grace and provider
//! included, and a self-declared relationship is no path of a caseload, as no
//! path implies another. No openEHR specification defines the members: our
//! own design over the draft (#6056).

use std::sync::Arc;

use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::ForeignCode;
use uuid::Uuid;

use crate::access_eval::condition::care::ProjectedCare;
use crate::party_binding::grants::caller::CareTeam;
use crate::party_binding::grants::caller::care_of::CareOf;
use crate::party_binding::grants::caller::facts::CallerFacts;
use crate::storage::access_projection::EhrSubject;

/// The request member that names the path.
pub const CASELOAD: &str = "caseload";
/// The request member that names the roles.
pub const CASELOAD_ROLES: &str = "caseload_roles";
/// The form `caseload_roles` must take.
pub const ROLES_FORM: &str = "a non-empty list of role codes, each a non-empty text";

/// How a relationship reaches the caller for its caseload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Path {
    /// The relationship names the caller's `PERSON` or a `ROLE` it performs.
    Direct,
    /// The relationship names a care team or unit the caller is a member of.
    Team,
    /// Either.
    All,
}

/// One role a request names: a code of any terminology, or one of the
/// terminology it names as `terminology::code`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RoleAsk {
    /// The terminology, when the request names one.
    pub terminology: Option<String>,
    /// The code.
    pub code: String,
}

impl RoleAsk {
    /// Whether the role `role` of a relationship is this one.
    #[must_use]
    pub fn matches(&self, role: &ForeignCode) -> bool {
        role.code == self.code
            && self
                .terminology
                .as_ref()
                .is_none_or(|terminology| *terminology == role.terminology)
    }
}

/// The caseload a request asks for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ask {
    /// The path.
    pub path: Path,
    /// The roles; empty means any role.
    pub roles: Vec<RoleAsk>,
}

impl Ask {
    /// Whether the relationship `care` counts for this caseload by its role.
    #[must_use]
    pub fn admits_role(&self, care: &CareOf) -> bool {
        self.roles.is_empty() || self.roles.iter().any(|role| role.matches(care.role()))
    }

    /// The caller's principals the path reaches, sorted: its `PERSON` and
    /// the `ROLE`s it performs for `direct`, its care teams and units for
    /// `team`.
    #[must_use]
    pub fn principals(&self, facts: &CallerFacts) -> Vec<Uuid> {
        let direct = facts
            .party()
            .into_iter()
            .chain(facts.performed().iter().copied());
        let team = facts.care_teams().iter().map(CareTeam::team);
        let mut principals: Vec<Uuid> = match self.path {
            Path::Direct => direct.collect(),
            Path::Team => team.collect(),
            Path::All => direct.chain(team).collect(),
        };
        principals.sort_unstable();
        principals.dedup();
        principals
    }
}

/// Why the caseload members of a request are refused, each a `400` naming
/// the member.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    /// `caseload` is no text of the four values.
    #[error("{CASELOAD}: {0} is none of direct, team, all, none")]
    Value(String),
    /// `caseload_roles` is no non-empty list of role codes.
    #[error("{CASELOAD_ROLES}: {0}")]
    Roles(&'static str),
    /// `caseload_roles` is given while `caseload` is `none`.
    #[error("{CASELOAD_ROLES}: there is no caseload to narrow while {CASELOAD} is none")]
    RolesWithoutCaseload,
    /// The access policy turns the caseload off.
    #[error("{CASELOAD}: the access policy does not let a query be narrowed to the caseload")]
    Off,
    /// The declared purpose is outside the policy's care purposes.
    #[error("{CASELOAD}: a caseload is read for a care purpose of the access policy alone")]
    Purpose,
    /// The query is rooted at a demographic class, so it reads no EHR.
    #[error("{CASELOAD}: a query rooted at a demographic class reads no EHR")]
    Demographic,
}

/// The caseload the members `caseload` and `caseload_roles` ask for, as the
/// texts the request carries; `None` for `none` or neither.
///
/// # Errors
/// [`Refused`] for an unknown value, roles that are no non-empty list of
/// codes, and roles with no caseload.
pub fn read(caseload: Option<&str>, roles: Option<&[String]>) -> Result<Option<Ask>, Refused> {
    let path = match caseload {
        None | Some("none") => None,
        Some("direct") => Some(Path::Direct),
        Some("team") => Some(Path::Team),
        Some("all") => Some(Path::All),
        Some(other) => return Err(Refused::Value(format!("{other:?}"))),
    };
    let roles = match roles {
        None => Vec::new(),
        Some([]) => return Err(Refused::Roles(ROLES_FORM)),
        Some(items) if items.iter().any(String::is_empty) => {
            return Err(Refused::Roles(ROLES_FORM));
        }
        Some(items) => items.iter().map(|text| role(text)).collect(),
    };
    match path {
        Some(path) => Ok(Some(Ask { path, roles })),
        None if roles.is_empty() => Ok(None),
        None => Err(Refused::RolesWithoutCaseload),
    }
}

/// The role `text` names: `terminology::code`, or a code alone.
fn role(text: &str) -> RoleAsk {
    match text.split_once("::") {
        Some((terminology, code)) if !terminology.is_empty() && !code.is_empty() => RoleAsk {
            terminology: Some(terminology.to_owned()),
            code: code.to_owned(),
        },
        _ => RoleAsk {
            terminology: None,
            code: text.to_owned(),
        },
    }
}

/// Refuses a caseload where `policy` turns it off, or the declared `purpose`
/// is outside the policy's `care_purposes`.
///
/// A care relationship is a ground for primary use only (The Decision
/// Procedure, Procedure).
///
/// # Errors
/// [`Refused::Off`] and [`Refused::Purpose`].
pub fn allowed(policy: Option<&AccessPolicy>, purpose: Option<&str>) -> Result<(), Refused> {
    if !crate::access_policy::care::CareFacts::of(policy).caseload() {
        return Err(Refused::Off);
    }
    let purposes = policy
        .and_then(|policy| policy.care_purposes.as_deref())
        .unwrap_or_default();
    match purpose {
        Some(code)
            if !purposes.is_empty()
                && !purposes
                    .iter()
                    .any(|care| care.defining_code.code_string == code) =>
        {
            Err(Refused::Purpose)
        }
        _ => Ok(()),
    }
}

/// The caseload a query's statement applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caseload {
    /// The subject references of every `PERSON` in the caseload, read from
    /// the care copy, beside the namespace this CDR names its own parties
    /// under, whose ids match case-folded.
    Census {
        /// The namespace of this CDR's own parties.
        namespace: Arc<str>,
        /// The subjects.
        subjects: Arc<[EhrSubject]>,
    },
    /// The clinical care projection, probed by the EHR row's subject digest
    /// with the path's principals and the roles.
    Projected {
        /// The probe of the care condition, its principals the path's.
        care: ProjectedCare,
        /// The roles; empty means any role.
        roles: Vec<RoleAsk>,
        /// Every principal the caller holds, none of which may have written
        /// a relationship that counts.
        principals: Arc<[Uuid]>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_owned()).collect()
    }

    #[test]
    fn the_four_values_and_the_roles_read() {
        assert_eq!(read(None, None), Ok(None));
        assert_eq!(read(Some("none"), None), Ok(None));
        for (text, path) in [
            ("direct", Path::Direct),
            ("team", Path::Team),
            ("all", Path::All),
        ] {
            assert_eq!(
                read(Some(text), None),
                Ok(Some(Ask {
                    path,
                    roles: Vec::new()
                }))
            );
        }
        let roles = texts(&["RESP", "HL7_ParticipationType::ATND"]);
        let ask = read(Some("direct"), Some(&roles))
            .expect("the roles read")
            .expect("a caseload");
        let code = |terminology: &str, code: &str| ForeignCode {
            terminology: terminology.to_owned(),
            code: code.to_owned(),
        };
        assert!(ask.roles[0].matches(&code("HL7_ParticipationType", "RESP")));
        assert!(ask.roles[1].matches(&code("HL7_ParticipationType", "ATND")));
        assert!(!ask.roles[1].matches(&code("local", "ATND")));
    }

    #[test]
    fn an_unknown_value_and_roles_without_a_caseload_are_refused() {
        assert_eq!(
            read(Some("indirect"), None),
            Err(Refused::Value("\"indirect\"".to_owned()))
        );
        let resp = texts(&["RESP"]);
        assert_eq!(
            read(Some("none"), Some(&resp)),
            Err(Refused::RolesWithoutCaseload)
        );
        assert_eq!(read(None, Some(&resp)), Err(Refused::RolesWithoutCaseload));
        assert_eq!(
            read(Some("all"), Some(&[])),
            Err(Refused::Roles(ROLES_FORM))
        );
        assert_eq!(
            read(Some("all"), Some(&texts(&[""]))),
            Err(Refused::Roles(ROLES_FORM))
        );
        for refused in [
            Refused::Value(String::new()),
            Refused::RolesWithoutCaseload,
            Refused::Off,
            Refused::Purpose,
            Refused::Demographic,
        ] {
            assert!(refused.to_string().starts_with("caseload"), "{refused}");
        }
    }

    #[test]
    #[expect(
        clippy::disallowed_types,
        reason = "the test policy is written as the canonical JSON the strict reader decodes"
    )]
    fn a_purpose_outside_the_care_purposes_and_the_switch_refuse() {
        use serde_json::{Value, json};

        let policy = |members: &Value| -> AccessPolicy {
            let mut body = json!({
                "_type": "ACCESS_POLICY",
                "name": { "_type": "DV_TEXT", "value": "Policy" },
                "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
                "controller": { "_type": "PARTY_REF", "namespace": "demographic",
                    "type": "ORGANISATION",
                    "id": { "_type": "HIER_OBJECT_ID",
                        "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" } },
                "care_relationship_required": false,
                "purpose_required": false
            });
            if let (Some(body), Some(members)) = (body.as_object_mut(), members.as_object()) {
                body.extend(members.clone());
            }
            openehr_its::json::from_canonical_value(&body).expect("the policy reads")
        };
        let treat = json!({ "_type": "DV_CODED_TEXT", "value": "treatment",
            "defining_code": { "terminology_id": { "value": "HL7_ActReason" },
                "code_string": "TREAT" } });
        let cared = policy(&json!({ "care_purposes": [treat] }));
        assert_eq!(allowed(Some(&cared), Some("TREAT")), Ok(()));
        assert_eq!(allowed(Some(&cared), None), Ok(()));
        assert_eq!(allowed(Some(&cared), Some("HRESCH")), Err(Refused::Purpose));
        assert_eq!(allowed(None, Some("HRESCH")), Ok(()));
        let off = policy(&json!({ "caseload": false }));
        assert_eq!(allowed(Some(&off), None), Err(Refused::Off));
    }
}
