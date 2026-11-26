// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Query › Cohort: AQL run over the EHRs the deployment's demographic
//! predicates select, as one cohort request ([`crate::bff::cohort`]).
//!
//! The predicates offered are the keys the deployment binds when the session
//! may read its configuration, else every bindable key, which the CDR judges.
//! A deployment that binds none (`404`, or a configuration binding none)
//! offers no cohort. The results draw as any result set, with the cohort's
//! size, the EHRs served and the suppression of a small cell. No openEHR spec
//! governs the viewer: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action};
use ferrohealth_ui::view::diagnostics::FileFindings;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::surface::{Header, definitions, page_heading};
use ferrohealth_ui::view::tabs::binder;

use super::aql_input::{grammar, parameters_json, parameters_of, parameters_view};
use super::{QUERY_FORM, tabs};
use crate::bff::cohort::{CohortMeta, KEYS, Predicate, bound_keys, cohort_json, run_cohort};
use crate::components::{EditorField, EditorLanguage, EditorSetup, code_editor};
use crate::locale::queries::QueriesMsg;
use crate::locale::{Msg, current, tr};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    Refused, info_note, marks_of, panel, plain_section, section, warn_note,
};
use crate::pages::results::Answer;
use crate::routes::Drawn;

/// The route of the tab.
const ROUTE: &str = "/query/cohort";

/// What a run answered.
#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    /// Nothing ran.
    Waiting,
    /// The run, its cohort's facts beside its rows.
    Ran(CohortMeta, Answer),
    /// The CDR refused the run, or nothing could run.
    Stopped(Answer),
    /// The deployment binds no cohort predicate.
    Unbound,
}

/// The tab's model.
#[derive(Debug, Clone, PartialEq)]
struct Cohort {
    /// The AQL.
    aql: String,
    /// The keys offered, each with the value typed.
    predicates: Vec<Predicate>,
    /// Each parameter a run binds.
    rows: Vec<(String, String)>,
    /// The grammar check's findings.
    findings: Option<(String, FileFindings)>,
    /// What the run answered.
    outcome: Outcome,
}

/// The form field of the predicate `key`.
fn field_name(key: &str) -> String {
    format!("cohort-{key}")
}

/// The words of the predicate `key`.
fn key_words(key: &str) -> String {
    tr(match key {
        "city" => QueriesMsg::CohortCity,
        "postcode_area" => QueriesMsg::CohortPostcodeArea,
        "sex" => QueriesMsg::CohortSex,
        "age_band" => QueriesMsg::CohortAgeBand,
        "organisation" => QueriesMsg::CohortOrganisation,
        _ => return key.to_owned(),
    })
}

/// What the page reads.
async fn read_cohort() -> Cohort {
    let posted = crate::posted::posted();
    let text = |key: &str| {
        posted
            .as_ref()
            .map(|posted| posted.text(key))
            .unwrap_or_default()
    };
    let aql = posted
        .as_ref()
        .and_then(|posted| posted.get("aql"))
        .unwrap_or_default();
    let bound = bound_keys().await;
    let keys: Vec<String> = match &bound {
        Ok(Some(bound)) => KEYS
            .iter()
            .filter(|key| bound.iter().any(|each| each == *key))
            .map(|key| (*key).to_owned())
            .collect(),
        _ => KEYS.iter().map(|key| (*key).to_owned()).collect(),
    };
    let predicates: Vec<Predicate> = keys
        .into_iter()
        .map(|key| Predicate {
            value: text(&field_name(&key)),
            name: key,
        })
        .collect();
    let rows = parameters_of(posted.as_ref(), &aql);
    let findings = (!aql.trim().is_empty()).then(|| grammar(&aql)).flatten();
    let chosen: Vec<Predicate> = predicates
        .iter()
        .filter(|predicate| !predicate.value.is_empty())
        .cloned()
        .collect();
    let run = posted
        .as_ref()
        .is_some_and(|posted| posted.intent() == "run");
    let outcome = if predicates.is_empty() {
        Outcome::Unbound
    } else if !run {
        Outcome::Waiting
    } else if aql.trim().is_empty() {
        Outcome::Stopped(Answer::NotRun(tr(QueriesMsg::QueryNothingToRun)))
    } else if findings.is_some() {
        Outcome::Stopped(Answer::NotRun(tr(QueriesMsg::QueryGrammarStops)))
    } else if chosen.is_empty() {
        Outcome::Stopped(Answer::NotRun(tr(QueriesMsg::CohortNoPredicate)))
    } else {
        let parameters = parameters_json(&rows);
        let cohort = cohort_json(&chosen);
        match run_cohort(&aql, &parameters, &chosen).await {
            Ok(Some((page, meta))) => Outcome::Ran(
                meta,
                Answer::of_cohort(Ok(page), &aql, &parameters, &cohort),
            ),
            Ok(None) => Outcome::Unbound,
            Err(error) => Outcome::Stopped(Answer::Refused(Refused::new(
                tr(QueriesMsg::QueryRunRefused),
                error,
            ))),
        }
    };
    Cohort {
        aql,
        predicates,
        rows,
        findings,
        outcome,
    }
}

