// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

use super::*;

#[test]
fn every_group_reads_back_each_term_and_terms_are_unique_in_a_group() {
    for group in Group::ALL {
        let phrases = group.phrases();
        assert!(!phrases.is_empty(), "{group}");
        for (i, (terminology, code)) in phrases.iter().enumerate() {
            assert!(group.holds(terminology, code));
            assert!(
                !phrases[i + 1..].contains(&(*terminology, *code)),
                "{terminology}::{code} twice in {group}"
            );
        }
    }
    assert_eq!(
        AccessDefault::read(TERMINOLOGY_ID_ACCESS, "open"),
        Ok(AccessDefault::Open)
    );
    assert_eq!(
        Refusal::read(TERMINOLOGY_ID_ACCESS, "refused-opt-out"),
        Ok(Refusal::OptOut)
    );
}

#[test]
fn a_term_with_a_standard_code_is_read_by_that_code_alone() {
    assert_eq!(
        Effect::read(TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, "deny"),
        Ok(Effect::Deny)
    );
    assert_eq!(
        CallerKind::read(TERMINOLOGY_ID_SECURITY_ROLE_TYPE, "datasubject"),
        Ok(CallerKind::Subject)
    );
    assert_eq!(
        CareRelationship::read(TERMINOLOGY_ID_ROLE_CLASS, "MBR"),
        Ok(CareRelationship::MemberOf)
    );
    assert_eq!(
        Action::read(TERMINOLOGY_ID_AUDIT_EVENT_ACTION, "R"),
        Ok(Action::Read)
    );
    assert!(matches!(
        Effect::read(TERMINOLOGY_ID_ACCESS, "deny"),
        Err(OutsideGroup::Terminology {
            group: Group::AccessEffect,
            ..
        })
    ));
    assert_eq!(
        CallerKind::read(TERMINOLOGY_ID_ACCESS, "subject"),
        Err(OutsideGroup::Code {
            group: Group::AccessCallerKind,
            terminology: TERMINOLOGY_ID_ACCESS.to_owned(),
            code: "subject".to_owned()
        })
    );
    assert!(CareRelationship::read(TERMINOLOGY_ID_ACCESS, "member_of").is_err());
    assert!(Action::read(TERMINOLOGY_ID_ACCESS, "read").is_err());
    assert_eq!(
        CareRelationship::read(TERMINOLOGY_ID_ACCESS, "under_care_of"),
        Ok(CareRelationship::UnderCareOf)
    );
}

#[test]
fn a_code_outside_its_group_is_refused() {
    assert_eq!(
        Effect::read(TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, "open"),
        Err(OutsideGroup::Code {
            group: Group::AccessEffect,
            terminology: TERMINOLOGY_ID_CONSENT_PROVISION_TYPE.to_owned(),
            code: "open".to_owned()
        })
    );
    assert!(matches!(
        DirectiveKind::read("local", "consent"),
        Err(OutsideGroup::Terminology {
            group: Group::AccessDirectiveKind,
            ..
        })
    ));
    assert!(OperationClass::read(TERMINOLOGY_ID_ACCESS, "commit_composition").is_err());
    assert!(AccessDefault::read(TERMINOLOGY_ID_ACCESS, " open").is_err());
}

#[test]
fn a_foreign_code_is_of_another_terminology() {
    assert_eq!(
        ForeignCode::read(
            ForeignKind::Sensitivity,
            &coded_text("HL7_Confidentiality", "R", "R")
        ),
        Ok(ForeignCode {
            terminology: "HL7_Confidentiality".to_owned(),
            code: "R".to_owned()
        })
    );
    assert!(
        ForeignCode::read(
            ForeignKind::Purpose,
            &coded_text(TERMINOLOGY_ID_ACCESS, "TREAT", "TREAT")
        )
        .is_err()
    );
    assert!(Effect::Deny.is(&coded_text(
        TERMINOLOGY_ID_CONSENT_PROVISION_TYPE,
        "deny",
        "deny"
    )));
    assert!(!Effect::Deny.is(&coded_text("local", "deny", "deny")));
}

#[test]
fn a_confidentiality_reads_as_its_level_in_the_package_order() {
    let level =
        |code| Confidentiality::read(&coded_text(TERMINOLOGY_ID_CONFIDENTIALITY, code, code));
    for code in ["U", "L", "M", "N"] {
        assert_eq!(level(code), Ok(Confidentiality::Normal), "{code}");
    }
    assert_eq!(level("R"), Ok(Confidentiality::Restricted));
    assert_eq!(level("V"), Ok(Confidentiality::VeryRestricted));
    assert!(Confidentiality::Normal < Confidentiality::Restricted);
    assert!(Confidentiality::Restricted < Confidentiality::VeryRestricted);
    assert!(level("X").is_err());
    assert!(Confidentiality::read(&coded_text("HL7_ActCode", "R", "R")).is_err());
}

#[test]
fn the_conflict_and_binding_groups_read_their_codes() {
    assert_eq!(
        Conflict::read(TERMINOLOGY_ID_ACCESS, "related"),
        Ok(Conflict::Related)
    );
    assert_eq!(
        ValueSetBinding::read(TERMINOLOGY_ID_ACCESS, "kinship"),
        Ok(ValueSetBinding::Kinship)
    );
    assert!(Conflict::read(TERMINOLOGY_ID_ACCESS, "kinship").is_err());
}
