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

#![expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's CONTRIBUTION JSON wire for its audit and versions"
)]

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::model::{Paging, Window};
use ferrohealth_ui::table::pager::{Listing as PagerListing, PostTo, post_pager};
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::chart::{ActivityChart, activity_chart};
use ferrohealth_ui::view::diagnostics::DiagnosticsWords;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{
    Crumb, Header, Mode, definitions, editor_heading, page_heading,
};
use ferrohealth_ui::view::template::list::list_empty;
use ferrohealth_ui::widget::editor::EditorWords;
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;
use serde_json::Value;

use crate::bff::ehr_detail::commit::commit_contribution;
use crate::bff::ehr_detail::commit::staged::{StagedChange, StagedKind};
use crate::bff::ehr_detail::contributions::{
    ContributionPage as Listed, contribution_activity, fetch_contribution, list_contributions,
};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::islands::{CodeEditor, EditorField, EditorLanguage, EditorSetup};
use crate::locale::{Msg, message, tr, tr_with};
use crate::pages::common::{
    EditorDock, ago, awaited, card_panel, count, discarded, dock_editor, document_section, panel,
    refused, section, shortened, version_number, when,
};
use crate::pages::ehr::{EhrTab, crumbs, ehr_href, ehr_id_param, ehr_page, tab_href};
use crate::paging::Listing;

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
fn activity(points: Result<Vec<crate::activity::ActivityPoint>, ViewerError>) -> AnyView {
    let points = match points {
        Ok(points) => points,
        Err(error) => return refused(tr(Msg::ContributionsActivityRefused), &error, None),
    };
    let days = crate::pages::common::last_days(&points, ACTIVITY_DAYS);
    let total: u32 = days.counts.iter().map(|(_, count)| *count).sum();
    let most = days
        .counts
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(0);
    let chart = ActivityChart {
        name: "contributions-per-day",
        label: tr(Msg::ContributionsPerDay),
        summary: tr_with(
            Msg::ContributionsPerDaySummary,
            &[
                ("total", u64::from(total).into()),
                ("most", u64::from(most).into()),
                ("first", days.first.clone().into()),
                ("last", days.last.clone().into()),
            ],
        ),
        points: days.counts,
        ends: (days.first, days.last),
    };
    activity_chart(&chart).into_any()
}

/// The contributions of one page, as a table.
fn listing(ehr_id: &str, page: &Listed) -> AnyView {
    let columns = [
        Column::text("contribution", tr(Msg::ColumnContribution)).role(Role::Title),
        Column::text("committed", tr(Msg::ColumnCommitted)).role(Role::Sub),
        Column::text("committer", tr(Msg::ColumnCommitter)).role(Role::Meta),
        Column::text("change", tr(Msg::ColumnChange)).role(Role::Meta),
    ];
    let rows = page
        .rows
        .iter()
        .map(|row| {
            let href = contribution_href(ehr_id, &row.uid);
            let title = title_cell(Some(&href), shortened(&row.uid), None);
            let when_cell = cell(&columns[1], ago(&row.time_committed));
            let who_cell = cell(&columns[2], row.committer.clone());
            let change = cell(&columns[3], change_words(&row.change_type_rubric));
            view! { <tr>{title} {when_cell} {who_cell} {change}</tr> }.into_any()
        })
        .collect_view()
        .into_any();
    let caption = tr(Msg::EhrTabContributions);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(rows),
    })
    .into_any()
}

