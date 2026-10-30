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

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action, Item, Menu};
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, danger_form_dialog};
use ferrohealth_ui::view::identifier::{Domain, KEEP, Masked, Reveal, Revealed};
use ferrohealth_ui::view::subject::{Subject, SubjectLine, subject_line};
use ferrohealth_ui::view::surface::{Crumb, Header, page_heading};
use ferrohealth_ui::view::tabs::{Tab, binder};

use crate::bff::ehr_detail::EhrSummary;
use crate::bff::ehr_detail::status::EhrStatusState;
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::{Refused, is_status, refused, shortened};

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
    crate::routes::params().get("ehr_id").unwrap_or_default()
}

/// The page of the EHR `ehr_id`. The id is user- or CDR-supplied, so the
/// path segment is percent-encoded with `urlencoding`: an id carrying `/`,
/// `#`, `?` or `%` would otherwise address a different route.
///
/// Encoding is also what keeps a redirect to it safe: the path becomes a
/// `Location` header value, and `urlencoding::encode` emits only unreserved
/// ASCII plus `%XX`, so an id carrying a control character or a non-ASCII
/// byte never reaches the header builder.
#[must_use]
pub fn ehr_href(ehr_id: &str) -> String {
    format!("/ehrs/{}", urlencoding::encode(ehr_id))
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
            tr_with(RecordsMsg::EhrTitle, &[("id", shortened(ehr_id).into())]),
            ehr_href(ehr_id),
        ),
    ]
}

/// What the CDR says of an EHR, read once per page by its loader.
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

/// Reads the facts of the EHR `ehr_id`; `Err` when the EHR itself cannot be
/// read, a `404` among them.
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
    let admin = crate::admin::renders_admin_ops(&crate::admin::probe_admin_api().await);
    Ok(EhrFacts {
        summary,
        status,
        has_directory,
        admin,
    })
}

/// Who the subject line names.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Who {
    /// The status names no subject.
    Nobody,
    /// The subject id, revealed for this page, with its mask.
    Revealed {
        /// The id masked, as the identifier draws it once its seconds run out.
        masked: Masked,
        /// The id itself.
        value: String,
    },
    /// The subject id, masked.
    Masked(Masked),
}

/// The subject line (E1) of an EHR.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SubjectFacts {
    /// Who the subject is.
    who: Who,
    /// Whether the EHR is queryable.
    queryable: bool,
    /// Whether the EHR is modifiable.
    modifiable: bool,
}

/// What the header of an EHR shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EhrHead {
    /// When the EHR was created.
    time_created: String,
    /// The system that holds it.
    system_id: String,
    /// Its subject line, absent when its status could not be read.
    subject: Option<SubjectFacts>,
    /// Whether it has a directory.
    has_directory: bool,
    /// Whether the CDR's admin group is mounted, which Delete EHR needs.
    admin: bool,
}

/// What a post to an EHR's page did not do, drawn above its tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Acted {
    /// The CDR refused it.
    Refused(Refused),
    /// Delete EHR's confirmation did not match the id.
    Mismatch,
}

/// The model of an EHR's page: what its post did not do, its header, and
/// the model `P` of its tab, read only when the EHR itself could be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EhrModel<P> {
    /// What the post did not do.
    acted: Option<Acted>,
    /// The header, or the CDR's refusal of the EHR itself.
    head: Result<EhrHead, ViewerError>,
    /// The tab's own model.
    panel: Option<P>,
}

/// The subject line of the EHR `ehr_id` whose status is `status`; a
/// revealed subject id is written to the log with who revealed it.
fn subject_facts(ehr_id: &str, status: &EhrStatusState) -> SubjectFacts {
    let masked = Masked::new(
        Domain::new(if status.subject_namespace.is_empty() {
            tr(RecordsMsg::SubjectId)
        } else {
            status.subject_namespace.clone()
        }),
        &status.subject,
        KEEP,
    );
    let who = if status.subject.is_empty() {
        Who::Nobody
    } else if crate::request::query("reveal").is_some() {
        let reader = crate::request::session()
            .map(|session| session.identity)
            .unwrap_or_default();
        tracing::info!(reader = %reader, ehr = %ehr_id, "revealed an EHR's subject id");
        Who::Revealed {
            masked,
            value: status.subject.clone(),
        }
    } else {
        Who::Masked(masked)
    };
    SubjectFacts {
        who,
        queryable: status.is_queryable,
        modifiable: status.is_modifiable,
    }
}

/// Loads the page of the EHR `ehr_id`: answers its post, reads its facts and
/// then the tab's model from `panel`; an EHR the CDR does not hold answers
/// `404`.
pub async fn load_ehr<P, F>(ehr_id: &str, panel: F) -> EhrModel<P>
where
    F: AsyncFnOnce(EhrFacts) -> P,
{
    let acted = act(ehr_id).await;
    let facts = match facts(ehr_id).await {
        Ok(facts) => facts,
        Err(error) => {
            if is_status(&error, http::StatusCode::NOT_FOUND) {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
            }
            return EhrModel {
                acted,
                head: Err(error),
                panel: None,
            };
        }
    };
    let head = EhrHead {
        time_created: facts.summary.time_created.clone(),
        system_id: facts.summary.system_id.clone(),
        subject: facts
            .status
            .as_ref()
            .ok()
            .map(|status| subject_facts(ehr_id, status)),
        has_directory: facts.has_directory,
        admin: facts.admin,
    };
    EhrModel {
        acted,
        head: Ok(head),
        panel: Some(panel(facts).await),
    }
}

