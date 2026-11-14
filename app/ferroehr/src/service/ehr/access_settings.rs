// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `access_directives` scheme of `EHR_ACCESS.settings`: the one scheme
//! FerroEHR writes and reads.
//!
//! `EHR_ACCESS` is the openEHR access-decision authority: "All access
//! decisions to data in the EHR must be made in accordance with the policies
//! and rules in this object" (RM `org.openehr.rm.ehr.ehr_access.adoc`
//! §`EHR_ACCESS` Class). Its `settings: ACCESS_CONTROL_SETTINGS [0..1]` is
//! abstract and "Currently implementation dependent" (RM
//! `org.openehr.rm.ehr.access_control_settings.adoc`), and `scheme()` names
//! the concrete subtype. FerroEHR's subtype is the draft class
//! `ACCESS_DIRECTIVE_SETTINGS` ([`AccessDirectiveSettings`], generated from
//! the draft access control BMM), checked against its BMM invariants and the
//! access control terminology before it is stored. No openEHR spec governs
//! the concrete scheme: our own design (#5487).

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 10): EHR_ACCESS.settings is the RM-mandated \
              open slot (RM ehr access_control_settings.adoc — abstract, implementation-dependent \
              by specification)"
)]

use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::Value;

use crate::error::ServiceError;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, ForeignCode, ForeignKind, OperationClass, Term,
};

/// The `_type` of the settings on the wire (canonical JSON).
pub const SETTINGS_TYPE: &str = "ACCESS_DIRECTIVE_SETTINGS";

/// The scheme name `EHR_ACCESS.scheme()` reports for settings of
/// [`SETTINGS_TYPE`] (`Scheme_valid`, RM `org.openehr.rm.ehr.ehr_access.adoc`).
pub const SCHEME: &str = "access_directives";

/// The default definition of the scheme's settings, as its file name and ADL 2
/// text.
///
/// Every boot stores it when no definition of its id is stored
/// ([`crate::service::FerroEhrService::seed_default_definitions`]). It
/// constrains the settings to [`SETTINGS_TYPE`], which the definition path
/// validates against the RM with the draft packages
/// ([`crate::draft_model::RM_WITH_DRAFTS`]). FerroEHR reads the settings by
/// their codes, never by this archetype's id.
pub const DEFINITIONS: [(&str, &str); 1] = [(
    "openEHR-EHR-EHR_ACCESS.access_directives.v1.0.0.adls",
    include_str!(
        "../../../assets/definitions/access/openEHR-EHR-EHR_ACCESS.access_directives.v1.0.0.adls"
    ),
)];

/// The EHRs whose current `EHR_ACCESS` carries settings, with those settings,
/// shared from the service's copy
/// ([`crate::service::FerroEhrService::ehr_access_settings_index`]).
pub type EhrAccessIndex = std::sync::Arc<Vec<(crate::ids::EhrId, EhrAccessSettings)>>;

/// The current `EHR_ACCESS.settings` of an EHR, as the access rules read them.
#[derive(Debug, Clone, PartialEq)]
pub enum EhrAccessSettings {
    /// Settings of the `access_directives` scheme.
    Directives(Box<AccessDirectiveSettings>),
    /// Stored settings that do not read as the scheme: the EHR is held
    /// closed, since a privacy control that cannot be read is never read as
    /// permissive.
    HeldClosed,
}