/// The words of an `audit_change_type` rubric, the rubric itself when the
/// viewer has none for it.
fn change_words(rubric: &str) -> String {
    match rubric {
        "creation" => tr(Msg::ChangeCreation),
        "amendment" => tr(Msg::ChangeAmendment),
        "modification" => tr(Msg::ChangeModification),
        "deleted" => tr(Msg::ChangeDeleted),
        "attestation" => tr(Msg::ChangeAttestation),
        "synthesis" => tr(Msg::ChangeSynthesis),
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

/// An EHR's Contributions tab.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn ContributionsTab() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let paging = crate::paging::current(Listing::Contributions);
    let fields = crate::paging::fields(Listing::Contributions);
    let title = tr_with(
        Msg::ContributionsTitle,
        &[("id", shortened(&ehr_id).into())],
    );
    let body = awaited({
        let ehr_id = ehr_id.clone();
        async move {
            ehr_page(
                ehr_id.clone(),
                EhrTab::Contributions,
                move |_facts| async move {
                    let chart = activity(contribution_activity(ehr_id.clone()).await);
                    let list = match page_of(&ehr_id, paging).await {
                        Ok((page, _)) if page.total == 0 => section(
                            Some(tr(Msg::EhrTabContributions)),
                            Some(0),
                            None,
                            list_empty(tr(Msg::ContributionsEmpty), None),
                        ),
                        Ok((page, window)) => {
                            let pager = (window.pages > 1).then(|| {
                                post_pager(
                                    PagerListing::new(tr(Msg::ContributionsPages)),
                                    window,
                                    PostTo {
                                        id: "contributions-pager",
                                        action: crate::paging::PAGING_ROUTE,
                                        fields: &fields,
                                    },
                                )
                            });
                            section(
                                Some(tr(Msg::EhrTabContributions)),
                                Some(u64::from(page.total)),
                                None,
                                view! {
                                    {listing(&ehr_id, &page)}
                                    {pager
                                        .map(|pager| {
                                            view! { <div class="fui-list-foot">{pager}</div> }
                                        })}
                                },
                            )
                        }
                        Err(error) => section(
                            Some(tr(Msg::EhrTabContributions)),
                            None,
                            None,
                            refused(tr(Msg::ContributionsRefused), &error, None),
                        ),
                    };
                    panel(view! {
                        {section(Some(tr(Msg::ContributionsPerDayTitle)), None, None, chart)}
                        {list}
                    })
                },
            )
            .await
        }
    });
    view! {
        <Title text=title />
        {body}
    }
}

/// One version a contribution made, as its `OBJECT_REF` names it.
#[derive(Debug, Clone)]
struct Made {
    /// The `OBJECT_VERSION_ID`.
    id: String,
    /// The reference's `type`.
    kind: String,
    /// The reference's `namespace`.
    namespace: String,
}

/// The audit and the versions of a `CONTRIBUTION` document.
#[derive(Debug, Clone, Default)]
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

/// Reads the facts of a `CONTRIBUTION` document.
fn read(body: &str) -> Result<Read, serde_json::Error> {
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
        committer: text("/audit/committer/name").unwrap_or_default(),
        change: text("/audit/change_type/value").unwrap_or_default(),
        description: text("/audit/description/value"),
        system: text("/audit/system_id").unwrap_or_default(),
        made,
    })
}

/// The versions a contribution made, as a table.
fn made_table(ehr_id: &str, made: &[Made]) -> AnyView {
    let columns = [
        Column::text("object", tr(Msg::ColumnObject)).role(Role::Title),
        Column::text("kind", tr(Msg::ColumnKind)).role(Role::Sub),
        Column::text("version", tr(Msg::ColumnVersion)).role(Role::Meta),
    ];
    let rows = made
        .iter()
        .map(|version| {
            let object = crate::uid::container_uid_of(&version.id);
            let href = (version.namespace == "local" && version.kind.contains("COMPOSITION"))
                .then(|| crate::pages::composition::composition_href(ehr_id, &object));
            let title = title_cell(href.as_deref(), shortened(&object), None);
            let kind = cell(&columns[1], version.kind.clone());
            let number = cell(&columns[2], version_number(&version.id));
            view! { <tr>{title} {kind} {number}</tr> }.into_any()
        })
        .collect_view()
        .into_any();
    let caption = tr(Msg::ContributionVersions);
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(rows),
    })
    .into_any()
}