/// The More menu of an EHR: Commit several changes, Create the directory
/// when it has none, Copy EHR id, then Delete EHR… last when the admin group
/// is mounted.
fn more(ehr_id: &str, facts: &EhrHead) -> Menu {
    let mut items = vec![Item::new(Action::new(
        Some(Glyph::GIT_COMMIT),
        tr(RecordsMsg::EhrCommitSeveral),
        Act::Link(format!("{}/contribution/new", ehr_href(ehr_id))),
    ))];
    if !facts.has_directory {
        items.push(Item::new(Action::new(
            Some(Glyph::FOLDER_PLUS),
            tr(RecordsMsg::EhrCreateDirectory),
            Act::Post {
                action: ehr_href(ehr_id),
                fields: vec![("intent".to_owned(), "create-directory".to_owned())],
            },
        )));
    }
    items.push(Item::new(Action::new(
        Some(Glyph::COPY),
        tr(RecordsMsg::EhrCopyId),
        Act::Copy(ehr_id.to_owned()),
    )));
    let destructive = if facts.admin {
        vec![Item::new(Action::new(
            Some(Glyph::TRASH),
            tr(RecordsMsg::EhrDelete),
            Act::Dialog(DELETE_DIALOG.to_owned()),
        ))]
    } else {
        Vec::new()
    };
    Menu::new("ehr-more", tr(RecordsMsg::EhrMore), items).destructive(destructive)
}

/// Delete EHR's confirmation: what a physical delete removes, and the field
/// that takes the first eight characters of the id.
fn delete_dialog(ehr_id: &str) -> Html {
    let fields = h(format!(
        r#"<input type="hidden" name="intent" value="delete-ehr"><div class="fui-field"><label for="delete-confirm" class="fui-label">{label}</label><input id="delete-confirm" name="confirm" type="text" class="fui-input fui-mono" autocomplete="off" spellcheck="false" required></div>"#,
        label = t(tr(RecordsMsg::EhrDeleteConfirmLabel)),
    ));
    let confirm = h(format!(
        r#"<button type="submit" class="fui-button fui-button-danger">{}{}</button>"#,
        ferrohealth_ui::icon::icon(Glyph::TRASH, "16"),
        t(tr(RecordsMsg::EhrDeleteConfirm))
    ));
    danger_form_dialog(
        &current(),
        Confirmation {
            id: DELETE_DIALOG.into(),
            title: tr_with(
                RecordsMsg::EhrDeleteTitle,
                &[("id", shortened(ehr_id).into())],
            )
            .into(),
            message: tr(RecordsMsg::EhrDeleteMessage).into(),
        },
        ConfirmForm {
            action: ehr_href(ehr_id),
            fields,
        },
        confirm,
    )
}

/// The subject line (E1) of the EHR `ehr_id`.
fn subject(ehr_id: &str, subject: Option<&SubjectFacts>) -> Html {
    let Some(subject) = subject else {
        return Html::empty();
    };
    let reveal = |masked: &Masked, state: Reveal| {
        crate::components::subject_reveal(
            "subject".to_owned(),
            masked.clone(),
            state,
            format!("{}?reveal=1", ehr_href(ehr_id)),
        )
    };
    let who = match &subject.who {
        Who::Nobody => Subject::None,
        Who::Revealed { masked, value } => Subject::External(reveal(
            masked,
            Reveal::Shown(Revealed::new(value.clone(), crate::prefs::reveal_seconds())),
        )),
        Who::Masked(masked) => Subject::External(reveal(masked, Reveal::Masked)),
    };
    subject_line(
        &current(),
        SubjectLine {
            subject: who,
            queryable: subject.queryable,
            modifiable: subject.modifiable,
        },
    )
}

