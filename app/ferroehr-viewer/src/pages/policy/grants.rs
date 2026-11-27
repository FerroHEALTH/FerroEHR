// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Roles and grants, and the mark a party page draws on a policy party.
//!
//! The page lists every role holding an `openehr_access` grant and every
//! system agent, read through AQL ([`crate::bff::grants`]), so the CDR's
//! policy-party rule decides what is shown.
//!
//! A grant is given and ended by a new version of its `ROLE`, written
//! through the party editor; the page links each role and performer to its
//! party. No openEHR specification governs grants: FerroEHR's own design
//! over its draft access control package.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    ACTIONS, Body, Column, DataTable, Role, cell, data_table, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::coding::{Coding, Layout, coding};
use ferrohealth_ui::view::empty::empty_quiet;
use ferrohealth_ui::view::flow::FlowState;
use ferrohealth_ui::view::surface::{Header, page_heading, section_intro};

use crate::bff::grants::{Code, Grants, PartyRef, RoleGrant, SystemAgent, Validity, fetch_grants};
use crate::error::ViewerError;
use crate::locale::policy::PolicyMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, t};
use crate::pages::common::{
    Refused, card_panel, count, info_note, plain_section, section, short, state_mark,
};
use crate::routes::Drawn;

/// The route of Roles and grants.
pub const GRANTS_ROUTE: &str = "/grants";

/// The page of the party `party` names, when its type is a party kind.
#[must_use]
pub fn party_href(party: &PartyRef) -> Option<String> {
    crate::bff::demographics::PartyKind::from_rm_type(&party.rm_type)
        .map(|kind| format!("/parties/{}/{}", kind.segment(), party.id))
}

/// A code of the access terminology, drawn as a coding.
fn code(code: &Code) -> Html {
    coding(
        &current(),
        Coding::new(code.code.clone(), code.rubric.clone()),
        Layout::Inline,
    )
}

/// A validity as its bounds, an open one as "always".
fn validity(validity: &Validity) -> String {
    match (&validity.lower, &validity.upper) {
        (None, None) => tr(PolicyMsg::GrantsAlways),
        (lower, upper) => tr_with(
            PolicyMsg::GrantsBetween,
            &[
                ("from", lower.clone().unwrap_or_default().into()),
                ("to", upper.clone().unwrap_or_default().into()),
            ],
        ),
    }
}

/// Whether a grant is in force at the instant the page is drawn, as its
/// state's tone and word.
fn in_force(grant: &RoleGrant) -> Html {
    if grant.in_force(crate::request::now()) {
        state_mark(FlowState::Up, &tr(PolicyMsg::GrantsInForce))
    } else {
        state_mark(FlowState::Off, &tr(PolicyMsg::GrantsNotInForce))
    }
}

/// A party a grant names, as its type and short id.
fn party_words(party: &PartyRef) -> Html {
    h(format!("{} {}", t(&party.rm_type), short(&party.id)))
}

/// The cell of a row's link to `party`, beside the row's own link.
#[must_use]
pub fn party_action(party: Option<&PartyRef>, words: PolicyMsg) -> Html {
    let link = party.and_then(party_href).map_or_else(Html::empty, |href| {
        ferrohealth_ui::view::button::link_button(
            href,
            ferrohealth_ui::view::button::Variant::Ghost,
            Some(Glyph::EXTERNAL_LINK),
            tr(words),
        )
    });
    h(format!(
        r#"<td class="{ACTIONS}" data-role="wide">{link}</td>"#
    ))
}

