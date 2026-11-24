// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caseload a query request may be narrowed to: the EHRs whose subjects
//! the caller holds a live care relationship to, by a path and in roles the
//! request names.
//!
//! The draft access control package lets a query carry the members
//! `caseload` (`direct`, `team`, `all`, `none`), `caseload_roles` and
//! `caseload_episodes` beside the AQL text, which stays portable (draft
//! ITS-REST page, Queries and the Caseload). The caseload only narrows: the decision procedure still
//! decides every row, and the narrowing is a condition of the query's one
//! statement, applied before it is counted, limited or paged (The Decision
//! Procedure, Queries). Liveness is the decision's own, grace and provider
//! included, and a self-declared relationship is no path of a caseload, as no
//! path implies another. No openEHR specification defines the members: our
//! own design over the draft (#6056).

use std::sync::Arc;

use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::care::CarePath;
use openehr_rm::draft::access_control::terminology::{
    ForeignCode, TERMINOLOGY_ID_PARTICIPATION_TYPE,
};

use crate::access_eval::care_tie::{CareNeed, Tie};
use crate::access_eval::condition::care::ProjectedCare;
use crate::access_eval::procedure::Policy;
use crate::access_eval::scope::Pred;
use crate::storage::access_projection::EhrSubject;
use crate::versioning::episode::EpisodeKey;

/// The request member that names the path.
pub const CASELOAD: &str = "caseload";
/// The request member that names the roles.
pub const CASELOAD_ROLES: &str = "caseload_roles";
/// The form `caseload_roles` must take.
pub const ROLES_FORM: &str = "a non-empty list of role codes, each a non-empty text";
/// The request member that names the episodes.
pub const CASELOAD_EPISODES: &str = "caseload_episodes";
/// The form `caseload_episodes` must take.
pub const EPISODES_FORM: &str =
    "a non-empty list of episodes, each the token issuer|id with neither part empty";

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

/// The caseload a request asks for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ask {
    /// The path.
    pub path: Path,
    /// The roles; empty means any role.
    pub roles: Vec<ForeignCode>,
    /// The episodes each EHR is narrowed to, sorted; empty means every
    /// episode the caller's relationships cover.
    pub episodes: Vec<EpisodeKey>,
}

impl Ask {
    /// What a relationship must meet to count: one need per path the
    /// caseload names, in one of its roles. A self-declared relationship
    /// meets none, as no path implies another.
    #[must_use]
    pub fn needs(&self) -> Vec<CareNeed> {
        let roles = (!self.roles.is_empty()).then(|| self.roles.clone());
        let paths: &[CarePath] = match self.path {
            Path::Direct => &[CarePath::Direct],
            Path::Team => &[CarePath::Team],
            Path::All => &[CarePath::Direct, CarePath::Team],
        };
        paths
            .iter()
            .map(|path| CareNeed::new(roles.clone(), Some(*path)))
            .collect()
    }

