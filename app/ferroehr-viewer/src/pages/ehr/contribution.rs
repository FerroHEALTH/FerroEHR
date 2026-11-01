// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Contributions tab, one contribution, and New contribution
//! (FerroUI `design/consoles/ferroehr/`, pages 12.12 and 13.4 to 13.7).
//!
//! The tab draws the contributions per day over the last 30 days and the
//! list the CDR states a total for, paged by number (ITS-REST
//! `contribution_list`). A contribution's page reads its audit, the versions
//! it made and the record as the CDR holds it (`contribution_get`).
//!
//! New contribution stages several changes and commits them as one
//! `CONTRIBUTION`: all of them or none (ITS-REST `contribution_create`). The
//! staged changes live in the form, so staging, removing and changing what a
//! change is are plain posts the server answers with the form drawn again;
//! nothing is kept between requests. No openEHR spec governs the viewer's
//! pages: our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::model::Paging;
use ferrohealth_ui::table::model::Window;
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_pager};
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::chart::{ActivityChart, activity_chart};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::layout::{FieldRow, field_row};
use ferrohealth_ui::view::surface::{
    Crumb, Header, Mode, definitions, editor_heading, page_heading,
};
use ferrohealth_ui::view::template::list::list_empty;

use crate::bff::ehr_detail::commit::commit_contribution;
use crate::bff::ehr_detail::commit::staged::{StagedChange, StagedKind};
use crate::bff::ehr_detail::contributions::ContributionPage as Listed;
use crate::bff::ehr_detail::contributions::{
    contribution_activity, fetch_contribution, list_contributions,
};
use crate::bff::wire::committer_label;
use crate::components::{EditorField, EditorLanguage, EditorSetup, code_editor};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::records::RecordsMsg;
use crate::locale::{Msg, current, message, tr, tr_with};
use crate::markup::{h, opt, t};
use crate::pages::common::discarded;
use crate::pages::common::{
    EditorDock, LastDays, Refused, ago, card_panel, count, dock_editor, document_section, drawn,
    panel, section, shortened, version_number, when,
};
use crate::pages::ehr::{EhrModel, EhrTab, crumbs, ehr_href, ehr_id_param, ehr_view, tab_href};
use crate::paging::Listing;
use crate::routes::Drawn;

/// How many days the activity chart covers.
const ACTIVITY_DAYS: u16 = 30;

/// The page of the contribution `uid` of the EHR `ehr_id`.
#[must_use]
pub fn contribution_href(ehr_id: &str, uid: &str) -> String {
    format!(
        "{}/contributions/{}",
        ehr_href(ehr_id),
        urlencoding::encode(uid)
    )
}

/// The contributions per day of the last [`ACTIVITY_DAYS`] days.
fn activity(days: Result<LastDays, Refused>) -> Html {
    let days = match days {
        Ok(days) => days,
        Err(refusal) => return refusal.view(),
    };
    let total: u32 = days.counts.iter().map(|(_, count)| *count).sum();
    let most = days
        .counts
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);
    let chart = ActivityChart {
        name: "contributions-per-day",
        label: tr(RecordsMsg::ContributionsPerDay),
        summary: tr_with(
            RecordsMsg::ContributionsPerDaySummary,
            &[
                ("total", u64::from(total).into()),
                ("most", u64::from(most).into()),
                ("first", days.first.clone().into()),
                ("last", days.last.clone().into()),
            ],
        ),
        points: days.counts,
        ticks: days.ticks,
    };
    activity_chart(&chart)
}

/// The contributions of one page, as a table.
fn listing(ehr_id: &str, page: &Listed) -> Html {
    let columns = [
        Column::text("contribution", tr(RecordsMsg::ColumnContribution)).role(Role::Title),
        Column::text("committed", tr(RecordsMsg::ColumnCommitted)).role(Role::Sub),
        Column::text("committer", tr(RecordsMsg::ColumnCommitter)).role(Role::Meta),
        Column::text("change", tr(RecordsMsg::ColumnChange)).role(Role::Meta),
    ];
    let rows = page
        .rows
        .iter()
        .map(|row| {
            let href = contribution_href(ehr_id, &row.uid);
            let title = title_cell(Some(&href), t(shortened(&row.uid)), None);
            let when_cell = cell(&columns[1], ago(&row.time_committed));
            let who_cell = cell(&columns[2], t(&row.committer));
            let change = cell(&columns[3], t(change_words(&row.change_type_rubric)));
            h(format!("<tr>{title}{when_cell}{who_cell}{change}</tr>"))
        })
        .collect();
    let caption = tr(RecordsMsg::EhrTabContributions);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(rows),
    })
}

