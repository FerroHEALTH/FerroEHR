// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Terminology: what the CDR can answer about the code sets it serves.
//!
//! Four tabs, Terminologies, Look up, Value sets and Hierarchy, and one
//! terminology's descriptor (FerroUI `design/consoles/ferroehr/`, pages 17.1
//! to 17.7).
//!
//! The operations are the openEHR Terminology Service's
//! (`docs/specs/openehr/SM/docs/UML/classes/terminology_service.adoc`) as the
//! CDR serves them under `terminology` ([`crate::terminology`]); a CDR with
//! the terminology API off answers `404`, and the page says so once. Every
//! form is a `GET`, so an answer keeps its address, and each tab reads its
//! answers on the server into one model that both halves draw. No openEHR spec governs
//! the viewer's pages: our own design.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::coding::{Coding, Layout, coding};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::load_report::{Arrival, LoadReport, Outcome, load_report};
use ferrohealth_ui::view::surface::{Crumb, Header, definitions, page_heading};
use ferrohealth_ui::view::tabs::{Tab, binder};
use leptos::prelude::*;
use leptos::server;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{awaited, card_panel, panel, plain_section, refused, section};
use crate::session_client::SessionAwareClient;
use crate::terminology::{TermRow, TerminologyDescriptor};
#[cfg(not(target_arch = "wasm32"))]
use crate::terminology::{
    check_subsumption, fetch_term, fetch_terminology_description, fetch_value_set,
    list_terminologies, validate_value_set_code,
};

/// The value sets of the openEHR terminology the Value sets tab offers, as
/// its groups (openEHR TERM `openehr_terminology.xml`).
const OPENEHR_GROUPS: [&str; 12] = [
    "attestation_reason",
    "audit_change_type",
    "composition_category",
    "event_math_function",
    "instruction_states",
    "null_flavours",
    "participation_function",
    "participation_mode",
    "property",
    "setting",
    "term_mapping_purpose",
    "version_lifecycle_state",
];

/// The tabs of Terminology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TermTab {
    /// The terminologies the CDR serves.
    Terminologies,
    /// Look up a code.
    Lookup,
    /// Expand a value set and validate a code against it.
    ValueSets,
    /// Test subsumption.
    Subsumption,
}

impl TermTab {
    /// Every tab, in order.
    const ALL: [Self; 4] = [
        Self::Terminologies,
        Self::Lookup,
        Self::ValueSets,
        Self::Subsumption,
    ];

    /// The tab's key.
    const fn key(self) -> &'static str {
        match self {
            Self::Terminologies => "terminologies",
            Self::Lookup => "lookup",
            Self::ValueSets => "valuesets",
            Self::Subsumption => "subsumption",
        }
    }

    /// The tab's page.
    fn href(self) -> String {
        match self {
            Self::Terminologies => "/terminology".to_owned(),
            other => format!("/terminology/{}", other.key()),
        }
    }

    /// The tab's label.
    fn label(self) -> String {
        tr(match self {
            Self::Terminologies => Msg::TermTerminologies,
            Self::Lookup => Msg::TermLookup,
            Self::ValueSets => Msg::TermValueSets,
            Self::Subsumption => Msg::TermHierarchy,
        })
    }
}

