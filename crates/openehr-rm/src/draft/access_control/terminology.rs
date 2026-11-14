// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The access control groups of FerroEHR's terminology, the only codes the
//! access evaluator and the access write checks read.
//!
//! Each group is a closed set of codes of [`TERMINOLOGY_ID_ACCESS_CONTROL`],
//! FerroEHR's operation terminology, which the operation grants are coded in
//! too, and each code is the group's rubric until
//! codes are assigned. A coded value is read into its group's type with
//! [`Term::read`]: a value of another terminology, or of a code outside the
//! group, is refused with [`OutsideGroup`], never read as a default.
//! Sensitivity, category, purpose and legal-ground codes come from the
//! terminology a jurisdiction names, so FerroEHR holds no group for them and
//! [`ForeignCode::read`] compares them by terminology id and code.
//!
//! The RM leaves `ACCESS_CONTROL_SETTINGS` "Currently implementation
//! dependent" (RM EHR IM, `org.openehr.rm.ehr.access_control_settings.adoc`)
//! and openEHR defines no access control terminology, so every group, code
//! and refusal here is our own design; the draft BMM names the terminology
//! `Terminology_id_access_control` and each group `Group_id_<group>`.

use std::fmt;

use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;

/// The BMM's `Terminology_id_access_control`: FerroEHR's operation terminology,
/// which holds the access control groups until openEHR adopts them.
pub const TERMINOLOGY_ID_ACCESS_CONTROL: &str = "ferroehr-operations";

/// One access control group of [`TERMINOLOGY_ID_ACCESS_CONTROL`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    /// `access default`: the disposition of an EHR where no directive applies.
    AccessDefault,
    /// `access directive kind`: what kind of instruction a directive is.
    AccessDirectiveKind,
    /// `access effect`: whether a directive permits or denies.
    AccessEffect,
    /// `access relationship`: the caller's relationship to the subject.
    AccessRelationship,
    /// `access caller kind`: the kind of caller a directive binds.
    AccessCallerKind,
    /// `access operation`: the operation classes a directive names.
    AccessOperation,
    /// `access sensitivity clearance`: the credentials element whose value is
    /// a sensitivity code the capability clears.
    AccessSensitivityClearance,
    /// `access permitted purpose`: the credentials element whose value is a
    /// purpose of use the capability lets its holder declare.
    AccessPermittedPurpose,
    /// `care relationship`: the demographic relationships that put a caller
    /// in care of a subject.
    CareRelationship,
    /// `access refusal`: the code an access refusal is recorded under.
    AccessRefusal,
    /// `access log withheld`: the attributes of an access-log accessor the
    /// subject view withholds under EHDS Art. 9(3).
    AccessLogWithheld,
}

impl Group {
    /// Every group, in contract order.
    pub const ALL: [Self; 11] = [
        Self::AccessDefault,
        Self::AccessDirectiveKind,
        Self::AccessEffect,
        Self::AccessRelationship,
        Self::AccessCallerKind,
        Self::AccessOperation,
        Self::AccessSensitivityClearance,
        Self::AccessPermittedPurpose,
        Self::CareRelationship,
        Self::AccessRefusal,
        Self::AccessLogWithheld,
    ];

