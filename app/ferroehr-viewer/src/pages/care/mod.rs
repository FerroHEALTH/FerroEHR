// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Care relationships: an EHR's Care tab, a party's Care page, and one
//! relationship's page with its versions and its writes.
//!
//! Every list and read goes to the CDR's care relationship resources
//! ([`crate::bff::care`]), which decide per subject what a caller sees: a
//! `403` or an empty list is drawn as the CDR answered it. A relationship is
//! added on its subject's `PERSON` page, changed or ended with `If-Match`
//! on its own page, and deleted there when it was written in error. No
//! openEHR specification governs these pages: our own design.

pub mod party;
pub mod relationship;

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    ACTIONS, Body, Column, DataTable, Role, cell, data_table, title_cell,
};
use ferrohealth_ui::view::coding::{Coding, Layout, coding};
use ferrohealth_ui::view::empty::empty_quiet;
use ferrohealth_ui::view::form::Field;

use crate::bff::care::{CarePath, CareRow, Role as CareRole, Span};
use crate::bff::grants::{PartyRef, Validity};
use crate::error::ViewerError;
use crate::locale::policy::PolicyMsg;
use crate::locale::{current, tr, tr_with};
use crate::markup::{flag, h, t};
use crate::pages::common::{Refused, date_field, short};
use crate::pages::policy::grants::party_href;

/// The page of the care relationship `id`, its current version or the
/// version a version uid names.
#[must_use]
pub fn care_href(id: &str) -> String {
    format!("/care/{}", urlencoding::encode(id))
}

/// The care roles a form offers: `HL7_ParticipationType`, the codes the CDR
/// reads when the access policy binds no value set for `care_role`.
pub const ROLES: [(&str, PolicyMsg); 6] = [
    ("RESP", PolicyMsg::RoleResp),
    ("ATND", PolicyMsg::RoleAtnd),
    ("CON", PolicyMsg::RoleCon),
    ("PPRF", PolicyMsg::RolePprf),
    ("SPRF", PolicyMsg::RoleSprf),
    ("PART", PolicyMsg::RolePart),
];

/// The terminology of [`ROLES`].
pub const ROLE_TERMINOLOGY: &str = "HL7_ParticipationType";

/// The care role a form posted as `name`: a code of [`ROLES`], or another as
/// `terminology|code`.
#[must_use]
pub fn posted_role(code: &str) -> CareRole {
    if let Some((terminology, code)) = code.split_once('|') {
        return CareRole {
            rubric: code.to_owned(),
            code: code.to_owned(),
            terminology: terminology.to_owned(),
        };
    }
    let rubric = ROLES
        .iter()
        .find(|(known, _)| *known == code)
        .map_or_else(|| code.to_owned(), |(_, words)| tr(*words));
    CareRole {
        rubric,
        code: code.to_owned(),
        terminology: ROLE_TERMINOLOGY.to_owned(),
    }
}

/// A role select over [`ROLES`], `chosen` selected; a chosen role outside
/// them is kept as its own option.
#[must_use]
pub fn role_select(id: &str, chosen: &CareRole) -> Html {
    let token = |role: &CareRole| {
        if role.terminology == ROLE_TERMINOLOGY {
            role.code.clone()
        } else {
            format!("{}|{}", role.terminology, role.code)
        }
    };
    let wanted = token(chosen);
    let mut options: Vec<String> = ROLES
        .iter()
        .map(|(code, words)| {
            format!(
                r#"<option value="{code}"{}>{} ({code})</option>"#,
                flag("selected", *code == wanted),
                t(tr(*words))
            )
        })
        .collect();
    if !wanted.is_empty() && !ROLES.iter().any(|(code, _)| *code == wanted) {
        options.push(format!(
            r#"<option value="{value}" selected>{rubric} ({value})</option>"#,
            value = t(&wanted),
            rubric = t(&chosen.rubric)
        ));
    }
    Field::new(id, tr(PolicyMsg::CareRole))
        .help(tr(PolicyMsg::CareRoleHelp))
        .view(h(format!(
            r#"<select id="{id}" name="role" class="fui-select">{}</select>"#,
            options.concat()
        )))
}

/// The From and To date fields of a period.
#[must_use]
pub fn period_fields(prefix: &str, period: &Validity) -> Html {
    date_field(
        &format!("{prefix}-from"),
        "from",
        tr(PolicyMsg::CareFrom),
        period.lower.as_deref().unwrap_or_default(),
        false,
    )
    .then(date_field(
        &format!("{prefix}-to"),
        "to",
        tr(PolicyMsg::CareTo),
        period.upper.as_deref().unwrap_or_default(),
        false,
    ))
}

/// The period a form posted, an empty bound open.
#[must_use]
pub fn posted_period(posted: &crate::posted::Posted) -> Validity {
    let bound = |name: &str| Some(posted.text(name)).filter(|text| !text.trim().is_empty());
    Validity {
        lower: bound("from"),
        upper: bound("to"),
    }
}

/// A period in words: its bounds, an open one said so.
#[must_use]
pub fn period_words(period: &Validity) -> String {
    let open = tr(PolicyMsg::CareOpen);
    tr_with(
        PolicyMsg::GrantsBetween,
        &[
            (
                "from",
                period.lower.clone().unwrap_or_else(|| open.clone()).into(),
            ),
            ("to", period.upper.clone().unwrap_or(open).into()),
        ],
    )
}