/// The heading of Terminology.
fn heading() -> AnyView {
    page_heading(
        Header {
            title: tr(Msg::NavTerminology).into(),
            subtitle: Some(tr(Msg::TermSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    )
    .into_any()
}

/// The note of a CDR whose terminology API is off.
fn off() -> AnyView {
    card_panel(view! {
        <div class="fui-panel-body fui-stack">
            <p>
                <strong>{tr(Msg::TermOff)}</strong>
            </p>
            <p class="fui-small fui-muted">{tr(Msg::TermOffLine)}</p>
        </div>
    })
}

/// What a tab of Terminology read: the terminology ids for its forms and the
/// tab's own answer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tabbed<T> {
    /// The terminologies the CDR serves.
    ids: Vec<String>,
    /// What the tab read.
    read: T,
}

/// A tab's model: `None` when the CDR has the terminology API off, and the
/// refusal of the terminology list as the error.
type TabModel<T> = Result<Option<Tabbed<T>>, ViewerError>;

/// Reads the terminologies the CDR serves, then the tab's answer over them.
#[cfg(not(target_arch = "wasm32"))]
async fn tabbed<T, F, Fut>(read: F) -> TabModel<T>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: Future<Output = T>,
{
    crate::session::require_session().await?;
    let Some(ids) = list_terminologies().await? else {
        return Ok(None);
    };
    let read = read(ids.clone()).await;
    Ok(Some(Tabbed { ids, read }))
}

/// A tab of Terminology over what `body` draws of its model; the note alone
/// when the API is off.
fn term_page<T>(
    tab: TermTab,
    model: TabModel<T>,
    body: impl FnOnce(&[String], T) -> AnyView,
) -> AnyView {
    let Tabbed { ids, read } = match model {
        Ok(Some(tabbed)) => tabbed,
        Ok(None) => {
            return view! {
                {heading()}
                <div class="fui-stack">{off()}</div>
            }
            .into_any();
        }
        Err(error) => {
            return view! {
                {heading()}
                {refused(tr(Msg::TermRefused), &error, None)}
            }
            .into_any();
        }
    };
    let tabs = TermTab::ALL
        .into_iter()
        .map(|each| Tab::new(each.key(), each.href(), each.label()))
        .collect();
    let panel = body(&ids, read);
    view! {
        {heading()}
        <div class="fui-stack">{binder(tr(Msg::TermOperations), tabs, tab.key(), panel)}</div>
    }
    .into_any()
}

/// The terminology select of a form, `chosen` selected.
fn terminology_field(id: &'static str, ids: &[String], chosen: &str) -> AnyView {
    let options = ids
        .iter()
        .map(|each| {
            view! {
                <option value=each.clone() selected=*each == chosen>
                    {each.clone()}
                </option>
            }
        })
        .collect_view();
    Field::new(id, tr(Msg::TermTerminology)).view(view! {
        <select id=id name="terminology" class="fui-select">
            {options}
        </select>
    })
}

/// A code text field of a form.
fn code_field(id: &'static str, name: &'static str, label: String, value: &str) -> AnyView {
    Field::new(id, label).view(view! {
        <input
            id=id
            name=name
            type="text"
            class="fui-input fui-mono"
            spellcheck="false"
            value=value.to_owned()
        />
    })
}

/// The run button of a form.
fn run(glyph: Glyph, words: String) -> AnyView {
    view! {
        <div class="fui-transform-run">
            <button type="submit" class="fui-button fui-button-primary">
                {ferrohealth_ui::icon::icon(glyph, "16")}
                {words}
            </button>
        </div>
    }
    .into_any()
}

/// An answer in the given outcome.
fn answer(outcome: Outcome, title: AnyView) -> AnyView {
    load_report(LoadReport {
        outcome,
        arrival: Arrival::AfterAction,
        title,
        counts: Vec::new(),
        action: None,
    })
}

/// The terminology a form names, `asked`, the first the CDR serves for none.
#[cfg(not(target_arch = "wasm32"))]
fn chosen(ids: &[String], asked: String) -> String {
    Some(asked)
        .filter(|id| !id.is_empty())
        .or_else(|| ids.iter().find(|id| *id == "openehr").cloned())
        .or_else(|| ids.first().cloned())
        .unwrap_or_default()
}

/// The page of the terminology `id`.
#[cfg(not(target_arch = "wasm32"))]
fn terminology_href(id: &str) -> String {
    format!("/terminology/{}", urlencoding::encode(id))
}

/// One served terminology, as its row reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Served {
    /// Its id.
    id: String,
    /// Its name, the id for none.
    name: String,
    /// Its publisher.
    publisher: String,
    /// Its versions, joined.
    version: String,
    /// Its page.
    href: String,
}

/// Reads the terminologies the CDR serves, each with its descriptor.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session, and the
/// refusal of the terminology list.
#[server(client = SessionAwareClient)]
pub async fn load_terminologies() -> Result<Option<Tabbed<Vec<Served>>>, ViewerError> {
    tabbed(|ids| async move {
        let mut rows = Vec::new();
        for id in &ids {
            let descriptor = fetch_terminology_description(id.clone()).await;
            let (name, publisher, version) = match &descriptor {
                Ok(Some(descriptor)) => (
                    if descriptor.name.is_empty() {
                        id.clone()
                    } else {
                        descriptor.name.clone()
                    },
                    descriptor.publisher.clone(),
                    descriptor.available_versions.join(", "),
                ),
                _ => (id.clone(), String::new(), String::new()),
            };
            rows.push(Served {
                id: id.clone(),
                name,
                publisher,
                version,
                href: terminology_href(id),
            });
        }
        rows
    })
    .await
}

/// Terminologies: the code sets the CDR serves, each with its descriptor.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn TerminologiesTab() -> impl IntoView {
    let model = OnceResource::new(load_terminologies());
    let page = awaited(async move {
        term_page(TermTab::Terminologies, model.await, |ids, served| {
            let columns = [
                Column::text("terminology", tr(Msg::TermTerminology)).role(Role::Title),
                Column::text("publisher", tr(Msg::TermPublisher)).role(Role::Sub),
                Column::text("version", tr(Msg::ColumnVersion)).role(Role::Meta),
            ];
            let rows = served
                .into_iter()
                .map(|row| {
                    let sub = view! {
                        <span class="fui-row-sub">
                            <code>{row.id.clone()}</code>
                        </span>
                    }
                    .into_any();
                    view! {
                        <tr>
                            {title_cell(Some(&row.href), row.name, Some(sub))}
                            {cell(&columns[1], row.publisher)} {cell(&columns[2], row.version)}
                        </tr>
                    }
                    .into_any()
                })
                .collect_view();
            let caption = tr(Msg::TermTerminologies);
            let table = data_table(DataTable {
                caption: &caption,
                columns: &columns,
                sorting: Sorting::default(),
                body: Body::Rows(rows.into_any()),
            });
            panel(section(
                Some(tr(Msg::TermServed)),
                u64::try_from(ids.len()).ok(),
                None,
                table,
            ))
        })
    });
    view! {
        <Title text=tr(Msg::NavTerminology) />
        {page}
    }
}