    /// The group's rubric, such as `access default`.
    #[must_use]
    pub const fn rubric(self) -> &'static str {
        match self {
            Self::AccessDefault => "access default",
            Self::AccessDirectiveKind => "access directive kind",
            Self::AccessEffect => "access effect",
            Self::AccessRelationship => "access relationship",
            Self::AccessCallerKind => "access caller kind",
            Self::AccessOperation => "access operation",
            Self::AccessSensitivityClearance => "access sensitivity clearance",
            Self::AccessPermittedPurpose => "access permitted purpose",
            Self::CareRelationship => "care relationship",
            Self::AccessRefusal => "access refusal",
            Self::AccessLogWithheld => "access log withheld",
        }
    }

    /// The codes of the group, in contract order.
    #[must_use]
    pub const fn codes(self) -> &'static [&'static str] {
        match self {
            Self::AccessDefault => AccessDefault::CODES,
            Self::AccessDirectiveKind => DirectiveKind::CODES,
            Self::AccessEffect => Effect::CODES,
            Self::AccessRelationship => Relationship::CODES,
            Self::AccessCallerKind => CallerKind::CODES,
            Self::AccessOperation => OperationClass::CODES,
            Self::AccessSensitivityClearance => SensitivityClearance::CODES,
            Self::AccessPermittedPurpose => PermittedPurpose::CODES,
            Self::CareRelationship => CareRelationship::CODES,
            Self::AccessRefusal => Refusal::CODES,
            Self::AccessLogWithheld => Withheld::CODES,
        }
    }

    /// Whether `code` of [`TERMINOLOGY_ID_ACCESS_CONTROL`] is a code of the group.
    #[must_use]
    pub fn holds(self, code: &str) -> bool {
        self.codes().contains(&code)
    }
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.rubric())
    }
}

/// A coded value that is no code of the group its attribute is bound to.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutsideGroup {
    /// The value is of another terminology than [`TERMINOLOGY_ID_ACCESS_CONTROL`].
    #[error(
        "{terminology}::{code} is of no access control group: the {group} group is of {TERMINOLOGY_ID_ACCESS_CONTROL}"
    )]
    Terminology {
        /// The group the attribute is bound to.
        group: Group,
        /// The value's terminology id.
        terminology: String,
        /// The value's code.
        code: String,
    },
    /// The value is a code of [`TERMINOLOGY_ID_ACCESS_CONTROL`] the group does not hold.
    #[error("{TERMINOLOGY_ID_ACCESS_CONTROL}::{code} is no code of the {group} group")]
    Code {
        /// The group the attribute is bound to.
        group: Group,
        /// The value's code.
        code: String,
    },
}

/// The terms of one access control group, read from coded values.
pub trait Term: Copy + PartialEq + Sized + 'static {
    /// The group the terms belong to.
    const GROUP: Group;
    /// Every term of the group, in contract order.
    const ALL: &'static [Self];
    /// The code of every term, in the order of [`Term::ALL`].
    const CODES: &'static [&'static str];

    /// The term's code in [`TERMINOLOGY_ID_ACCESS_CONTROL`].
    fn code(self) -> &'static str;

    /// Reads the term a `CODE_PHRASE` of `terminology` and `code` names.
    ///
    /// # Errors
    /// [`OutsideGroup`] when `terminology` is not [`TERMINOLOGY_ID_ACCESS_CONTROL`] or `code`
    /// is no code of the group, compared exactly.
    fn read(terminology: &str, code: &str) -> Result<Self, OutsideGroup> {
        if terminology != TERMINOLOGY_ID_ACCESS_CONTROL {
            return Err(OutsideGroup::Terminology {
                group: Self::GROUP,
                terminology: terminology.to_owned(),
                code: code.to_owned(),
            });
        }
        Self::ALL
            .iter()
            .copied()
            .find(|term| term.code() == code)
            .ok_or_else(|| OutsideGroup::Code {
                group: Self::GROUP,
                code: code.to_owned(),
            })
    }

    /// Reads the term the `defining_code` of `coded` names.
    ///
    /// # Errors
    /// [`OutsideGroup`] as [`Term::read`].
    fn read_coded(coded: &DvCodedText) -> Result<Self, OutsideGroup> {
        Self::read(
            &coded.defining_code.terminology_id.value,
            &coded.defining_code.code_string,
        )
    }

    /// Whether `coded` names this term.
    fn is(self, coded: &DvCodedText) -> bool {
        Self::read_coded(coded).is_ok_and(|term| term == self)
    }
}