/// A care role drawn as a coding.
#[must_use]
pub fn role_coding(role: &CareRole) -> Html {
    coding(
        &current(),
        Coding::new(role.code.clone(), role.rubric.clone()).system(role.terminology.clone()),
        Layout::Inline,
    )
}

/// How a relationship reaches its carer, in words.
#[must_use]
pub fn path_words(path: CarePath) -> String {
    tr(match path {
        CarePath::Direct => PolicyMsg::CareDirect,
        CarePath::Team => PolicyMsg::CareTeam,
    })
}

/// A party a relationship names, as its type and short id.
fn party_words(party: &PartyRef) -> Html {
    h(format!("{} {}", t(&party.rm_type), short(&party.id)))
}

/// Which party a care table names in its rows beside the relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Named {
    /// The carer, for a subject's list.
    Carer,
    /// The subject, for a carer's list.
    Subject,
}

/// A table of `rows`, each opening its relationship.
#[must_use]
pub fn care_table(rows: &[CareRow], named: Named) -> Html {
    let who = match named {
        Named::Carer => PolicyMsg::CareCarer,
        Named::Subject => PolicyMsg::CareSubject,
    };
    let columns = [
        Column::text("who", tr(who)).role(Role::Title),
        Column::text("role", tr(PolicyMsg::CareRole)).role(Role::Sub),
        Column::text("period", tr(PolicyMsg::CarePeriod)).role(Role::Meta),
        Column::text("path", tr(PolicyMsg::CarePath)).role(Role::Meta),
        Column::text("open", tr(PolicyMsg::GrantsOpen)).role(Role::Wide),
    ];
    let body = rows
        .iter()
        .map(|row| {
            let party = match named {
                Named::Carer => &row.carer,
                Named::Subject => &row.subject,
            };
            let party_link = party_href(party).map_or_else(Html::empty, |href| {
                ferrohealth_ui::view::button::link_button(
                    href,
                    ferrohealth_ui::view::button::Variant::Ghost,
                    Some(Glyph::EXTERNAL_LINK),
                    tr(who),
                )
            });
            h(format!(
                r#"<tr>{}{}{}{}<td class="{ACTIONS}" data-role="wide">{party_link}</td></tr>"#,
                title_cell(None, Some(&care_href(&row.id)), party_words(party), None),
                cell(&columns[1], role_coding(&row.role)),
                cell(&columns[2], t(period_words(&row.period))),
                cell(&columns[3], t(path_words(row.path()))),
            ))
        })
        .collect();
    let caption = tr(PolicyMsg::CareCaption);
    data_table(DataTable::new(&caption, &columns, Body::Rows(body)))
}

/// A list of relationships, or the quiet line of an empty one.
#[must_use]
pub fn care_list(rows: &[CareRow], named: Named) -> Html {
    if rows.is_empty() {
        empty_quiet(
            Glyph::HEART_PULSE,
            tr(PolicyMsg::CareNone),
            tr(PolicyMsg::CareNoneLine),
        )
    } else {
        care_table(rows, named)
    }
}

/// The span the request asks a subject's list for: `all` and `at` of the
/// query.
#[must_use]
pub fn asked_span() -> Span {
    Span {
        at: crate::request::query("at").filter(|at| !at.is_empty()),
        all: crate::request::query("all").is_some_and(|all| all == "true"),
    }
}

/// The filter of a subject's list: ended relationships too, and the day
/// they are live on, a `GET` form to `action`.
#[must_use]
pub fn span_form(action: &str, span: &Span) -> Html {
    h(format!(
        r#"<form method="get" action="{action}" class="fui-transform-options"><label class="fui-check"><input type="checkbox" name="all" value="true" class="fui-checkbox"{all}><span class="fui-check-text">{all_words}</span></label>{at}<div class="fui-transform-run"><button type="submit" class="fui-button fui-button-secondary">{show}</button></div></form>"#,
        action = t(action),
        all = flag("checked", span.all),
        all_words = t(tr(PolicyMsg::CareAll)),
        at = date_field(
            "care-at",
            "at",
            tr(PolicyMsg::CareAt),
            span.at.as_deref().unwrap_or_default(),
            false
        ),
        show = t(tr(PolicyMsg::CareShow)),
    ))
}

/// The refusal of a read of care relationships, worded for a `403`.
#[must_use]
pub fn refusal_of(error: ViewerError) -> Refused {
    let summary = if matches!(error, ViewerError::Forbidden(_)) {
        PolicyMsg::CareForbidden
    } else {
        PolicyMsg::CareRefused
    };
    Refused::new(tr(summary), error)
}

#[cfg(test)]
mod tests {
    use super::{ROLE_TERMINOLOGY, posted_role};

    #[test]
    fn a_posted_role_reads_as_a_code_of_the_default_terminology_or_its_own() {
        let attender = posted_role("ATND");
        assert_eq!(ROLE_TERMINOLOGY, attender.terminology);
        assert_eq!("ATND", attender.code);
        let own = posted_role("urn:example:roles|NURSE");
        assert_eq!("urn:example:roles", own.terminology);
        assert_eq!("NURSE", own.code);
    }
}