/// Reads the descriptor of the terminology `id`; `None` when the CDR has the
/// terminology API off.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session, and the CDR's
/// refusal of the descriptor, a `404` setting the page's status.
#[server(client = SessionAwareClient)]
pub async fn load_terminology(
    /// The terminology's id.
    id: String,
) -> Result<Option<TerminologyDescriptor>, ViewerError> {
    crate::session::require_session().await?;
    let descriptor = fetch_terminology_description(id).await;
    if let Err(error) = &descriptor
        && crate::pages::common::is_status(error, http::StatusCode::NOT_FOUND)
    {
        crate::request::set_status(http::StatusCode::NOT_FOUND);
    }
    descriptor
}

/// One terminology: its descriptor.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn TerminologyPage() -> impl IntoView {
    let id = crate::routes::params()
        .with(|params| params.get("id"))
        .unwrap_or_default();
    let model = OnceResource::new(load_terminology(id.clone()));
    let page = awaited(async move {
        let descriptor = model.await;
        let name = match &descriptor {
            Ok(Some(descriptor)) if !descriptor.name.is_empty() => descriptor.name.clone(),
            _ => id.clone(),
        };
        let heading = page_heading(
            Header {
                title: name.clone().into(),
                subtitle: Some(id.clone().into()),
                crumbs: vec![Crumb::new(tr(Msg::NavTerminology), "/terminology")],
                mono: false,
                badge: None,
                subtitle_view: None,
            },
            ActionGroup::none(),
        );
        let body = match descriptor {
            Ok(Some(descriptor)) => {
                let facts = definitions(vec![
                    (
                        tr(Msg::TermId),
                        view! { <code>{descriptor.terminology_id.clone()}</code> }.into_any(),
                    ),
                    (
                        tr(Msg::TermPublisher),
                        descriptor.publisher.clone().into_any(),
                    ),
                    (
                        tr(Msg::TermUri),
                        view! { <code>{descriptor.uri.clone()}</code> }.into_any(),
                    ),
                    (
                        tr(Msg::ColumnVersion),
                        descriptor.available_versions.join(", ").into_any(),
                    ),
                    (
                        tr(Msg::TermFields),
                        descriptor
                            .attributes
                            .iter()
                            .map(|attribute| view! { <code>{attribute.clone()}</code> " " })
                            .collect_view()
                            .into_any(),
                    ),
                ]);
                let lookup = format!(
                    "/terminology/lookup?terminology={}",
                    urlencoding::encode(&id)
                );
                let expand = (id == "openehr").then(|| {
                    ferrohealth_ui::view::button::link_button(
                        format!(
                            "/terminology/valuesets?terminology={}",
                            urlencoding::encode(&id)
                        ),
                        ferrohealth_ui::view::button::Variant::Secondary,
                        Some(Glyph::LIST_CHECKS),
                        tr(Msg::TermExpandGroup),
                    )
                });
                card_panel(view! {
                    {section(Some(tr(Msg::TermDescriptor)), None, None, facts)}
                    {plain_section(
                        view! {
                            <div class="fui-cluster">
                                {ferrohealth_ui::view::button::link_button(
                                    lookup,
                                    ferrohealth_ui::view::button::Variant::Secondary,
                                    Some(Glyph::SEARCH),
                                    tr(Msg::TermLookupIn),
                                )} {expand}
                            </div>
                        },
                    )}
                })
            }
            Ok(None) => off(),
            Err(error) => refused(tr(Msg::TermRefused), &error, None),
        };
        view! {
            <Title text=name />
            {heading}
            <div class="fui-stack">{body}</div>
        }
        .into_any()
    });
    view! { {page} }
}