/// The words of an `audit_change_type` rubric, the rubric itself when the
/// viewer has none for it.
fn change_words(rubric: &str) -> String {
    match rubric {
        "creation" => tr(Msg::ChangeCreation),
        "amendment" => tr(Msg::ChangeAmendment),
        "modification" => tr(Msg::ChangeModification),
        "deleted" => tr(Msg::ChangeDeleted),
        "attestation" => tr(RecordsMsg::ChangeAttestation),
        "synthesis" => tr(RecordsMsg::ChangeSynthesis),
        other => other.to_owned(),
    }
}

/// Reads the page of contributions the paging asks for, the last page when
/// it asks past the end.
async fn page_of(ehr_id: &str, paging: Paging) -> Result<(Listed, Window), ViewerError> {
    let asked = Window::over(u32::MAX, paging);
    let page = list_contributions(ehr_id.to_owned(), asked.start, asked.size).await?;
    let window = Window::over(page.total, paging);
    if window.start == asked.start {
        return Ok((page, window));
    }
    let page = list_contributions(ehr_id.to_owned(), window.start, window.size).await?;
    Ok((page, window))
}

/// The Contributions tab's model: the contributions per day and the page of
/// the list.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ContributionsPanel {
    /// The contributions per day, or the CDR's refusal of them.
    activity: Result<LastDays, Refused>,
    /// The page of the list, or the CDR's refusal of it.
    list: Result<Listed, Refused>,
}

/// Loads the Contributions tab.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)).
async fn load_tab(
    // The EHR the route names.
    ehr_id: String,
) -> Result<EhrModel<ContributionsPanel>, ViewerError> {
    crate::session::require_session()?;
    Ok(read_tab(ehr_id).await)
}

/// What [`load_tab`] reads.
async fn read_tab(ehr_id: String) -> EhrModel<ContributionsPanel> {
    crate::pages::ehr::load_ehr(&ehr_id, async |_facts| {
        let activity = contribution_activity(ehr_id.clone())
            .await
            .map(|points| crate::pages::common::last_days(&points, ACTIVITY_DAYS))
            .map_err(|error| Refused::new(tr(RecordsMsg::ContributionsActivityRefused), error));
        let list = page_of(&ehr_id, crate::paging::current(Listing::Contributions))
            .await
            .map(|(page, _)| page)
            .map_err(|error| Refused::new(tr(RecordsMsg::ContributionsRefused), error));
        ContributionsPanel { activity, list }
    })
    .await
}

