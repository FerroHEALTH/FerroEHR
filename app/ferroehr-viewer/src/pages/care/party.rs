// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A party's Care page.
//!
//! For a `PERSON` it shows the care relationships of the subject it is and
//! the form that adds one; for every party, the subjects in its care
//! (`carer=`), paged through the session ([`crate::paging`]). No openEHR specification governs the page: our own
//! design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::table::model::OpenWindow;
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_open_pager};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{Crumb, Header, page_heading, section_intro};

use crate::bff::care::{
    CareRow, NewCare, Role, Span, create, fetch_of_carer, fetch_of_person, new_document,
};
use crate::bff::demographics::PartyKind;
use crate::bff::grants::{PartyRef, Validity};
use crate::flash::{Done, redirect_with};
use crate::locale::policy::PolicyMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{flag, h, opt, t};
use crate::pages::care::{
    Named, ROLE_TERMINOLOGY, asked_span, care_href, care_list, period_fields, posted_period,
    posted_role, refusal_of, role_select, span_form,
};
use crate::pages::common::{Refused, card_panel, section, shortened};
use crate::paging::Listing;
use crate::routes::Drawn;

/// The page of the party `kind` `uid`'s care relationships.
#[must_use]
pub fn party_care_href(kind: PartyKind, uid: &str) -> String {
    format!(
        "/parties/{}/{}/care",
        kind.segment(),
        urlencoding::encode(uid)
    )
}

/// The way from a party's page to its Care page.
#[must_use]
pub fn care_link(kind: PartyKind, uid: &str) -> Html {
    h(format!(
        r#"<div class="fui-cluster">{}</div>"#,
        ferrohealth_ui::view::button::link_button(
            party_care_href(kind, uid),
            ferrohealth_ui::view::button::Variant::Secondary,
            Some(ferrohealth_ui::icon::Glyph::HEART_PULSE),
            tr(PolicyMsg::CareCaption),
        )
    ))
}

/// The carer kinds a care relationship may name.
const CARER_KINDS: [&str; 4] = ["PERSON", "ROLE", "GROUP", "ORGANISATION"];

/// What the add form holds: as posted, or empty.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Draft {
    /// The carer's kind.
    carer_kind: String,
    /// The carer's id.
    carer_id: String,
    /// The care role.
    role: Role,
    /// The provider's identifier issuer.
    issuer: String,
    /// The provider's identifier.
    provider: String,
    /// The period.
    period: Validity,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            carer_kind: "PERSON".to_owned(),
            carer_id: String::new(),
            role: Role {
                rubric: tr(PolicyMsg::RoleAtnd),
                code: "ATND".to_owned(),
                terminology: ROLE_TERMINOLOGY.to_owned(),
            },
            issuer: String::new(),
            provider: String::new(),
            period: Validity::default(),
        }
    }
}

/// The page's model.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Model {
    /// The subject's relationships, for a `PERSON`, with the span read.
    subject: Option<(Span, Result<Vec<CareRow>, Refused>)>,
    /// The add form, for a `PERSON`, and the CDR's refusal of a post.
    draft: Option<(Draft, Option<Refused>)>,
    /// The subjects in the party's care, one page.
    carer: Result<Vec<CareRow>, Refused>,
}

/// The add the post asks, done; `Some` holds the draft and the CDR's
/// refusal.
async fn add(person: &str) -> Option<(Draft, Option<Refused>)> {
    let posted = crate::posted::posted()?;
    if posted.intent() != "add" {
        return None;
    }
    let draft = Draft {
        carer_kind: posted.text("carer_kind"),
        carer_id: posted.text("carer_id").trim().to_owned(),
        role: posted_role(&posted.text("role")),
        issuer: posted.text("issuer").trim().to_owned(),
        provider: posted.text("provider").trim().to_owned(),
        period: posted_period(&posted),
    };
    let document = new_document(&NewCare {
        subject: person.to_owned(),
        carer: PartyRef {
            rm_type: draft.carer_kind.clone(),
            id: draft.carer_id.clone(),
        },
        role: draft.role.clone(),
        provider: (draft.issuer.clone(), draft.provider.clone()),
        period: draft.period.clone(),
    });
    match create(person.to_owned(), document).await {
        Ok(id) => {
            redirect_with(&care_href(&id), &Done::ok(tr(PolicyMsg::CareAdded)));
            None
        }
        Err(error) => Some((
            draft,
            Some(Refused::new(tr(PolicyMsg::CareAddRefused), error)),
        )),
    }
}

/// What the page reads.
async fn read(kind: PartyKind, uid: &str) -> Model {
    let (subject, draft) = if kind == PartyKind::Person {
        let draft = add(uid).await.unwrap_or_else(|| (Draft::default(), None));
        let span = asked_span();
        let rows = fetch_of_person(uid.to_owned(), span.clone())
            .await
            .map_err(refusal_of);
        (Some((span, rows)), Some(draft))
    } else {
        (None, None)
    };
    let window = OpenWindow::of(crate::paging::current(Listing::CareOfCarer), 0);
    let carer = fetch_of_carer(uid.to_owned(), window.start, window.size)
        .await
        .map_err(refusal_of);
    Model {
        subject,
        draft,
        carer,
    }
}

