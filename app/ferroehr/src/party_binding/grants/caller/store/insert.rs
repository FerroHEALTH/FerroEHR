// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The insert of a head's caller facts into `binding_caller_fact`, one row
//! per fact with its columns read off the fact. No openEHR spec governs the
//! side table or its SQL: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): the capability and relationship \
              `time_validity` fragments the fact rows are derived from"
)]

use serde_json::Value;
use uuid::Uuid;

use super::super::care_of::CareFact;
use super::super::care_ref::{Cover, Episode};
use super::super::{CallerFact, FactValue};
use super::{FACT_TEXT, HOLDER_VERSION, WRITER};
use crate::party_binding::grants::derive::Bound;
use crate::party_binding::grants::store::GrantHead;
use crate::party_binding::step::{Step, Tag};
use openehr_rm::draft::access_control::terminology::ForeignCode;

/// The code a fact names: a role's, an organisation type's, a care
/// relationship's `care_role` or a kinship's; `None` for a membership.
fn code_of(fact: &CallerFact) -> Option<&ForeignCode> {
    match &fact.value {
        FactValue::Code(code) => Some(code),
        FactValue::Care(care) => Some(&care.role),
        FactValue::Kin(kin) => Some(&kin.kinship),
        FactValue::Team(_) => None,
    }
}

/// What a care relationship fact states; `None` for any other fact.
fn care_of(fact: &CallerFact) -> Option<&CareFact> {
    match &fact.value {
        FactValue::Care(care) => Some(care),
        FactValue::Code(_) | FactValue::Team(_) | FactValue::Kin(_) => None,
    }
}

/// The issuer, the id and the kind of the `covers_for` each of `facts`
/// names, each `None` for a fact that names none.
type CoversColumns<'a> = (
    Vec<Option<&'a str>>,
    Vec<Option<&'a str>>,
    Vec<Option<&'static str>>,
);

/// The [`CoversColumns`] of `facts`.
fn covers_of<'a>(facts: &[&'a CallerFact]) -> CoversColumns<'a> {
    let covers: Vec<Option<&Cover>> = facts
        .iter()
        .map(|f| care_of(f).and_then(|care| care.covers_for.as_ref()))
        .collect();
    (
        covers.iter().map(|c| c.map(|c| &*c.key.issuer)).collect(),
        covers.iter().map(|c| c.map(|c| &*c.key.id)).collect(),
        covers.iter().map(|c| c.map(|c| c.kind.rm_type())).collect(),
    )
}

/// The issuer and the id of the episode each of a set of facts names.
type EpisodeColumns<'a> = (Vec<Option<&'a str>>, Vec<Option<&'a str>>);

/// The [`EpisodeColumns`] of `facts`, each `None` for a fact that names none;
/// `None` when no fact names one, so the insert names no episode column.
fn episodes_of<'a>(facts: &[&'a CallerFact]) -> Option<EpisodeColumns<'a>> {
    let episodes: Vec<Option<&Episode>> = facts
        .iter()
        .map(|f| care_of(f).and_then(|care| care.episode.as_ref()))
        .collect();
    episodes.iter().any(Option::is_some).then(|| {
        episodes
            .iter()
            .map(|e| e.map(|e| (e.issuer.as_str(), e.id.as_str())))
            .map(|pair| (pair.map(|p| p.0), pair.map(|p| p.1)))
            .unzip()
    })
}

/// The container each of `facts` targets and, for a care relationship, the
/// carer's RM type; each `None` for a fact that has none.
fn targets_of(facts: &[&CallerFact]) -> (Vec<Option<Uuid>>, Vec<Option<&'static str>>) {
    facts
        .iter()
        .map(|f| {
            let target = match &f.value {
                FactValue::Team(team) => Some(*team),
                FactValue::Care(care) => Some(care.carer),
                FactValue::Kin(kin) => Some(kin.person),
                FactValue::Code(_) => None,
            };
            (target, care_of(f).map(|care| care.kind.rm_type()))
        })
        .unzip()
}

/// The issuer and the id of the provider each of `facts` names, each `None`
/// for a fact that is no care relationship.
fn providers_of<'a>(facts: &[&'a CallerFact]) -> (Vec<Option<&'a str>>, Vec<Option<&'a str>>) {
    facts
        .iter()
        .map(|f| care_of(f).map(|care| (care.provider.issuer.as_str(), care.provider.id.as_str())))
        .map(|pair| (pair.map(|p| p.0), pair.map(|p| p.1)))
        .unzip()
}

/// The insert's column list, select list, `unnest` arguments and alias list
/// entries of the episode columns, empty when `named` is false.
const fn episode_sql(named: bool) -> [&'static str; 4] {
    if named {
        [
            ", episode_issuer, episode_id",
            ", t.episode_issuer, t.episode_id",
            ", $21::text[], $22::text[]",
            ", episode_issuer, episode_id",
        ]
    } else {
        ["", "", "", ""]
    }
}

