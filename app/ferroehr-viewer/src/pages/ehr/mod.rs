// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One EHR, its tabs and its editors.
//!
//! The pages of FerroUI `design/consoles/ferroehr/` 12 and 13: its header
//! with the subject line (E1) and its tabs, Compositions, Directory, Status,
//! Contributions and Tags, each its own route; the editors of its status and
//! directory; the contribution builder and the contribution page.
//!
//! The header reads the EHR itself (`GET /ehr/{ehr_id}`, so an unknown id
//! fails once at the top), its current `EHR_STATUS` (the subject and the
//! capability flags of RM `docs/specs/openehr/RM/docs/ehr/master04-ehr_package.adoc`
//! §`EHR_STATUS`), whether it has a directory (a tab it cannot have is left
//! out) and whether the CDR's admin group is mounted (Delete EHR is left out,
//! never disabled). Its More menu posts to the EHR's page: Create the
//! directory, and Delete EHR, whose confirmation asks for the first eight
//! characters of the id. No openEHR spec governs the viewer's pages: our own
//! design.

pub mod compositions;
pub mod contribution;
pub mod directory;
pub mod status;
pub mod tags;

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::dialog::{Confirmation, danger_dialog};
use ferrohealth_ui::view::subject::{Subject, SubjectLine, subject_line};
use ferrohealth_ui::view::surface::{Crumb, Header, page_heading};
use ferrohealth_ui::view::tabs::{Tab, binder};
use ferrohealth_ui::widget::action_menu::{Act, Action, Item, Menu};
use ferrohealth_ui::widget::identifier::{Domain, IdentifierWords, KEEP, Masked};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

use crate::bff::ehr_detail::EhrSummary;
use crate::bff::ehr_detail::status::EhrStatusState;
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{is_status, refused, shortened};

/// The id of Delete EHR's confirmation dialog.
const DELETE_DIALOG: &str = "delete-ehr";

/// The tabs of an EHR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EhrTab {
    /// Its compositions.
    Compositions,
    /// Its directory.
    Directory,
    /// Its status.
    Status,
    /// Its contributions.
    Contributions,
    /// Its tags.
    Tags,
}

impl EhrTab {
    /// The tab's key.
    const fn key(self) -> &'static str {
        match self {
            Self::Compositions => "compositions",
            Self::Directory => "directory",
            Self::Status => "status",
            Self::Contributions => "contributions",
            Self::Tags => "tags",
        }
    }
}

/// The EHR id the route names.
#[must_use]
pub fn ehr_id_param() -> String {
    use_params_map()
        .with_untracked(|params| params.get("ehr_id"))
        .unwrap_or_default()
}

/// The page of the EHR `ehr_id`.
#[must_use]
pub fn ehr_href(ehr_id: &str) -> String {
    crate::bff::ehrs::ehr_detail_href(ehr_id)
}

/// The page of the tab `tab` of the EHR `ehr_id`.
#[must_use]
pub fn tab_href(ehr_id: &str, tab: EhrTab) -> String {
    match tab {
        EhrTab::Compositions => ehr_href(ehr_id),
        other => format!("{}/{}", ehr_href(ehr_id), other.key()),
    }
}

/// The trail of every page under the EHR `ehr_id`: EHRs, then the EHR.
#[must_use]
pub fn crumbs(ehr_id: &str) -> Vec<Crumb> {
    vec![
        Crumb::new(tr(Msg::NavEhrs), "/ehrs"),
        Crumb::new(
            tr_with(Msg::EhrTitle, &[("id", shortened(ehr_id).into())]),
            ehr_href(ehr_id),
        ),
    ]
}

/// What the header of an EHR shows, read once per page.
#[derive(Debug, Clone)]
pub struct EhrFacts {
    /// The EHR itself.
    pub summary: EhrSummary,
    /// Its current status, or the CDR's refusal of it.
    pub status: Result<EhrStatusState, ViewerError>,
    /// Whether it has a directory.
    pub has_directory: bool,
    /// Whether the CDR's admin group is mounted, which Delete EHR needs.
    pub admin: bool,
}

/// Reads the header's facts of the EHR `ehr_id`; `Err` when the EHR itself
/// cannot be read, a `404` among them.
///
/// # Errors
/// The CDR's refusal of `GET /ehr/{ehr_id}`.
pub async fn facts(ehr_id: &str) -> Result<EhrFacts, ViewerError> {
    let summary = crate::bff::ehr_detail::fetch_ehr_summary(ehr_id.to_owned()).await?;
    let status = crate::bff::ehr_detail::status::fetch_ehr_status(ehr_id.to_owned()).await;
    // NOTE: no openEHR spec governs this: our own design; a directory the CDR
    // refuses to read is shown on its own tab, so the tab stays offered.
    let has_directory = !matches!(
        crate::bff::ehr_detail::directory::fetch_directory(ehr_id.to_owned()).await,
        Ok(None)
    );
    let admin = crate::admin::probe_admin_api()
        .await
        .is_ok_and(crate::admin::AdminAvailability::usable);
    Ok(EhrFacts {
        summary,
        status,
        has_directory,
        admin,
    })
}