    /// Whether a subject the caller holds by `ties` is in the caseload.
    #[must_use]
    pub fn holds(&self, ties: &[Tie]) -> bool {
        let needs = self.needs();
        ties.iter()
            .any(|tie| needs.iter().any(|need| need.met_by(tie)))
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
    /// `caseload_episodes` is no non-empty list of `issuer|id` tokens.
    #[error("{CASELOAD_EPISODES}: {0}")]
    Episodes(&'static str),
    /// `caseload_episodes` is given while `caseload` is `none`.
    #[error("{CASELOAD_EPISODES}: there is no caseload to narrow while {CASELOAD} is none")]
    EpisodesWithoutCaseload,
    /// `caseload_episodes` names an episode none of the caller's live
    /// relationships names.
    #[error("{CASELOAD_EPISODES}: an episode no live care relationship of the caller names")]
    UnknownEpisode,
}

/// The caseload the members `caseload`, `caseload_roles` and
/// `caseload_episodes` ask for, as the texts the request carries; `None` for
/// `none` or none of them.
///
/// A role is `terminology::code`, or a code alone of `HL7_ParticipationType`,
/// the default code system of `care_role` (draft access control,
/// Terminology). An episode is `issuer|id` (draft ITS-REST page, Queries and
/// the Caseload).
///
/// # Errors
/// [`Refused`] for an unknown value, roles that are no non-empty list of
/// codes, episodes that are no non-empty list of tokens, and roles or
/// episodes with no caseload.
pub fn read(
    caseload: Option<&str>,
    roles: Option<&[String]>,
    episodes: Option<&[String]>,
) -> Result<Option<Ask>, Refused> {
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
    let episodes = match episodes {
        None => Vec::new(),
        Some([]) => return Err(Refused::Episodes(EPISODES_FORM)),
        Some(items) => {
            let mut keys = items
                .iter()
                .map(|text| episode(text).ok_or(Refused::Episodes(EPISODES_FORM)))
                .collect::<Result<Vec<_>, _>>()?;
            keys.sort_unstable();
            keys.dedup();
            keys
        }
    };
    match path {
        Some(path) => Ok(Some(Ask {
            path,
            roles,
            episodes,
        })),
        None if !roles.is_empty() => Err(Refused::RolesWithoutCaseload),
        None if !episodes.is_empty() => Err(Refused::EpisodesWithoutCaseload),
        None => Ok(None),
    }
}

/// The episode `text` names as `issuer|id`; `None` when either part is
/// empty or the separator is missing.
#[must_use]
pub fn episode(text: &str) -> Option<EpisodeKey> {
    let (issuer, id) = text.split_once('|')?;
    (!issuer.is_empty() && !id.is_empty()).then(|| EpisodeKey::of(issuer, id))
}

/// The role `text` names: `terminology::code`, or a code of
/// `HL7_ParticipationType`.
fn role(text: &str) -> ForeignCode {
    let (terminology, code) = match text.split_once("::") {
        Some((terminology, code)) if !terminology.is_empty() && !code.is_empty() => {
            (terminology, code)
        }
        _ => (TERMINOLOGY_ID_PARTICIPATION_TYPE, text),
    };
    ForeignCode {
        terminology: terminology.to_owned(),
        code: code.to_owned(),
    }
}

/// Refuses a caseload where `policy` turns it off, or the declared `purpose`
/// is outside the policy's `care_purposes`.
///
/// A care relationship is a ground for primary use only (The Decision
/// Procedure, Procedure), so a caseload under another purpose holds nobody.
///
/// # Errors
/// [`Refused::Off`] and [`Refused::Purpose`].
pub fn allowed(policy: Option<&AccessPolicy>, purpose: Option<&str>) -> Result<(), Refused> {
    if !crate::access_policy::care::CareFacts::of(policy).caseload() {
        return Err(Refused::Off);
    }
    if !Policy::of(policy).admits_care(purpose) {
        return Err(Refused::Purpose);
    }
    Ok(())
}

/// The caseload a query's statement applies: the EHRs it reaches, and the
/// data of each it narrows to when the request names episodes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caseload {
    /// The EHRs in the caseload.
    pub reach: Reach,
    /// The data of the episodes the request names, the unplaced data and the
    /// data `ACCESS_POLICY.episode_shared` names; `None` for every episode.
    pub within: Option<Pred>,
}

/// The EHRs in a caseload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
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
    /// for a relationship that meets one of the needs.
    Projected {
        /// The probe of the care condition.
        care: ProjectedCare,
        /// What a relationship must meet ([`Ask::needs`]).
        needs: Vec<CareNeed>,
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
        assert_eq!(read(None, None, None), Ok(None));
        assert_eq!(read(Some("none"), None, None), Ok(None));
        for (text, path) in [
            ("direct", Path::Direct),
            ("team", Path::Team),
            ("all", Path::All),
        ] {
            assert_eq!(
                read(Some(text), None, None),
                Ok(Some(Ask {
                    path,
                    roles: Vec::new(),
                    episodes: Vec::new(),
                }))
            );
        }
        let roles = texts(&["RESP", "HL7_ParticipationType::ATND"]);
        let ask = read(Some("direct"), Some(&roles), None)
            .expect("the roles read")
            .expect("a caseload");
        let code = |terminology: &str, code: &str| ForeignCode {
            terminology: terminology.to_owned(),
            code: code.to_owned(),
        };
        assert_eq!(
            ask.roles,
            vec![
                code("HL7_ParticipationType", "RESP"),
                code("HL7_ParticipationType", "ATND")
            ]
        );
        let local = read(Some("all"), Some(&texts(&["local::lead"])), None)
            .expect("the role reads")
            .expect("a caseload");
        assert_eq!(local.roles, vec![code("local", "lead")]);
        assert_eq!(local.needs().len(), 2, "all is direct and team");
    }

    #[test]
    fn an_unknown_value_and_roles_without_a_caseload_are_refused() {
        assert_eq!(
            read(Some("indirect"), None, None),
            Err(Refused::Value("\"indirect\"".to_owned()))
        );
        let resp = texts(&["RESP"]);
        assert_eq!(
            read(Some("none"), Some(&resp), None),
            Err(Refused::RolesWithoutCaseload)
        );
        assert_eq!(
            read(None, Some(&resp), None),
            Err(Refused::RolesWithoutCaseload)
        );
        assert_eq!(
            read(Some("all"), Some(&[]), None),
            Err(Refused::Roles(ROLES_FORM))
        );
        assert_eq!(
            read(Some("all"), Some(&texts(&[""])), None),
            Err(Refused::Roles(ROLES_FORM))
        );
        for refused in [
            Refused::Value(String::new()),
            Refused::RolesWithoutCaseload,
            Refused::Off,
            Refused::Purpose,
            Refused::Demographic,
            Refused::Episodes(EPISODES_FORM),
            Refused::EpisodesWithoutCaseload,
            Refused::UnknownEpisode,
        ] {
            assert!(refused.to_string().starts_with("caseload"), "{refused}");
        }
    }

    #[test]
    fn the_episodes_read_as_issuer_and_id_and_need_a_caseload() {
        let named = texts(&["urn:h|C-17", "urn:h|D-4", "urn:h|C-17"]);
        let ask = read(Some("team"), None, Some(&named))
            .expect("the episodes read")
            .expect("a caseload");
        let mut expected = vec![
            EpisodeKey::of("urn:h", "C-17"),
            EpisodeKey::of("urn:h", "D-4"),
        ];
        expected.sort_unstable();
        assert_eq!(ask.episodes, expected);
        for bad in [
            Vec::new(),
            texts(&["C-17"]),
            texts(&["|C-17"]),
            texts(&["urn:h|"]),
        ] {
            assert_eq!(
                read(Some("team"), None, Some(&bad)),
                Err(Refused::Episodes(EPISODES_FORM)),
                "{bad:?}"
            );
        }
        assert_eq!(
            read(Some("none"), None, Some(&named)),
            Err(Refused::EpisodesWithoutCaseload)
        );
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
        assert_eq!(allowed(None, Some("HRESCH")), Err(Refused::Purpose));
        assert_eq!(allowed(None, Some("ETREAT")), Ok(()));
        let off = policy(&json!({ "caseload": false }));
        assert_eq!(allowed(Some(&off), None), Err(Refused::Off));
    }
}