/// One contribution: its audit, the versions it made and its record.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn ContributionPage() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let uid = use_params_map()
        .with_untracked(|params| params.get("uid"))
        .unwrap_or_default();
    let title = tr_with(Msg::ContributionTitle, &[("id", shortened(&uid).into())]);
    let mut trail = crumbs(&ehr_id);
    trail.push(Crumb::new(
        tr(Msg::EhrTabContributions),
        tab_href(&ehr_id, EhrTab::Contributions),
    ));
    let heading = page_heading(
        Header {
            title: title.clone().into(),
            subtitle: Some(tr(Msg::ContributionSubtitle).into()),
            crumbs: trail,
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let body = awaited({
        let ehr_id = ehr_id.clone();
        async move {
            let body = match fetch_contribution(ehr_id.clone(), uid.clone()).await {
                Ok(body) => body,
                Err(error) => {
                    if crate::pages::common::is_status(&error, http::StatusCode::NOT_FOUND) {
                        crate::request::set_status(http::StatusCode::NOT_FOUND);
                    }
                    return refused(tr(Msg::ContributionRefused), &error, None);
                }
            };
            let facts = match read(&body) {
                Ok(facts) => facts,
                Err(error) => {
                    let error = ViewerError::Internal(tr_with(
                        Msg::ContributionsJson,
                        &[("e", error.to_string().into())],
                    ));
                    return refused(tr(Msg::ContributionRefused), &error, None);
                }
            };
            let mut rows = vec![
                (tr(Msg::FactCommitted), when(&facts.committed)),
                (tr(Msg::FactCommitter), facts.committer.clone().into_any()),
                (tr(Msg::FactChange), change_words(&facts.change).into_any()),
            ];
            if let Some(description) = facts.description.clone() {
                rows.push((tr(Msg::FactDescription), description.into_any()));
            }
            rows.push((
                tr(Msg::FactSystem),
                view! { <code>{facts.system.clone()}</code> }.into_any(),
            ));
            let made = u64::try_from(facts.made.len()).unwrap_or(u64::MAX);
            card_panel(view! {
                {section(Some(tr(Msg::ContributionAudit)), None, None, definitions(rows))}
                {section(
                    Some(tr(Msg::ContributionVersions)),
                    Some(made),
                    None,
                    made_table(&ehr_id, &facts.made),
                )}
                {document_section(
                    tr(Msg::ContributionRecord),
                    Language::Json,
                    crate::bff::pretty_body(&body, crate::format::ReprFormat::CanonicalJson),
                    "contribution-record",
                )}
            })
        }
    });
    view! {
        <Title text=title />
        {heading}
        <div class="fui-stack">{body}</div>
    }
}

/// The id of New contribution's form.
const NEW_FORM: &str = "contribution-new";

/// One change as the form holds it.
#[derive(Debug, Clone)]
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
            StagedKind::StatusModify => tr(Msg::EhrTabStatus),
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
async fn fill(ehr_id: &str, draft: &mut Draft) -> Option<AnyView> {
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
                Err(error) => Some(refused(tr(Msg::StatusRefused), &error, None)),
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
                Err(error) => Some(refused(
                    tr(Msg::ContributionCompositionRefused),
                    &error,
                    None,
                )),
            }
        }
        _ => None,
    }
}

/// The choices of what a change is.
fn kind_options(kind: StagedKind) -> AnyView {
    [
        (StagedKind::CompositionCreate, Msg::StagedCreate),
        (StagedKind::CompositionAmend, Msg::StagedAmend),
        (StagedKind::StatusModify, Msg::StagedStatus),
    ]
    .into_iter()
    .map(|(option, words)| {
        view! {
            <option value=option.as_value() selected=option == kind>
                {tr(words)}
            </option>
        }
    })
    .collect_view()
    .into_any()
}

/// What the form offers to pick from: the templates and the EHR's
/// compositions.
#[derive(Debug, Clone, Default)]
struct Choices {
    /// The operational templates' ids.
    templates: Vec<String>,
    /// The compositions: their latest version uid and name.
    compositions: Vec<(String, String)>,
}