/// Stored settings that do not read as the `access_directives` scheme.
#[derive(Debug, thiserror::Error)]
#[error("the EHR_ACCESS settings do not read as the {SCHEME} scheme")]
pub struct UnreadableSettings(#[source] Defect);

impl EhrAccessSettings {
    /// Reads the settings of an `EHR_ACCESS` canonical-JSON object; `None`
    /// when it carries none.
    ///
    /// # Errors
    /// [`UnreadableSettings`] when the settings are not settings of the
    /// `access_directives` scheme that pass [`check_written_settings`]; the
    /// caller holds the EHR closed.
    pub fn from_ehr_access(access: &Value) -> Result<Option<Self>, UnreadableSettings> {
        let Some(settings) = access.get("settings").filter(|s| !s.is_null()) else {
            return Ok(None);
        };
        read_settings(settings)
            .map(|read| Some(Self::Directives(Box::new(read))))
            .map_err(UnreadableSettings)
    }

    /// Reads the settings the projection stored as their canonical JSON
    /// text, which were read as the scheme before they were stored
    /// ([`crate::service::ehr::access_projection`]).
    ///
    /// # Errors
    /// [`UnreadableSettings`] when the text does not decode as the class.
    pub fn from_projected(settings: &str) -> Result<Self, UnreadableSettings> {
        serde_json::from_str::<AccessDirectiveSettings>(settings)
            .map(|read| Self::Directives(Box::new(read)))
            .map_err(|error| UnreadableSettings(Defect::Decode(error)))
    }

    /// The settings `access` carries, or [`Self::HeldClosed`] for settings
    /// that do not read; `None` when it carries none.
    #[must_use]
    pub fn held_or_read(access: &Value) -> Option<Self> {
        Self::from_ehr_access(access).unwrap_or_else(|error| {
            let cause: &(dyn std::error::Error + 'static) = &error;
            tracing::error!(
                error = cause,
                "EHR_ACCESS settings do not read; the EHR is held closed"
            );
            Some(Self::HeldClosed)
        })
    }

    /// What the EHR admits where no directive applies: the settings'
    /// `default_access`, and [`AccessDefault::Restricted`] for an EHR held
    /// closed.
    #[must_use]
    pub fn default_access(&self) -> AccessDefault {
        match self {
            // NOTE: our own design, the write check holds the code to its group, so a
            // stored code outside it reads as the closed disposition, never as open.
            Self::Directives(settings) => AccessDefault::read_coded(&settings.default_access)
                .unwrap_or(AccessDefault::Restricted),
            Self::HeldClosed => AccessDefault::Restricted,
        }
    }

    /// The parties who may commit the next `EHR_ACCESS` version; none for an
    /// EHR held closed.
    #[must_use]
    pub fn gate_keepers(&self) -> &[PartyProxy] {
        match self {
            Self::Directives(settings) => settings.gate_keepers.as_deref().unwrap_or_default(),
            Self::HeldClosed => &[],
        }
    }
}

/// The defect of written settings that are not settings of the scheme.
#[derive(Debug, thiserror::Error)]
enum Defect {
    /// The settings name another scheme.
    #[error(
        "EHR_ACCESS.settings must be an {SETTINGS_TYPE} of the {SCHEME} scheme; _type {0} is no \
         scheme this server keeps"
    )]
    Scheme(String),
    /// The settings name no scheme.
    #[error(
        "EHR_ACCESS.settings carry no _type, so they name no scheme (EHR_ACCESS.Scheme_valid); \
         they must be an {SETTINGS_TYPE} of the {SCHEME} scheme"
    )]
    NoScheme,
    /// The settings do not decode as the class.
    #[error("EHR_ACCESS.settings do not read as an {SETTINGS_TYPE}")]
    Decode(#[source] serde_json::Error),
    /// A coded value is outside the terminology its attribute is bound to.
    #[error("EHR_ACCESS.settings.{path}: {reason}")]
    Code {
        /// The attribute path, such as `directives[0].operations[1]`.
        path: String,
        /// Why the code is refused.
        reason: String,
    },
    /// The settings break a BMM invariant of the draft access control.
    #[error("EHR_ACCESS.settings break the invariant {0}")]
    Invariant(openehr_rm::draft::invariants::Violation),
}

/// Reads written settings as the scheme, checking every invariant.
fn read_settings(settings: &Value) -> Result<AccessDirectiveSettings, Defect> {
    match settings.get("_type").and_then(Value::as_str) {
        Some(SETTINGS_TYPE) => {}
        Some(other) => return Err(Defect::Scheme(format!("{other:?}"))),
        None => return Err(Defect::NoScheme),
    }
    let read = serde_json::from_value::<AccessDirectiveSettings>(settings.clone())
        .map_err(Defect::Decode)?;
    if let Err(violations) = openehr_rm::draft::access_control::invariants::check_settings(&read)
        && let Some(first) = violations.0.first()
    {
        return Err(Defect::Invariant(*first));
    }
    check_codes(&read)?;
    Ok(read)
}

