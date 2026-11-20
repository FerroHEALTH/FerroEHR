// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The attributes of the draft access control package that no access
//! decision of this server reads yet, refused at write.
//!
//! The draft package states each attribute's meaning. A server that stored
//! one and decided without it would admit what the attribute narrows, so a
//! settings version that sets one is refused whole, naming the attribute,
//! until the decision reads it. A rule's care roles and care path, the
//! policy's care purposes and its time zone are decided (#6075), and so are
//! the record's marking, a label's and a classification's confidentiality
//! and a data scope's confidentialities (#6057). Our own design.

use openehr_rm::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;

/// The path of the first attribute of `settings` no decision reads yet.
#[must_use]
pub fn in_settings(settings: &AccessDirectiveSettings) -> Option<String> {
    // TODO(#6077): decide on the controller's exclusions.
    settings
        .exclusions
        .is_some()
        .then(|| "exclusions".to_owned())
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
    fn a_record_marking_is_decided() {
        let marked = settings(&format!(
            r#", "marking": {{ "_type": "CONFIDENTIALITY_MARKING",
                "confidentiality": {},
                "set_by": {{ "_type": "PARTY_SELF" }},
                "set_at": {{ "_type": "DV_DATE_TIME", "value": "2026-11-19T10:00:00Z" }} }}"#,
            term("HL7_Confidentiality", "R")
        ));
        assert_eq!(in_settings(&marked), None);
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