/// The field of what a change follows: its template, the composition it
/// amends, or the status version it supersedes.
fn own_field(draft: &Draft, choices: &Choices) -> AnyView {
    let id = |name: &str| format!("{name}-{}", draft.number);
    match draft.kind {
        StagedKind::CompositionCreate => {
            let options = choices
                .templates
                .iter()
                .map(|template| {
                    view! {
                        <option value=template.clone() selected=*template == draft.template>
                            {template.clone()}
                        </option>
                    }
                })
                .collect_view();
            Field::new(id("template"), tr(Msg::FilterTemplate)).view(view! {
                <select id=id("template") name=id("template") class="fui-select">
                    <option value="">{tr(Msg::StagedFromDocument)}</option>
                    {options}
                </select>
            })
        }
        StagedKind::CompositionAmend => {
            let options = choices
                .compositions
                .iter()
                .map(|(uid, name)| {
                    view! {
                        <option value=uid.clone() selected=*uid == draft.target>
                            {format!("{name} · {}", shortened(uid))}
                        </option>
                    }
                })
                .collect_view();
            Field::new(id("target"), tr(Msg::StagedComposition))
                .help(tr(Msg::StagedCompositionHelp))
                .view(view! {
                    <select id=id("target") name=id("target") class="fui-select">
                        <option value="">{tr(Msg::StagedPick)}</option>
                        {options}
                    </select>
                })
        }
        StagedKind::StatusModify => view! {
            <p class="fui-small fui-muted">
                {tr_with(
                    Msg::StagedStatusFrom,
                    &[("version", version_number(&draft.preceding).into())],
                )}
            </p>
        }
        .into_any(),
    }
}

