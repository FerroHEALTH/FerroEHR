// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `care_role` of a care relationship and the `basis` of a
//! representation checked at write against the value set the access policy
//! in force binds for each, on the terminology server; unbound, a role is a
//! code of `HL7_ParticipationType` and a basis of `HL7_RoleCode` (draft
//! access control package, Care Relationships and Terminology, Value Sets).
//! A relationship whose code is outside it is refused `422` naming the
//! element, as content
//! that "was well-formed but was unable to be followed due to semantic
//! errors" (ITS-REST overview `Requests_and_responses.md` §HTTP status
//! codes). The check runs before the commit holds a connection. Which
//! failure is `422` is our own design.

use openehr_base::validate::InvariantViolation;
use openehr_rm::draft::access_control::terminology::representation::{
    Representation, TERMINOLOGY_ID_ROLE_CODE,
};
use openehr_rm::draft::access_control::terminology::{
    CareRelationship, ForeignCode, ForeignKind, TERMINOLOGY_ID_PARTICIPATION_TYPE, Term,
    ValueSetBinding,
};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use crate::access_policy::care::CareFacts;
use crate::error::ServiceError;
use crate::party_binding::grants::caller::care_of;
use crate::party_binding::policy_mark::carries;
use crate::service::FerroEhrService;
use crate::versioning::tree::Tree;

/// The path a refusal names: the `care_role` element of the relationship's
/// `details`.
const AT: &str = "/details/care_role";

/// The path a refusal names: the `basis` element of a representation's
/// `details`.
const BASIS_AT: &str = "/details/basis";

impl FerroEhrService {
    /// Refuse the `PARTY_RELATIONSHIP` `body` when it is a care relationship
    /// whose `care_role` is outside the value set the policy in force binds
    /// for it, or, unbound, of a terminology other than
    /// `HL7_ParticipationType`. A relationship with no readable role is left
    /// to the commit, which refuses it naming why.
    ///
    /// The policy in force is first confirmed on the one indexed read of the
    /// policy heads ([`Self::confirm_policy`]), before the commit holds a
    /// party connection, so the commit's bounds on the relationship
    /// ([`crate::party_binding::grants::caller::bounds`]) read the version
    /// that read found and name it.
    ///
    /// A care relationship's writer is then judged at the confidentiality of
    /// the subject's record ([`Self::care_writer_check`]), both reads given
    /// back before the commit takes its party connection.
    ///
    /// # Errors
    /// [`ServiceError::ValidationFailed`] (`422`) naming the element; the
    /// `503` of [`Self::confirm_policy`]; the `403` of
    /// [`Self::care_writer_check`].
    pub(crate) async fn care_role_check<'a, T: Tree<'a>>(
        &self,
        body: T,
    ) -> Result<(), ServiceError> {
        let care = CareRelationship::UnderCareOf;
        let represents = Representation::Represents;
        // NOTE: our own design (#6055, #6079), a code that does not read is refused by the
        // commit, which names why; this check judges only one that reads.
        let cares = carries(body, care.terminology(), care.code());
        let (coded, binding, unbound, at) = if cares {
            let Ok(role) = care_of::role_text(body) else {
                return Ok(());
            };
            (
                role,
                ValueSetBinding::CareRole,
                TERMINOLOGY_ID_PARTICIPATION_TYPE,
                AT,
            )
        } else if carries(body, represents.terminology(), represents.code()) {
            let Ok(basis) = crate::party_binding::representation::basis_text(body) else {
                return Ok(());
            };
            (
                basis,
                ValueSetBinding::RepresentationBasis,
                TERMINOLOGY_ID_ROLE_CODE,
                BASIS_AT,
            )
        } else {
            return Ok(());
        };
        self.confirm_policy(false, true).await?;
        if cares
            && let Some(level) = self.care_writer_check(body).await?
            && let Some(recorder) = crate::system_log::recorder::current()
        {
            recorder.mark_for_review(level);
        }
        let in_force = self.access_policy().in_force();
        let facts = CareFacts::of(in_force.policy.as_deref());
        self.bound_code(facts, (binding, unbound), at, &coded)
            .await
            .map_or(Ok(()), |violation| {
                Err(ServiceError::ValidationFailed(vec![violation]))
            })
    }

    /// The care role a verified credential attests for the `attested` path,
    /// when `attested` is a code of the `care_role` value set the policy in
    /// force binds, or, unbound, of `HL7_ParticipationType`; `None` when it
    /// attests none, so the relationship takes `PART` (draft access control
    /// decision procedure, Holding a Care Relationship, `attested`).
    ///
    /// A code the value set does not hold, or one no terminology server could
    /// check under `terminology.on_error = "refuse"`, is no role of the value
    /// set, so it reads as `PART` and never as the role it names.
    pub async fn attested_care_role(&self, attested: Option<&DvCodedText>) -> Option<ForeignCode> {
        let coded = attested?;
        // NOTE: our own design (#6078), a code of the access terminology is no care role, so the
        // credential attests none and the relationship takes PART.
        let role = ForeignCode::read(ForeignKind::CareRole, coded).ok()?;
        let in_force = self.access_policy().in_force();
        let facts = CareFacts::of(in_force.policy.as_deref());
        match self.care_role(facts, "trc.care_role", coded).await {
            None => Some(role),
            Some(violation) => {
                tracing::info!(
                    reason = %violation.message,
                    "an attested care role is outside the care_role value set; the attested \
                     relationship takes PART"
                );
                None
            }
        }
    }

    /// The refusal of the care role `role` at `path`: outside the value set
    /// `facts` bind for `care_role`, or, unbound, of a terminology other than
    /// `HL7_ParticipationType`.
    pub(crate) async fn care_role(
        &self,
        facts: CareFacts<'_>,
        path: &str,
        role: &DvCodedText,
    ) -> Option<InvariantViolation> {
        let binding = (ValueSetBinding::CareRole, TERMINOLOGY_ID_PARTICIPATION_TYPE);
        self.bound_code(facts, binding, path, role).await
    }

    /// The refusal of `coded` at `path`: outside the value set `facts` bind
    /// for `binding`, or, unbound, of a terminology other than `unbound`.
    async fn bound_code(
        &self,
        facts: CareFacts<'_>,
        (binding, unbound): (ValueSetBinding, &str),
        path: &str,
        coded: &DvCodedText,
    ) -> Option<InvariantViolation> {
        if let Some(url) = facts.value_set(binding) {
            return self.in_value_set(path, url, coded).await;
        }
        let phrase = &coded.defining_code;
        (phrase.terminology_id.value != unbound).then(|| {
            InvariantViolation::at(
                path,
                format!(
                    "{}::{} is no code of {unbound}, which codes a {} the access policy binds \
                     no value set for",
                    phrase.terminology_id.value,
                    phrase.code_string,
                    binding.code()
                ),
            )
        })
    }
}
