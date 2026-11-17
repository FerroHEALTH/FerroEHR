// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the access evaluator reads about one caller beside its grants.
//!
//! They are the roles it performs, its care teams, its provider's `GROUP`s
//! and the types of the `ORGANISATION` it acts for. No openEHR spec gives
//! these an access meaning: our own design (#5621, #5691).

use openehr_rm::draft::access_control::terminology::{ForeignCode, ForeignKind};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use uuid::Uuid;

use crate::party_binding::grants::caller::CareTeam;

/// What the access evaluator reads about one caller beside its grants.
///
/// They are the roles of the `ROLE`s in force the caller performs, the care
/// teams it is a member of on the request's date, the `GROUP`s the provider
/// a client acts for is a member of on that date, and the types of the
/// `ORGANISATION` it acts for. A caller admitted under an agreement adds the
/// roles and organisation types its verified credential attests
/// ([`Self::with_attested`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallerFacts {
    roles: Vec<ForeignCode>,
    care_teams: Vec<CareTeam>,
    provider_groups: Vec<Uuid>,
    organisation_types: Vec<ForeignCode>,
    party: Option<Uuid>,
}

impl CallerFacts {
    /// The facts holding `roles` and `care_teams`, each sorted and distinct.
    #[must_use]
    pub fn new(mut roles: Vec<ForeignCode>, mut care_teams: Vec<CareTeam>) -> Self {
        roles.sort();
        roles.dedup();
        care_teams.sort();
        care_teams.dedup();
        Self {
            roles,
            care_teams,
            provider_groups: Vec::new(),
            organisation_types: Vec::new(),
            party: None,
        }
    }

    /// These facts as the facts of the party `party`, which a request whose
    /// copy was older than the store reads again (#5806).
    #[must_use]
    pub fn with_party(mut self, party: Uuid) -> Self {
        self.party = Some(party);
        self
    }

    /// The party these facts are of; `None` for facts no copy names.
    #[must_use]
    pub fn party(&self) -> Option<Uuid> {
        self.party
    }

    /// These facts with `types`, the coded names of the `ORGANISATION` the
    /// caller acts for, sorted and distinct.
    #[must_use]
    pub fn with_organisation_types(mut self, mut types: Vec<ForeignCode>) -> Self {
        types.sort();
        types.dedup();
        self.organisation_types = types;
        self
    }

    /// These facts with the professional `roles` and the organisation
    /// `types` a caller's verified credential attests, beside those the
    /// demographic service holds.
    #[must_use]
    pub fn with_attested(mut self, roles: Vec<ForeignCode>, types: Vec<ForeignCode>) -> Self {
        // NOTE: our own design (#5691), a role or organisation type an issuer attests is read as
        // the CDR's own, so one controller rule binds a local and an external caller alike.
        self.roles.extend(roles);
        self.roles.sort();
        self.roles.dedup();
        self.organisation_types.extend(types);
        self.organisation_types.sort();
        self.organisation_types.dedup();
        self
    }

    /// These facts with `groups`, the `GROUP`s the client's provider is a
    /// member of, sorted and distinct.
    #[must_use]
    pub fn with_provider_groups(mut self, mut groups: Vec<Uuid>) -> Self {
        groups.sort_unstable();
        groups.dedup();
        self.provider_groups = groups;
        self
    }

    /// The roles of the `ROLE`s in force the caller performs.
    #[must_use]
    pub fn roles(&self) -> &[ForeignCode] {
        &self.roles
    }

    /// Whether the caller performs a `ROLE` in force named `role`, compared
    /// by terminology id and code (`ACCESS_ACTOR_SCOPE.role`).
    #[must_use]
    pub fn performs(&self, role: &DvCodedText) -> bool {
        ForeignCode::read(ForeignKind::Role, role)
            .is_ok_and(|code| self.roles.binary_search(&code).is_ok())
    }

    /// The types of the `ORGANISATION` the caller acts for.
    #[must_use]
    pub fn organisation_types(&self) -> &[ForeignCode] {
        &self.organisation_types
    }

    /// Whether the caller acts for an `ORGANISATION` of the type `kind`,
    /// compared by terminology id and code
    /// (`ACCESS_ACTOR_SCOPE.organisation_type`).
    #[must_use]
    pub fn acts_for_type(&self, kind: &DvCodedText) -> bool {
        ForeignCode::read(ForeignKind::OrganisationType, kind)
            .is_ok_and(|code| self.organisation_types.binary_search(&code).is_ok())
    }

    /// The care teams the caller is a member of on the request's date.
    #[must_use]
    pub fn care_teams(&self) -> &[CareTeam] {
        &self.care_teams
    }

    /// The containers of the `GROUP`s the provider a client acts for is a
    /// member of on the request's date, by a membership (`MBR`) relationship of the
    /// provider `ORGANISATION`.
    #[must_use]
    pub fn provider_groups(&self) -> &[Uuid] {
        &self.provider_groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(terminology: &str, code: &str) -> ForeignCode {
        ForeignCode {
            terminology: terminology.to_owned(),
            code: code.to_owned(),
        }
    }

    fn named(terminology: &str, value: &str) -> DvCodedText {
        openehr_its::json::from_canonical_value(&serde_json::json!({
            "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": {"_type": "CODE_PHRASE",
                              "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
                              "code_string": value}}))
        .expect("a code")
    }

    #[test]
    fn provider_groups_are_sorted_and_distinct() {
        let (a, b) = (Uuid::from_u128(2), Uuid::from_u128(1));
        let facts = CallerFacts::default().with_provider_groups(vec![a, b, a]);
        assert_eq!(facts.provider_groups(), &[b, a]);
    }

    #[test]
    fn caller_facts_answer_by_terminology_and_code() {
        let nurse = code("urn:example:roles", "NURSE");
        let facts = CallerFacts::new(vec![nurse.clone(), nurse], Vec::new());
        assert_eq!(facts.roles().len(), 1);
        assert!(facts.performs(&named("urn:example:roles", "NURSE")));
        assert!(!facts.performs(&named("local", "NURSE")));
        assert!(!facts.performs(&named("urn:example:roles", "nurse")));
    }

    #[test]
    fn an_attested_role_and_organisation_type_join_the_held_facts() {
        let facts = CallerFacts::new(vec![code("urn:example:roles", "NURSE")], Vec::new())
            .with_organisation_types(vec![code("urn:example:organisation-types", "GP")])
            .with_attested(
                vec![code("urn:example:roles", "PHYSICIAN")],
                vec![code("urn:example:organisation-types", "HOSPITAL")],
            );
        assert!(facts.performs(&named("urn:example:roles", "NURSE")));
        assert!(facts.performs(&named("urn:example:roles", "PHYSICIAN")));
        assert!(facts.acts_for_type(&named("urn:example:organisation-types", "GP")));
        assert!(facts.acts_for_type(&named("urn:example:organisation-types", "HOSPITAL")));
        assert!(!facts.acts_for_type(&named("urn:example:organisation-types", "INSURER")));
        assert!(!facts.acts_for_type(&named("local", "GP")));
    }
}
