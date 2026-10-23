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
//! form is a `GET`, so an answer keeps its address. No openEHR spec governs
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
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;

use crate::error::ViewerError;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{awaited, card_panel, panel, plain_section, refused, section};
use crate::terminology::{
    TerminologyDescriptor, check_subsumption, fetch_term, fetch_terminology_description,
    fetch_value_set, list_terminologies, validate_value_set_code,
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

/// A tab of Terminology over `body`, with the terminology ids for its forms;
/// the note alone when the API is off.
async fn term_page<F, Fut>(tab: TermTab, body: F) -> AnyView
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: Future<Output = AnyView>,
{
    let ids = match list_terminologies().await {
        Ok(Some(ids)) => ids,
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
    let panel = body(ids).await;
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

/// The terminology a form names, the first the CDR serves for none.
fn chosen(ids: &[String]) -> String {
    crate::request::query("terminology")
        .filter(|id| !id.is_empty())
        .or_else(|| ids.iter().find(|id| *id == "openehr").cloned())
        .or_else(|| ids.first().cloned())
        .unwrap_or_default()
}

/// The page of the terminology `id`.
fn terminology_href(id: &str) -> String {
    format!("/terminology/{}", urlencoding::encode(id))
}

/// Terminologies: the code sets the CDR serves, each with its descriptor.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn TerminologiesTab() -> impl IntoView {
    let page = awaited(term_page(TermTab::Terminologies, |ids| async move {
        let columns = [
            Column::text("terminology", tr(Msg::TermTerminology)).role(Role::Title),
            Column::text("publisher", tr(Msg::TermPublisher)).role(Role::Sub),
            Column::text("version", tr(Msg::ColumnVersion)).role(Role::Meta),
        ];
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
            let href = terminology_href(id);
            let sub = view! {
                <span class="fui-row-sub">
                    <code>{id.clone()}</code>
                </span>
            }
            .into_any();
            rows.push(
                view! {
                    <tr>
                        {title_cell(Some(&href), name, Some(sub))} {cell(&columns[1], publisher)}
                        {cell(&columns[2], version)}
                    </tr>
                }
                .into_any(),
            );
        }
        let caption = tr(Msg::TermTerminologies);
        let table = data_table(DataTable {
            caption: &caption,
            columns: &columns,
            sorting: Sorting::default(),
            body: Body::Rows(rows.collect_view().into_any()),
        });
        panel(section(
            Some(tr(Msg::TermServed)),
            u64::try_from(ids.len()).ok(),
            None,
            table,
        ))
    }));
    view! {
        <Title text=tr(Msg::NavTerminology) />
        {page}
    }
}

/// One terminology: its descriptor.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn TerminologyPage() -> impl IntoView {
    let id = use_params_map()
        .with_untracked(|params| params.get("id"))
        .unwrap_or_default();
    let page = awaited(async move {
        let descriptor: Result<Option<TerminologyDescriptor>, ViewerError> =
            fetch_terminology_description(id.clone()).await;
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
            Err(error) => {
                if crate::pages::common::is_status(&error, http::StatusCode::NOT_FOUND) {
                    crate::request::set_status(http::StatusCode::NOT_FOUND);
                }
                refused(tr(Msg::TermRefused), &error, None)
            }
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

/// Look up a code.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn LookupTab() -> impl IntoView {
    let page = awaited(term_page(TermTab::Lookup, |ids| async move {
        let terminology = chosen(&ids);
        let code = crate::request::query("code").unwrap_or_default();
        let at = crate::request::query("at").unwrap_or_default();
        let form = view! {
            <form method="get" action="/terminology/lookup" class="fui-transform-options">
                {terminology_field("l-t", &ids, &terminology)}
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
        let shown = if code.trim().is_empty() {
            None
        } else {
            Some(
                match fetch_term(terminology.clone(), code.clone(), at.clone()).await {
                    Ok(Some(extract)) => match extract.terms.first() {
                        Some(term) => {
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
                        None => answer(
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
                    },
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
                },
            )
        };
        panel(view! {
            {plain_section(form)}
            {shown.map(plain_section)}
        })
    }));
    view! {
        <Title text=tr(Msg::TermLookup) />
        {page}
    }
}

/// Value sets: expand one, and validate a code against it.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn ValueSetsTab() -> impl IntoView {
    let page = awaited(term_page(TermTab::ValueSets, |ids| async move {
        let terminology = chosen(&ids);
        let set = crate::request::query("set").unwrap_or_default();
        let candidate = crate::request::query("candidate").unwrap_or_default();
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
                {terminology_field("v-t", &ids, &terminology)}
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
        if set.trim().is_empty() {
            return panel(plain_section(form));
        }
        let members = match fetch_value_set(terminology.clone(), set.clone()).await {
            Ok(Some(extract)) => {
                let total = extract.terms.len();
                let list = view! {
                    <ul class="fui-code-list">
                        {extract
                            .terms
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
        let verdict = if candidate.trim().is_empty() {
            None
        } else {
            Some(
                match validate_value_set_code(
                    terminology.clone(),
                    set.clone(),
                    candidate.clone(),
                    String::new(),
                )
                .await
                {
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
                },
            )
        };
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
    }));
    view! {
        <Title text=tr(Msg::TermValueSets) />
        {page}
    }
}

/// Hierarchy: whether one code is broader than another.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn SubsumptionTab() -> impl IntoView {
    let page = awaited(term_page(TermTab::Subsumption, |ids| async move {
        let terminology = chosen(&ids);
        let broader = crate::request::query("broader").unwrap_or_default();
        let narrower = crate::request::query("narrower").unwrap_or_default();
        let form = view! {
            <form method="get" action="/terminology/subsumption" class="fui-transform-options">
                {terminology_field("s-t", &ids, &terminology)}
                {code_field("s-a", "broader", tr(Msg::TermBroader), &broader)}
                {code_field("s-b", "narrower", tr(Msg::TermNarrower), &narrower)}
                {run(Glyph::NETWORK, tr(Msg::TermTest))}
            </form>
        };
        let verdict = if broader.trim().is_empty() || narrower.trim().is_empty() {
            None
        } else {
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
            Some(match check_subsumption(terminology.clone(), broader.clone(), narrower.clone()).await {
                Ok(Some(true)) => answer(Outcome::Ok, words(Msg::TermSubsumes)),
                Ok(Some(false)) => view! {
                    {answer(Outcome::Warn, words(Msg::TermNotSubsumes))}
                    {(terminology == "openehr")
                        .then(|| view! { <p class="fui-small fui-muted">{tr(Msg::TermFlat)}</p> })}
                }
                .into_any(),
                Ok(None) => off(),
                Err(error) => refused(tr(Msg::TermSubsumptionRefused), &error, None),
            })
        };
        panel(view! {
            {plain_section(form)}
            {verdict.map(plain_section)}
        })
    }));
    view! {
        <Title text=tr(Msg::TermHierarchy) />
        {page}
    }
}