/// Defines one group's term type, its [`Term`] impl and its `Display`.
macro_rules! group {
    (
        $(#[$doc:meta])*
        $name:ident in $group:ident {
            $( $(#[$term_doc:meta])* $term:ident => $code:literal, )+
        }
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $( $(#[$term_doc])* $term, )+
        }

        impl Term for $name {
            const GROUP: Group = Group::$group;
            const ALL: &'static [Self] = &[$(Self::$term),+];
            const CODES: &'static [&'static str] = &[$($code),+];

            fn code(self) -> &'static str {
                match self {
                    $(Self::$term => $code,)+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.code())
            }
        }
    };
}

group! {
    /// The disposition of an EHR where no directive applies
    /// (`ACCESS_DIRECTIVE_SETTINGS.default_access`).
    AccessDefault in AccessDefault {
        /// Every caller the capability check admits is admitted.
        Open => "open",
        /// Only a caller an applying permit directive, a gate-keeper entry
        /// or a live care relationship names is admitted.
        Restricted => "restricted",
        /// Only an applying permit directive or the subject admits.
        ConsentRequired => "consent_required",
    }
}

group! {
    /// The kind of instruction a directive is (`ACCESS_DIRECTIVE.kind`).
    DirectiveKind in AccessDirectiveKind {
        /// A permit or deny the subject consented to.
        Consent => "consent",
        /// An EHDS Art. 8 restriction, binding professionals; always a deny.
        Restriction => "restriction",
        /// An EHDS Art. 10 opt-out, binding access services; a whole-record
        /// deny.
        OptOut => "opt_out",
        /// The scope of what a proxy may do (EHDS Art. 4(2)).
        Delegation => "delegation",
    }
}

group! {
    /// Whether a directive permits or denies (`ACCESS_DIRECTIVE.effect`).
    Effect in AccessEffect {
        /// The directive admits what it covers.
        Permit => "permit",
        /// The directive refuses what it covers; a deny overrides a permit.
        Deny => "deny",
    }
}

group! {
    /// The caller's relationship to the subject a directive names
    /// (`ACCESS_ACTOR_SCOPE.relationship`).
    Relationship in AccessRelationship {
        /// The caller has a live care relationship to the subject.
        Care => "care",
        /// The caller represents the subject: a parent, a guardian or a
        /// legal representative.
        Representative => "representative",
        /// The caller is a proxy the subject authorised (EHDS Art. 4(2)).
        Proxy => "proxy",
    }
}

group! {
    /// The kind of caller a directive binds (`ACCESS_ACTOR_SCOPE.caller_kind`).
    CallerKind in AccessCallerKind {
        /// A health professional, or a system that acts for a provider.
        Professional => "professional",
        /// An EHDS Art. 4 and 12 access service: a caller holding the
        /// `access_service` grant.
        AccessService => "access_service",
        /// The subject of the EHR.
        Subject => "subject",
        /// A representative or proxy of the subject.
        Representative => "representative",
        /// A system client acting on its own account.
        System => "system",
    }
}

group! {
    /// The operation classes a directive applies to
    /// (`ACCESS_DIRECTIVE.operations`); no capability grants one.
    OperationClass in AccessOperation {
        /// Reading data, by id or by version.
        Read => "read",
        /// Committing data.
        Write => "write",
        /// Exporting data out of the CDR, such as an EHR extract.
        Export => "export",
        /// Querying data with AQL.
        Query => "query",
    }
}

group! {
    /// The credentials element naming a sensitivity clearance: its value is
    /// a sensitivity code of the jurisdiction's terminology.
    SensitivityClearance in AccessSensitivityClearance {
        /// The element whose value is a sensitivity code the capability
        /// clears.
        SensitivityClearance => "sensitivity_clearance",
    }
}

group! {
    /// The credentials element naming a permitted purpose: its value is a
    /// purpose-of-use code of the jurisdiction's terminology.
    PermittedPurpose in AccessPermittedPurpose {
        /// The element whose value is a purpose of use the capability lets
        /// its holder declare.
        PermittedPurpose => "permitted_purpose",
    }
}

group! {
    /// The demographic relationships of care, coded in the `name` of a
    /// `PARTY_RELATIONSHIP`.
    CareRelationship in CareRelationship {
        /// A professional's `PERSON` is a member of a care-team `GROUP`.
        MemberOf => "member_of",
        /// A patient's `PERSON` is under the care of a team `GROUP` or an
        /// organisational unit.
        UnderCareOf => "under_care_of",
    }
}

group! {
    /// The code an access refusal is recorded under, one per step of the
    /// decision procedure that can refuse.
    Refusal in AccessRefusal {
        /// The subject opted out (EHDS Art. 10); the EHR reads as absent.
        OptOut => "refused-opt-out",
        /// The data are restricted (EHDS Art. 8); they read as absent.
        Restricted => "refused-restricted",
        /// A deny directive applies, with no exception that applies.
        ConsentDenied => "refused-consent-denied",
        /// The data carry a sensitivity the caller is not cleared for.
        Sensitivity => "refused-sensitivity",
        /// The policy needs a care relationship and the caller has none.
        NoCareRelationship => "refused-no-care-relationship",
        /// The policy needs a declared purpose and the request has none.
        PurposeMissing => "refused-purpose-missing",
        /// The caller's capabilities do not permit the declared purpose.
        PurposeNotPermitted => "refused-purpose-not-permitted",
        /// The EHR needs consent and no permit directive applies.
        ConsentRequired => "refused-consent-required",
        /// A subject caller asks for an EHR other than the one its patient
        /// context names: it is the subject of that EHR alone.
        NotSubject => "refused-not-subject",
        /// A representative asks for an EHR other than the one whose
        /// subject a recorded representation names: it acts for that
        /// subject alone.
        NotRepresented => "refused-not-represented",
    }
}

group! {
    /// The attributes of an access-log `ACCESSOR` the subject view withholds
    /// (`ACCESS_LOG_RESTRICTION.withheld`, EHDS Art. 9(3)).
    Withheld in AccessLogWithheld {
        /// `ACCESSOR.party`: the person or system that accessed.
        AccessorParty => "accessor_party",
        /// `ACCESSOR.provider`: the organisation the accessor acted for.
        AccessorProvider => "accessor_provider",
        /// `ACCESSOR.roles`: the roles the accessor held.
        AccessorRoles => "accessor_roles",
        /// `ACCESSOR.groups`: the groups whose membership the access relied
        /// on.
        AccessorGroups => "accessor_groups",
    }
}

impl Withheld {
    /// The member of `ACCESSOR` in its ITS-JSON form the term withholds.
    #[must_use]
    pub const fn member(self) -> &'static str {
        match self {
            Self::AccessorParty => "party",
            Self::AccessorProvider => "provider",
            Self::AccessorRoles => "roles",
            Self::AccessorGroups => "groups",
        }
    }
}

impl Refusal {
    /// Whether the refusal answers as if the data did not exist (`404`), so
    /// that a restriction or opt-out is not visible to the caller (EHDS Art.
    /// 8 third paragraph); every other refusal is a `403` naming its code.
    #[must_use]
    pub const fn is_invisible(self) -> bool {
        matches!(self, Self::OptOut | Self::Restricted)
    }
}

/// What a code of a jurisdiction's terminology stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ForeignKind {
    /// A sensitivity, such as HL7 v3 `Confidentiality` or `ActCode`.
    Sensitivity,
    /// A data category, such as an EHDS priority category.
    Category,
    /// A purpose of use, such as HL7 `PurposeOfUse`.
    Purpose,
    /// A legal provision, such as GDPR Art. 9(2)(a).
    LegalGround,
}