/// The predicate fields.
fn predicate_fields(predicates: &[Predicate]) -> Html {
    let fields = predicates.iter().map(|predicate| {
        let name = field_name(&predicate.name);
        Field::new(&name, key_words(&predicate.name))
            .holds_id()
            .view(h(format!(
                r#"<input id="{name}" name="{name}" form="{QUERY_FORM}" type="text" class="fui-input" value="{}" autocomplete="off">"#,
                t(&predicate.value)
            )))
    });
    h(format!(
        r#"<div class="fui-transform-options">{}</div>"#,
        Html::join(fields)
    ))
}

/// The cohort's facts.
fn facts(meta: &CohortMeta) -> Html {
    let suppressed = meta
        .suppressed
        .then(|| warn_note(t(tr(QueriesMsg::CohortSuppressed))));
    section(
        Some(tr(QueriesMsg::CohortFacts)),
        None,
        None,
        opt(suppressed).then(definitions(vec![
            (tr(QueriesMsg::CohortSize), t(meta.size.to_string())),
            (
                tr(QueriesMsg::CohortServed),
                t(meta.served_ehrs.to_string()),
            ),
            (
                tr(QueriesMsg::CohortDefinition),
                h(format!("<code>{}</code>", t(&meta.definition))),
            ),
        ])),
    )
}

/// The tab, drawn.
fn cohort(model: Cohort) -> Html {
    let Cohort {
        aql,
        predicates,
        rows,
        findings,
        outcome,
    } = model;
    let heading = page_heading(
        &current(),
        Header {
            title: tr(Msg::NavQuery).into(),
            subtitle: Some(tr(QueriesMsg::QuerySubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        if matches!(outcome, Outcome::Unbound) {
            ActionGroup::default()
        } else {
            ActionGroup::primary(Action::new(
                Some(Glyph::PLAY),
                tr(QueriesMsg::QueryRun),
                Act::Submit(QUERY_FORM.to_owned()),
            ))
        },
    );
    let body = match outcome {
        Outcome::Unbound => panel(plain_section(info_note(t(tr(QueriesMsg::CohortUnbound))))),
        outcome => {
            let editor = code_editor(EditorSetup {
                field: EditorField::Aql,
                language: EditorLanguage::Aql,
                text: aql,
                opened: None,
                marks: marks_of(findings.as_ref()),
                label: tr(QueriesMsg::QueryTheAql),
                lead: tr(QueriesMsg::QueryAql),
                facts: vec![tr(Msg::EditorUtf8)],
                findings,
            });
            let (facts, answer) = match outcome {
                Outcome::Ran(meta, answer) => (Some(facts(&meta)), Some(answer)),
                Outcome::Stopped(answer) => (None, Some(answer)),
                Outcome::Waiting | Outcome::Unbound => (None, None),
            };
            panel(Html::join([
                section(
                    Some(tr(QueriesMsg::CohortPredicates)),
                    None,
                    None,
                    ferrohealth_ui::view::surface::section_intro(tr(QueriesMsg::CohortIntro))
                        .then(predicate_fields(&predicates)),
                ),
                plain_section(editor),
                section(
                    Some(tr(QueriesMsg::QueryParameters)),
                    u64::try_from(rows.len()).ok(),
                    None,
                    parameters_view(&rows),
                ),
                opt(facts),
                Answer::view(answer),
            ]))
        }
    };
    let tabs = binder(
        &current(),
        tr(QueriesMsg::QueryWays),
        tabs(),
        "cohort",
        body,
    );
    let form = h(format!(
        r#"<form id="{QUERY_FORM}" method="post" action="{ROUTE}"><input type="hidden" name="default-intent" value="run"><button type="submit" class="fui-sr-only" tabindex="-1">{}</button>{tabs}</form>"#,
        t(tr(QueriesMsg::QueryRun))
    ));
    heading.then(h(format!(r#"<div class="fui-stack">{form}</div>"#)))
}

/// Query › Cohort.
pub async fn page() -> Drawn {
    Drawn::page(tr(QueriesMsg::CohortTitle), cohort(read_cohort().await))
}
