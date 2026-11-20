// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `care_role` of a care relationship checked at write against the value
//! set the access policy in force binds for it, on the terminology server;
//! unbound, it is a code of `HL7_ParticipationType` (draft access control
//! package, Care Relationships and Terminology, Value Sets). A relationship
//! whose role is outside it is refused `422` naming the element, as content
//! that "was well-formed but was unable to be followed due to semantic
//! errors" (ITS-REST overview `Requests_and_responses.md` §HTTP status
//! codes). The check runs before the commit holds a connection. Which
//! failure is `422` is our own design.

use openehr_base::validate::InvariantViolation;
use openehr_rm::draft::access_control::terminology::{
    CareRelationship, TERMINOLOGY_ID_PARTICIPATION_TYPE, Term, ValueSetBinding,
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
    /// # Errors
    /// [`ServiceError::ValidationFailed`] (`422`) naming the element; the
    /// `503` of [`Self::confirm_policy`].
    pub(crate) async fn care_role_check<'a, T: Tree<'a>>(
        &self,
        body: T,
    ) -> Result<(), ServiceError> {
        let care = CareRelationship::UnderCareOf;
        if !carries(body, care.terminology(), care.code()) {
            return Ok(());
        }
        self.confirm_policy(false, true).await?;
        // NOTE: our own design (#6055), a role that does not read is refused by the commit,
        // which names why; this check judges only a role that reads.
        let Ok(role) = care_of::role_text(body) else {
            return Ok(());
        };
        let in_force = self.access_policy().in_force();
        let facts = CareFacts::of(in_force.policy.as_deref());
        self.care_role(facts, AT, &role)
            .await
            .map_or(Ok(()), |violation| {
                Err(ServiceError::ValidationFailed(vec![violation]))
            })
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
        if let Some(url) = facts.value_set(ValueSetBinding::CareRole) {
            return self.in_value_set(path, url, role).await;
        }
        let phrase = &role.defining_code;
        (phrase.terminology_id.value != TERMINOLOGY_ID_PARTICIPATION_TYPE).then(|| {
            InvariantViolation::at(
                path,
                format!(
                    "{}::{} is no code of {TERMINOLOGY_ID_PARTICIPATION_TYPE}, which codes a \
                     care_role the access policy binds no value set for",
                    phrase.terminology_id.value, phrase.code_string
                ),
            )
        })
    }
}