impl fmt::Display for ForeignKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sensitivity => "sensitivity",
            Self::Category => "category",
            Self::Purpose => "purpose",
            Self::LegalGround => "legal ground",
        })
    }
}

/// A coded value FerroEHR's terminology names no code for.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "{TERMINOLOGY_ID_ACCESS_CONTROL}::{code} is no {kind}: {TERMINOLOGY_ID_ACCESS_CONTROL} holds no {kind} codes"
)]
pub struct NotForeign {
    /// What the value stands for.
    pub kind: ForeignKind,
    /// The value's code.
    pub code: String,
}

/// A sensitivity, category, purpose or legal-ground code of the terminology
/// a jurisdiction names, compared by terminology id and code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ForeignCode {
    /// The `TERMINOLOGY_ID` value.
    pub terminology: String,
    /// The `code_string`.
    pub code: String,
}

impl ForeignCode {
    /// Reads `coded` as a code of `kind`.
    ///
    /// # Errors
    /// [`NotForeign`] when `coded` is of [`TERMINOLOGY_ID_ACCESS_CONTROL`], which holds no code
    /// of any [`ForeignKind`].
    pub fn read(kind: ForeignKind, coded: &DvCodedText) -> Result<Self, NotForeign> {
        let phrase = &coded.defining_code;
        if phrase.terminology_id.value == TERMINOLOGY_ID_ACCESS_CONTROL {
            return Err(NotForeign {
                kind,
                code: phrase.code_string.clone(),
            });
        }
        Ok(Self {
            terminology: phrase.terminology_id.value.clone(),
            code: phrase.code_string.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use openehr_base::prelude::TerminologyId;

    use super::*;

    #[test]
    fn every_group_reads_back_each_code_and_codes_are_unique_in_a_group() {
        for group in Group::ALL {
            let codes = group.codes();
            assert!(!codes.is_empty(), "{group}");
            for (i, code) in codes.iter().enumerate() {
                assert!(group.holds(code));
                assert!(!codes[i + 1..].contains(code), "{code} twice in {group}");
            }
        }
        assert_eq!(
            AccessDefault::read(TERMINOLOGY_ID_ACCESS_CONTROL, "open"),
            Ok(AccessDefault::Open)
        );
        assert_eq!(
            Refusal::read(TERMINOLOGY_ID_ACCESS_CONTROL, "refused-opt-out"),
            Ok(Refusal::OptOut)
        );
    }

    #[test]
    fn a_code_outside_its_group_is_refused() {
        assert_eq!(
            Effect::read(TERMINOLOGY_ID_ACCESS_CONTROL, "open"),
            Err(OutsideGroup::Code {
                group: Group::AccessEffect,
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
        assert!(OperationClass::read(TERMINOLOGY_ID_ACCESS_CONTROL, "commit_composition").is_err());
        assert!(AccessDefault::read(TERMINOLOGY_ID_ACCESS_CONTROL, " open").is_err());
    }

    #[test]
    fn a_foreign_code_is_of_another_terminology() {
        let coded = |terminology: &str, code: &str| DvCodedText {
            value: code.to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: None,
            language: None,
            encoding: None,
            defining_code: CodePhrase {
                terminology_id: TerminologyId {
                    value: terminology.to_owned(),
                },
                code_string: code.to_owned(),
                preferred_term: None,
            },
        };
        assert_eq!(
            ForeignCode::read(ForeignKind::Sensitivity, &coded("Confidentiality", "R")),
            Ok(ForeignCode {
                terminology: "Confidentiality".to_owned(),
                code: "R".to_owned()
            })
        );
        assert!(
            ForeignCode::read(
                ForeignKind::Purpose,
                &coded(TERMINOLOGY_ID_ACCESS_CONTROL, "TREAT")
            )
            .is_err()
        );
        assert!(Effect::Deny.is(&coded(TERMINOLOGY_ID_ACCESS_CONTROL, "deny")));
        assert!(!Effect::Deny.is(&coded("local", "deny")));
    }
}