/// Checks the coded values the draft BMM binds no invariant to: each
/// directive operation is a code of the `access operation` group, and each
/// purpose, category, sensitivity and legal ground is a code of the
/// jurisdiction's terminology, never of FerroEHR's.
fn check_codes(settings: &AccessDirectiveSettings) -> Result<(), Defect> {
    for (index, directive) in settings
        .directives
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        check_directive(&format!("directives[{index}]"), directive)?;
    }
    for (index, label) in settings
        .labels
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        foreign(
            &format!("labels[{index}].sensitivities"),
            ForeignKind::Sensitivity,
            &label.sensitivities,
        )?;
    }
    Ok(())
}

/// [`check_codes`] over one directive at `path` and its exceptions.
fn check_directive(path: &str, directive: &AccessDirective) -> Result<(), Defect> {
    for (index, operation) in directive
        .operations
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        OperationClass::read_coded(operation).map_err(|error| Defect::Code {
            path: format!("{path}.operations[{index}]"),
            reason: error.to_string(),
        })?;
    }
    foreign(
        &format!("{path}.purposes"),
        ForeignKind::Purpose,
        directive.purposes.as_deref().unwrap_or_default(),
    )?;
    foreign(
        &format!("{path}.legal_grounds"),
        ForeignKind::LegalGround,
        directive.legal_grounds.as_deref().unwrap_or_default(),
    )?;
    for (index, scope) in directive
        .data
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let scope_path = format!("{path}.data[{index}]");
        foreign(
            &format!("{scope_path}.categories"),
            ForeignKind::Category,
            scope.categories.as_deref().unwrap_or_default(),
        )?;
        foreign(
            &format!("{scope_path}.sensitivities"),
            ForeignKind::Sensitivity,
            scope.sensitivities.as_deref().unwrap_or_default(),
        )?;
    }
    for (index, exception) in directive
        .exceptions
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        check_directive(&format!("{path}.exceptions[{index}]"), exception)?;
    }
    Ok(())
}

/// Refuses the first of `codes`, at `path`, that is no code of `kind`.
fn foreign(path: &str, kind: ForeignKind, codes: &[DvCodedText]) -> Result<(), Defect> {
    for (index, coded) in codes.iter().enumerate() {
        ForeignCode::read(kind, coded).map_err(|error| Defect::Code {
            path: format!("{path}[{index}]"),
            reason: error.to_string(),
        })?;
    }
    Ok(())
}

