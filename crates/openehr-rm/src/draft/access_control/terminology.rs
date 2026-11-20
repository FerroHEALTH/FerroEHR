// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The groups of the draft access terminology, the only codes the access
//! evaluator, the access write checks and the access log read.
//!
//! Each group is a closed set of terms. A term is a code of
//! [`TERMINOLOGY_ID_ACCESS`], the draft `openehr_access` terminology both
//! draft packages propose, or, where an HL7 code system publishes a code
//! of the same meaning, that system's code under its `HL7_<name>` identifier,
//! such as [`TERMINOLOGY_ID_CONSENT_PROVISION_TYPE`]. Each draft code is its
//! rubric until codes are assigned. A coded value is read into its group's
//! type with [`Term::read`]: a value of a terminology the group holds no
//! term of, or of a code outside the group, is refused with
//! [`OutsideGroup`], never read as a default. Sensitivity, category, purpose
//! and legal-ground codes come from the terminology a jurisdiction names, so
//! the draft holds no group for them and [`ForeignCode::read`] compares them
//! by terminology id and code.
//!
//! The RM leaves `ACCESS_CONTROL_SETTINGS` "Currently implementation
//! dependent" (RM EHR IM, `org.openehr.rm.ehr.access_control_settings.adoc`)
//! and openEHR defines no access control terminology, so every group, code
//! and refusal here is our own design; the draft BMMs name the terminology
//! `Terminology_id_access` and each group `Group_id_<group>`.

use std::fmt;

use crate::v1_2::data_types::text::code_phrase::CodePhrase;
use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
use openehr_base::prelude::TerminologyId;

use care::{CareBasis, CareElement, CarePath};
use log::{AccessorKind, Action, Authorisation, CredentialBinding, Outcome, RefusalReason};
use representation::{Representation, RepresentationElement};

/// The BMMs' `Terminology_id_access`: the draft `openehr_access` terminology,
/// which holds the access groups' own codes until openEHR adopts them.
pub const TERMINOLOGY_ID_ACCESS: &str = "openehr_access";

/// HL7 FHIR `http://hl7.org/fhir/consent-provision-type`, by its terminology
/// identifier `HL7_` and its code system name: the `permit` and `deny` of a
/// consent provision.
pub const TERMINOLOGY_ID_CONSENT_PROVISION_TYPE: &str = "HL7_ConsentProvisionType";

/// HL7 Terminology `http://terminology.hl7.org/CodeSystem/extra-security-role-type`,
/// by its terminology identifier `HL7_` and its code system name: its
/// `datasubject` is the subject of the data.
pub const TERMINOLOGY_ID_SECURITY_ROLE_TYPE: &str = "HL7_SecurityRoleType";

/// HL7 Terminology `http://terminology.hl7.org/CodeSystem/v3-RoleClass`, by its
/// terminology identifier `HL7_` and its code system name: its `MBR` is the
/// member of a group.
pub const TERMINOLOGY_ID_ROLE_CLASS: &str = "HL7_RoleClass";

/// HL7 FHIR `http://hl7.org/fhir/audit-event-action`, by its terminology
/// identifier `HL7_` and its code system name: the create, read, update and
/// delete of an audited access.
pub const TERMINOLOGY_ID_AUDIT_EVENT_ACTION: &str = "HL7_AuditEventAction";

/// HL7 Terminology `http://terminology.hl7.org/CodeSystem/v3-Confidentiality`,
/// the BMMs' `Terminology_id_confidentiality`: the levels a record or a kind
/// of data carries.
pub const TERMINOLOGY_ID_CONFIDENTIALITY: &str = "HL7_Confidentiality";

/// HL7 Terminology `v3-ParticipationType` by `HL7_` and its code system
/// name: the code system of an unbound `care_role`.
pub const TERMINOLOGY_ID_PARTICIPATION_TYPE: &str = "HL7_ParticipationType";

/// A `DV_CODED_TEXT` of `code` in `terminology`, displayed as `rubric`.
#[must_use]
pub fn coded_text(terminology: &str, code: &str, rubric: &str) -> DvCodedText {
    DvCodedText {
        value: rubric.to_owned(),
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
    }
}