/// The Contributions tab's panel.
fn contributions_panel(ehr_id: &str, model: ContributionsPanel) -> Html {
    let ContributionsPanel { activity, list } = model;
    let chart = self::activity(activity);
    let list = match list {
        Ok(page) if page.total == 0 => section(
            Some(tr(RecordsMsg::EhrTabContributions)),
            Some(0),
            None,
            list_empty(tr(RecordsMsg::ContributionsEmpty), None),
        ),
        Ok(page) => {
            let window = Window::over(page.total, crate::paging::current(Listing::Contributions));
            let fields = crate::paging::fields(Listing::Contributions);
            let pager = (window.pages > 1).then(|| {
                post_pager(
                    &current(),
                    PagerListing::new(tr(RecordsMsg::ContributionsPages)),
                    window,
                    PostTo {
                        id: "contributions-pager",
                        action: crate::paging::PAGING_ROUTE,
                        fields: &fields,
                    },
                )
            });
            section(
                Some(tr(RecordsMsg::EhrTabContributions)),
                Some(u64::from(page.total)),
                None,
                listing(ehr_id, &page).then(opt(
                    pager.map(|pager| h(format!(r#"<div class="fui-list-foot">{pager}</div>"#)))
                )),
            )
        }
        Err(refusal) => section(
            Some(tr(RecordsMsg::EhrTabContributions)),
            None,
            None,
            refusal.view(),
        ),
    };
    panel(
        section(
            Some(tr(RecordsMsg::ContributionsPerDayTitle)),
            None,
            None,
            chart,
        )
        .then(list),
    )
}

/// An EHR's Contributions tab.
pub async fn tab_page() -> Drawn {
    let ehr_id = ehr_id_param();
    let title = tr_with(
        RecordsMsg::ContributionsTitle,
        &[("id", shortened(&ehr_id).into())],
    );
    let body = drawn(load_tab(ehr_id.clone()).await, |model| {
        ehr_view(&ehr_id, EhrTab::Contributions, model, |listed| {
            contributions_panel(&ehr_id, listed)
        })
    });
    Drawn::page(title, body)
}

/// One version a contribution made, as its `OBJECT_REF` names it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Made {
    /// The `OBJECT_VERSION_ID`.
    id: String,
    /// The reference's `type`.
    kind: String,
    /// The reference's `namespace`.
    namespace: String,
}

/// The audit and the versions of a `CONTRIBUTION` document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Read {
    /// When it was committed.
    committed: String,
    /// Who committed it.
    committer: String,
    /// Its change type's rubric.
    change: String,
    /// Its description, when it carries one.
    description: Option<String>,
    /// The system that recorded it.
    system: String,
    /// The versions it made.
    made: Vec<Made>,
}

#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's CONTRIBUTION JSON wire for its audit and versions"
)]
/// Reads the facts of a `CONTRIBUTION` document.
fn read(body: &str) -> Result<Read, serde_json::Error> {
    use serde_json::Value;
    let doc: Value = serde_json::from_str(body)?;
    let text = |pointer: &str| {
        doc.pointer(pointer)
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let field = |value: &Value, pointer: &str| {
        value
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let made = doc
        .get("versions")
        .and_then(Value::as_array)
        .map(|versions| {
            versions
                .iter()
                .map(|version| Made {
                    id: field(version, "/id/value"),
                    kind: field(version, "/type"),
                    namespace: field(version, "/namespace"),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Read {
        committed: text("/audit/time_committed/value").unwrap_or_default(),
        committer: committer_label(|key| text(&format!("/audit/committer/{key}"))),
        change: text("/audit/change_type/value").unwrap_or_default(),
        description: text("/audit/description/value"),
        system: text("/audit/system_id").unwrap_or_default(),
        made,
    })
}

/// The versions a contribution made, as a table.
fn made_table(ehr_id: &str, made: &[Made]) -> Html {
    let columns = [
        Column::text("object", tr(RecordsMsg::ColumnObject)).role(Role::Title),
        Column::text("kind", tr(Msg::ColumnKind)).role(Role::Sub),
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Meta),
    ];
    let rows = made
        .iter()
        .map(|version| {
            let object = crate::uid::container_uid_of(&version.id);
            let href = (version.namespace == "local" && version.kind.contains("COMPOSITION"))
                .then(|| crate::pages::composition::composition_href(ehr_id, &object));
            let title = title_cell(href.as_deref(), t(shortened(&object)), None);
            let kind = cell(&columns[1], t(&version.kind));
            let number = cell(&columns[2], t(version_number(&version.id)));
            h(format!("<tr>{title}{kind}{number}</tr>"))
        })
        .collect();
    let caption = tr(RecordsMsg::ContributionVersions);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(rows),
    })
}

/// One contribution's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContributionModel {
    /// Its audit and versions, and its record pretty-printed.
    Read(Box<Read>, String),
    /// The CDR's refusal, or a record that is not JSON.
    Refused(Refused),
}

/// Loads the contribution `uid` of the EHR `ehr_id`; one the CDR does not
/// hold answers `404`.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)).
async fn load_contribution(
    // The EHR the route names.
    ehr_id: String,
    // The id the route names.
    uid: String,
) -> Result<ContributionModel, ViewerError> {
    crate::session::require_session()?;
    Ok(read_contribution(ehr_id, uid).await)
}

/// What [`load_contribution`] reads.
async fn read_contribution(ehr_id: String, uid: String) -> ContributionModel {
    let body = match fetch_contribution(ehr_id, uid).await {
        Ok(body) => body,
        Err(error) => {
            if crate::pages::common::is_status(&error, http::StatusCode::NOT_FOUND) {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
            }
            return ContributionModel::Refused(Refused::new(
                tr(RecordsMsg::ContributionRefused),
                error,
            ));
        }
    };
    match read(&body) {
        Ok(facts) => ContributionModel::Read(
            Box::new(facts),
            crate::bff::pretty_body(&body, crate::format::ReprFormat::CanonicalJson),
        ),
        Err(error) => ContributionModel::Refused(Refused::new(
            tr(RecordsMsg::ContributionRefused),
            ViewerError::Internal(tr_with(
                RecordsMsg::ContributionsJson,
                &[("e", error.to_string().into())],
            )),
        )),
    }
}

/// One contribution's audit, the versions it made and its record.
fn contribution_view(ehr_id: &str, facts: &Read, record: String) -> Html {
    let mut rows = vec![
        (tr(RecordsMsg::FactCommitted), when(&facts.committed)),
        (tr(RecordsMsg::FactCommitter), t(&facts.committer)),
        (tr(RecordsMsg::FactChange), t(change_words(&facts.change))),
    ];
    if let Some(description) = &facts.description {
        rows.push((tr(RecordsMsg::FactDescription), t(description)));
    }
    rows.push((
        tr(RecordsMsg::FactSystem),
        h(format!("<code>{}</code>", t(&facts.system))),
    ));
    let made = u64::try_from(facts.made.len()).unwrap_or(u64::MAX);
    card_panel(Html::join([
        section(
            Some(tr(RecordsMsg::ContributionAudit)),
            None,
            None,
            definitions(rows),
        ),
        section(
            Some(tr(RecordsMsg::ContributionVersions)),
            Some(made),
            None,
            made_table(ehr_id, &facts.made),
        ),
        document_section(
            tr(RecordsMsg::ContributionRecord),
            Language::Json,
            record,
            "contribution-record",
        ),
    ]))
}

/// One contribution: its audit, the versions it made and its record.
pub async fn page() -> Drawn {
    let ehr_id = ehr_id_param();
    let uid = crate::routes::params().get("uid").unwrap_or_default();
    let title = tr_with(
        RecordsMsg::ContributionTitle,
        &[("id", shortened(&uid).into())],
    );
    let mut trail = crumbs(&ehr_id);
    trail.push(Crumb::new(
        tr(RecordsMsg::EhrTabContributions),
        tab_href(&ehr_id, EhrTab::Contributions),
    ));
    let heading = page_heading(
        &current(),
        Header {
            title: title.clone().into(),
            subtitle: Some(tr(RecordsMsg::ContributionSubtitle).into()),
            crumbs: trail,
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let body = drawn(
        load_contribution(ehr_id.clone(), uid).await,
        |model| match model {
            ContributionModel::Read(facts, record) => contribution_view(&ehr_id, &facts, record),
            ContributionModel::Refused(refusal) => refusal.view(),
        },
    );
    Drawn::page(
        title,
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}

/// The id of New contribution's form.
const NEW_FORM: &str = "contribution-new";

/// One change as the form holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Draft {
    /// Its number in the form, from 1.
    number: u16,
    /// What it changes.
    kind: StagedKind,
    /// The template a new composition follows.
    template: String,
    /// The composition version an amendment supersedes.
    target: String,
    /// The version it supersedes.
    preceding: String,
    /// Its document.
    body: String,
}

impl Draft {
    /// A new change numbered `number`: a new composition.
    fn new(number: u16) -> Self {
        Self {
            number,
            kind: StagedKind::CompositionCreate,
            template: String::new(),
            target: String::new(),
            preceding: String::new(),
            body: String::new(),
        }
    }

    /// The change numbered `number` as `posted` holds it.
    fn posted(posted: &crate::posted::Posted, number: u16) -> Self {
        let field = |name: &str| posted.text(&format!("{name}-{number}"));
        let kind = StagedKind::from_value(&field("kind"));
        let preceding = field("preceding");
        let previous = posted.text(&format!("was-{number}"));
        // NOTE: a change whose kind the post changed starts again from what its new kind loads.
        let changed = previous != kind.as_value();
        Self {
            number,
            kind,
            template: field("template"),
            target: field("target"),
            preceding: if changed { String::new() } else { preceding },
            body: if changed {
                String::new()
            } else {
                posted
                    .get(&EditorField::Staged(number).name())
                    .unwrap_or_default()
            },
        }
    }

    /// The change as the commit takes it.
    fn staged(&self) -> StagedChange {
        let target = match self.kind {
            StagedKind::CompositionCreate => {
                crate::bff::ehr_detail::commit::staged::create_target_label(
                    &self.template,
                    &self.body,
                )
            }
            StagedKind::CompositionAmend => shortened(&self.target),
            StagedKind::StatusModify => tr(RecordsMsg::EhrTabStatus),
        };
        StagedChange {
            seq: u32::from(self.number),
            kind: self.kind,
            change_type: self.kind.default_change_type(),
            preceding_version_uid: if self.kind.supersedes() {
                self.preceding.clone()
            } else {
                String::new()
            },
            target,
            body: self.body.clone(),
        }
    }
}

/// The changes the form holds after the post's own step: staged, removed or
/// drawn again.
fn drafts(posted: Option<&crate::posted::Posted>) -> Vec<Draft> {
    let Some(posted) = posted else {
        return vec![Draft::new(1)];
    };
    let mut drafts: Vec<Draft> = Vec::new();
    for number in posted.all("seq") {
        // NOTE: the numbers are the form's own hidden fields; one that does not parse names no change.
        let Ok(number) = number.trim().parse::<u16>() else {
            continue;
        };
        if number >= 1 && drafts.iter().all(|draft| draft.number != number) {
            drafts.push(Draft::posted(posted, number));
        }
    }
    let intent = posted.intent();
    if let Some(number) = intent
        .strip_prefix("remove:")
        .and_then(|number| number.parse::<u16>().ok())
    {
        drafts.retain(|draft| draft.number != number);
    }
    if intent == "stage"
        && let Some(number) =
            (1..=u16::MAX).find(|number| drafts.iter().all(|draft| draft.number != *number))
    {
        drafts.push(Draft::new(number));
    }
    if drafts.is_empty() {
        drafts.push(Draft::new(1));
    }
    drafts
}

/// Fills what a change of its kind starts from, when the form has none yet:
/// the current status for a status change, the composition for an
/// amendment. `Some` holds the refusal that kept it empty.
async fn fill(ehr_id: &str, draft: &mut Draft) -> Option<Refused> {
    match draft.kind {
        StagedKind::StatusModify if draft.body.trim().is_empty() || draft.preceding.is_empty() => {
            match crate::bff::ehr_detail::status::fetch_ehr_status(ehr_id.to_owned()).await {
                Ok(status) => {
                    draft.preceding = status.version_uid;
                    draft.body = crate::bff::pretty_body(
                        &status.body,
                        crate::format::ReprFormat::CanonicalJson,
                    );
                    None
                }
                Err(error) => Some(Refused::new(tr(RecordsMsg::StatusRefused), error)),
            }
        }
        StagedKind::CompositionAmend if !draft.target.is_empty() => {
            let moved = draft.preceding != draft.target;
            draft.preceding.clone_from(&draft.target);
            if !moved && !draft.body.trim().is_empty() {
                return None;
            }
            match crate::bff::composition::fetch_composition(
                ehr_id.to_owned(),
                draft.target.clone(),
                crate::format::ReprFormat::CanonicalJson,
            )
            .await
            {
                Ok(Some(body)) => {
                    draft.body =
                        crate::bff::pretty_body(&body, crate::format::ReprFormat::CanonicalJson);
                    None
                }
                Ok(None) => None,
                Err(error) => Some(Refused::new(
                    tr(RecordsMsg::ContributionCompositionRefused),
                    error,
                )),
            }
        }
        _ => None,
    }
}

/// The choices of what a change is.
fn kind_options(kind: StagedKind) -> Html {
    [
        (StagedKind::CompositionCreate, RecordsMsg::StagedCreate),
        (StagedKind::CompositionAmend, RecordsMsg::StagedAmend),
        (StagedKind::StatusModify, RecordsMsg::StagedStatus),
    ]
    .into_iter()
    .map(|(option, words)| option_of(option.as_value(), &tr(words), option == kind))
    .collect()
}

/// One `<option>` of a select: its value, its words, and whether it is
/// chosen.
fn option_of(value: &str, words: &str, selected: bool) -> Html {
    h(format!(
        r#"<option value="{}"{}>{}</option>"#,
        t(value),
        crate::markup::flag("selected", selected),
        t(words)
    ))
}

/// What the form offers to pick from: the templates and the EHR's
/// compositions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Choices {
    /// The operational templates' ids.
    templates: Vec<String>,
    /// The compositions: their latest version uid and name.
    compositions: Vec<(String, String)>,
}

/// The field of what a change follows: its template, the composition it
/// amends, or the status version it supersedes.
fn own_field(draft: &Draft, choices: &Choices) -> Html {
    let id = |name: &str| format!("{name}-{}", draft.number);
    match draft.kind {
        StagedKind::CompositionCreate => {
            let options: Html = choices
                .templates
                .iter()
                .map(|template| option_of(template, template, *template == draft.template))
                .collect();
            Field::new(id("template"), tr(Msg::FilterTemplate)).view(h(format!(
                r#"<select id="{id}" name="{id}" class="fui-select"><option value="">{none}</option>{options}</select>"#,
                id = t(id("template")),
                none = t(tr(RecordsMsg::StagedFromDocument)),
            )))
        }
        StagedKind::CompositionAmend => {
            let options: Html = choices
                .compositions
                .iter()
                .map(|(uid, name)| {
                    option_of(
                        uid,
                        &format!("{name} · {}", shortened(uid)),
                        *uid == draft.target,
                    )
                })
                .collect();
            Field::new(id("target"), tr(RecordsMsg::StagedComposition))
                .help(tr(RecordsMsg::StagedCompositionHelp))
                .view(h(format!(
                    r#"<select id="{id}" name="{id}" class="fui-select"><option value="">{none}</option>{options}</select>"#,
                    id = t(id("target")),
                    none = t(tr(RecordsMsg::StagedPick)),
                )))
        }
        StagedKind::StatusModify => h(format!(
            r#"<p class="fui-small fui-muted">{}</p>"#,
            t(tr_with(
                RecordsMsg::StagedStatusFrom,
                &[("version", version_number(&draft.preceding).into())],
            ))
        )),
    }
}

/// One staged change as a section: what it changes, its own field and its
/// document.
fn change_section(
    draft: &Draft,
    choices: &Choices,
    removable: bool,
    trouble: Option<Refused>,
) -> Html {
    let number = draft.number;
    let id = |name: &str| format!("{name}-{number}");
    let change_type = draft.kind.default_change_type();
    let kind = Field::new(id("kind"), tr(RecordsMsg::StagedWhat))
        .help(tr_with(
            RecordsMsg::StagedRecordedAs,
            &[("change", change_words(change_type.rubric()).into())],
        ))
        .view(field_row(
            FieldRow::WithActions,
            h(format!(
                r#"<select id="{id}" name="{id}" class="fui-select">{options}</select><button type="submit" name="intent" value="edit" class="fui-button fui-button-secondary">{apply}</button>"#,
                id = t(id("kind")),
                options = kind_options(draft.kind),
                apply = t(tr(Msg::StagedApply)),
            )),
        ));
    let editor = code_editor(EditorSetup {
        field: EditorField::Staged(number),
        language: EditorLanguage::Json,
        text: draft.body.clone(),
        opened: None,
        marks: Vec::new(),
        label: tr_with(
            RecordsMsg::StagedDocument,
            &[("number", u64::from(number).into())],
        ),
        lead: tr(RecordsMsg::EditorJson),
        facts: vec![tr(Msg::EditorUtf8)],
        findings: None,
    });
    let remove = removable.then(|| {
        h(format!(
            r#"<button type="submit" name="intent" value="remove:{number}" class="fui-icon-button" aria-label="{label}">{glyph}</button>"#,
            label = t(tr_with(
                RecordsMsg::StagedRemove,
                &[("number", u64::from(number).into())],
            )),
            glyph = ferrohealth_ui::icon::icon(Glyph::X, "16"),
        ))
    });
    let title = tr_with(
        RecordsMsg::StagedTitle,
        &[
            ("number", u64::from(number).into()),
            ("what", draft.staged().target.into()),
        ],
    );
    section(
        Some(title),
        None,
        remove,
        h(format!(
            r#"<input type="hidden" name="seq" value="{number}"><input type="hidden" name="{was}" value="{kind_value}"><input type="hidden" name="{preceding_name}" value="{preceding}">{trouble}<div class="fui-form-grid"><div class="fui-span-2">{kind}</div><div class="fui-span-2">{own}</div></div>{editor}"#,
            was = t(id("was")),
            kind_value = t(draft.kind.as_value()),
            preceding_name = t(id("preceding")),
            preceding = t(&draft.preceding),
            trouble = opt(trouble.map(|trouble| trouble.view())),
            own = own_field(draft, choices),
        )),
    )
}

/// What the picks offer, read once for the form. `Err` is the refusal of a
/// pick the form needs.
async fn choices(ehr_id: &str, drafts: &[Draft]) -> Result<Choices, ViewerError> {
    let mut choices = Choices::default();
    if drafts
        .iter()
        .any(|draft| draft.kind == StagedKind::CompositionCreate)
    {
        let mut templates: Vec<String> = crate::bff::templates::list_templates()
            .await?
            .into_iter()
            .filter(crate::bff::templates::TemplateRow::is_composition)
            .map(|row| row.template_id)
            .collect();
        templates.sort();
        templates.dedup();
        choices.templates = templates;
    }
    if drafts
        .iter()
        .any(|draft| draft.kind == StagedKind::CompositionAmend)
    {
        let page =
            crate::bff::ehr_detail::compositions::all_compositions(ehr_id.to_owned(), 0).await?;
        choices.compositions = page
            .rows
            .iter()
            .map(|row| {
                (
                    row.first()
                        .map(crate::bff::ehrs::cell_text)
                        .unwrap_or_default(),
                    row.get(1)
                        .map(crate::bff::ehrs::cell_text)
                        .unwrap_or_default(),
                )
            })
            .collect();
    }
    Ok(choices)
}

/// What New contribution's form holds.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NewForm {
    /// The changes, each with the refusal that kept it empty.
    drafts: Vec<(Draft, Option<Refused>)>,
    /// The audit's description.
    description: String,
    /// The audit's committer.
    committer: String,
    /// The commit the CDR refused, when the post was one.
    refused: Option<Refused>,
    /// What the picks offer.
    choices: Choices,
    /// The refusal of a pick the form needs.
    pick_trouble: Option<Refused>,
}

/// New contribution's model.
#[derive(Debug, Clone, PartialEq, Eq)]
enum NewModel {
    /// The form.
    Form(Box<NewForm>),
    /// The post redirected.
    Left,
}

/// Loads New contribution: answers its post (Discard, a step of the form,
/// Commit) and reads what its changes start from and what the picks offer.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr)).
async fn load_new(
    // The EHR the route names.
    ehr_id: String,
) -> Result<NewModel, ViewerError> {
    crate::session::require_session()?;
    Ok(read_new(ehr_id).await)
}

/// What [`load_new`] reads.
async fn read_new(ehr_id: String) -> NewModel {
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        crate::request::redirect(&tab_href(&ehr_id, EhrTab::Contributions));
        return NewModel::Left;
    }
    let mut drafts = drafts(posted.as_ref());
    let mut troubles = Vec::new();
    for draft in &mut drafts {
        troubles.push(fill(&ehr_id, draft).await);
    }
    let description = posted
        .as_ref()
        .map(|posted| posted.text("description"))
        .unwrap_or_default();
    let committer = posted
        .as_ref()
        .map(|posted| posted.text("committer"))
        .unwrap_or_default();
    let mut refused = None;
    if posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "save")
    {
        let staged: Vec<StagedChange> = drafts.iter().map(Draft::staged).collect();
        match commit_contribution(
            ehr_id.clone(),
            staged,
            committer.clone(),
            description.clone(),
        )
        .await
        {
            Ok(outcome) => {
                let href = if outcome.contribution_uid.is_empty() {
                    tab_href(&ehr_id, EhrTab::Contributions)
                } else {
                    contribution_href(&ehr_id, &outcome.contribution_uid)
                };
                redirect_with(
                    &href,
                    &Done::ok(tr(RecordsMsg::ContributionCommitted)).line(tr_with(
                        RecordsMsg::ContributionMadeVersions,
                        &[("count", count(outcome.version_uids.len()))],
                    )),
                );
                return NewModel::Left;
            }
            Err(error) => {
                refused = Some(Refused::new(
                    tr(RecordsMsg::ContributionCommitRefused),
                    error,
                ));
            }
        }
    }
    let (choices, pick_trouble) = match choices(&ehr_id, &drafts).await {
        Ok(choices) => (choices, None),
        Err(error) => (
            Choices::default(),
            Some(Refused::new(
                tr(RecordsMsg::ContributionChoicesRefused),
                error,
            )),
        ),
    };
    NewModel::Form(Box::new(NewForm {
        drafts: drafts.into_iter().zip(troubles).collect(),
        description,
        committer,
        refused,
        choices,
        pick_trouble,
    }))
}