/// What Look up read: the terminology the form names and, for a code
/// typed, the code's first term, `None` for an undefined code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lookup {
    /// The terminology looked in.
    terminology: String,
    /// The CDR's answer, when a code was typed.
    term: Option<Result<Option<TermRow>, ViewerError>>,
}

/// Reads Look up's answer for `code` in the terminology `asked`, at `at`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session, and the
/// refusal of the terminology list.
#[server(client = SessionAwareClient)]
pub async fn load_lookup(
    /// The terminology the address names.
    asked: String,
    /// The code typed.
    code: String,
    /// The effective date typed.
    at: String,
) -> Result<Option<Tabbed<Lookup>>, ViewerError> {
    tabbed(|ids| async move {
        let terminology = chosen(&ids, asked);
        let term = if code.trim().is_empty() {
            None
        } else {
            Some(
                fetch_term(terminology.clone(), code, at)
                    .await
                    .map(|extract| extract.and_then(|extract| extract.terms.into_iter().next())),
            )
        };
        Lookup { terminology, term }
    })
    .await
}

/// Look up a code.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn LookupTab() -> impl IntoView {
    let code = crate::request::query("code").unwrap_or_default();
    let at = crate::request::query("at").unwrap_or_default();
    let model = OnceResource::new(load_lookup(
        crate::request::query("terminology").unwrap_or_default(),
        code.clone(),
        at.clone(),
    ));
    let page = awaited(async move {
        term_page(TermTab::Lookup, model.await, |ids, lookup| {
            let Lookup { terminology, term } = lookup;
            let form = view! {
                <form method="get" action="/terminology/lookup" class="fui-transform-options">
                    {terminology_field("l-t", ids, &terminology)}
                    {code_field("l-c", "code", tr(Msg::TermCode), &code)}
                    {crate::pages::common::date_field(
                        "l-d",
                        "at",
                        tr(Msg::TermEffectiveDate),
                        &at,
                        false,
                    )}
                    {run(Glyph::SEARCH, tr(Msg::TermLookUp))}
                </form>
            };
            let shown = term.map(|term| match term {
                Ok(Some(term)) => {
                    let facts = definitions(vec![
                        (tr(Msg::TermLanguage), term.language.clone().into_any()),
                        (
                            tr(Msg::TermPreferred),
                            tr(if term.preferred { Msg::Yes } else { Msg::No }).into_any(),
                        ),
                    ]);
                    view! {
                        {answer(
                            Outcome::Ok,
                            view! {
                                {tr(Msg::TermDefined)}
                                " "
                                {coding(
                                    Coding::new(term.code.clone(), term.text.clone())
                                        .system(terminology.clone()),
                                    Layout::Inline,
                                )}
                            }
                                .into_any(),
                        )}
                        {facts}
                    }
                    .into_any()
                }
                Ok(None) => answer(
                    Outcome::Warn,
                    tr_with(
                        Msg::TermUndefined,
                        &[
                            ("terminology", terminology.clone().into()),
                            ("code", code.clone().into()),
                        ],
                    )
                    .into_any(),
                ),
                Err(error) => refused(tr(Msg::TermLookupRefused), &error, None),
            });
            panel(view! {
                {plain_section(form)}
                {shown.map(plain_section)}
            })
        })
    });
    view! {
        <Title text=tr(Msg::TermLookup) />
        {page}
    }
}