/// The step that inserts `facts` of `head`, each about the party at its
/// position in `about` ([`CallerFact::about`], else the head's), returning
/// each row's party and text ([`Tag::FactsCame`]); it reads nothing when
/// `facts` is empty.
pub(super) fn insert<'q>(
    head: GrantHead<'q>,
    about: Vec<Uuid>,
    role_validity: Option<&'q Value>,
    facts: &[&'q CallerFact],
) -> Step<'q> {
    let instant = |bound: Bound| match bound {
        Bound::At(at) => Some(jiff_sqlx::Timestamp::from(at)),
        Bound::Open | Bound::Unreadable => None,
    };
    let ordinal: Vec<i32> = facts.iter().map(|f| f.ordinal).collect();
    let kind: Vec<&str> = facts.iter().map(|f| f.kind.code()).collect();
    let terminology: Vec<Option<&str>> = facts
        .iter()
        .map(|f| code_of(f).map(|code| code.terminology.as_str()))
        .collect();
    let code_string: Vec<Option<&str>> = facts
        .iter()
        .map(|f| code_of(f).map(|code| code.code.as_str()))
        .collect();
    let (target, target_kind) = targets_of(facts);
    let (provider_issuer, provider_id) = providers_of(facts);
    let self_declared: Vec<bool> = facts
        .iter()
        .map(|f| care_of(f).is_some_and(|care| care.self_declared))
        .collect();
    let (covers_issuer, covers_id, covers_kind) = covers_of(facts);
    let validity: Vec<Option<&Value>> = facts.iter().map(|f| f.validity.as_ref()).collect();
    let unreadable: Vec<bool> = facts
        .iter()
        .map(|f| f.not_before == Bound::Unreadable || f.not_after == Bound::Unreadable)
        .collect();
    let not_before: Vec<_> = facts.iter().map(|f| instant(f.not_before)).collect();
    let not_after: Vec<_> = facts.iter().map(|f| instant(f.not_after)).collect();
    let episodes = episodes_of(facts);
    let [
        episode_columns,
        episode_values,
        episode_unnest,
        episode_names,
    ] = episode_sql(episodes.is_some());
    let statement = format!(
        "INSERT INTO binding_caller_fact \
             (holder_id, ordinal, kind, party_id, terminology_id, code_string, target_id, \
              target_kind, provider_issuer, provider_id, self_declared, writer_id, \
              lifecycle_state, role_validity, validity, not_before, not_after, \
              holder_version, covers_issuer, covers_id, covers_kind{episode_columns}) \
         SELECT $1, t.ordinal, t.kind, t.party_id, t.terminology_id, t.code_string, t.target_id, \
                t.target_kind, t.provider_issuer, t.provider_id, t.self_declared, writer.id, $3, \
                party.binding_validity($4::jsonb), \
                CASE WHEN t.unreadable THEN 'empty'::daterange \
                     WHEN c.effective_until IS NOT NULL AND t.kind = 'under_care_of' \
                          AND NOT isempty(party.binding_validity(t.validity)) \
                     THEN daterange(lower(party.binding_validity(t.validity)), c.effective_until) \
                     ELSE party.binding_validity(t.validity) END, \
                t.not_before, t.not_after, holder.version, t.covers_issuer, t.covers_id, \
                t.covers_kind{episode_values} \
         FROM unnest($5::integer[], $6::text[], $7::text[], $8::text[], $9::uuid[], \
                     $10::text[], $11::text[], $12::text[], \
                     $13::jsonb[], $14::boolean[], $15::timestamptz[], $16::timestamptz[], \
                     $17::boolean[], $18::text[], $19::text[], $20::text[], $2::uuid[]\
                     {episode_unnest}) \
              AS t(ordinal, kind, terminology_id, code_string, target_id, target_kind, \
                   provider_issuer, provider_id, validity, unreadable, not_before, not_after, \
                   self_declared, covers_issuer, covers_id, covers_kind, party_id\
                   {episode_names}) \
         CROSS JOIN (SELECT {WRITER} AS id) writer \
         CROSS JOIN (SELECT {HOLDER_VERSION} AS version) holder \
         LEFT JOIN binding_care_validity c ON c.vo_id = $1 \
         WHERE cardinality($5::integer[]) > 0 \
         ON CONFLICT DO NOTHING \
         RETURNING party_id AS p, {FACT_TEXT} AS t"
    );
    let writes = !facts.is_empty();
    Step::new("fi", statement, Some(Tag::FactsCame), move |query| {
        let query = query
            .bind(head.vo_id)
            .bind(about)
            .bind(head.lifecycle_state)
            .bind(role_validity)
            .bind(ordinal)
            .bind(kind)
            .bind(terminology)
            .bind(code_string)
            .bind(target)
            .bind(target_kind)
            .bind(provider_issuer)
            .bind(provider_id)
            .bind(validity)
            .bind(unreadable)
            .bind(not_before)
            .bind(not_after)
            .bind(self_declared)
            .bind(covers_issuer)
            .bind(covers_id)
            .bind(covers_kind);
        match episodes {
            Some((issuers, ids)) => query.bind(issuers).bind(ids),
            None => query,
        }
    })
    .writing(writes)
}
