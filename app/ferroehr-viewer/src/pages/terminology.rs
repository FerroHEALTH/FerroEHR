// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Terminology: what the CDR can answer about the code sets it serves.
//!
//! Four tabs, Terminologies, Look up, Value sets and Hierarchy, and one
//! terminology's descriptor (FerroUI `design/consoles/ferroehr/`, pages 17.1
//! to 17.7).
//!
//! The operations are the terminology calls the CDR serves under
//! `terminology` ([`crate::terminology`]); a CDR with
//! the terminology API off answers `404`, and the page says so once. Every
//! form is a `GET`, so an answer keeps its address, and each tab reads its
//! answers on the server into one model the page draws. No openEHR spec governs
//! the viewer's pages: our own design.

use ferrohealth_ui::html::Html;
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

use crate::error::ViewerError;
use crate::locale::definitions::DefinitionsMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{flag, h, opt, t};
use crate::pages::common::{card_panel, panel, plain_section, refused, section};
use crate::routes::Drawn;
use crate::terminology::{TermRow, TerminologyDescriptor};
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
            Self::Terminologies => DefinitionsMsg::TermTerminologies,
            Self::Lookup => DefinitionsMsg::TermLookup,
            Self::ValueSets => DefinitionsMsg::TermValueSets,
            Self::Subsumption => DefinitionsMsg::TermHierarchy,
        })
    }
}