/// The header of the EHR `ehr_id`: its trail, its title with New composition
/// and More, the line on when it was created, and its subject line.
fn header(ehr_id: &str, facts: &EhrHead) -> Html {
    let locale = current();
    let actions = ActionGroup::primary(Action::new(
        Some(Glyph::PLUS),
        tr(RecordsMsg::NewComposition),
        Act::Link(format!("{}/compositions/new", ehr_href(ehr_id))),
    ))
    .more(more(ehr_id, facts));
    let heading = page_heading(
        &locale,
        Header {
            title: tr_with(RecordsMsg::EhrTitle, &[("id", shortened(ehr_id).into())]).into(),
            subtitle: Some(
                tr_with(
                    RecordsMsg::EhrSubtitle,
                    &[
                        (
                            "created",
                            ferrohealth_ui::view::time::reading(&locale, &facts.time_created)
                                .into(),
                        ),
                        ("system", facts.system_id.clone().into()),
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
    Html::join([
        heading,
        subject(ehr_id, facts.subject.as_ref()),
        opt(facts.admin.then(|| delete_dialog(ehr_id))),
    ])
}

/// The tabs of the EHR, the Directory tab only when it has one.
fn tabs(ehr_id: &str, facts: &EhrHead) -> Vec<Tab> {
    let mut tabs = vec![Tab::new(
        EhrTab::Compositions.key(),
        tab_href(ehr_id, EhrTab::Compositions),
        tr(RecordsMsg::EhrTabCompositions),
    )];
    if facts.has_directory {
        tabs.push(Tab::new(
            EhrTab::Directory.key(),
            tab_href(ehr_id, EhrTab::Directory),
            tr(RecordsMsg::EhrTabDirectory),
        ));
    }
    tabs.extend([
        Tab::new(
            EhrTab::Status.key(),
            tab_href(ehr_id, EhrTab::Status),
            tr(RecordsMsg::EhrTabStatus),
        ),
        Tab::new(
            EhrTab::Contributions.key(),
            tab_href(ehr_id, EhrTab::Contributions),
            tr(RecordsMsg::EhrTabContributions),
        ),
        Tab::new(
            EhrTab::Tags.key(),
            tab_href(ehr_id, EhrTab::Tags),
            tr(RecordsMsg::EhrTabTags),
        ),
    ]);
    tabs
}

/// What a post to an EHR's page asks, done: Create the directory and Delete
/// EHR. `Some` holds what it did not do, drawn above the tabs.
async fn act(ehr_id: &str) -> Option<Acted> {
    let posted = crate::posted::posted()?;
    match posted.intent().as_str() {
        "create-directory" => {
            let body = crate::bff::ehr_detail::directory::empty_root_folder().to_string();
            match crate::bff::ehr_detail::directory::create_directory(ehr_id.to_owned(), body).await
            {
                Ok(_) => {
                    redirect_with(
                        &tab_href(ehr_id, EhrTab::Directory),
                        &Done::ok(tr(RecordsMsg::DirectoryCreated)),
                    );
                    None
                }
                Err(error) => Some(Acted::Refused(Refused::new(
                    tr(RecordsMsg::DirectoryCreateRefused),
                    error,
                ))),
            }
        }
        "delete-ehr" => {
            let typed = posted.text("confirm");
            let head: String = ehr_id.chars().take(8).collect();
            if typed != head {
                return Some(Acted::Mismatch);
            }
            match crate::admin::admin_delete_ehr(ehr_id.to_owned()).await {
                Ok(()) => {
                    redirect_with(
                        "/ehrs",
                        &Done::ok(tr(RecordsMsg::EhrDeleted)).line(shortened(ehr_id)),
                    );
                    None
                }
                Err(error) => Some(Acted::Refused(Refused::new(
                    tr(RecordsMsg::EhrDeleteRefused),
                    error,
                ))),
            }
        }
        _ => None,
    }
}

/// An EHR's page: its header and the tab `tab` over the panel `draw` makes
/// of the tab's model; an EHR the CDR does not hold says so once.
pub fn ehr_view<P, F>(ehr_id: &str, tab: EhrTab, model: EhrModel<P>, draw: F) -> Html
where
    F: FnOnce(P) -> Html,
{
    let EhrModel { acted, head, panel } = model;
    let acted = acted.map(|acted| match acted {
        Acted::Refused(refusal) => refusal.view(),
        Acted::Mismatch => crate::pages::common::warn_note(t(tr(RecordsMsg::EhrDeleteMismatch))),
    });
    let head = match head {
        Ok(head) => head,
        Err(error) => return missing(ehr_id, &error),
    };
    let header = header(ehr_id, &head);
    let tabs = tabs(ehr_id, &head);
    let body = opt(panel.map(draw));
    let binder = binder(&current(), tr(RecordsMsg::EhrViews), tabs, tab.key(), body);
    header.then(h(format!(
        r#"<div class="fui-stack">{}{binder}</div>"#,
        opt(acted)
    )))
}

/// The page of an EHR the CDR cannot read: no EHR with this id (404), said
/// once at the top with no tabs, or the CDR's other refusal.
fn missing(ehr_id: &str, error: &ViewerError) -> Html {
    let title = if is_status(error, http::StatusCode::NOT_FOUND) {
        tr_with(
            RecordsMsg::EhrNotFoundTitle,
            &[("id", shortened(ehr_id).into())],
        )
    } else {
        tr_with(RecordsMsg::EhrTitle, &[("id", shortened(ehr_id).into())])
    };
    let heading = page_heading(
        &current(),
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
        tr(RecordsMsg::EhrBack),
    );
    heading.then(h(format!(
        r#"<div class="fui-stack">{}<div class="fui-cluster">{back}</div></div>"#,
        refused(tr(RecordsMsg::EhrRefused), error, None)
    )))
}