/// What Value sets read: the terminology the form names, the members of the
/// value set typed and the verdict on the code typed against it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValueSets {
    /// The terminology the value set belongs to.
    terminology: String,
    /// The value set's members, `None` for no value set of that id, when one
    /// was typed.
    members: Option<Result<Option<Vec<TermRow>>, ViewerError>>,
    /// Whether the code typed is a member, `None` for no value set of that
    /// id, when a value set and a code were typed.
    verdict: Option<Result<Option<bool>, ViewerError>>,
}

/// Reads Value sets' answers: the value set `set` of the terminology
/// `asked` expanded, and `candidate` validated against it.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session, and the
/// refusal of the terminology list.
#[server(client = SessionAwareClient)]
pub async fn load_value_sets(
    /// The terminology the address names.
    asked: String,
    /// The value set typed.
    set: String,
    /// The code typed to validate.
    candidate: String,
) -> Result<Option<Tabbed<ValueSets>>, ViewerError> {
    tabbed(|ids| async move {
        let terminology = chosen(&ids, asked);
        if set.trim().is_empty() {
            return ValueSets {
                terminology,
                members: None,
                verdict: None,
            };
        }
        let members = fetch_value_set(terminology.clone(), set.clone())
            .await
            .map(|extract| extract.map(|extract| extract.terms));
        let verdict = if candidate.trim().is_empty() {
            None
        } else {
            Some(validate_value_set_code(terminology.clone(), set, candidate, String::new()).await)
        };
        ValueSets {
            terminology,
            members: Some(members),
            verdict,
        }
    })
    .await
}

/// Value sets: expand one, and validate a code against it.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the model it reads and the view it draws"
)]
#[component]
pub fn ValueSetsTab() -> impl IntoView {
    let set = crate::request::query("set").unwrap_or_default();
    let candidate = crate::request::query("candidate").unwrap_or_default();
    let model = OnceResource::new(load_value_sets(
        crate::request::query("terminology").unwrap_or_default(),
        set.clone(),
        candidate.clone(),
    ));
    let page = awaited(async move {
        term_page(TermTab::ValueSets, model.await, |ids, read| {
            let ValueSets {
                terminology,
                members,
                verdict,
            } = read;
            let suggestions = (terminology == "openehr").then(|| {
                view! {
                    <datalist id="v-groups">
                        {OPENEHR_GROUPS
                            .iter()
                            .map(|group| view! { <option value=*group></option> })
                            .collect_view()}
                    </datalist>
                }
            });
            let form = view! {
                <form method="get" action="/terminology/valuesets" class="fui-transform-options">
                    {terminology_field("v-t", ids, &terminology)}
                    {Field::new("v-g", tr(Msg::TermValueSet))
                        .view(
                            view! {
                                <input
                                    id="v-g"
                                    name="set"
                                    type="text"
                                    class="fui-input fui-mono"
                                    list="v-groups"
                                    value=set.clone()
                                />
                            },
                        )}
                    {suggestions}
                    {run(Glyph::LIST_CHECKS, tr(Msg::TermExpand))}
                </form>
            };
            let Some(members) = members else {
                return panel(plain_section(form));
            };
            let members = match members {
                Ok(Some(terms)) => {
                    let total = terms.len();
                    let list = view! {
                        <ul class="fui-code-list">
                            {terms
                                .iter()
                                .map(|term| {
                                    view! {
                                        <li>
                                            {coding(
                                                Coding::new(term.code.clone(), term.text.clone()),
                                                Layout::Inline,
                                            )}
                                        </li>
                                    }
                                })
                                .collect_view()}
                        </ul>
                    };
                    section(
                        Some(tr(Msg::TermMembers)),
                        u64::try_from(total).ok(),
                        None,
                        list,
                    )
                }
                Ok(None) => plain_section(answer(
                    Outcome::Warn,
                    tr_with(Msg::TermNoValueSet, &[("set", set.clone().into())]).into_any(),
                )),
                Err(error) => plain_section(refused(tr(Msg::TermValueSetRefused), &error, None)),
            };
            let validate_form = view! {
                <form method="get" action="/terminology/valuesets" class="fui-transform-options">
                    <input type="hidden" name="terminology" value=terminology.clone() />
                    <input type="hidden" name="set" value=set.clone() />
                    {code_field("v-c", "candidate", tr(Msg::TermCode), &candidate)}
                    <div class="fui-transform-run">
                        <button type="submit" class="fui-button fui-button-secondary">
                            {ferrohealth_ui::icon::icon(Glyph::SHIELD_CHECK, "16")}
                            {tr(Msg::TermValidate)}
                        </button>
                    </div>
                </form>
            };
            let verdict = verdict.map(|verdict| match verdict {
                Ok(Some(true)) => answer(
                    Outcome::Ok,
                    tr_with(
                        Msg::TermMember,
                        &[
                            ("code", candidate.clone().into()),
                            ("set", set.clone().into()),
                        ],
                    )
                    .into_any(),
                ),
                Ok(Some(false)) => answer(
                    Outcome::Warn,
                    tr_with(
                        Msg::TermNotMember,
                        &[
                            ("code", candidate.clone().into()),
                            ("set", set.clone().into()),
                        ],
                    )
                    .into_any(),
                ),
                Ok(None) => answer(
                    Outcome::Warn,
                    tr_with(Msg::TermNoValueSet, &[("set", set.clone().into())]).into_any(),
                ),
                Err(error) => refused(tr(Msg::TermValidateRefused), &error, None),
            });
            panel(view! {
                {plain_section(form)}
                {members}
                {section(
                    Some(tr(Msg::TermValidateTitle)),
                    None,
                    None,
                    view! {
                        {validate_form}
                        {verdict}
                    },
                )}
            })
        })
    });
    view! {
        <Title text=tr(Msg::TermValueSets) />
        {page}
    }
}