/// The heading of Terminology.
fn heading() -> Html {
    page_heading(
        &current(),
        Header {
            title: tr(Msg::NavTerminology).into(),
            subtitle: Some(tr(DefinitionsMsg::TermSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    )
}

/// The note of a CDR whose terminology API is off.
fn off() -> Html {
    card_panel(h(format!(
        r#"<div class="fui-panel-body fui-stack"><p><strong>{}</strong></p><p class="fui-small fui-muted">{}</p></div>"#,
        t(tr(DefinitionsMsg::TermOff)),
        t(tr(DefinitionsMsg::TermOffLine))
    )))
}

/// What a tab of Terminology read: the terminology ids for its forms and the
/// tab's own answer.
#[derive(Debug, Clone)]
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
async fn tabbed<T, F, Fut>(read: F) -> TabModel<T>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: Future<Output = T>,
{
    let Some(ids) = list_terminologies().await? else {
        return Ok(None);
    };
    let read = read(ids.clone()).await;
    Ok(Some(Tabbed { ids, read }))
}

/// A tab of Terminology over what `body` draws of its model; the note alone
/// when the API is off.
fn term_page<T>(tab: TermTab, model: TabModel<T>, body: impl FnOnce(&[String], T) -> Html) -> Html {
    let Tabbed { ids, read } = match model {
        Ok(Some(tabbed)) => tabbed,
        Ok(None) => {
            return heading().then(h(format!(r#"<div class="fui-stack">{}</div>"#, off())));
        }
        Err(error) => {
            return heading().then(refused(tr(DefinitionsMsg::TermRefused), &error, None));
        }
    };
    let tabs = TermTab::ALL
        .into_iter()
        .map(|each| Tab::new(each.key(), each.href(), each.label()))
        .collect();
    let panel = body(&ids, read);
    let binder = binder(
        &current(),
        tr(DefinitionsMsg::TermOperations),
        tabs,
        tab.key(),
        panel,
    );
    heading().then(h(format!(r#"<div class="fui-stack">{binder}</div>"#)))
}

/// The terminology select of a form, `chosen` selected.
fn terminology_field(id: &'static str, ids: &[String], chosen: &str) -> Html {
    let options: Html = ids
        .iter()
        .map(|each| {
            h(format!(
                r#"<option value="{}"{}>{}</option>"#,
                t(each),
                flag("selected", *each == chosen),
                t(each)
            ))
        })
        .collect();
    Field::new(id, tr(DefinitionsMsg::TermTerminology)).view(h(format!(
        r#"<select id="{id}" name="terminology" class="fui-select">{options}</select>"#
    )))
}

/// A code text field of a form.
fn code_field(id: &'static str, name: &'static str, label: String, value: &str) -> Html {
    Field::new(id, label).view(h(format!(
        r#"<input id="{id}" name="{name}" type="text" class="fui-input fui-mono" spellcheck="false" value="{}">"#,
        t(value)
    )))
}

/// The run button of a form.
fn run(glyph: Glyph, words: String) -> Html {
    h(format!(
        r#"<div class="fui-transform-run"><button type="submit" class="fui-button fui-button-primary">{}{}</button></div>"#,
        ferrohealth_ui::icon::icon(glyph, "16"),
        t(words)
    ))
}

/// An answer in the given outcome.
fn answer(outcome: Outcome, title: Html) -> Html {
    load_report(LoadReport {
        outcome,
        arrival: Arrival::AfterAction,
        title,
        counts: Vec::new(),
        action: None,
    })
}

/// The terminology a form names, `asked`, the first the CDR serves for none.
fn chosen(ids: &[String], asked: String) -> String {
    Some(asked)
        .filter(|id| !id.is_empty())
        .or_else(|| ids.iter().find(|id| *id == "openehr").cloned())
        .or_else(|| ids.first().cloned())
        .unwrap_or_default()
}

/// The page of the terminology `id`.
fn terminology_href(id: &str) -> String {
    format!("/terminology/{}", urlencoding::encode(id))
}

/// One served terminology, as its row reads.
#[derive(Debug, Clone)]
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
/// Terminology › Terminologies.
pub async fn terminologies_page() -> Drawn {
    let page = term_page(
        TermTab::Terminologies,
        load_terminologies().await,
        |ids, served| {
            let columns = [
                Column::text("terminology", tr(DefinitionsMsg::TermTerminology)).role(Role::Title),
                Column::text("publisher", tr(DefinitionsMsg::TermPublisher)).role(Role::Sub),
                Column::text("version", tr(Msg::ColumnVersion)).role(Role::Meta),
            ];
            let rows = served
                .into_iter()
                .map(|row| {
                    let sub = h(format!(
                        r#"<span class="fui-row-sub"><code>{}</code></span>"#,
                        t(&row.id)
                    ));
                    h(format!(
                        "<tr>{}{}{}</tr>",
                        title_cell(Some(&row.href), t(&row.name), Some(sub)),
                        cell(&columns[1], t(&row.publisher)),
                        cell(&columns[2], t(&row.version))
                    ))
                })
                .collect();
            let caption = tr(DefinitionsMsg::TermTerminologies);
            let table = data_table(DataTable {
                caption: &caption,
                columns: &columns,
                sorting: Sorting::default(),
                body: Body::Rows(rows),
            });
            panel(section(
                Some(tr(DefinitionsMsg::TermServed)),
                u64::try_from(ids.len()).ok(),
                None,
                table,
            ))
        },
    );
    Drawn::page(tr(Msg::NavTerminology), page)
}

/// Reads the descriptor of the terminology `id`; `None` when the CDR has the
/// terminology API off.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session, and the CDR's
/// refusal of the descriptor, a `404` setting the page's status.
pub async fn load_terminology(
    // The terminology's id.
    id: String,
) -> Result<Option<TerminologyDescriptor>, ViewerError> {
    let descriptor = fetch_terminology_description(id).await;
    if let Err(error) = &descriptor
        && crate::pages::common::is_status(error, http::StatusCode::NOT_FOUND)
    {
        crate::request::set_status(http::StatusCode::NOT_FOUND);
    }
    descriptor
}

/// One terminology: its descriptor.
/// One terminology: its descriptor and the ways to read it.
pub async fn terminology_page() -> Drawn {
    let id = crate::routes::params().get("id").unwrap_or_default();
    let descriptor = load_terminology(id.clone()).await;
    let name = match &descriptor {
        Ok(Some(descriptor)) if !descriptor.name.is_empty() => descriptor.name.clone(),
        _ => id.clone(),
    };
    let heading = page_heading(
        &current(),
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
            let code = |text: &str| h(format!("<code>{}</code>", t(text)));
            let facts = definitions(vec![
                (tr(DefinitionsMsg::TermId), code(&descriptor.terminology_id)),
                (tr(DefinitionsMsg::TermPublisher), t(&descriptor.publisher)),
                (tr(DefinitionsMsg::TermUri), code(&descriptor.uri)),
                (
                    tr(Msg::ColumnVersion),
                    t(descriptor.available_versions.join(", ")),
                ),
                (
                    tr(DefinitionsMsg::TermFields),
                    descriptor
                        .attributes
                        .iter()
                        .map(|attribute| h(format!("<code>{}</code> ", t(attribute))))
                        .collect(),
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
                    tr(DefinitionsMsg::TermExpandGroup),
                )
            });
            card_panel(
                section(Some(tr(DefinitionsMsg::TermDescriptor)), None, None, facts).then(
                    plain_section(h(format!(
                        r#"<div class="fui-cluster">{}{}</div>"#,
                        ferrohealth_ui::view::button::link_button(
                            lookup,
                            ferrohealth_ui::view::button::Variant::Secondary,
                            Some(Glyph::SEARCH),
                            tr(DefinitionsMsg::TermLookupIn),
                        ),
                        opt(expand)
                    ))),
                ),
            )
        }
        Ok(None) => off(),
        Err(error) => refused(tr(DefinitionsMsg::TermRefused), &error, None),
    };
    Drawn::page(
        name,
        heading.then(h(format!(r#"<div class="fui-stack">{body}</div>"#))),
    )
}

/// What Look up read: the terminology the form names and, for a code
/// typed, the code's first term, `None` for an undefined code.
#[derive(Debug, Clone)]
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
pub async fn load_lookup(
    // The terminology the address names.
    asked: String,
    // The code typed.
    code: String,
    // The effective date typed.
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
/// Terminology › Look up.
pub async fn lookup_page() -> Drawn {
    let code = crate::request::query("code").unwrap_or_default();
    let at = crate::request::query("at").unwrap_or_default();
    let model = load_lookup(
        crate::request::query("terminology").unwrap_or_default(),
        code.clone(),
        at.clone(),
    )
    .await;
    let page = term_page(TermTab::Lookup, model, |ids, lookup| {
        let Lookup { terminology, term } = lookup;
        let form = h(format!(
            r#"<form method="get" action="/terminology/lookup" class="fui-transform-options">{}{}{}{}</form>"#,
            terminology_field("l-t", ids, &terminology),
            code_field("l-c", "code", tr(DefinitionsMsg::TermCode), &code),
            crate::pages::common::date_field(
                "l-d",
                "at",
                tr(DefinitionsMsg::TermEffectiveDate),
                &at,
                false,
            ),
            run(Glyph::SEARCH, tr(DefinitionsMsg::TermLookUp))
        ));
        let shown = term.map(|term| match term {
            Ok(Some(term)) => {
                let facts = definitions(vec![
                    (tr(DefinitionsMsg::TermLanguage), t(&term.language)),
                    (
                        tr(DefinitionsMsg::TermPreferred),
                        t(tr(if term.preferred { Msg::Yes } else { Msg::No })),
                    ),
                ]);
                let title = h(format!(
                    "{} {}",
                    t(tr(DefinitionsMsg::TermDefined)),
                    coding(
                        &current(),
                        Coding::new(term.code.clone(), term.text.clone())
                            .system(terminology.clone()),
                        Layout::Inline,
                    )
                ));
                answer(Outcome::Ok, title).then(facts)
            }
            Ok(None) => answer(
                Outcome::Warn,
                t(tr_with(
                    DefinitionsMsg::TermUndefined,
                    &[
                        ("terminology", terminology.clone().into()),
                        ("code", code.clone().into()),
                    ],
                )),
            ),
            Err(error) => refused(tr(DefinitionsMsg::TermLookupRefused), &error, None),
        });
        panel(plain_section(form).then(opt(shown.map(plain_section))))
    });
    Drawn::page(tr(DefinitionsMsg::TermLookup), page)
}

/// What Value sets read: the terminology the form names, the members of the
/// value set typed and the verdict on the code typed against it.
#[derive(Debug, Clone)]
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
pub async fn load_value_sets(
    // The terminology the address names.
    asked: String,
    // The value set typed.
    set: String,
    // The code typed to validate.
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
/// Terminology › Value sets.
#[expect(
    clippy::too_many_lines,
    reason = "one page: the model it reads and the view it draws"
)]
pub async fn value_sets_page() -> Drawn {
    let set = crate::request::query("set").unwrap_or_default();
    let candidate = crate::request::query("candidate").unwrap_or_default();
    let model = load_value_sets(
        crate::request::query("terminology").unwrap_or_default(),
        set.clone(),
        candidate.clone(),
    )
    .await;
    let page = term_page(TermTab::ValueSets, model, |ids, read| {
        let ValueSets {
            terminology,
            members,
            verdict,
        } = read;
        let suggestions = (terminology == "openehr").then(|| {
            let options: Html = OPENEHR_GROUPS
                .iter()
                .map(|group| h(format!(r#"<option value="{}"></option>"#, t(group))))
                .collect();
            h(format!(r#"<datalist id="v-groups">{options}</datalist>"#))
        });
        let form = h(format!(
            r#"<form method="get" action="/terminology/valuesets" class="fui-transform-options">{}{}{}{}</form>"#,
            terminology_field("v-t", ids, &terminology),
            Field::new("v-g", tr(DefinitionsMsg::TermValueSet)).view(h(format!(
                r#"<input id="v-g" name="set" type="text" class="fui-input fui-mono" list="v-groups" value="{}">"#,
                t(&set)
            ))),
            opt(suggestions),
            run(Glyph::LIST_CHECKS, tr(DefinitionsMsg::TermExpand))
        ));
        let Some(members) = members else {
            return panel(plain_section(form));
        };
        let members = match members {
            Ok(Some(terms)) => {
                let total = terms.len();
                let items: Html = terms
                    .iter()
                    .map(|term| {
                        h(format!(
                            "<li>{}</li>",
                            coding(
                                &current(),
                                Coding::new(term.code.clone(), term.text.clone()),
                                Layout::Inline,
                            )
                        ))
                    })
                    .collect();
                section(
                    Some(tr(DefinitionsMsg::TermMembers)),
                    u64::try_from(total).ok(),
                    None,
                    h(format!(r#"<ul class="fui-code-list">{items}</ul>"#)),
                )
            }
            Ok(None) => plain_section(answer(
                Outcome::Warn,
                t(tr_with(
                    DefinitionsMsg::TermNoValueSet,
                    &[("set", set.clone().into())],
                )),
            )),
            Err(error) => plain_section(refused(
                tr(DefinitionsMsg::TermValueSetRefused),
                &error,
                None,
            )),
        };
        let validate_form = h(format!(
            r#"<form method="get" action="/terminology/valuesets" class="fui-transform-options"><input type="hidden" name="terminology" value="{}"><input type="hidden" name="set" value="{}">{}<div class="fui-transform-run"><button type="submit" class="fui-button fui-button-secondary">{}{}</button></div></form>"#,
            t(&terminology),
            t(&set),
            code_field("v-c", "candidate", tr(DefinitionsMsg::TermCode), &candidate),
            ferrohealth_ui::icon::icon(Glyph::SHIELD_CHECK, "16"),
            t(tr(DefinitionsMsg::TermValidate))
        ));
        let said = |msg| {
            t(tr_with(
                msg,
                &[
                    ("code", candidate.clone().into()),
                    ("set", set.clone().into()),
                ],
            ))
        };
        let verdict = verdict.map(|verdict| match verdict {
            Ok(Some(true)) => answer(Outcome::Ok, said(DefinitionsMsg::TermMember)),
            Ok(Some(false)) => answer(Outcome::Warn, said(DefinitionsMsg::TermNotMember)),
            Ok(None) => answer(
                Outcome::Warn,
                t(tr_with(
                    DefinitionsMsg::TermNoValueSet,
                    &[("set", set.clone().into())],
                )),
            ),
            Err(error) => refused(tr(DefinitionsMsg::TermValidateRefused), &error, None),
        });
        panel(Html::join([
            plain_section(form),
            members,
            section(
                Some(tr(DefinitionsMsg::TermValidateTitle)),
                None,
                None,
                validate_form.then(opt(verdict)),
            ),
        ]))
    });
    Drawn::page(tr(DefinitionsMsg::TermValueSets), page)
}

/// What Subsumption reads: the terminology chosen and the verdict asked for.
#[derive(Debug, Clone)]
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
pub async fn load_subsumption(
    // The terminology the address names.
    asked: String,
    // The broader code typed.
    broader: String,
    // The narrower code typed.
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
/// Terminology › Subsumption.
pub async fn subsumption_page() -> Drawn {
    let broader = crate::request::query("broader").unwrap_or_default();
    let narrower = crate::request::query("narrower").unwrap_or_default();
    let model = load_subsumption(
        crate::request::query("terminology").unwrap_or_default(),
        broader.clone(),
        narrower.clone(),
    )
    .await;
    let page = term_page(TermTab::Subsumption, model, |ids, read| {
        let Subsumption {
            terminology,
            verdict,
        } = read;
        let form = h(format!(
            r#"<form method="get" action="/terminology/subsumption" class="fui-transform-options">{}{}{}{}</form>"#,
            terminology_field("s-t", ids, &terminology),
            code_field("s-a", "broader", tr(DefinitionsMsg::TermBroader), &broader),
            code_field(
                "s-b",
                "narrower",
                tr(DefinitionsMsg::TermNarrower),
                &narrower
            ),
            run(Glyph::NETWORK, tr(DefinitionsMsg::TermTest))
        ));
        let words = |msg| {
            t(tr_with(
                msg,
                &[
                    ("broader", broader.clone().into()),
                    ("narrower", narrower.clone().into()),
                ],
            ))
        };
        let verdict = verdict.map(|verdict| match verdict {
            Ok(Some(true)) => answer(Outcome::Ok, words(DefinitionsMsg::TermSubsumes)),
            Ok(Some(false)) => answer(Outcome::Warn, words(DefinitionsMsg::TermNotSubsumes)).then(
                opt((terminology == "openehr").then(|| {
                    h(format!(
                        r#"<p class="fui-small fui-muted">{}</p>"#,
                        t(tr(DefinitionsMsg::TermFlat))
                    ))
                })),
            ),
            Ok(None) => off(),
            Err(error) => refused(tr(DefinitionsMsg::TermSubsumptionRefused), &error, None),
        });
        panel(plain_section(form).then(opt(verdict.map(plain_section))))
    });
    Drawn::page(tr(DefinitionsMsg::TermHierarchy), page)
}