/// One group of the draft access terminology.
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
    /// `care path`: how a care relationship reaches the caller.
    AccessCarePath,
    /// `access caller kind`: the kind of caller a directive binds.
    AccessCallerKind,
    /// `access operation`: the operation classes a directive names.
    AccessOperation,
    /// `access conflict`: a conflict of interest between a professional
    /// caller and the subject.
    AccessConflict,
    /// `access value set binding`: what a value set of the policy codes.
    AccessValueSetBinding,
    /// `care relationship`: the demographic relationships that put a caller
    /// in care of a subject.
    CareRelationship,
    /// `care relationship element`: the elements in the `details` of an
    /// `under_care_of` relationship.
    CareRelationshipElement,
    /// `care basis`: who wrote a care relationship.
    CareBasis,
    /// `group kind`: what kind of group a demographic `GROUP` is.
    GroupKind,
    /// `record subject`: the demographic element naming an EHR subject
    /// reference the `PERSON` stands for.
    RecordSubject,
    /// `representation`: the demographic relationship that makes its source
    /// a representative of its target.
    Representation,
    /// `representation element`: the elements in the `details` of a
    /// `represents` relationship.
    RepresentationElement,
    /// `access refusal`: the code an access refusal is recorded under.
    AccessRefusal,
    /// `access log withheld`: the attributes of an access-log accessor the
    /// subject view withholds under EHDS Art. 9(3).
    AccessLogWithheld,
    /// `access action`: what an access recorded in the access log did.
    AccessAction,
    /// `access outcome`: the result of an access the access log records.
    AccessOutcome,
    /// `accessor kind`: the kind of accessor the access log records.
    AccessorKind,
    /// `access authorisation`: the kind of ground an access ran under.
    AccessAuthorisation,
    /// `access refusal reason`: why an access was refused, beside the
    /// `access refusal` codes of the decision procedure.
    AccessRefusalReason,
    /// `credential binding`: how the system bound a credential to a party,
    /// or why it could not.
    CredentialBinding,
}

impl Group {
    /// Every group, in contract order.
    pub const ALL: [Self; 23] = [
        Self::AccessDefault,
        Self::AccessDirectiveKind,
        Self::AccessEffect,
        Self::AccessRelationship,
        Self::AccessCarePath,
        Self::AccessCallerKind,
        Self::AccessOperation,
        Self::AccessConflict,
        Self::AccessValueSetBinding,
        Self::CareRelationship,
        Self::CareRelationshipElement,
        Self::CareBasis,
        Self::GroupKind,
        Self::RecordSubject,
        Self::Representation,
        Self::RepresentationElement,
        Self::AccessRefusal,
        Self::AccessLogWithheld,
        Self::AccessAction,
        Self::AccessOutcome,
        Self::AccessorKind,
        Self::AccessAuthorisation,
        Self::AccessRefusalReason,
        Self::CredentialBinding,
    ];

    /// The group's rubric, such as `access default`.
    #[must_use]
    pub const fn rubric(self) -> &'static str {
        match self {
            Self::AccessDefault => "access default",
            Self::AccessDirectiveKind => "access directive kind",
            Self::AccessEffect => "access effect",
            Self::AccessRelationship => "access relationship",
            Self::AccessCarePath => "care path",
            Self::AccessCallerKind => "access caller kind",
            Self::AccessOperation => "access operation",
            Self::AccessConflict => "access conflict",
            Self::AccessValueSetBinding => "access value set binding",
            Self::CareRelationship => "care relationship",
            Self::CareRelationshipElement => "care relationship element",
            Self::CareBasis => "care basis",
            Self::GroupKind => "group kind",
            Self::RecordSubject => "record subject",
            Self::Representation => "representation",
            Self::RepresentationElement => "representation element",
            Self::AccessRefusal => "access refusal",
            Self::AccessLogWithheld => "access log withheld",
            Self::AccessAction => "access action",
            Self::AccessOutcome => "access outcome",
            Self::AccessorKind => "accessor kind",
            Self::AccessAuthorisation => "access authorisation",
            Self::AccessRefusalReason => "access refusal reason",
            Self::CredentialBinding => "credential binding",
        }
    }

    /// The `(terminology id, code)` of every term of the group, in contract
    /// order.
    #[must_use]
    pub fn phrases(self) -> Vec<(&'static str, &'static str)> {
        match self {
            Self::AccessDefault => phrases::<AccessDefault>(),
            Self::AccessDirectiveKind => phrases::<DirectiveKind>(),
            Self::AccessEffect => phrases::<Effect>(),
            Self::AccessRelationship => phrases::<Relationship>(),
            Self::AccessCarePath => phrases::<CarePath>(),
            Self::AccessCallerKind => phrases::<CallerKind>(),
            Self::AccessOperation => phrases::<OperationClass>(),
            Self::AccessConflict => phrases::<Conflict>(),
            Self::AccessValueSetBinding => phrases::<ValueSetBinding>(),
            Self::CareRelationship => phrases::<CareRelationship>(),
            Self::CareRelationshipElement => phrases::<CareElement>(),
            Self::CareBasis => phrases::<CareBasis>(),
            Self::GroupKind => phrases::<GroupKind>(),
            Self::RecordSubject => phrases::<RecordSubject>(),
            Self::Representation => phrases::<Representation>(),
            Self::RepresentationElement => phrases::<RepresentationElement>(),
            Self::AccessRefusal => phrases::<Refusal>(),
            Self::AccessLogWithheld => phrases::<Withheld>(),
            Self::AccessAction => phrases::<Action>(),
            Self::AccessOutcome => phrases::<Outcome>(),
            Self::AccessorKind => phrases::<AccessorKind>(),
            Self::AccessAuthorisation => phrases::<Authorisation>(),
            Self::AccessRefusalReason => phrases::<RefusalReason>(),
            Self::CredentialBinding => phrases::<CredentialBinding>(),
        }
    }

    /// Whether `code` of `terminology` is a term of the group.
    #[must_use]
    pub fn holds(self, terminology: &str, code: &str) -> bool {
        self.phrases().contains(&(terminology, code))
    }
}

