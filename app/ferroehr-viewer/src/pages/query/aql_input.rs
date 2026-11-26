// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The AQL a page runs, as its form holds it.
//!
//! The grammar check is placed on the editor's lines, and the parameters a
//! run binds ride the form. The AQL tab and the cohort tab share them. No
//! openEHR spec governs the viewer: our own design.

use std::collections::BTreeMap;

use ferrohealth_ui::html::Html;
use ferrohealth_ui::view::diagnostics::{FileFindings, Finding};
use ferrohealth_ui::view::editor::caret_at;
use ferrohealth_ui::view::repeat::{Fields, ROW, Repeat, RepeatAdd, RepeatRow, RowEnd, repeat};

use crate::components::EditorField;
use crate::locale::queries::QueriesMsg;
use crate::locale::{Msg, tr, tr_with};
use crate::markup::{h, t};
use crate::pages::common::count;

/// The grammar check of `aql`: its findings, placed on their lines, or
/// `None` when it parses.
#[must_use]
pub fn grammar(aql: &str) -> Option<(String, FileFindings)> {
    use ferrohealth_ui::view::code::Gravity;
    use ferrohealth_ui::view::diagnostics::Position;
    let at = |offset: usize| -> Option<Position> {
        let caret = caret_at(aql, u32::try_from(offset).ok()?);
        Some(Position {
            line: caret.line,
            column: caret.column,
        })
    };
    let findings: Vec<Finding> = match openehr_query::parser::parse_str(aql) {
        Ok(_) => return None,
        Err(openehr_query::parser::ParseError::Lex(error)) => vec![Finding {
            gravity: Gravity::Error,
            code: "AQL-LEX".to_owned(),
            at: at(error.span.start),
            message: tr_with(
                QueriesMsg::QueryUnexpected,
                &[("token", error.slice.clone().into())],
            ),
            mapping: None,
        }],
        Err(openehr_query::parser::ParseError::Syntax { faults }) => faults
            .iter()
            .map(|fault| Finding {
                gravity: Gravity::Error,
                code: "AQL-SYNTAX".to_owned(),
                at: fault.bytes.as_ref().and_then(|bytes| at(bytes.start)),
                message: match &fault.found {
                    Some(token) => tr_with(
                        QueriesMsg::QueryUnexpected,
                        &[("token", format!("{token:?}").into())],
                    ),
                    None => tr(QueriesMsg::QueryEndedEarly),
                },
                mapping: None,
            })
            .collect(),
        Err(openehr_query::parser::ParseError::TooDeep {
            limit, at: fault, ..
        }) => vec![Finding {
            gravity: Gravity::Error,
            code: "AQL-DEPTH".to_owned(),
            at: fault.bytes.as_ref().and_then(|bytes| at(bytes.start)),
            message: tr_with(QueriesMsg::QueryNestsTooDeep, &[("limit", count(limit))]),
            mapping: None,
        }],
    };
    let total = findings.len();
    Some((
        tr_with(QueriesMsg::QueryGrammarHeading, &[("count", count(total))]),
        FileFindings {
            file: EditorField::Aql.name().into_owned(),
            href: None,
            lines: format!("#{}-L", EditorField::Aql.name()),
            current: None,
            findings,
            all: None,
        },
    ))
}

/// The parameters a run binds, as the form holds them: each name and value.
#[must_use]
pub fn parameters_of(posted: Option<&crate::posted::Posted>, aql: &str) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Some(posted) = posted {
        for place in 0.. {
            let Some(name) = posted.get(&format!("param-{place}-name")) else {
                break;
            };
            rows.push((
                name.trim().to_owned(),
                posted.text(&format!("param-{place}-value")),
            ));
        }
        let intent = posted.intent();
        if let Some(place) = intent
            .strip_prefix("remove-param:")
            .and_then(|place| place.parse::<usize>().ok())
            && place < rows.len()
        {
            drop(rows.remove(place));
        }
        if intent == "add-param" {
            rows.push((String::new(), String::new()));
        }
    }
    for name in crate::aql_text::placeholders(aql) {
        if rows.iter().all(|(known, _)| *known != name) {
            rows.push((name, String::new()));
        }
    }
    rows
}

/// The parameters repeater.
#[must_use]
pub fn parameters_view(rows: &[(String, String)]) -> Html {
    let rows = rows
        .iter()
        .enumerate()
        .map(|(place, (name, value))| {
            let name_id = format!("param-{place}-name");
            let value_id = format!("param-{place}-value");
            RepeatRow {
                key: place.to_string(),
                heading: None,
                fields: h(format!(
                    r#"<div class="fui-field"><label class="fui-sr-only" for="{name_id}">{name_label}</label><input id="{name_id}" name="{name_id}" type="text" class="fui-input fui-mono" value="{name}"></div><div class="fui-field"><label class="fui-sr-only" for="{value_id}">{value_label}</label><input id="{value_id}" name="{value_id}" type="text" class="fui-input fui-mono" spellcheck="false" value="{value}"></div>"#,
                    name_label = t(tr_with(
                        QueriesMsg::QueryParamNameN,
                        &[("number", count(place.saturating_add(1)))],
                    )),
                    value_label = t(tr_with(
                        QueriesMsg::QueryParamValueN,
                        &[("number", count(place.saturating_add(1)))],
                    )),
                    name = t(name),
                    value = t(value),
                )),
                end: RowEnd::Remove {
                    label: tr_with(
                        QueriesMsg::QueryRemoveParam,
                        &[("name", name.clone().into())],
                    ),
                    name: "intent".to_owned(),
                    value: format!("remove-param:{place}"),
                },
                after: None,
            }
        })
        .collect();
    repeat(Repeat {
        id: "parameters".to_owned(),
        label: tr(QueriesMsg::QueryParameters),
        fields: Fields::Two,
        head: vec![tr(Msg::QueryName), tr(Msg::TagsValue)],
        rows,
        token: ROW,
        blank: None,
        add: Some(RepeatAdd {
            label: tr(QueriesMsg::QueryAddParam),
            name: "intent".to_owned(),
            value: "add-param".to_owned(),
        }),
        empty: tr(QueriesMsg::QueryNoParams),
        form: None,
    })
}

/// The parameters as the JSON object a run sends.
#[must_use]
pub fn parameters_json(rows: &[(String, String)]) -> String {
    let bindings: BTreeMap<String, String> = rows
        .iter()
        .filter(|(name, _)| !name.is_empty())
        .cloned()
        .collect();
    crate::aql_text::parameters_json(&bindings).to_string()
}
