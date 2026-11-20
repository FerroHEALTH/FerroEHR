// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What the access evaluator reads about one caller beside its grants.
//!
//! They are the roles it performs and the `ROLE`s they are, its care teams,
//! its provider's `GROUP`s, the types of the `ORGANISATION` it acts for, the
//! providers it acts for and the persons it is kin to. No openEHR spec gives
//! these an access meaning: our own design (#5621, #5691, #6055, #6077).

use openehr_rm::draft::access_control::terminology::{ForeignCode, ForeignKind};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use uuid::Uuid;

use crate::party_binding::grants::caller::CareTeam;
use crate::party_binding::grants::caller::care_of::ProviderKey;

/// What the access evaluator reads about one caller beside its grants.
///
/// They are the roles of the `ROLE`s in force the caller performs, the care
/// teams it is a member of on the request's date, the `GROUP`s the provider
/// a client acts for is a member of on that date, and the types of the
/// `ORGANISATION` it acts for. A caller admitted under an agreement adds the
/// roles and organisation types its verified credential attests
/// ([`Self::with_attested`]). A care relationship reaches it through its
/// principals ([`Self::principals`]) while it acts for the relationship's
/// provider ([`Self::acts_for_provider`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallerFacts {
    roles: Vec<ForeignCode>,
    performed: Vec<Uuid>,
    care_teams: Vec<CareTeam>,
    provider_groups: Vec<Uuid>,
    organisation_types: Vec<ForeignCode>,
    providers: Vec<ProviderKey>,
    kin: Vec<Kin>,
    party: Option<Uuid>,
}

/// A person the caller is kin to on the request's date, by a `related_to`
/// relationship in force in either direction, with the kinship it codes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Kin {
    /// The other person's `PERSON` container.
    pub person: Uuid,
    /// The kinship the relationship codes.
    pub kinship: ForeignCode,
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
            performed: Vec::new(),
            care_teams,
            provider_groups: Vec::new(),
            organisation_types: Vec::new(),
            providers: Vec::new(),
            kin: Vec::new(),
            party: None,
        }
    }

    /// These facts with `kin`, the persons the caller is kin to, sorted and
    /// distinct.
    #[must_use]
    pub fn with_kin(mut self, mut kin: Vec<Kin>) -> Self {
        kin.sort();
        kin.dedup();
        self.kin = kin;
        self
    }

    /// The kinships that join the caller to the `PERSON` `person`, sorted;
    /// empty when none does.
    #[must_use]
    pub fn kinships_with(&self, person: Uuid) -> Vec<ForeignCode> {
        self.kin
            .iter()
            .filter(|kin| kin.person == person)
            .map(|kin| kin.kinship.clone())
            .collect()
    }

    /// The persons the caller is kin to, each with its kinship.
    #[must_use]
    pub fn kin(&self) -> &[Kin] {
        &self.kin
    }

    /// Whether the caller performs a `ROLE` in force, or its credential
    /// attests a professional role: it reaches a record as a professional.
    #[must_use]
    pub fn performs_a_role(&self) -> bool {
        !self.performed.is_empty() || !self.roles.is_empty()
    }

    /// These facts with `roles`, the containers of the `ROLE`s in force the
    /// caller performs, sorted and distinct.
    #[must_use]
    pub fn with_performed(mut self, mut roles: Vec<Uuid>) -> Self {
        roles.sort_unstable();
        roles.dedup();
        self.performed = roles;
        self
    }

    /// These facts with `providers`, the providers the caller acts for, by
    /// the `issuer` and `id` a care relationship's `provider` names them by,
    /// beside those it holds, sorted and distinct.
    #[must_use]
    pub fn with_providers(mut self, providers: impl IntoIterator<Item = ProviderKey>) -> Self {
        self.providers.extend(providers);
        self.providers.sort();
        self.providers.dedup();
        self
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

    /// The containers of the `ROLE`s in force the caller performs.
    #[must_use]
    pub fn performed(&self) -> &[Uuid] {
        &self.performed
    }

    /// What a care relationship's target names to reach the caller, sorted
    /// and distinct: its `PERSON` and the `ROLE`s it performs (the `direct`
    /// path) and the care teams and units it is a member of (the `team`
    /// path; draft access control decision procedure, Holding a Care
    /// Relationship).
    #[must_use]
    pub fn principals(&self) -> Vec<Uuid> {
        let mut principals: Vec<Uuid> = self
            .party
            .into_iter()
            .chain(self.performed.iter().copied())
            .chain(self.care_teams.iter().map(CareTeam::team))
            .collect();
        principals.sort_unstable();
        principals.dedup();
        principals
    }

    /// The providers the caller acts for, sorted and distinct.
    #[must_use]
    pub fn providers(&self) -> &[ProviderKey] {
        &self.providers
    }

    /// Whether the caller acts for `provider`, a care relationship's
    /// `provider` compared by issuer and id.
    #[must_use]
    pub fn acts_for_provider(&self, provider: &ProviderKey) -> bool {
        self.providers.binary_search(provider).is_ok()
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
    fn kinships_are_read_per_person() {
        let (mother, brother) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let kin = |person, code: &str| Kin {
            person,
            kinship: self::code("openehr", code),
        };
        let facts = CallerFacts::default().with_kin(vec![
            kin(mother, "254"),
            kin(brother, "26"),
            kin(mother, "254"),
        ]);
        assert_eq!(facts.kin().len(), 2);
        assert_eq!(facts.kinships_with(mother), vec![code("openehr", "254")]);
        assert_eq!(facts.kinships_with(Uuid::from_u128(3)), Vec::new());
        assert!(!facts.performs_a_role());
        assert!(
            CallerFacts::default()
                .with_performed(vec![mother])
                .performs_a_role()
        );
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