/// The add form.
fn add_form(action: &str, draft: &Draft, refused: Option<&Refused>) -> Html {
    let input = |id: &str, name: &str, value: &str| {
        h(format!(
            r#"<input id="{id}" name="{name}" type="text" class="fui-input fui-mono" spellcheck="false" autocomplete="off" required value="{}">"#,
            t(value)
        ))
    };
    let kinds = CARER_KINDS
        .iter()
        .map(|kind| {
            format!(
                r#"<option value="{kind}"{}>{kind}</option>"#,
                flag("selected", *kind == draft.carer_kind)
            )
        })
        .collect::<Vec<_>>()
        .concat();
    let fields = Html::join([
        Field::new("care-kind", tr(PolicyMsg::CareCarerKind)).view(h(format!(
            r#"<select id="care-kind" name="carer_kind" class="fui-select">{kinds}</select>"#
        ))),
        Field::new("care-carer", tr(PolicyMsg::CareCarerId))
            .help(tr(PolicyMsg::CareCarerIdHelp))
            .holds_id()
            .view(input("care-carer", "carer_id", &draft.carer_id)),
        role_select("care-new-role", &draft.role),
        Field::new("care-issuer", tr(PolicyMsg::CareProviderIssuer))
            .help(tr(PolicyMsg::CareProviderHelp))
            .view(input("care-issuer", "issuer", &draft.issuer)),
        Field::new("care-provider", tr(PolicyMsg::CareProviderId)).view(input(
            "care-provider",
            "provider",
            &draft.provider,
        )),
        period_fields("care-new", &draft.period),
    ]);
    opt(refused.map(Refused::view)).then(h(format!(
        r#"<form method="post" action="{}" class="fui-form">{fields}<div class="fui-cluster"><button type="submit" name="intent" value="add" class="fui-button fui-button-primary">{}</button></div></form>"#,
        t(action),
        t(tr(PolicyMsg::CareAdd))
    )))
}

/// The carer listing, with its pager.
fn carer_section(rows: Result<Vec<CareRow>, Refused>) -> Html {
    let rows = match rows {
        Ok(rows) => rows,
        Err(refusal) => {
            return section(Some(tr(PolicyMsg::CareInCare)), None, None, refusal.view());
        }
    };
    let paging = crate::paging::current(Listing::CareOfCarer);
    let fields = crate::paging::fields(Listing::CareOfCarer);
    let window = OpenWindow::of(paging, u32::try_from(rows.len()).unwrap_or(u32::MAX));
    let pager = (window.page > 1 || window.has_next()).then(|| {
        post_open_pager(
            &current(),
            PagerListing::new(tr(PolicyMsg::CarePages)),
            window,
            PostTo {
                id: "care-pager",
                action: crate::paging::PAGING_ROUTE,
                fields: &fields,
            },
        )
    });
    section(
        Some(tr(PolicyMsg::CareInCare)),
        None,
        None,
        section_intro(tr(PolicyMsg::CareInCareIntro))
            .then(care_list(&rows, Named::Subject))
            .then(opt(pager)),
    )
}

/// A party's Care page.
pub async fn page() -> Drawn {
    let params = crate::routes::params();
    let segment = params.get("kind").unwrap_or_default();
    let uid = params.get("uid").unwrap_or_default();
    let Some(kind) = PartyKind::from_segment(&segment) else {
        return crate::pages::errors::not_found_page();
    };
    let action = party_care_href(kind, &uid);
    let Model {
        subject,
        draft,
        carer,
    } = read(kind, &uid).await;
    let subject = subject.map(|(span, rows)| {
        let list = match rows {
            Ok(rows) => care_list(&rows, Named::Carer),
            Err(refusal) => refusal.view(),
        };
        section(
            Some(tr(PolicyMsg::CareOfPerson)),
            None,
            None,
            section_intro(tr(PolicyMsg::CareOfPersonIntro))
                .then(span_form(&action, &span))
                .then(list),
        )
    });
    let add = draft.map(|(draft, refused)| {
        section(
            Some(tr(PolicyMsg::CareAddTitle)),
            None,
            None,
            section_intro(tr(PolicyMsg::CareAddIntro)).then(add_form(
                &action,
                &draft,
                refused.as_ref(),
            )),
        )
    });
    let party = format!("/parties/{}/{}", kind.segment(), urlencoding::encode(&uid));
    let heading = page_heading(
        &current(),
        Header {
            title: tr_with(PolicyMsg::CarePartyTitle, &[("id", shortened(&uid).into())]).into(),
            subtitle: Some(tr(PolicyMsg::CarePartySubtitle).into()),
            crumbs: vec![
                Crumb::new(tr(Msg::NavParties), "/parties"),
                Crumb::new(shortened(&uid), party),
            ],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::default(),
    );
    let body = card_panel(Html::join([opt(subject), opt(add), carer_section(carer)]));
    Drawn::page(
        tr(PolicyMsg::CareTitle),
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}