/// The More menu of an EHR: Commit several changes, Create the directory
/// when it has none, then Delete EHR… last when the admin group is mounted.
fn more(ehr_id: &str, facts: &EhrFacts) -> Menu {
    let mut items = vec![Item::new(Action::new(
        Some(Glyph::FILE_PLUS),
        tr(Msg::EhrCommitSeveral),
        Act::Link(format!("{}/contribution/new", ehr_href(ehr_id))),
    ))];
    if !facts.has_directory {
        items.push(Item::new(Action::new(
            Some(Glyph::FOLDER_PLUS),
            tr(Msg::EhrCreateDirectory),
            Act::Post {
                action: ehr_href(ehr_id),
                fields: vec![("intent".to_owned(), "create-directory".to_owned())],
            },
        )));
    }
    let destructive = if facts.admin {
        vec![Item::new(Action::new(
            Some(Glyph::TRASH),
            tr(Msg::EhrDelete),
            Act::Dialog(DELETE_DIALOG.to_owned()),
        ))]
    } else {
        Vec::new()
    };
    // TODO(#3942): Copy EHR id joins the menu with FerroHEALTH/FerroUI#351.
    Menu::new("ehr-more", tr(Msg::EhrMore), items).destructive(destructive)
}

/// Delete EHR's confirmation: what a physical delete removes, and the field
/// that takes the first eight characters of the id.
fn delete_dialog(ehr_id: &str) -> AnyView {
    let confirm = view! {
        <form method="post" action=ehr_href(ehr_id) class="fui-stack">
            <input type="hidden" name="intent" value="delete-ehr" />
            <div class="fui-field">
                <label for="delete-confirm" class="fui-label">
                    {tr(Msg::EhrDeleteConfirmLabel)}
                </label>
                <input
                    id="delete-confirm"
                    name="confirm"
                    type="text"
                    class="fui-input fui-mono"
                    autocomplete="off"
                    spellcheck="false"
                    required
                />
            </div>
            <button type="submit" class="fui-button fui-button-danger">
                {ferrohealth_ui::icon::icon(Glyph::TRASH, "16")}
                {tr(Msg::EhrDeleteConfirm)}
            </button>
        </form>
    };
    danger_dialog(
        Confirmation {
            id: DELETE_DIALOG.into(),
            title: tr_with(Msg::EhrDeleteTitle, &[("id", shortened(ehr_id).into())]).into(),
            message: tr(Msg::EhrDeleteMessage).into(),
        },
        confirm,
    )
    .into_any()
}

/// The subject line (E1) of the EHR whose status is `status`.
fn subject(ehr_id: &str, status: &Result<EhrStatusState, ViewerError>) -> AnyView {
    let Ok(status) = status else {
        return ().into_any();
    };
    let who = if status.subject.is_empty() {
        Subject::None
    } else {
        let masked = Masked::new(
            Domain::new(if status.subject_namespace.is_empty() {
                tr(Msg::SubjectId)
            } else {
                status.subject_namespace.clone()
            }),
            &status.subject,
            KEEP,
        );
        let revealed = crate::request::query("reveal").is_some();
        if revealed {
            let reader = crate::request::session()
                .map(|session| session.identity)
                .unwrap_or_default();
            tracing::info!(reader = %reader, ehr = %ehr_id, "revealed an EHR's subject id");
            Subject::External(
                view! {
                    <span class="fui-identifier">
                        <code>{status.subject.clone()}</code>
                    </span>
                }
                .into_any(),
            )
        } else {
            Subject::External(
                view! {
                    <crate::islands::SubjectReveal
                        id="subject".to_owned()
                        ehr_id=ehr_id.to_owned()
                        masked=masked
                        seconds=crate::prefs::reveal_seconds()
                        fallback=format!("{}?reveal=1", ehr_href(ehr_id))
                        words=IdentifierWords::of_page()
                    />
                }
                .into_any(),
            )
        }
    };
    subject_line(SubjectLine {
        subject: who,
        queryable: status.is_queryable,
        modifiable: status.is_modifiable,
    })
}