/// The grants of `roles` as a table, each row opening its role.
#[must_use]
pub fn roles_table(roles: &[RoleGrant]) -> Html {
    let columns = [
        Column::text("role", tr(PolicyMsg::GrantsRole)).role(Role::Title),
        Column::text("grants", tr(PolicyMsg::GrantsCodes)).role(Role::Sub),
        Column::text("terms", tr(PolicyMsg::GrantsTerms)).role(Role::Meta),
        Column::text("performer", tr(PolicyMsg::GrantsHeldBy)).role(Role::Meta),
        Column::text("validity", tr(PolicyMsg::GrantsValidity)).role(Role::Wide),
        Column::text("state", tr(PolicyMsg::GrantsState)).role(Role::Meta),
        Column::text("open", tr(PolicyMsg::GrantsOpen)).role(Role::Wide),
    ];
    let rows = roles
        .iter()
        .map(|grant| {
            let href = party_href(&PartyRef {
                rm_type: "ROLE".to_owned(),
                id: grant.role.clone(),
            });
            let codes = Html::join(grant.operations.iter().map(code));
            let terms = Html::join(grant.qualifiers.iter().map(|(name, text)| {
                h(format!(
                    r#"<span class="fui-small">{}: <code>{}</code></span> "#,
                    t(&name.rubric),
                    t(text)
                ))
            }));
            let capability = grant
                .capability_validity
                .iter()
                .map(validity)
                .collect::<Vec<_>>()
                .join(", ");
            let validity_words = tr_with(
                PolicyMsg::GrantsValidityWords,
                &[
                    ("role", validity(&grant.role_validity).into()),
                    ("capability", capability.into()),
                ],
            );
            h(format!(
                "<tr>{}{}{}{}{}{}{}</tr>",
                title_cell(None, href.as_deref(), t(&grant.name), None),
                cell(&columns[1], codes),
                cell(&columns[2], terms),
                cell(
                    &columns[3],
                    grant
                        .performer
                        .as_ref()
                        .map_or_else(Html::empty, party_words)
                ),
                cell(&columns[4], t(validity_words)),
                cell(&columns[5], in_force(grant)),
                party_action(grant.performer.as_ref(), PolicyMsg::GrantsOpenPerformer),
            ))
        })
        .collect();
    let caption = tr(PolicyMsg::GrantsRoles);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// The system agents as a table, each row opening its agent.
fn agents_table(agents: &[SystemAgent]) -> Html {
    let columns = [
        Column::text("agent", tr(PolicyMsg::GrantsAgent)).role(Role::Title),
        Column::text("client", tr(PolicyMsg::GrantsClient)).role(Role::Sub),
        Column::text("issuer", tr(PolicyMsg::GrantsIssuer)).role(Role::Meta),
        Column::text("acts-for", tr(PolicyMsg::GrantsActsFor)).role(Role::Meta),
        Column::text("open", tr(PolicyMsg::GrantsOpen)).role(Role::Wide),
    ];
    let rows = agents
        .iter()
        .map(|agent| {
            let href = party_href(&PartyRef {
                rm_type: "AGENT".to_owned(),
                id: agent.id.clone(),
            });
            let acts_for = if agent.acts_for.is_empty() {
                h(format!(
                    r#"<span class="fui-muted">{}</span>"#,
                    t(tr(Msg::RelationshipNone))
                ))
            } else {
                Html::join(agent.acts_for.iter().map(party_words))
            };
            h(format!(
                "<tr>{}{}{}{}{}</tr>",
                title_cell(None, href.as_deref(), t(&agent.name), None),
                cell(
                    &columns[1],
                    h(format!("<code>{}</code>", t(&agent.client_id)))
                ),
                cell(&columns[2], t(&agent.issuer)),
                cell(&columns[3], acts_for),
                party_action(agent.acts_for.first(), PolicyMsg::GrantsOpenProvider),
            ))
        })
        .collect();
    let caption = tr(PolicyMsg::GrantsAgents);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// The page's panel: the roles, the system agents and how a grant changes.
fn grants_panel(read: Result<Grants, Refused>) -> Html {
    let grants = match read {
        Ok(grants) => grants,
        Err(refusal) => return card_panel(plain_section(refusal.view())),
    };
    let roles = if grants.roles.is_empty() {
        empty_quiet(
            Glyph::KEY_ROUND,
            tr(PolicyMsg::GrantsNoRoles),
            tr(PolicyMsg::GrantsNoRolesLine),
        )
    } else {
        roles_table(&grants.roles)
    };
    let agents = if grants.agents.is_empty() {
        empty_quiet(
            Glyph::KEY_ROUND,
            tr(PolicyMsg::GrantsNoAgents),
            tr(PolicyMsg::GrantsNoAgentsLine),
        )
    } else {
        agents_table(&grants.agents)
    };
    let truncated = grants.truncated.then(|| {
        plain_section(info_note(t(tr_with(
            PolicyMsg::GrantsTruncated,
            &[("count", count(crate::bff::grants::LIMIT))],
        ))))
    });
    card_panel(Html::join([
        truncated.unwrap_or_default(),
        plain_section(info_note(t(tr(PolicyMsg::GrantsHow)))),
        section(
            Some(tr(PolicyMsg::GrantsRoles)),
            u64::try_from(grants.roles.len()).ok(),
            None,
            section_intro(tr(PolicyMsg::GrantsRolesIntro)).then(roles),
        ),
        section(
            Some(tr(PolicyMsg::GrantsAgents)),
            u64::try_from(grants.agents.len()).ok(),
            None,
            section_intro(tr(PolicyMsg::GrantsAgentsIntro)).then(agents),
        ),
    ]))
}

/// The refusal of a read of the grants, worded for a `403`.
fn refusal_of(error: ViewerError) -> Refused {
    let summary = if matches!(error, ViewerError::Forbidden(_)) {
        PolicyMsg::GrantsForbidden
    } else {
        PolicyMsg::GrantsRefused
    };
    Refused::new(tr(summary), error)
}

/// Roles and grants.
pub async fn page() -> Drawn {
    let read = fetch_grants().await.map_err(refusal_of);
    let heading = page_heading(
        &current(),
        Header {
            title: tr(Msg::NavGrants).into(),
            subtitle: Some(tr(PolicyMsg::GrantsSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    );
    Drawn::page(
        tr(Msg::NavGrants),
        heading.then(h(format!(
            r#"<div class="fui-stack">{}</div>"#,
            grants_panel(read)
        ))),
    )
}

/// The mark a party page draws on a policy party: the client it binds and
/// the grants its roles carry, or nothing for an ordinary party.
#[must_use]
pub fn policy_party_mark(client: Option<&SystemAgent>, grants: &[RoleGrant]) -> Option<Html> {
    if client.is_none() && grants.is_empty() {
        return None;
    }
    let client = client.map(|agent| {
        info_note(t(tr_with(
            PolicyMsg::MarkClient,
            &[
                ("client", agent.client_id.clone().into()),
                ("issuer", agent.issuer.clone().into()),
            ],
        )))
    });
    let roles = (!grants.is_empty()).then(|| roles_table(grants));
    Some(section(
        Some(tr(PolicyMsg::MarkTitle)),
        None,
        Some(ferrohealth_ui::view::button::link_button(
            GRANTS_ROUTE.to_owned(),
            ferrohealth_ui::view::button::Variant::Secondary,
            Some(Glyph::KEY_ROUND),
            tr(Msg::NavGrants),
        )),
        section_intro(tr(PolicyMsg::MarkIntro))
            .then(client.unwrap_or_default())
            .then(roles.unwrap_or_default()),
    ))
}

/// The mark of the party `current` reads, when it is a policy party: the
/// client it binds and the grants it holds, from its own document for a
/// `ROLE` and through AQL for an actor.
pub async fn read_mark(
    current: Option<&crate::bff::demographics::party::PartyState>,
) -> Option<Html> {
    let state = current?;
    let client = crate::bff::grants::client_in(&state.body);
    let grants = if state.rm_type == "ROLE" {
        Ok(crate::bff::grants::role_grant_in(&state.body)
            .into_iter()
            .collect())
    } else {
        crate::bff::grants::fetch_grants_of(state.versioned_object_uid.clone()).await
    };
    match grants {
        Ok(grants) => policy_party_mark(client.as_ref(), &grants),
        Err(error) => Some(section(
            Some(tr(PolicyMsg::MarkTitle)),
            None,
            None,
            refusal_of(error).view(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{party_href, policy_party_mark};
    use crate::bff::grants::{Code, PartyRef, RoleGrant, Validity};

    fn grant() -> RoleGrant {
        RoleGrant {
            role: "r1".to_owned(),
            name: "write_access_policy".to_owned(),
            operations: vec![Code {
                rubric: "write_access_policy".to_owned(),
                code: "write_access_policy".to_owned(),
            }],
            qualifiers: Vec::new(),
            performer: Some(PartyRef {
                rm_type: "PERSON".to_owned(),
                id: "p1".to_owned(),
            }),
            role_validity: Validity::default(),
            capability_validity: vec![Validity::default()],
        }
    }

    #[test]
    fn a_party_links_by_its_kind() {
        let person = PartyRef {
            rm_type: "PERSON".to_owned(),
            id: "p1".to_owned(),
        };
        assert_eq!(Some("/parties/person/p1".to_owned()), party_href(&person));
        let other = PartyRef {
            rm_type: "EHR".to_owned(),
            id: "e1".to_owned(),
        };
        assert_eq!(None, party_href(&other));
    }

    #[test]
    fn an_ordinary_party_carries_no_mark() {
        assert!(policy_party_mark(None, &[]).is_none());
        let html = policy_party_mark(None, &[grant()])
            .map(String::from)
            .unwrap_or_default();
        assert!(html.contains("write_access_policy"), "{html}");
        assert!(html.contains("/parties/role/r1"), "{html}");
    }
}