/// What Hierarchy read: the terminology the form names and, for two codes
/// typed, whether the first subsumes the second.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subsumption {
    /// The terminology tested in.
    terminology: String,
    /// The CDR's verdict, `None` with the terminology API off, when two codes
    /// were typed.
    verdict: Option<Result<Option<bool>, ViewerError>>,
}

/// Reads whether `broader` subsumes `narrower` in the terminology `asked`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session, and the
/// refusal of the terminology list.
#[server(client = SessionAwareClient)]
pub async fn load_subsumption(
    /// The terminology the address names.
    asked: String,
    /// The broader code typed.
    broader: String,
    /// The narrower code typed.
    narrower: String,
) -> Result<Option<Tabbed<Subsumption>>, ViewerError> {
    tabbed(|ids| async move {
        let terminology = chosen(&ids, asked);
        let verdict = if broader.trim().is_empty() || narrower.trim().is_empty() {
            None
        } else {
            Some(check_subsumption(terminology.clone(), broader, narrower).await)
        };
        Subsumption {
            terminology,
            verdict,
        }
    })
    .await
}

/// Hierarchy: whether one code is broader than another.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn SubsumptionTab() -> impl IntoView {
    let broader = crate::request::query("broader").unwrap_or_default();
    let narrower = crate::request::query("narrower").unwrap_or_default();
    let model = OnceResource::new(load_subsumption(
        crate::request::query("terminology").unwrap_or_default(),
        broader.clone(),
        narrower.clone(),
    ));
    let page = awaited(async move {
        term_page(TermTab::Subsumption, model.await, |ids, read| {
            let Subsumption {
                terminology,
                verdict,
            } = read;
            let form = view! {
                <form method="get" action="/terminology/subsumption" class="fui-transform-options">
                    {terminology_field("s-t", ids, &terminology)}
                    {code_field("s-a", "broader", tr(Msg::TermBroader), &broader)}
                    {code_field("s-b", "narrower", tr(Msg::TermNarrower), &narrower)}
                    {run(Glyph::NETWORK, tr(Msg::TermTest))}
                </form>
            };
            let words = |msg| {
                tr_with(
                    msg,
                    &[
                        ("broader", broader.clone().into()),
                        ("narrower", narrower.clone().into()),
                    ],
                )
                .into_any()
            };
            let verdict = verdict.map(|verdict| match verdict {
                Ok(Some(true)) => answer(Outcome::Ok, words(Msg::TermSubsumes)),
                Ok(Some(false)) => view! {
                    {answer(Outcome::Warn, words(Msg::TermNotSubsumes))}
                    {(terminology == "openehr")
                        .then(|| view! { <p class="fui-small fui-muted">{tr(Msg::TermFlat)}</p> })}
                }
                .into_any(),
                Ok(None) => off(),
                Err(error) => refused(tr(Msg::TermSubsumptionRefused), &error, None),
            });
            panel(view! {
                {plain_section(form)}
                {verdict.map(plain_section)}
            })
        })
    });
    view! {
        <Title text=tr(Msg::TermHierarchy) />
        {page}
    }
}