/// One staged change as a section: what it changes, its own field and its
/// document.
fn change_section(
    draft: &Draft,
    choices: &Choices,
    removable: bool,
    trouble: Option<AnyView>,
) -> AnyView {
    let number = draft.number;
    let id = |name: &str| format!("{name}-{number}");
    let change_type = draft.kind.default_change_type();
    let kind = Field::new(id("kind"), tr(Msg::StagedWhat))
        .help(tr_with(
            Msg::StagedRecordedAs,
            &[("change", change_words(change_type.rubric()).into())],
        ))
        .view(view! {
            <div class="fui-cluster">
                <select id=id("kind") name=id("kind") class="fui-select">
                    {kind_options(draft.kind)}
                </select>
                <button
                    type="submit"
                    name="intent"
                    value="edit"
                    class="fui-button fui-button-secondary fui-button-small"
                >
                    {tr(Msg::StagedApply)}
                </button>
            </div>
        });
    let editor = view! {
        <CodeEditor
            setup=EditorSetup {
                field: EditorField::Staged(number),
                language: EditorLanguage::Json,
                text: draft.body.clone(),
                opened: None,
                marks: Vec::new(),
                label: tr_with(Msg::StagedDocument, &[("number", u64::from(number).into())]),
                lead: tr(Msg::EditorJson),
                facts: vec![tr(Msg::EditorUtf8)],
                findings: None,
            }
            words=EditorWords::of_page()
            diagnostics=DiagnosticsWords::of_page()
        />
    };
    let remove = removable.then(|| {
        view! {
            <button
                type="submit"
                name="intent"
                value=format!("remove:{number}")
                class="fui-icon-button"
                aria-label=tr_with(Msg::StagedRemove, &[("number", u64::from(number).into())])
            >
                {ferrohealth_ui::icon::icon(Glyph::X, "16")}
            </button>
        }
        .into_any()
    });
    let title = tr_with(
        Msg::StagedTitle,
        &[
            ("number", u64::from(number).into()),
            ("what", draft.staged().target.into()),
        ],
    );
    section(
        Some(title),
        None,
        remove,
        view! {
            <input type="hidden" name="seq" value=number.to_string() />
            <input type="hidden" name=id("was") value=draft.kind.as_value() />
            <input type="hidden" name=id("preceding") value=draft.preceding.clone() />
            {trouble}
            <div class="fui-form-grid">
                <div class="fui-span-2">{kind}</div>
                <div class="fui-span-2">{own_field(draft, choices)}</div>
            </div>
            {editor}
        },
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

/// New contribution: several changes staged and committed together.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn NewContributionPage() -> impl IntoView {
    let ehr_id = ehr_id_param();
    let posted = crate::posted::posted();
    if discarded(posted.as_ref()) {
        leptos_axum::redirect(&tab_href(&ehr_id, EhrTab::Contributions));
    }
    let dock_promise = crate::shell::promise_dock();
    let body = awaited({
        let ehr_id = ehr_id.clone();
        async move {
            let _dock_promise = dock_promise;
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
            let mut top = None;
            let mut line = (
                Glyph::GIT_COMMIT,
                tr_with(Msg::StagedCount, &[("count", count(drafts.len()))]),
            );
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
                            &Done::ok(tr(Msg::ContributionCommitted)).line(tr_with(
                                Msg::ContributionMadeVersions,
                                &[("count", count(outcome.version_uids.len()))],
                            )),
                        );
                        return ().into_any();
                    }
                    Err(error) => {
                        line = (Glyph::CIRCLE_X, tr(Msg::ContributionNotCommitted));
                        top = Some(refused(tr(Msg::ContributionCommitRefused), &error, None));
                    }
                }
            }
            let (choices, pick_trouble) = match choices(&ehr_id, &drafts).await {
                Ok(choices) => (choices, None),
                Err(error) => (
                    Choices::default(),
                    Some(refused(tr(Msg::ContributionChoicesRefused), &error, None)),
                ),
            };
            let removable = drafts.len() > 1;
            let sections = drafts
                .iter()
                .zip(troubles)
                .map(|(draft, trouble)| change_section(draft, &choices, removable, trouble))
                .collect_view();
            let stage = (drafts.len() < usize::from(u16::MAX)).then(|| {
                view! {
                    <div class="fui-add-row">
                        <button
                            type="submit"
                            name="intent"
                            value="stage"
                            class="fui-button fui-button-ghost fui-button-small"
                        >
                            {ferrohealth_ui::icon::icon(Glyph::PLUS, "14")}
                            {tr(Msg::StagedStage)}
                        </button>
                    </div>
                }
            });
            let audit = view! {
                <div class="fui-form-grid">
                    <div class="fui-span-2">
                        {Field::new("au-d", tr(Msg::FactDescription))
                            .view(
                                view! {
                                    <input
                                        id="au-d"
                                        name="description"
                                        type="text"
                                        class="fui-input"
                                        value=description
                                    />
                                },
                            )}
                    </div>
                    <div class="fui-span-2">
                        {Field::new("au-c", tr(Msg::FactCommitter))
                            .view(
                                view! {
                                    <input
                                        id="au-c"
                                        name="committer"
                                        type="text"
                                        class="fui-input"
                                        value=committer
                                        placeholder=tr(Msg::ContributionCommitterPlaceholder)
                                    />
                                },
                            )}
                    </div>
                </div>
            };
            dock_editor(EditorDock {
                form: NEW_FORM.to_owned(),
                line: line.1,
                glyph: line.0,
                save: tr_with(Msg::ContributionCommit, &[("count", count(drafts.len()))]),
                blocked: None,
            });
            view! {
                {top}
                {pick_trouble}
                <form
                    id=NEW_FORM
                    method="post"
                    action=format!("{}/contribution/new", ehr_href(&ehr_id))
                >
                    {card_panel(
                        view! {
                            {sections}
                            {crate::pages::common::plain_section(stage)}
                            {section(Some(tr(Msg::ContributionAudit)), None, None, audit)}
                        },
                    )}
                </form>
            }
            .into_any()
        }
    });
    let heading = editor_heading(
        message(Msg::ContributionNewTitle),
        &Mode::Creating,
        Header {
            title: String::new().into(),
            subtitle: Some(tr(Msg::ContributionNewSubtitle).into()),
            crumbs: crumbs(&ehr_id),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    view! {
        <Title text=tr(Msg::ContributionNewTitle) />
        {heading}
        <div class="fui-stack">{body}</div>
    }
}

#[cfg(test)]
mod tests {
    use super::drafts;
    use crate::bff::ehr_detail::commit::staged::StagedKind;
    use crate::islands::EditorField;
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
