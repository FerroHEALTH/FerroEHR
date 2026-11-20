// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The attributes of the draft access control package that no access
//! decision of this server reads yet, refused at write.
//!
//! The draft package states each attribute's meaning (care relationships,
//! confidentiality, exclusions). A server that stored
//! one and decided without it would admit what the attribute narrows, so a
//! settings or policy version that sets one is refused whole, naming the
//! attribute, until the decision reads it. An attribute whose unread value
//! admits nothing more than its void (the self-declared and attested paths,
//! the caseload switch, conflict allowances, concealed levels and value-set
//! bindings) is stored, read through [`crate::access_policy::care`]. A rule's
//! care roles and care path, the policy's care purposes and its time zone are
//! decided (#6075). Our own design.

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;

/// The path of the first attribute of `settings` no decision reads yet.
#[must_use]
pub fn in_settings(settings: &AccessDirectiveSettings) -> Option<String> {
    // TODO(#6057): decide on the record's confidentiality marking.
    if settings.marking.is_some() {
        return Some("marking".to_owned());
    }
    // TODO(#6077): decide on the controller's exclusions.
    if settings.exclusions.is_some() {
        return Some("exclusions".to_owned());
    }
    for (index, label) in settings
        .labels
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        // TODO(#6057): decide on an object's confidentiality.
        if label.confidentiality.is_some() {
            return Some(format!("labels[{index}].confidentiality"));
        }
    }
    directives(
        "directives",
        settings.directives.as_deref().unwrap_or_default(),
    )
}

/// The path of the first attribute of `policy` no decision reads yet.
#[must_use]
pub fn in_policy(policy: &AccessPolicy) -> Option<String> {
    for (index, classification) in policy
        .classifications
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        // TODO(#6057): decide on a classification's confidentiality.
        if classification.confidentiality.is_some() {
            return Some(format!("classifications[{index}].confidentiality"));
        }
        if classification.data.confidentialities.is_some() {
            return Some(format!("classifications[{index}].data.confidentialities"));
        }
    }
    directives("rules", policy.rules.as_deref().unwrap_or_default())
        .or_else(|| directives("catalogue", policy.catalogue.as_deref().unwrap_or_default()))
}

/// The path of the first undecided attribute of the directives at `path`
/// and their exceptions.
fn directives(path: &str, directives: &[AccessDirective]) -> Option<String> {
    directives
        .iter()
        .enumerate()
        .find_map(|(index, directive)| {
            let at = format!("{path}[{index}]");
            directive_attribute(&at, directive).or_else(|| {
                directives_of(
                    &format!("{at}.exceptions"),
                    directive.exceptions.as_deref().unwrap_or_default(),
                )
            })
        })
}

/// [`directives`] one level down, for exceptions, which hold none of their
/// own.
fn directives_of(path: &str, exceptions: &[AccessDirective]) -> Option<String> {
    exceptions
        .iter()
        .enumerate()
        .find_map(|(index, exception)| directive_attribute(&format!("{path}[{index}]"), exception))
}

/// The path of the first undecided attribute of one directive at `path`.
fn directive_attribute(path: &str, directive: &AccessDirective) -> Option<String> {
    directive
        .data
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
        // TODO(#6057): match the confidentiality levels a data scope names.
        .find(|(_, scope)| scope.confidentialities.is_some())
        .map(|(index, _)| format!("{path}.data[{index}].confidentialities"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings of the access directive scheme, with `extra` members after
    /// the default disposition.
    fn settings(extra: &str) -> AccessDirectiveSettings {
        let body = format!(
            r#"{{ "_type": "ACCESS_DIRECTIVE_SETTINGS",
                "default_access": {{ "_type": "DV_CODED_TEXT", "value": "open",
                    "defining_code": {{ "terminology_id": {{ "value": "openehr_access" }},
                        "code_string": "open" }} }}{extra} }}"#
        );
        serde_json::from_str(&body).expect("the settings should decode")
    }

    /// A coded term of `terminology`, as JSON text.
    fn term(terminology: &str, code: &str) -> String {
        format!(
            r#"{{ "_type": "DV_CODED_TEXT", "value": "{code}",
                "defining_code": {{ "terminology_id": {{ "value": "{terminology}" }},
                    "code_string": "{code}" }} }}"#
        )
    }

    #[test]
    fn a_settings_version_without_new_attributes_is_decided() {
        assert_eq!(in_settings(&settings("")), None);
    }

    #[test]
    fn a_record_marking_is_refused_until_the_decision_reads_it() {
        let marked = settings(&format!(
            r#", "marking": {{ "_type": "CONFIDENTIALITY_MARKING",
                "confidentiality": {},
                "set_by": {{ "_type": "PARTY_SELF" }},
                "set_at": {{ "_type": "DV_DATE_TIME", "value": "2026-11-19T10:00:00Z" }} }}"#,
            term("HL7_Confidentiality", "R")
        ));
        assert_eq!(in_settings(&marked).as_deref(), Some("marking"));
    }

    #[test]
    fn a_care_role_in_a_directive_exception_is_decided() {
        let exception = format!(
            r#"{{ "_type": "ACCESS_DIRECTIVE", "archetype_node_id": "id2",
                "name": {{ "_type": "DV_TEXT", "value": "exception" }},
                "kind": {}, "effect": {}, "emergency_override": false,
                "applies_to": [{{ "_type": "ACCESS_ACTOR_SCOPE",
                    "relationship": {}, "care_roles": [{}] }}] }}"#,
            term("openehr_access", "consent"),
            term("HL7_ConsentProvisionType", "permit"),
            term("openehr_access", "care"),
            term("HL7_ParticipationType", "RESP")
        );
        let directive = format!(
            r#"{{ "_type": "ACCESS_DIRECTIVE", "archetype_node_id": "id1",
                "name": {{ "_type": "DV_TEXT", "value": "rule" }},
                "kind": {}, "effect": {}, "emergency_override": false,
                "exceptions": [{exception}] }}"#,
            term("openehr_access", "consent"),
            term("HL7_ConsentProvisionType", "deny")
        );
        let with_role = settings(&format!(r#", "directives": [{directive}]"#));
        assert_eq!(in_settings(&with_role), None);
    }
}
