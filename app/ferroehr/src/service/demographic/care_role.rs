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
    /// # Errors
    /// [`ServiceError::ValidationFailed`] (`422`) naming the element; the
    /// `503` of [`Self::refresh_access_policy`].
    pub(crate) async fn care_role_check<'a, T: Tree<'a>>(
        &self,
        body: T,
    ) -> Result<(), ServiceError> {
        let care = CareRelationship::UnderCareOf;
        if !carries(body, care.terminology(), care.code()) {
            return Ok(());
        }
        // NOTE: our own design (#6055), a role that does not read is refused by the commit,
        // which names why; this check judges only a role that reads.
        let Ok(role) = care_of::role_text(body) else {
            return Ok(());
        };
        self.refresh_access_policy().await?;
        let in_force = self.access_policy().in_force();
        let facts = CareFacts::of(in_force.policy.as_deref());
        let phrase = &role.defining_code;
        let wrong = if let Some(url) = facts.value_set(ValueSetBinding::CareRole) {
            self.in_value_set(AT, url, &role).await
        } else {
            (phrase.terminology_id.value != TERMINOLOGY_ID_PARTICIPATION_TYPE).then(|| {
                InvariantViolation::at(
                    AT,
                    format!(
                        "{}::{} is no code of {TERMINOLOGY_ID_PARTICIPATION_TYPE}, which codes a \
                         care_role the access policy binds no value set for",
                        phrase.terminology_id.value, phrase.code_string
                    ),
                )
            })
        };
        wrong.map_or(Ok(()), |violation| {
            Err(ServiceError::ValidationFailed(vec![violation]))
        })
    }
}