/// Checks written `EHR_ACCESS.settings` before they are committed.
///
/// They must be an `ACCESS_DIRECTIVE_SETTINGS` that the strict
/// canonical-JSON reader reads and whose every BMM invariant and terminology
/// group holds.
///
/// Settings of any other scheme are refused, so a body of a scheme this
/// server no longer keeps never governs an EHR. No openEHR spec governs the
/// scheme: our own design.
///
/// # Errors
/// [`ServiceError::bad_request`] (400) naming the first defect: another
/// scheme's `_type`, a member that does not decode, or the first broken
/// invariant as `CLASS.Invariant`.
pub fn check_written_settings(settings: &Value) -> Result<(), ServiceError> {
    read_settings(settings).map(drop).map_err(|defect| {
        let detail = match std::error::Error::source(&defect) {
            Some(cause) => format!("{defect}: {cause}"),
            None => defect.to_string(),
        };
        ServiceError::bad_request(detail, defect)
    })
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::party_binding::policy_mark::TERMINOLOGY;

    /// A term of the access control groups.
    fn term(code: &str) -> Value {
        json!({ "_type": "DV_CODED_TEXT", "value": code,
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": TERMINOLOGY },
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

    /// Settings of the default `code` holding `directives`.
    fn settings(code: &str, directives: &[Value]) -> Value {
        let mut s = json!({ "_type": SETTINGS_TYPE, "default_access": term(code) });
        if !directives.is_empty() {
            s["directives"] = json!(directives);
        }
        s
    }

    /// The refusal message `settings` draw, which must be a `400`.
    fn refused(settings: &Value) -> String {
        match check_written_settings(settings) {
            Err(ServiceError::BadRequest(refusal)) => refusal.message,
            other => panic!("a 400 for {settings}, got {other:?}"),
        }
    }

    #[test]
    fn settings_of_the_scheme_are_accepted() {
        let consent = directive(
            "consent",
            "permit",
            &json!({ "applies_to": [{ "caller_kind": term("professional") }] }),
        );
        check_written_settings(&settings("restricted", &[consent])).expect("valid settings");
        check_written_settings(&settings("open", &[])).expect("open settings");
    }

    /// A body of the removed scheme or of any other scheme is refused naming
    /// the `_type`, and one of none naming `Scheme_valid`.
    #[test]
    fn another_scheme_is_refused() {
        let old = json!({ "_type": "FERROEHR_ACCESS_CONTROL_V1", "default_access": "restricted" });
        assert!(refused(&old).contains("\"FERROEHR_ACCESS_CONTROL_V1\""));
        assert!(refused(&json!({ "default_access": term("open") })).contains("Scheme_valid"));
        assert!(refused(&json!({ "_type": "SOME_SCHEME" })).contains("SOME_SCHEME"));
    }

    /// A member the class does not declare, or an empty list, is refused by the
    /// strict reader.
    #[test]
    fn a_member_that_does_not_decode_is_refused() {
        let mut extra = settings("open", &[]);
        extra["access_list"] = json!([]);
        assert!(refused(&extra).contains("do not read"));
        let mut empty = settings("open", &[]);
        empty["gate_keepers"] = json!([]);
        assert!(refused(&empty).contains("do not read"));
    }

    /// Each invariant of the settings and the classes they hold is refused,
    /// named as the BMM names it.
    #[test]
    fn each_invariant_is_refused_by_name() {
        let opt_out = || directive("opt_out", "deny", &json!({}));
        let cases = [
            (
                settings("closed", &[]),
                "ACCESS_DIRECTIVE_SETTINGS.Default_access_valid",
            ),
            (
                settings("open", &[opt_out(), opt_out()]),
                "ACCESS_DIRECTIVE_SETTINGS.Opt_out_unique",
            ),
            (
                settings("open", &[directive("wish", "deny", &json!({}))]),
                "ACCESS_DIRECTIVE.Kind_valid",
            ),
            (
                settings("open", &[directive("consent", "allow", &json!({}))]),
                "ACCESS_DIRECTIVE.Effect_valid",
            ),
            (
                settings("open", &[directive("restriction", "permit", &json!({}))]),
                "ACCESS_DIRECTIVE.Restriction_denies",
            ),
            (
                settings(
                    "open",
                    &[directive(
                        "opt_out",
                        "deny",
                        &json!({ "data": [{ "templates": ["t.v1"] }] }),
                    )],
                ),
                "ACCESS_DIRECTIVE.Opt_out_denies_whole_record",
            ),
            (
                settings(
                    "open",
                    &[directive(
                        "restriction",
                        "deny",
                        &json!({ "exceptions": [directive("consent", "deny", &json!({}))] }),
                    )],
                ),
                "ACCESS_DIRECTIVE.Exceptions_opposite",
            ),
            (
                settings(
                    "open",
                    &[directive(
                        "consent",
                        "permit",
                        &json!({ "applies_to": [{}] }),
                    )],
                ),
                "ACCESS_ACTOR_SCOPE.Not_empty",
            ),
            (
                settings(
                    "open",
                    &[directive(
                        "consent",
                        "permit",
                        &json!({ "applies_to": [{ "relationship": term("friend") }] }),
                    )],
                ),
                "ACCESS_ACTOR_SCOPE.Relationship_valid",
            ),
            (
                settings(
                    "open",
                    &[directive(
                        "consent",
                        "permit",
                        &json!({ "applies_to": [{ "caller_kind": term("robot") }] }),
                    )],
                ),
                "ACCESS_ACTOR_SCOPE.Caller_kind_valid",
            ),
            (
                settings(
                    "open",
                    &[directive("restriction", "deny", &json!({ "data": [{}] }))],
                ),
                "ACCESS_DATA_SCOPE.Not_empty",
            ),
        ];
        for (body, invariant) in cases {
            let message = refused(&body);
            assert!(message.contains(invariant), "{invariant}: {message}");
        }
    }

    /// A directive operation outside the `access operation` group, and a
    /// purpose, category, sensitivity or legal ground coded in FerroEHR's
    /// terminology, are refused naming the attribute.
    #[test]
    fn a_code_outside_its_terminology_is_refused() {
        let foreign = |code: &str| {
            json!({ "_type": "DV_CODED_TEXT", "value": code,
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "v3-ActReason" },
                        "code_string": code } })
        };
        let restriction = |extra: Value| directive("restriction", "deny", &extra);
        check_written_settings(&settings(
            "open",
            &[restriction(json!({
                "operations": [term("read")],
                "purposes": [foreign("TREAT")],
                "legal_grounds": [foreign("GDPR-9-2-a")],
                "data": [{ "categories": [foreign("LAB")], "sensitivities": [foreign("PSY")] }]
            }))],
        ))
        .expect("codes of their terminologies");
        let cases = [
            (
                json!({ "operations": [term("delete")] }),
                "directives[0].operations[0]",
            ),
            (
                json!({ "operations": [foreign("read")] }),
                "directives[0].operations[0]",
            ),
            (
                json!({ "purposes": [term("TREAT")] }),
                "directives[0].purposes[0]",
            ),
            (
                json!({ "legal_grounds": [term("GDPR")] }),
                "directives[0].legal_grounds[0]",
            ),
            (
                json!({ "data": [{ "categories": [term("LAB")] }] }),
                "directives[0].data[0].categories[0]",
            ),
            (
                json!({ "data": [{ "sensitivities": [term("PSY")] }] }),
                "directives[0].data[0].sensitivities[0]",
            ),
        ];
        for (extra, path) in cases {
            let message = refused(&settings("open", &[restriction(extra)]));
            assert!(message.contains(path), "{path}: {message}");
        }
        let mut labelled = settings("open", &[]);
        labelled["labels"] = json!([{
            "target": { "_type": "OBJECT_REF", "namespace": "local", "type": "VERSIONED_COMPOSITION",
                        "id": { "_type": "HIER_OBJECT_ID", "value": "8849182c-82ad-4088-a07f-48ead4180515" } },
            "sensitivities": [term("PSY")]
        }]);
        assert!(refused(&labelled).contains("labels[0].sensitivities[0]"));
        let exception = directive(
            "restriction",
            "deny",
            &json!({ "exceptions": [directive("consent", "permit", &json!({ "operations": [term("delete")] }))] }),
        );
        assert!(
            refused(&settings("open", &[exception]))
                .contains("directives[0].exceptions[0].operations[0]")
        );
    }

    /// A code of the right group in another terminology is outside the group.
    #[test]
    fn a_code_of_another_terminology_is_refused() {
        let mut s = settings("open", &[]);
        s["default_access"]["defining_code"]["terminology_id"]["value"] = json!("local");
        assert!(refused(&s).contains("Default_access_valid"));
    }

    #[test]
    fn stored_settings_read_their_default_and_gate_keepers() {
        let mut s = settings("consent_required", &[]);
        s["gate_keepers"] = json!([{ "_type": "PARTY_SELF" }]);
        let access = json!({ "_type": "EHR_ACCESS", "settings": s });
        let read = EhrAccessSettings::from_ehr_access(&access)
            .expect("the settings read")
            .expect("settings");
        assert_eq!(read.default_access(), AccessDefault::ConsentRequired);
        assert_eq!(read.gate_keepers().len(), 1);
        assert!(matches!(
            EhrAccessSettings::from_ehr_access(&json!({ "_type": "EHR_ACCESS" })),
            Ok(None)
        ));
    }

    /// Stored settings that do not read hold the EHR closed: restricted, with no
    /// gate-keeper.
    #[test]
    fn unreadable_stored_settings_hold_the_ehr_closed() {
        let access = json!({ "_type": "EHR_ACCESS",
                             "settings": { "_type": "FERROEHR_ACCESS_CONTROL_V1" } });
        assert!(EhrAccessSettings::from_ehr_access(&access).is_err());
        let held = EhrAccessSettings::held_or_read(&access).expect("held");
        assert_eq!(held, EhrAccessSettings::HeldClosed);
        assert_eq!(held.default_access(), AccessDefault::Restricted);
        assert_eq!(held.gate_keepers(), &[]);
    }
}