/// The `(terminology id, code)` of every term of `T`.
fn phrases<T: Term>() -> Vec<(&'static str, &'static str)> {
    T::ALL.iter().map(|term| term.phrase()).collect()
}

impl fmt::Display for Group {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.rubric())
    }
}

/// A coded value that is no term of the group its attribute is bound to.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OutsideGroup {
    /// The value is of a terminology the group holds no term of.
    #[error(
        "{terminology}::{code} is of no access group: the {group} group holds no {terminology} code"
    )]
    Terminology {
        /// The group the attribute is bound to.
        group: Group,
        /// The value's terminology id.
        terminology: String,
        /// The value's code.
        code: String,
    },
    /// The value is a code of a terminology of the group that the group does
    /// not hold.
    #[error("{terminology}::{code} is no code of the {group} group")]
    Code {
        /// The group the attribute is bound to.
        group: Group,
        /// The value's terminology id.
        terminology: String,
        /// The value's code.
        code: String,
    },
}

/// The terms of one access group, read from coded values.
pub trait Term: Copy + PartialEq + Sized + 'static {
    /// The group the terms belong to.
    const GROUP: Group;
    /// Every term of the group, in contract order.
    const ALL: &'static [Self];

    /// The term's terminology id: [`TERMINOLOGY_ID_ACCESS`], or the HL7 code
    /// system whose code the term is.
    fn terminology(self) -> &'static str;

    /// The term's code in [`Term::terminology`].
    fn code(self) -> &'static str;

    /// The term's rubric, the text a `DV_CODED_TEXT` of it shows.
    fn rubric(self) -> &'static str;

    /// The term's `(terminology id, code)`.
    fn phrase(self) -> (&'static str, &'static str) {
        (self.terminology(), self.code())
    }

    /// The term as a `DV_CODED_TEXT`, displayed as its rubric.
    fn coded(self) -> DvCodedText {
        coded_text(self.terminology(), self.code(), self.rubric())
    }

    /// Reads the term a `CODE_PHRASE` of `terminology` and `code` names.
    ///
    /// # Errors
    /// [`OutsideGroup`] when no term of the group is of `terminology`, or
    /// none of `terminology` has `code`, compared exactly.
    fn read(terminology: &str, code: &str) -> Result<Self, OutsideGroup> {
        let mut of_terminology = Self::ALL
            .iter()
            .copied()
            .filter(|term| term.terminology() == terminology)
            .peekable();
        if of_terminology.peek().is_none() {
            return Err(OutsideGroup::Terminology {
                group: Self::GROUP,
                terminology: terminology.to_owned(),
                code: code.to_owned(),
            });
        }
        of_terminology
            .find(|term| term.code() == code)
            .ok_or_else(|| OutsideGroup::Code {
                group: Self::GROUP,
                terminology: terminology.to_owned(),
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
///
/// A term is `Name => "code"`, a code of [`TERMINOLOGY_ID_ACCESS`] that is
/// its own rubric, or `Name => "code" of TERMINOLOGY, "rubric"`, the code of
/// another terminology.
macro_rules! group {
    (@terminology) => { TERMINOLOGY_ID_ACCESS };
    (@terminology $terminology:ident) => { $terminology };
    (@rubric $code:literal) => { $code };
    (@rubric $code:literal $rubric:literal) => { $rubric };
    (
        $(#[$doc:meta])*
        $name:ident in $group:ident {
            $( $(#[$term_doc:meta])* $term:ident => $code:literal $(of $terminology:ident, $rubric:literal)?, )+
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

            fn terminology(self) -> &'static str {
                match self {
                    $(Self::$term => group!(@terminology $($terminology)?),)+
                }
            }

            fn code(self) -> &'static str {
                match self {
                    $(Self::$term => $code,)+
                }
            }

            fn rubric(self) -> &'static str {
                match self {
                    $(Self::$term => group!(@rubric $code $($rubric)?),)+
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

pub mod care;
pub mod hl7;
pub mod log;
pub mod representation;

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
        /// A rule of the controller's `ACCESS_POLICY` for every EHR it keeps,
        /// never a directive of an EHR (EHDS Art. 11(3)).
        ControllerRule => "controller_rule",
    }
}

group! {
    /// Whether a directive permits or denies (`ACCESS_DIRECTIVE.effect`).
    Effect in AccessEffect {
        /// The directive admits what it covers: HL7 FHIR
        /// `HL7_ConsentProvisionType#permit`.
        Permit => "permit" of TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, "permit",
        /// The directive refuses what it covers; a deny overrides a permit:
        /// HL7 FHIR `HL7_ConsentProvisionType#deny`.
        Deny => "deny" of TERMINOLOGY_ID_CONSENT_PROVISION_TYPE, "deny",
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
        /// The subject of the EHR: HL7 `HL7_SecurityRoleType#datasubject`.
        Subject => "datasubject" of TERMINOLOGY_ID_SECURITY_ROLE_TYPE, "subject",
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
    /// A conflict of interest between a professional caller and the subject
    /// (`ACCESS_CONFLICT_RULE.conflict`).
    Conflict in AccessConflict {
        /// The caller's own `PERSON` is the subject's.
        SelfAccess => "self",
        /// A `related_to` relationship in force joins the caller and the
        /// subject.
        Related => "related",
    }
}

group! {
    /// What a value set of the policy codes (`ACCESS_VALUE_SET.binding`).
    ValueSetBinding in AccessValueSetBinding {
        /// The role of a care relationship and of a rule's care roles.
        CareRole => "care_role",
        /// The basis of a representation.
        RepresentationBasis => "representation_basis",
        /// The kinship of a `related_to` relationship and of a conflict rule.
        Kinship => "kinship",
        /// The type of an `ORGANISATION` a rule or a validity entry names.
        OrganisationType => "organisation_type",
        /// The purposes for which a care relationship is a ground.
        CarePurpose => "care_purpose",
        /// The purposes the system reads as declaring an emergency.
        EmergencyPurpose => "emergency_purpose",
    }
}

group! {
    /// The demographic relationships of care, coded in the `name` of a
    /// `PARTY_RELATIONSHIP`.
    CareRelationship in CareRelationship {
        /// A professional's `PERSON` is a member of a care-team `GROUP`: HL7
        /// v3 `RoleClass#MBR`, "a member of a group".
        MemberOf => "MBR" of TERMINOLOGY_ID_ROLE_CLASS, "member of",
        /// A patient's `PERSON` is under the care of a team `GROUP` or an
        /// organisational unit.
        UnderCareOf => "under_care_of",
    }
}

group! {
    /// What kind of group a demographic `GROUP` is, coded in its `name`: RM
    /// demographic `party.adoc` makes `type()` the `name` (`Type_valid`).
    GroupKind in GroupKind {
        /// The `GROUP` is a care team: a membership (`MBR`) relationship to it puts
        /// its member in the care of the subjects under its care.
        CareTeam => "care_team",
    }
}

group! {
    /// The element of a `PERSON`'s `details` naming an EHR subject reference.
    ///
    /// The reference is one the `PERSON` stands for, such as a pseudonym an
    /// identity source issued: the element's value is a `DV_IDENTIFIER` whose
    /// `issuer` is the reference's namespace and whose `id` is its id.
    RecordSubject in RecordSubject {
        /// The element naming an `EHR_STATUS` subject reference.
        RecordSubject => "record_subject",
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
        /// A deny rule of the controller's policy applies, with no exception
        /// that applies; no permit of the subject lifts it.
        PolicyDenied => "refused-policy-denied",
        /// The data carry a sensitivity the caller is not cleared for.
        Sensitivity => "refused-sensitivity",
        /// The policy needs a care relationship and the caller has none.
        NoCareRelationship => "refused-no-care-relationship",
        /// The policy needs a declared purpose and the request has none.
        PurposeMissing => "refused-purpose-missing",
        /// The EHR needs consent and no permit directive applies.
        ConsentRequired => "refused-consent-required",
        /// A subject caller asks for an EHR other than the one its patient
        /// context names: it is the subject of that EHR alone.
        NotSubject => "refused-not-subject",
        /// A representative asks for an EHR other than the one whose
        /// subject a recorded representation names: it acts for that
        /// subject alone.
        NotRepresented => "refused-not-represented",
        /// A caller admitted under an agreement that conditions its reads on
        /// a consent decision asks to read with no reference to one.
        ConsentReferenceMissing => "refused-consent-reference-missing",
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

/// A confidentiality level of [`TERMINOLOGY_ID_CONFIDENTIALITY`], in the
/// package's order `N` < `R` < `V`.
///
/// The code system's `U`, `L` and `M` read as `N`, as the package fixes
/// (draft access control Terminology, Codes from Other Terminologies).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Confidentiality {
    /// `N`, normal, and the `U`, `L` and `M` that read as it.
    Normal,
    /// `R`, restricted.
    Restricted,
    /// `V`, very restricted.
    VeryRestricted,
}

/// A coded value that is no confidentiality level.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{terminology}::{code} is no {TERMINOLOGY_ID_CONFIDENTIALITY} level")]
pub struct NotConfidentiality {
    /// The value's terminology id.
    pub terminology: String,
    /// The value's code.
    pub code: String,
}

impl Confidentiality {
    /// Reads the level `coded` names.
    ///
    /// # Errors
    /// [`NotConfidentiality`] when `coded` is of another terminology than
    /// [`TERMINOLOGY_ID_CONFIDENTIALITY`], or names no code of it.
    pub fn read(coded: &DvCodedText) -> Result<Self, NotConfidentiality> {
        let phrase = &coded.defining_code;
        let level = match (
            phrase.terminology_id.value.as_str(),
            phrase.code_string.as_str(),
        ) {
            (TERMINOLOGY_ID_CONFIDENTIALITY, "U" | "L" | "M" | "N") => Some(Self::Normal),
            (TERMINOLOGY_ID_CONFIDENTIALITY, "R") => Some(Self::Restricted),
            (TERMINOLOGY_ID_CONFIDENTIALITY, "V") => Some(Self::VeryRestricted),
            _ => None,
        };
        level.ok_or_else(|| NotConfidentiality {
            terminology: phrase.terminology_id.value.clone(),
            code: phrase.code_string.clone(),
        })
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
    /// A role a `ROLE` is named by, such as a professional role code
    /// (RM Demographic IM, `PARTY.type()`).
    Role,
    /// A type an `ORGANISATION` is named by, such as a healthcare facility
    /// type code (RM Demographic IM, `PARTY.type()`).
    OrganisationType,
    /// The part a carer plays in a care relationship, its `care_role`, such
    /// as an HL7 `ParticipationType` code.
    CareRole,
    /// The `basis` of a representation, such as an HL7 `RoleCode` code.
    RepresentationBasis,
}

impl fmt::Display for ForeignKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sensitivity => "sensitivity",
            Self::Category => "category",
            Self::Purpose => "purpose",
            Self::LegalGround => "legal ground",
            Self::Role => "role",
            Self::OrganisationType => "organisation type",
            Self::CareRole => "care role",
            Self::RepresentationBasis => "representation basis",
        })
    }
}

/// A coded value of [`TERMINOLOGY_ID_ACCESS`], which holds no code of any
/// [`ForeignKind`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "{TERMINOLOGY_ID_ACCESS}::{code} is no {kind}: {TERMINOLOGY_ID_ACCESS} holds no {kind} codes"
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
    /// [`NotForeign`] when `coded` is of [`TERMINOLOGY_ID_ACCESS`], which holds no code
    /// of any [`ForeignKind`].
    pub fn read(kind: ForeignKind, coded: &DvCodedText) -> Result<Self, NotForeign> {
        let phrase = &coded.defining_code;
        if phrase.terminology_id.value == TERMINOLOGY_ID_ACCESS {
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
mod tests;