/// New contribution's form, its decision bar docked.
fn new_form(ehr_id: &str, form: NewForm) -> Html {
    let NewForm {
        drafts,
        description,
        committer,
        refused,
        choices,
        pick_trouble,
    } = form;
    let you = crate::request::signed()
        .map(|session| session.identity)
        .unwrap_or_default();
    let line = if refused.is_some() {
        (Glyph::CIRCLE_X, tr(RecordsMsg::ContributionNotCommitted))
    } else {
        (
            Glyph::GIT_COMMIT,
            tr_with(RecordsMsg::StagedCount, &[("count", count(drafts.len()))]),
        )
    };
    let staged = drafts.len();
    let removable = staged > 1;
    let sections: Html = drafts
        .into_iter()
        .map(|(draft, trouble)| change_section(&draft, &choices, removable, trouble))
        .collect();
    let stage = (staged < usize::from(u16::MAX)).then(|| {
        h(format!(
            r#"<div class="fui-add-row"><button type="submit" name="intent" value="stage" class="fui-button fui-button-ghost fui-button-small">{}{}</button></div>"#,
            ferrohealth_ui::icon::icon(Glyph::PLUS, "14"),
            t(tr(RecordsMsg::StagedStage))
        ))
    });
    let description = Field::new("au-d", tr(RecordsMsg::FactDescription)).view(h(format!(
        r#"<input id="au-d" name="description" type="text" class="fui-input" value="{}">"#,
        t(description)
    )));
    let committer = Field::new("au-c", tr(RecordsMsg::FactCommitter)).view(h(format!(
        r#"<input id="au-c" name="committer" type="text" class="fui-input" value="{}" placeholder="{}">"#,
        t(committer),
        t(tr_with(
            RecordsMsg::ContributionCommitterPlaceholder,
            &[("name", you.clone().into())],
        ))
    )));
    let audit = h(format!(
        r#"<div class="fui-form-grid"><div class="fui-span-2">{description}</div><div class="fui-span-2">{committer}</div></div>"#
    ));
    dock_editor(EditorDock {
        form: NEW_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save: tr_with(RecordsMsg::ContributionCommit, &[("count", count(staged))]),
        blocked: None,
    });
    let panel = card_panel(Html::join([
        sections,
        crate::pages::common::plain_section(opt(stage)),
        section(
            Some(tr(RecordsMsg::ContributionNewAudit)),
            None,
            None,
            audit,
        ),
    ]));
    Html::join([
        opt(refused.map(|refusal| refusal.view())),
        opt(pick_trouble.map(|refusal| refusal.view())),
        h(format!(
            r#"<form id="{NEW_FORM}" method="post" action="{}">{panel}</form>"#,
            t(format!("{}/contribution/new", ehr_href(ehr_id)))
        )),
    ])
}