/// The header of the EHR `ehr_id`: its trail, its title with New composition
/// and More, the line on when it was created, and its subject line.
fn header(ehr_id: &str, facts: &EhrFacts) -> AnyView {
    let actions = ActionGroup::primary(Action::new(
        Some(Glyph::PLUS),
        tr(Msg::NewComposition),
        Act::Link(format!("{}/compositions/new", ehr_href(ehr_id))),
    ))
    .more(more(ehr_id, facts));
    let heading = page_heading(
        Header {
            title: tr_with(Msg::EhrTitle, &[("id", shortened(ehr_id).into())]).into(),
            subtitle: Some(
                tr_with(
                    Msg::EhrSubtitle,
                    &[
                        (
                            "created",
                            ferrohealth_ui::view::time::reading(&facts.summary.time_created).into(),
                        ),
                        ("system", facts.summary.system_id.clone().into()),
                    ],
                )
                .into(),
            ),
            crumbs: vec![Crumb::new(tr(Msg::NavEhrs), "/ehrs")],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        actions,
    );
    view! {
        {heading}
        {subject(ehr_id, &facts.status)}
        {facts.admin.then(|| delete_dialog(ehr_id))}
    }
    .into_any()
}

/// The tabs of the EHR, the Directory tab only when it has one.
fn tabs(ehr_id: &str, facts: &EhrFacts) -> Vec<Tab> {
    let mut tabs = vec![Tab::new(
        EhrTab::Compositions.key(),
        tab_href(ehr_id, EhrTab::Compositions),
        tr(Msg::EhrTabCompositions),
    )];
    if facts.has_directory {
        tabs.push(Tab::new(
            EhrTab::Directory.key(),
            tab_href(ehr_id, EhrTab::Directory),
            tr(Msg::EhrTabDirectory),
        ));
    }
    tabs.extend([
        Tab::new(
            EhrTab::Status.key(),
            tab_href(ehr_id, EhrTab::Status),
            tr(Msg::EhrTabStatus),
        ),
        Tab::new(
            EhrTab::Contributions.key(),
            tab_href(ehr_id, EhrTab::Contributions),
            tr(Msg::EhrTabContributions),
        ),
        Tab::new(
            EhrTab::Tags.key(),
            tab_href(ehr_id, EhrTab::Tags),
            tr(Msg::EhrTabTags),
        ),
    ]);
    tabs
}

/// What a post to an EHR's page asks, done: Create the directory and Delete
/// EHR. `Some` holds the refusal to draw above the tabs.
async fn act(ehr_id: &str) -> Option<AnyView> {
    let posted = crate::posted::posted()?;
    match posted.intent().as_str() {
        "create-directory" => {
            let body = crate::bff::ehr_detail::directory::empty_root_folder().to_string();
            match crate::bff::ehr_detail::directory::create_directory(ehr_id.to_owned(), body).await
            {
                Ok(_) => {
                    redirect_with(
                        &tab_href(ehr_id, EhrTab::Directory),
                        &Done::ok(tr(Msg::DirectoryCreated)),
                    );
                    None
                }
                Err(error) => Some(refused(tr(Msg::DirectoryCreateRefused), &error, None)),
            }
        }
        "delete-ehr" => {
            let typed = posted.text("confirm");
            let head: String = ehr_id.chars().take(8).collect();
            if typed != head {
                return Some(crate::pages::common::warn_note(tr(Msg::EhrDeleteMismatch)));
            }
            match crate::admin::admin_delete_ehr(ehr_id.to_owned()).await {
                Ok(()) => {
                    redirect_with(
                        "/ehrs",
                        &Done::ok(tr(Msg::EhrDeleted)).line(shortened(ehr_id)),
                    );
                    None
                }
                Err(error) => Some(refused(tr(Msg::EhrDeleteRefused), &error, None)),
            }
        }
        _ => None,
    }
}

/// An EHR's page: its header and the tab `tab` over `panel`, the body the
/// facts and the posts allow; an EHR the CDR does not hold says so once.
pub async fn ehr_page<F, Fut>(ehr_id: String, tab: EhrTab, panel: F) -> AnyView
where
    F: FnOnce(EhrFacts) -> Fut,
    Fut: Future<Output = AnyView>,
{
    let acted = act(&ehr_id).await;
    let facts = match facts(&ehr_id).await {
        Ok(facts) => facts,
        Err(error) => return missing(&ehr_id, &error),
    };
    let header = header(&ehr_id, &facts);
    let tabs = tabs(&ehr_id, &facts);
    let body = panel(facts).await;
    view! {
        {header}
        <div class="fui-stack">{acted} {binder(tr(Msg::EhrViews), tabs, tab.key(), body)}</div>
    }
    .into_any()
}

/// The page of an EHR the CDR cannot read: no EHR with this id (404), said
/// once at the top with no tabs, or the CDR's other refusal.
fn missing(ehr_id: &str, error: &ViewerError) -> AnyView {
    let title = if is_status(error, http::StatusCode::NOT_FOUND) {
        crate::request::set_status(http::StatusCode::NOT_FOUND);
        tr_with(Msg::EhrNotFoundTitle, &[("id", shortened(ehr_id).into())])
    } else {
        tr_with(Msg::EhrTitle, &[("id", shortened(ehr_id).into())])
    };
    let heading = page_heading(
        Header {
            title: title.into(),
            subtitle: None,
            crumbs: vec![Crumb::new(tr(Msg::NavEhrs), "/ehrs")],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let back = ferrohealth_ui::view::button::link_button(
        "/ehrs".to_owned(),
        ferrohealth_ui::view::button::Variant::Primary,
        Some(Glyph::ARROW_LEFT),
        tr(Msg::EhrBack),
    );
    view! {
        {heading}
        <div class="fui-stack">
            {refused(tr(Msg::EhrRefused), error, None)} <div class="fui-cluster">{back}</div>
        </div>
    }
    .into_any()
}