/// New contribution: several changes staged and committed together.
pub async fn new_page() -> Drawn {
    let ehr_id = ehr_id_param();
    let body = drawn(load_new(ehr_id.clone()).await, |model| match model {
        NewModel::Form(form) => new_form(&ehr_id, *form),
        NewModel::Left => Html::empty(),
    });
    let heading = editor_heading(
        &current(),
        message(RecordsMsg::ContributionNewTitle),
        &Mode::Creating,
        Header {
            title: String::new().into(),
            subtitle: Some(tr(RecordsMsg::ContributionNewSubtitle).into()),
            crumbs: crumbs(&ehr_id),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    Drawn::page(
        tr(RecordsMsg::ContributionNewTitle),
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}

#[cfg(test)]
mod tests {
    use super::drafts;
    use crate::bff::ehr_detail::commit::staged::StagedKind;
    use crate::components::EditorField;
    use crate::posted::Posted;

    fn posted(fields: &[(&str, &str)]) -> Posted {
        Posted::new(
            fields
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        )
    }

    #[test]
    fn staging_and_removing_change_the_form_and_a_changed_kind_starts_again() {
        assert_eq!(drafts(None).len(), 1, "a new form holds one change");
        let staged = drafts(Some(&posted(&[
            ("seq", "1"),
            ("was-1", "create"),
            ("kind-1", "create"),
            ("staged-1", "{}"),
            ("intent", "stage"),
        ])));
        assert_eq!(
            staged.iter().map(|draft| draft.number).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(staged.first().map(|draft| draft.body.as_str()), Some("{}"));
        let removed = drafts(Some(&posted(&[
            ("seq", "1"),
            ("seq", "2"),
            ("intent", "remove:1"),
        ])));
        assert_eq!(
            removed.iter().map(|draft| draft.number).collect::<Vec<_>>(),
            vec![2]
        );
        let changed = drafts(Some(&posted(&[
            ("seq", "1"),
            ("was-1", "create"),
            ("kind-1", "status"),
            ("staged-1", "{\"old\": true}"),
            ("intent", "edit"),
        ])));
        let first = changed.first().expect("the change");
        assert_eq!(first.kind, StagedKind::StatusModify);
        assert!(
            first.body.is_empty(),
            "a new kind starts from what it loads"
        );
        let forged = drafts(Some(&posted(&[
            ("seq", "0"),
            ("seq", "70000"),
            ("seq", "x"),
        ])));
        assert_eq!(
            forged.iter().map(|draft| draft.number).collect::<Vec<_>>(),
            vec![1],
            "zero, out-of-range and non-numeric numbers name no change"
        );
        let many = drafts(Some(&posted(&[("seq", "13"), ("seq", "99")])));
        assert_eq!(
            many.iter().map(|draft| draft.number).collect::<Vec<_>>(),
            vec![13, 99],
            "a change past the twelfth keeps an editor of its own"
        );
        assert_eq!(
            EditorField::Staged(99).name(),
            "staged-99",
            "each staged change posts its own field"
        );
    }
}
