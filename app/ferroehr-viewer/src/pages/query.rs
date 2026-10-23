// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Query: build AQL from a template, or write it, run it against the CDR and
//! save it as a stored query (FerroUI `design/consoles/ferroehr/`, pages 18.8
//! to 18.18).
//!
//! Build keeps the whole query in its form: the template, what it returns,
//! the data points (a repeater), the condition groups (E6) and the order and
//! limit. Every control posts as it stands and every add or remove is a
//! submit naming what it acts on, so the builder works with no WebAssembly;
//! the server reads the form, applies the step, and draws the page again
//! with the AQL the builder lowers to ([`crate::builder::lower`]). The AQL
//! tab is the code editor (B2) with the grammar check beside it and the
//! parameters a run binds. Open in AQL and Open in Build carry the query
//! across ([`crate::builder::lift`]). No openEHR spec governs the viewer's
//! pages: our own design; the queries are openEHR AQL.

use std::collections::BTreeMap;

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::button::Variant;
use ferrohealth_ui::view::code::{Code, CodeHead, code_section};
use ferrohealth_ui::view::conditions::{
    ADD_CONDITION, ADD_GROUP, Choice, Condition, ConditionGroup, Match, Operand, REMOVE,
    REMOVE_GROUP, condition_groups,
};
use ferrohealth_ui::view::diagnostics::{DiagnosticsWords, FileFindings, Finding};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::popover::{PopoverForm, popover_form};
use ferrohealth_ui::view::surface::{Header, page_heading};
use ferrohealth_ui::view::tabs::{Tab, binder};
use ferrohealth_ui::widget::action_menu::{Act, Action};
use ferrohealth_ui::widget::editor::{EditorWords, caret_at};
use ferrohealth_ui::widget::repeat::{Fields, Repeat, RepeatAdd, RepeatRow, RowEnd, repeat};
use leptos::prelude::*;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

use crate::builder::catalog::CatalogNode;
use crate::builder::model::{
    BoolOp, BuilderQuery, Criterion, CriterionKind, CriterionNode, OrderRule, QueryShape,
    SelectedColumn,
};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::islands::{CodeEditor, EditorField, EditorLanguage, EditorSetup};
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{
    awaited, count, info_note, marks_of, panel, plain_section, refused, section, warn_note,
};
use crate::pages::results::{Answered, chart_asked, not_run, results, shown_choice};

/// The id of the query's form.
pub const QUERY_FORM: &str = "query-form";

/// The id of Save to the CDR's popover.
const SAVE_POPOVER: &str = "save-query";

/// What a post asks the query page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Draw the page again with the form's changes.
    Edit,
    /// Run the query.
    Run,
    /// Save it to the CDR.
    Save,
}

/// The step a post asks: an add or remove is an edit whatever the form's
/// default intent says.
fn step(posted: Option<&crate::posted::Posted>) -> Step {
    let Some(posted) = posted else {
        return Step::Edit;
    };
    let structural = [REMOVE, REMOVE_GROUP, ADD_CONDITION, ADD_GROUP]
        .into_iter()
        .any(|name| posted.get(name).is_some());
    if structural {
        return Step::Edit;
    }
    match posted.intent().as_str() {
        "run" => Step::Run,
        "save" => Step::Save,
        _ => Step::Edit,
    }
}

/// The page heading of Query with Run, which submits the form, and Save to
/// the CDR beside it, whose fields belong to the form too.
// NOTE: FerroUI `view::action::ActionGroup` holds actions only, so the Save popover sits in its own row under the heading.
fn heading(save: AnyView) -> AnyView {
    let heading = page_heading(
        Header {
            title: tr(Msg::NavQuery).into(),
            subtitle: Some(tr(Msg::QuerySubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::PLAY),
            tr(Msg::QueryRun),
            Act::Submit(QUERY_FORM.to_owned()),
        )),
    );
    view! {
        {heading}
        <div class="fui-cluster fui-page-actions">{save}</div>
    }
    .into_any()
}

/// Save to the CDR: the namespace, the name and the version, fields of the
/// query's form, and the submit that saves. A stored query opened from the
/// CDR (`loaded`, its qualified name and version) fills them with its
/// namespace, its bare name and the next minor version, since its own
/// version is immutable on the CDR.
fn save_popover(
    posted: Option<&crate::posted::Posted>,
    problem: Option<String>,
    loaded: Option<(&str, &str)>,
) -> AnyView {
    let seed = loaded.map(|(name, version)| {
        let (namespace, bare) = crate::query_namespace::split_qualified(name);
        let next = crate::query_namespace::next_minor(version).unwrap_or_default();
        (namespace, bare, next)
    });
    let field = |name: &str| match (posted, &seed) {
        (Some(posted), _) => posted.text(name),
        (None, Some((namespace, bare, next))) => match name {
            "save_namespace" => namespace.clone(),
            "save_name" => bare.clone(),
            "save_version" => next.clone(),
            _ => String::new(),
        },
        (None, None) => String::new(),
    };
    let error = problem.map(|problem| view! { <p class="fui-field-error">{problem}</p> });
    popover_form(PopoverForm {
        id: SAVE_POPOVER,
        variant: Variant::Secondary,
        glyph: Glyph::SAVE,
        label: tr(Msg::QuerySave).into(),
        title: tr(Msg::QuerySave).into(),
        body: view! {
            {Field::new("sv-ns", tr(Msg::QueryNamespace))
                .view(view! { <input id="sv-ns" name="save_namespace" form=QUERY_FORM type="text" class="fui-input fui-mono" value=field("save_namespace") /> })}
            {Field::new("sv-n", tr(Msg::QueryName))
                .view(view! { <input id="sv-n" name="save_name" form=QUERY_FORM type="text" class="fui-input fui-mono" value=field("save_name") /> })}
            {Field::new("sv-v", tr(Msg::ColumnVersion))
                .help(tr(Msg::QueryVersionHelp))
                .after(error)
                .view(view! { <input id="sv-v" name="save_version" form=QUERY_FORM type="text" class="fui-input fui-mono" value=field("save_version") placeholder="1.0.0" /> })}
            <div class="fui-cluster">
                <button type="submit" form=QUERY_FORM name="intent" value="save" class="fui-button fui-button-primary">
                    {ferrohealth_ui::icon::icon(Glyph::SAVE, "16")}
                    {tr(Msg::QuerySaveAction)}
                </button>
            </div>
        }
        .into_any(),
    })
}

/// The newest version the CDR holds of the stored query `name`, the one it
/// assigned to a save that named none; empty when the listing does not say.
async fn saved_version(name: &str) -> String {
    // NOTE: the save succeeded; a listing that fails only leaves the version out of the toast.
    crate::queries_api::list_stored_queries()
        .await
        .ok()
        .and_then(|rows| {
            rows.into_iter()
                .filter(|row| row.name == name)
                .map(|row| row.version)
                .max_by_key(|version| {
                    version
                        .split('.')
                        .map(|part| part.parse::<u64>().unwrap_or(0))
                        .collect::<Vec<_>>()
                })
        })
        .unwrap_or_default()
}

/// Saves `aql` as the stored query the post names. `Ok(None)` is no save
/// asked; `Err` is what the popover says.
async fn save(
    posted: Option<&crate::posted::Posted>,
    aql: &str,
) -> Result<(), Box<(String, Option<AnyView>)>> {
    let Some(posted) = posted else {
        return Ok(());
    };
    // NOTE: the name is not `required` in the markup: the field belongs to the form every Run and
    // Add posts, so the browser would refuse those too; the save checks it here instead.
    if posted.text("save_name").trim().is_empty() {
        return Err(Box::new((tr(Msg::QuerySaveNameMissing), None)));
    }
    let name =
        crate::query_namespace::qualify(&posted.text("save_namespace"), &posted.text("save_name"));
    let version = Some(posted.text("save_version")).filter(|version| !version.is_empty());
    match crate::queries_api::store_query(name.clone(), version.clone(), aql.to_owned()).await {
        Ok(()) => {
            let shown = match version {
                Some(version) => version,
                None => saved_version(&name).await,
            };
            redirect_with(
                &format!(
                    "/query/aql?name={}&version={}",
                    urlencoding::encode(&name),
                    urlencoding::encode(&shown)
                ),
                &Done::ok(tr(Msg::QuerySaved))
                    .line(format!("{name} {shown}"))
                    .link(
                        tr(Msg::QueryOpenStored),
                        crate::pages::queries::stored_query_href(&name, &shown),
                    ),
            );
            Ok(())
        }
        Err(error) => {
            let problem = if crate::pages::common::is_status(&error, http::StatusCode::CONFLICT) {
                tr(Msg::QueryVersionTaken)
            } else {
                tr(Msg::QuerySaveRefused)
            };
            Err(Box::new((
                problem,
                Some(refused(tr(Msg::QuerySaveRefused), &error, None)),
            )))
        }
    }
}

/// The tabs of Query.
fn tabs() -> Vec<Tab> {
    vec![
        Tab::new("build", "/query", tr(Msg::QueryBuild)),
        Tab::new("aql", "/query/aql", tr(Msg::QueryAql)),
    ]
}

/// One condition as the form holds it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct UiCond {
    /// The data point's path.
    path: String,
    /// The operator's key.
    op: String,
    /// The value, as typed or picked.
    value: String,
}

/// One condition group as the form holds it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct UiGroup {
    /// Any of the conditions, rather than all of them.
    any: bool,
    /// Match the opposite.
    not: bool,
    /// Its conditions.
    conditions: Vec<UiCond>,
    /// Its nested groups.
    groups: Vec<UiGroup>,
}

/// The whole builder as the form holds it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Draft {
    /// The template.
    template: String,
    /// What the query returns.
    shape: String,
    /// The data points: their path and their column name.
    columns: Vec<(String, String)>,
    /// The condition groups.
    root: UiGroup,
    /// The path the rows are ordered by, empty for none.
    order: String,
    /// Newest first.
    descending: bool,
    /// The row limit, as typed.
    limit: String,
}

/// One data point a template offers.
#[derive(Debug, Clone)]
struct Leaf {
    /// Its path.
    path: String,
    /// Its name with its parents', such as "Blood pressure › Systolic".
    label: String,
    /// Its RM type.
    rm_type: String,
    /// The codes it takes, as code and words, or ordinal value and words.
    codes: Vec<(String, String)>,
    /// Its first unit.
    unit: Option<String>,
}

/// Every selectable data point under `node`, named with its parents (the
/// composition itself left out).
fn leaves(node: &CatalogNode, trail: &[String], out: &mut Vec<Leaf>) {
    let mut names = trail.to_vec();
    if node.rm_type != "COMPOSITION" {
        names.push(node.label.clone());
    }
    if node.selectable && !node.aql_path.is_empty() {
        let codes = node
            .code_options
            .iter()
            .map(|option| match option.ordinal {
                Some(ordinal) if node.rm_type == "DV_ORDINAL" => (ordinal.to_string(), option.label.clone()),
                _ => (option.code.clone(), option.label.clone()),
            })
            .collect();
        out.push(Leaf {
            path: node.aql_path.clone(),
            label: names.join(" › "),
            rm_type: node.rm_type.clone(),
            codes,
            unit: node.unit_options.first().cloned(),
        });
    }
    for child in &node.children {
        leaves(child, &names, out);
    }
}

/// The operators a data point of `rm_type` takes.
fn operators(rm_type: &str) -> Vec<(&'static str, &'static str)> {
    let mut ops = match rm_type {
        "DV_QUANTITY" | "DV_COUNT" | "DV_PROPORTION" | "DV_DATE_TIME" | "DV_DATE" | "DV_TIME" => {
            vec![("ge", "≥"), ("le", "≤"), ("eq", "=")]
        }
        "DV_CODED_TEXT" | "DV_ORDINAL" | "DV_BOOLEAN" => vec![("eq", "="), ("ne", "≠")],
        "DV_TEXT" => vec![("eq", "="), ("ne", "≠"), ("like", "like")],
        _ => Vec::new(),
    };
    ops.push(("exists", "exists"));
    ops
}

/// The key of the condition group at `place` under `parent`.
fn group_key(parent: &str, place: usize) -> String {
    format!("{parent}.{place}")
}

/// The key of the condition at `place` in the group `parent`.
fn condition_key(parent: &str, place: usize) -> String {
    format!("{parent}.c{place}")
}

/// The group's choices as the form posted them.
fn read_group(group: &mut UiGroup, key: &str, posted: &crate::posted::Posted) {
    if let Some(matching) = posted.get(&format!("{key}-match")) {
        group.any = matching == "any";
        group.not = posted.checked(&format!("{key}-not"));
    }
    for (place, condition) in group.conditions.iter_mut().enumerate() {
        let ckey = condition_key(key, place);
        if let Some(path) = posted.get(&format!("{ckey}-path")) {
            condition.path = path;
        }
        if let Some(op) = posted.get(&format!("{ckey}-op")) {
            condition.op = op;
        }
        if let Some(value) = posted.get(&format!("{ckey}-value")) {
            value.trim().clone_into(&mut condition.value);
        }
    }
    for (place, inner) in group.groups.iter_mut().enumerate() {
        read_group(inner, &group_key(key, place), posted);
    }
}

/// The group the key names.
fn group_at<'a>(root: &'a mut UiGroup, key: &str) -> Option<&'a mut UiGroup> {
    let mut parts = key.split('.');
    if parts.next() != Some("g") {
        return None;
    }
    let mut group = root;
    for part in parts {
        group = group.groups.get_mut(part.parse::<usize>().ok()?)?;
    }
    Some(group)
}

/// Applies the add or remove the post names to the groups.
fn restructure(root: &mut UiGroup, posted: &crate::posted::Posted, first: &str) {
    if let Some(key) = posted.get(ADD_CONDITION)
        && let Some(group) = group_at(root, &key) {
            group.conditions.push(UiCond {
                path: first.to_owned(),
                ..UiCond::default()
            });
        }
    if let Some(key) = posted.get(ADD_GROUP)
        && let Some(group) = group_at(root, &key) {
            group.groups.push(UiGroup {
                conditions: vec![UiCond {
                    path: first.to_owned(),
                    ..UiCond::default()
                }],
                ..UiGroup::default()
            });
        }
    if let Some(key) = posted.get(REMOVE_GROUP)
        && let Some((parent, place)) = key.rsplit_once('.')
            && let (Some(group), Ok(place)) = (group_at(root, parent), place.parse::<usize>())
                && place < group.groups.len() {
                    drop(group.groups.remove(place));
                }
    if let Some(key) = posted.get(REMOVE)
        && let Some((parent, place)) = key.rsplit_once(".c")
            && let (Some(group), Ok(place)) = (group_at(root, parent), place.parse::<usize>())
                && place < group.conditions.len() {
                    drop(group.conditions.remove(place));
                }
}

/// The builder as the post leaves it: the form's previous state, its fields
/// read over it, and its step applied.
fn draft_of_post(posted: &crate::posted::Posted, offered: &[String]) -> Draft {
    // NOTE: the state is the page's own hidden field; one that does not parse starts the builder empty.
    let mut draft: Draft = posted
        .get("state")
        .and_then(|state| serde_json::from_str(&state).ok())
        .unwrap_or_default();
    let template = posted.text("template");
    if !template.is_empty() && template != draft.template {
        draft = Draft {
            template,
            shape: draft.shape,
            limit: draft.limit,
            ..Draft::default()
        };
        return draft;
    }
    if let Some(shape) = posted.get("shape") {
        draft.shape = shape;
    }
    for (place, column) in draft.columns.iter_mut().enumerate() {
        if let Some(path) = posted.get(&format!("col-{place}-path")) {
            column.0 = path;
        }
        if let Some(alias) = posted.get(&format!("col-{place}-alias")) {
            alias.trim().clone_into(&mut column.1);
        }
    }
    let intent = posted.intent();
    if intent == "add-column" {
        draft
            .columns
            .push((offered.first().cloned().unwrap_or_default(), String::new()));
    }
    if let Some(place) = intent
        .strip_prefix("remove-column:")
        .and_then(|place| place.parse::<usize>().ok())
        && place < draft.columns.len() {
            drop(draft.columns.remove(place));
        }
    read_group(&mut draft.root, "g", posted);
    restructure(
        &mut draft.root,
        posted,
        offered.first().map_or("", String::as_str),
    );
    if let Some(order) = posted.get("order") {
        draft.order = order;
    }
    if posted.get("direction").is_some() {
        draft.descending = posted.text("direction") == "desc";
    }
    if let Some(limit) = posted.get("limit") {
        limit.trim().clone_into(&mut draft.limit);
    }
    draft
}

/// The words of a shape key.
const SHAPES: [(&str, Msg); 4] = [
    ("data", Msg::QueryShapeData),
    ("compositions", Msg::QueryShapeCompositions),
    ("count", Msg::QueryShapeCount),
    ("ehrs", Msg::QueryShapeEhrs),
];

/// The model shape of a shape key.
fn shape_of(key: &str) -> QueryShape {
    match key {
        "compositions" => QueryShape::Compositions,
        "count" => QueryShape::Count,
        "ehrs" => QueryShape::Ehrs,
        _ => QueryShape::DataValues,
    }
}

/// The shape key of a model shape.
const fn shape_key(shape: QueryShape) -> &'static str {
    match shape {
        QueryShape::Compositions => "compositions",
        QueryShape::Count => "count",
        QueryShape::Ehrs => "ehrs",
        QueryShape::DataValues => "data",
    }
}

/// Why the builder cannot lower its draft.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum DraftError {
    /// A condition names no data point the template offers.
    #[error("{0}")]
    Path(String),
    /// A value does not parse as its type takes it.
    #[error("{0}")]
    Value(String),
    /// The limit is not a whole number.
    #[error("{0}")]
    Limit(String),
}

/// The constraint of one condition on a data point of `rm_type`, and whether
/// it is negated.
fn kind_of(
    condition: &UiCond,
    rm_type: &str,
    label: &str,
) -> Result<(CriterionKind, bool), DraftError> {
    let value = condition.value.trim();
    let not_number = || {
        DraftError::Value(tr_with(
            Msg::QueryNotNumber,
            &[
                ("path", label.to_owned().into()),
                ("value", value.to_owned().into()),
            ],
        ))
    };
    // NOTE: the refusal names the field and its value; the parser's own words add nothing a reader can act on.
    let float = || value.parse::<f64>().ok().ok_or_else(not_number);
    let int = || value.parse::<i64>().ok().ok_or_else(not_number);
    let negated = condition.op == "ne";
    if condition.op == "exists" {
        return Ok((CriterionKind::Exists, false));
    }
    let (lower, upper) = match condition.op.as_str() {
        "ge" => (true, false),
        "le" => (false, true),
        _ => (true, true),
    };
    let kind = match rm_type {
        "DV_QUANTITY" => {
            let number = float()?;
            CriterionKind::QuantityRange {
                min: lower.then_some(number),
                max: upper.then_some(number),
                units: String::new(),
            }
        }
        "DV_PROPORTION" => {
            let number = float()?;
            CriterionKind::ProportionNumeratorRange {
                min: lower.then_some(number),
                max: upper.then_some(number),
            }
        }
        "DV_COUNT" => {
            let number = int()?;
            CriterionKind::CountRange {
                min: lower.then_some(number),
                max: upper.then_some(number),
            }
        }
        "DV_DATE_TIME" | "DV_DATE" | "DV_TIME" => {
            CriterionKind::DateTimeRange {
                from: if lower { value.to_owned() } else { String::new() },
                to: if upper { value.to_owned() } else { String::new() },
            }
        }
        "DV_CODED_TEXT" => CriterionKind::CodedIn {
            codes: vec![value.to_owned()],
            terminology: String::new(),
        },
        "DV_ORDINAL" => CriterionKind::OrdinalIn {
            values: vec![int()?],
        },
        "DV_BOOLEAN" => CriterionKind::BooleanIs {
            value: value == "true",
        },
        _ if condition.op == "like" => CriterionKind::TextLike {
            pattern: value.to_owned(),
        },
        _ => CriterionKind::TextEquals {
            text: value.to_owned(),
        },
    };
    Ok((kind, negated))
}

/// The criterion tree of a group, `None` for a group with nothing in it.
fn node_of(group: &UiGroup, offered: &[Leaf]) -> Result<Option<CriterionNode>, DraftError> {
    let mut children = Vec::new();
    for condition in &group.conditions {
        let leaf = offered
            .iter()
            .find(|leaf| leaf.path == condition.path)
            .ok_or_else(|| {
                DraftError::Path(tr_with(
                    Msg::QueryUnknownPath,
                    &[("path", condition.path.clone().into())],
                ))
            })?;
        let (kind, negated) = kind_of(condition, &leaf.rm_type, &leaf.label)?;
        children.push(CriterionNode::Leaf(Criterion {
            aql_path: condition.path.clone(),
            negated,
            kind,
        }));
    }
    for inner in &group.groups {
        if let Some(node) = node_of(inner, offered)? {
            children.push(node);
        }
    }
    if children.is_empty() {
        return Ok(None);
    }
    Ok(Some(CriterionNode::Group {
        op: if group.any { BoolOp::Or } else { BoolOp::And },
        negated: group.not,
        children,
    }))
}

/// The builder query a draft lowers from.
fn query_of(draft: &Draft, offered: &[Leaf]) -> Result<BuilderQuery, DraftError> {
    let shape = shape_of(&draft.shape);
    let limit = if draft.limit.is_empty() {
        None
    } else {
        Some(
            draft
                .limit
                .parse::<u32>()
                .ok()
                .ok_or_else(|| DraftError::Limit(tr(Msg::QueryBadLimit)))?,
        )
    };
    Ok(BuilderQuery {
        template_id: draft.template.clone(),
        shape,
        columns: if shape == QueryShape::DataValues {
            draft
                .columns
                .iter()
                .map(|(path, alias)| SelectedColumn {
                    aql_path: path.clone(),
                    alias: alias.clone(),
                })
                .collect()
        } else {
            Vec::new()
        },
        criteria: node_of(&draft.root, offered)?,
        order_by: if draft.order.is_empty() {
            Vec::new()
        } else {
            vec![OrderRule {
                aql_path: draft.order.clone(),
                descending: draft.descending,
            }]
        },
        limit,
    })
}

/// The form conditions of one lifted criterion: a negated range becomes a
/// group of its own that matches the opposite.
fn conditions_of(criterion: &Criterion, group: &mut UiGroup) {
    let path = criterion.aql_path.clone();
    let one = |op: &str, value: String| UiCond {
        path: path.clone(),
        op: op.to_owned(),
        value,
    };
    let float = |value: f64| value.to_string();
    let mut found: Vec<UiCond> = Vec::new();
    let range = |min: Option<String>, max: Option<String>, found: &mut Vec<UiCond>| match (min, max)
    {
        (Some(min), Some(max)) if min == max => found.push(one("eq", min)),
        (min, max) => {
            if let Some(min) = min {
                found.push(one("ge", min));
            }
            if let Some(max) = max {
                found.push(one("le", max));
            }
        }
    };
    let mut eqable = false;
    match &criterion.kind {
        CriterionKind::QuantityRange { min, max, .. }
        | CriterionKind::ProportionNumeratorRange { min, max } => {
            range(min.map(float), max.map(float), &mut found);
        }
        CriterionKind::CountRange { min, max } => range(
            min.map(|v| v.to_string()),
            max.map(|v| v.to_string()),
            &mut found,
        ),
        CriterionKind::DateTimeRange { from, to } => range(
            Some(from.clone()).filter(|from| !from.is_empty()),
            Some(to.clone()).filter(|to| !to.is_empty()),
            &mut found,
        ),
        CriterionKind::CodedIn { codes, .. } => {
            eqable = codes.len() == 1;
            found.extend(codes.iter().map(|code| one("eq", code.clone())));
        }
        CriterionKind::OrdinalIn { values } => {
            eqable = values.len() == 1;
            found.extend(values.iter().map(|value| one("eq", value.to_string())));
        }
        CriterionKind::TextEquals { text } => {
            eqable = true;
            found.push(one("eq", text.clone()));
        }
        CriterionKind::TextLike { pattern } => found.push(one("like", pattern.clone())),
        CriterionKind::BooleanIs { value } => {
            eqable = true;
            found.push(one("eq", value.to_string()));
        }
        CriterionKind::Exists => found.push(one("exists", String::new())),
    }
    let several_codes = matches!(&criterion.kind, CriterionKind::CodedIn { codes, .. } if codes.len() > 1)
        || matches!(&criterion.kind, CriterionKind::OrdinalIn { values } if values.len() > 1);
    if criterion.negated && eqable {
        for mut condition in found {
            "ne".clone_into(&mut condition.op);
            group.conditions.push(condition);
        }
    } else if criterion.negated || several_codes {
        group.groups.push(UiGroup {
            any: several_codes,
            not: criterion.negated,
            conditions: found,
            groups: Vec::new(),
        });
    } else {
        group.conditions.extend(found);
    }
}

/// The form group of a lifted criterion tree.
fn group_of(node: &CriterionNode) -> UiGroup {
    match node {
        CriterionNode::Leaf(criterion) => {
            let mut group = UiGroup::default();
            conditions_of(criterion, &mut group);
            group
        }
        CriterionNode::Group {
            op,
            negated,
            children,
        } => {
            let mut group = UiGroup {
                any: *op == BoolOp::Or,
                not: *negated,
                ..UiGroup::default()
            };
            for child in children {
                match child {
                    CriterionNode::Leaf(criterion) => conditions_of(criterion, &mut group),
                    inner @ CriterionNode::Group { .. } => group.groups.push(group_of(inner)),
                }
            }
            group
        }
    }
}

/// The draft of a lifted builder query.
fn draft_of_query(query: &BuilderQuery) -> Draft {
    Draft {
        template: query.template_id.clone(),
        shape: shape_key(query.shape).to_owned(),
        columns: query
            .columns
            .iter()
            .map(|column| (column.aql_path.clone(), column.alias.clone()))
            .collect(),
        root: query.criteria.as_ref().map(group_of).unwrap_or_default(),
        order: query
            .order_by
            .first()
            .map(|rule| rule.aql_path.clone())
            .unwrap_or_default(),
        descending: query.order_by.first().is_some_and(|rule| rule.descending),
        limit: query
            .limit
            .map(|limit| limit.to_string())
            .unwrap_or_default(),
    }
}

/// The path choice of a condition or a data point.
fn path_choice(offered: &[Leaf], chosen: &str) -> Choice {
    Choice {
        options: offered
            .iter()
            .map(|leaf| (leaf.path.clone(), leaf.label.clone()))
            .collect(),
        chosen: chosen.to_owned(),
    }
}

/// The FerroUI condition group of a form group.
fn view_group(group: &UiGroup, key: &str, number: u32, offered: &[Leaf]) -> ConditionGroup {
    let conditions = group
        .conditions
        .iter()
        .enumerate()
        .map(|(place, condition)| {
            let leaf = offered.iter().find(|leaf| leaf.path == condition.path);
            let rm_type = leaf.map_or("", |leaf| leaf.rm_type.as_str());
            let ops = operators(rm_type);
            let op = if ops.iter().any(|(key, _)| *key == condition.op) {
                condition.op.clone()
            } else {
                ops.first()
                    .map(|(key, _)| (*key).to_owned())
                    .unwrap_or_default()
            };
            let operand = match (leaf, rm_type) {
                (Some(leaf), "DV_CODED_TEXT" | "DV_ORDINAL") if !leaf.codes.is_empty() => {
                    Operand::Coded(Choice {
                        options: leaf
                            .codes
                            .iter()
                            .map(|(code, words)| (code.clone(), format!("{words} · {code}")))
                            .collect(),
                        chosen: condition.value.clone(),
                    })
                }
                (_, "DV_BOOLEAN") => Operand::Coded(Choice {
                    options: vec![
                        ("true".to_owned(), tr(Msg::Yes)),
                        ("false".to_owned(), tr(Msg::No)),
                    ],
                    chosen: condition.value.clone(),
                }),
                _ => Operand::Typed {
                    value: condition.value.clone(),
                    unit: leaf.and_then(|leaf| leaf.unit.clone()),
                },
            };
            Condition {
                key: condition_key(key, place),
                path: path_choice(offered, &condition.path),
                archetype_path: condition.path.clone(),
                operator: Choice {
                    options: ops
                        .iter()
                        .map(|(key, words)| ((*key).to_owned(), (*words).to_owned()))
                        .collect(),
                    chosen: op,
                },
                operand,
            }
        })
        .collect();
    let groups = group
        .groups
        .iter()
        .enumerate()
        .map(|(place, inner)| {
            let inner_key = group_key(key, place);
            view_group(
                inner,
                &inner_key,
                number
                    .saturating_add(u32::try_from(place).unwrap_or(u32::MAX))
                    .saturating_add(1),
                offered,
            )
        })
        .collect();
    ConditionGroup {
        key: key.to_owned(),
        number,
        matching: if group.any { Match::Any } else { Match::All },
        negated: group.not,
        conditions,
        groups,
    }
}

/// The data points repeater.
fn columns_view(draft: &Draft, offered: &[Leaf]) -> AnyView {
    let rows = draft
        .columns
        .iter()
        .enumerate()
        .map(|(place, (path, alias))| {
            let options = offered
                .iter()
                .map(|leaf| view! { <option value=leaf.path.clone() selected=leaf.path == *path>{leaf.label.clone()}</option> })
                .collect_view();
            let path_id = format!("col-{place}-path");
            let alias_id = format!("col-{place}-alias");
            RepeatRow {
                key: place.to_string(),
                fields: view! {
                    <div class="fui-field">
                        <label class="fui-sr-only" for=path_id.clone()>
                            {tr_with(Msg::QueryDataPointN, &[("number", count(place.saturating_add(1)))])}
                        </label>
                        <select id=path_id.clone() name=path_id class="fui-select">{options}</select>
                    </div>
                    <div class="fui-field">
                        <label class="fui-sr-only" for=alias_id.clone()>
                            {tr_with(Msg::QueryColumnNameN, &[("number", count(place.saturating_add(1)))])}
                        </label>
                        <input id=alias_id.clone() name=alias_id type="text" class="fui-input fui-mono" value=alias.clone() />
                    </div>
                }
                .into_any(),
                end: RowEnd::Remove {
                    label: tr_with(Msg::QueryRemoveDataPoint, &[("name", if alias.is_empty() { path.clone() } else { alias.clone() }.into())]),
                    name: "intent".to_owned(),
                    value: format!("remove-column:{place}"),
                },
                after: None,
            }
        })
        .collect();
    repeat(Repeat {
        id: "data-points".to_owned(),
        label: tr(Msg::QueryDataPoints),
        fields: Fields::Two,
        head: vec![tr(Msg::QueryDataPoint), tr(Msg::QueryColumnName)],
        rows,
        blank: None,
        add: Some(RepeatAdd {
            label: tr(Msg::QueryAddDataPoint),
            name: "intent".to_owned(),
            value: "add-column".to_owned(),
        }),
        empty: tr(Msg::QueryNoDataPoints),
        form: None,
    })
}

/// The draft as the form's hidden state.
#[expect(
    clippy::expect_used,
    reason = "a Draft holds only strings, bools and vectors of them, which serde_json always serializes"
)]
fn state_of(draft: &Draft) -> String {
    serde_json::to_string(draft).expect("a draft of strings and bools should always serialize")
}

/// The data points a template offers, or the refusal of its catalog.
async fn offered(template: &str) -> Result<Vec<Leaf>, ViewerError> {
    if template.is_empty() {
        return Ok(Vec::new());
    }
    let catalog = crate::bff::template_detail::fetch_template_catalog(template.to_owned()).await?;
    let mut out = Vec::new();
    leaves(&catalog, &[], &mut out);
    Ok(out)
}

/// The built AQL, as a code section with Open in AQL.
fn built_view(aql: &str) -> AnyView {
    let code = Code {
        label: tr(Msg::QueryBuiltAql),
        language: Language::Aql,
        text: aql.to_owned(),
        ids: Some("built-L".to_owned()),
        current: None,
        diagnostics: Vec::new(),
    };
    code_section(
        CodeHead {
            title: tr(Msg::QueryBuiltAql),
            tags: Vec::new(),
            digest: None,
            copy: Some("built-aql".to_owned()),
            actions: Some(
                view! {
                    <button
                        type="submit"
                        form=QUERY_FORM
                        formaction="/query/aql"
                        name="intent"
                        value="opened"
                        class="fui-button fui-button-ghost fui-button-small"
                    >
                        {ferrohealth_ui::icon::icon(Glyph::ARROW_RIGHT, "14")}
                        {tr(Msg::QueryOpenInAql)}
                    </button>
                }
                .into_any(),
            ),
        },
        &code,
    )
}

/// Query: Build.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn BuildTab() -> impl IntoView {
    let posted = crate::posted::posted();
    let page = awaited(async move {
        let templates = crate::bff::templates::list_templates().await;
        let mut ids: Vec<String> = templates
            .as_ref()
            .map(|rows| rows.iter().map(|row| row.template_id.clone()).collect())
            .unwrap_or_default();
        ids.sort();
        ids.dedup();
        let mut lifted = None;
        let mut draft = match &posted {
            Some(posted) if posted.intent() == "lift" => {
                match crate::builder::lift::from_aql(&posted.get("aql").unwrap_or_default()) {
                    Ok(query) => draft_of_query(&query),
                    Err(error) => {
                        lifted = Some(warn_note(tr_with(
                            Msg::QueryCannotBuild,
                            &[("reason", error.to_string().into())],
                        )));
                        Draft::default()
                    }
                }
            }
            Some(posted) => {
                let template = posted.text("template");
                let paths: Vec<String> = offered(&template)
                    .await
                    .map(|leaves| leaves.into_iter().map(|leaf| leaf.path).collect())
                    .unwrap_or_default();
                draft_of_post(posted, &paths)
            }
            None => Draft {
                template: crate::request::query("template").unwrap_or_default(),
                shape: "data".to_owned(),
                columns: crate::request::query("path")
                    .map(|path| vec![(path, String::new())])
                    .unwrap_or_default(),
                limit: "50".to_owned(),
                ..Draft::default()
            },
        };
        if draft.template.is_empty() {
            draft.template = ids.first().cloned().unwrap_or_default();
        }
        if draft.shape.is_empty() {
            "data".clone_into(&mut draft.shape);
        }
        let leaves = offered(&draft.template).await;
        let (leaves, catalog_trouble) = match leaves {
            Ok(leaves) => (leaves, None),
            Err(error) => (
                Vec::new(),
                Some(refused(tr(Msg::QueryCatalogRefused), &error, None)),
            ),
        };
        if posted.is_none()
            && draft.shape == "data"
            && draft.columns.is_empty()
            && let Some(first) = leaves.first()
        {
            draft.columns.push((first.path.clone(), String::new()));
        }
        let built = query_of(&draft, &leaves)
            .map_err(|error| error.to_string())
            .and_then(|query| {
                crate::builder::lower::to_aql(&query).map_err(|error| error.to_string())
            });
        let mut save_problem = None;
        let mut top = None;
        let chart = chart_asked();
        let mut answer = None;
        match (step(posted.as_ref()), &built) {
            (Step::Run, Ok(aql)) => {
                answer = Some(
                    match crate::queries_api::run_aql(aql.clone(), String::new(), 0).await {
                        Ok(page) => results(&Answered {
                            page: &page,
                            aql,
                            parameters: "",
                            chart,
                        }),
                        Err(error) => section(
                            Some(tr(Msg::ResultsTitle)),
                            None,
                            None,
                            refused(tr(Msg::QueryRunRefused), &error, None),
                        ),
                    },
                );
            }
            (Step::Save, Ok(aql)) => {
                if let Err(refused_save) = save(posted.as_ref(), aql).await {
                    let (problem, refusal) = *refused_save;
                    save_problem = Some(problem);
                    top = refusal;
                }
            }
            (Step::Run | Step::Save, Err(reason)) => {
                top = Some(warn_note(tr_with(
                    Msg::QueryNotBuilt,
                    &[("reason", reason.clone().into())],
                )));
            }
            _ => {}
        }
        let template_options = ids
            .iter()
            .map(|id| view! { <option value=id.clone() selected=*id == draft.template>{id.clone()}</option> })
            .collect_view();
        let shape_options = SHAPES
            .iter()
            .map(|(key, words)| view! { <option value=*key selected=*key == draft.shape>{tr(*words)}</option> })
            .collect_view();
        let options = view! {
            <div class="fui-transform-options">
                {Field::new("q-t", tr(Msg::FilterTemplate))
                    .view(view! { <select id="q-t" name="template" class="fui-select">{template_options}</select> })}
                {Field::new("q-shape", tr(Msg::QueryReturns))
                    .view(view! { <select id="q-shape" name="shape" class="fui-select">{shape_options}</select> })}
                <div class="fui-transform-run">
                    <button type="submit" name="intent" value="edit" class="fui-button fui-button-secondary">
                        {tr(Msg::StagedApply)}
                    </button>
                </div>
            </div>
        };
        let points = (draft.shape == "data").then(|| {
            section(
                Some(tr(Msg::QueryDataPoints)),
                u64::try_from(draft.columns.len()).ok(),
                None,
                columns_view(&draft, &leaves),
            )
        });
        let conditions = condition_groups(view_group(&draft.root, "g", 1, &leaves));
        let order_options = leaves
            .iter()
            .map(|leaf| view! { <option value=leaf.path.clone() selected=leaf.path == draft.order>{leaf.label.clone()}</option> })
            .collect_view();
        let order = view! {
            <div class="fui-transform-options">
                {Field::new("q-o", tr(Msg::QueryOrderBy))
                    .view(view! {
                        <select id="q-o" name="order" class="fui-select">
                            <option value="">{tr(Msg::QueryNoOrder)}</option>
                            {order_options}
                        </select>
                    })}
                {Field::new("q-d", tr(Msg::QueryDirection))
                    .view(view! {
                        <select id="q-d" name="direction" class="fui-select">
                            <option value="desc" selected=draft.descending>{tr(Msg::QueryNewestFirst)}</option>
                            <option value="asc" selected=!draft.descending>{tr(Msg::QueryOldestFirst)}</option>
                        </select>
                    })}
                {Field::new("q-l", tr(Msg::QueryLimit))
                    .view(view! { <input id="q-l" name="limit" type="text" inputmode="numeric" class="fui-input" value=draft.limit.clone() /> })}
            </div>
        };
        let built_section = match &built {
            Ok(aql) => section(Some(tr(Msg::QueryItBuilds)), None, None, built_view(aql)),
            Err(reason) => section(
                Some(tr(Msg::QueryItBuilds)),
                None,
                None,
                info_note(tr_with(
                    Msg::QueryNotBuilt,
                    &[("reason", reason.clone().into())],
                )),
            ),
        };
        let state = state_of(&draft);
        let aql = built.clone().unwrap_or_default();
        view! {
            <Title text=tr(Msg::NavQuery) />
            {heading(save_popover(posted.as_ref(), save_problem, None))}
            <div class="fui-stack">
                {lifted}
                {top}
                {catalog_trouble}
                <form id=QUERY_FORM method="post" action="/query">
                    <input type="hidden" name="default-intent" value="run" />
                    <button type="submit" class="fui-sr-only" tabindex="-1">{tr(Msg::QueryRun)}</button>
                    <input type="hidden" name="state" value=state />
                    <input type="hidden" name="aql" value=aql />
                    {binder(tr(Msg::QueryWays), tabs(), "build", panel(view! {
                        {plain_section(options)}
                        {points}
                        {section(Some(tr(Msg::QueryConditions)), None, None, conditions)}
                        {section(Some(tr(Msg::QueryOrderLimit)), None, None, order)}
                        {built_section}
                        {plain_section(shown_choice(chart))}
                        {answer.unwrap_or_else(|| not_run(tr(Msg::ResultsNotRun)))}
                    }))}
                </form>
            </div>
        }
        .into_any()
    });
    view! { {page} }
}

/// The grammar check of `aql`: its findings, placed on their lines, or
/// `None` when it parses.
fn grammar(aql: &str) -> Option<(String, FileFindings)> {
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
                Msg::QueryUnexpected,
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
                        Msg::QueryUnexpected,
                        &[("token", format!("{token:?}").into())],
                    ),
                    None => tr(Msg::QueryEndedEarly),
                },
                mapping: None,
            })
            .collect(),
    };
    let total = findings.len();
    Some((
        tr_with(Msg::QueryGrammarHeading, &[("count", count(total))]),
        FileFindings {
            file: EditorField::Aql.name().to_owned(),
            href: None,
            lines: format!("#{}-L", EditorField::Aql.name()),
            current: None,
            findings,
            all: None,
        },
    ))
}

/// The parameters a run binds, as the form holds them: each name and value.
fn parameters_of(posted: Option<&crate::posted::Posted>, aql: &str) -> Vec<(String, String)> {
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
            && place < rows.len() {
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
fn parameters_view(rows: &[(String, String)]) -> AnyView {
    let rows = rows
        .iter()
        .enumerate()
        .map(|(place, (name, value))| {
            let name_id = format!("param-{place}-name");
            let value_id = format!("param-{place}-value");
            RepeatRow {
                key: place.to_string(),
                fields: view! {
                    <div class="fui-field">
                        <label class="fui-sr-only" for=name_id.clone()>
                            {tr_with(Msg::QueryParamNameN, &[("number", count(place.saturating_add(1)))])}
                        </label>
                        <input id=name_id.clone() name=name_id type="text" class="fui-input fui-mono" value=name.clone() />
                    </div>
                    <div class="fui-field">
                        <label class="fui-sr-only" for=value_id.clone()>
                            {tr_with(Msg::QueryParamValueN, &[("number", count(place.saturating_add(1)))])}
                        </label>
                        <input id=value_id.clone() name=value_id type="text" class="fui-input fui-mono" spellcheck="false" value=value.clone() />
                    </div>
                }
                .into_any(),
                end: RowEnd::Remove {
                    label: tr_with(Msg::QueryRemoveParam, &[("name", name.clone().into())]),
                    name: "intent".to_owned(),
                    value: format!("remove-param:{place}"),
                },
                after: None,
            }
        })
        .collect();
    repeat(Repeat {
        id: "parameters".to_owned(),
        label: tr(Msg::QueryParameters),
        fields: Fields::Two,
        head: vec![tr(Msg::QueryName), tr(Msg::TagsValue)],
        rows,
        blank: None,
        add: Some(RepeatAdd {
            label: tr(Msg::QueryAddParam),
            name: "intent".to_owned(),
            value: "add-param".to_owned(),
        }),
        empty: tr(Msg::QueryNoParams),
        form: None,
    })
}

/// The parameters as the JSON object a run sends.
fn parameters_json(rows: &[(String, String)]) -> String {
    let bindings: BTreeMap<String, String> = rows
        .iter()
        .filter(|(name, _)| !name.is_empty())
        .cloned()
        .collect();
    crate::aql_text::parameters_json(&bindings).to_string()
}

/// Query: AQL.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one page component: the post it answers, the reads it waits on and the view it draws"
)]
#[component]
pub fn AqlTab() -> impl IntoView {
    let posted = crate::posted::posted();
    let page = awaited(async move {
        let mut note = None;
        let mut top = None;
        let mut loaded = None;
        let aql = match &posted {
            Some(posted) => posted.get("aql").unwrap_or_default(),
            None => match (
                crate::request::query("name"),
                crate::request::query("version"),
            ) {
                (Some(name), Some(version)) => {
                    match crate::queries_api::fetch_stored_query(name.clone(), version.clone())
                        .await
                    {
                        Ok(text) => {
                            loaded = Some((name.clone(), version.clone()));
                            note = Some(info_note(tr_with(
                                Msg::QueryOpenedStored,
                                &[("name", name.into()), ("version", version.into())],
                            )));
                            text
                        }
                        Err(error) => {
                            top = Some(refused(tr(Msg::QueryStoredRefused), &error, None));
                            String::new()
                        }
                    }
                }
                _ => String::new(),
            },
        };
        if posted
            .as_ref()
            .is_some_and(|posted| posted.intent() == "opened")
        {
            note = Some(info_note(tr(Msg::QueryBuildMade)));
        }
        let rows = parameters_of(posted.as_ref(), &aql);
        let parameters = parameters_json(&rows);
        let findings = (!aql.trim().is_empty()).then(|| grammar(&aql)).flatten();
        let chart = chart_asked();
        let mut save_problem = None;
        let mut answer = None;
        match step(posted.as_ref()) {
            Step::Run if aql.trim().is_empty() => {
                answer = Some(not_run(tr(Msg::QueryNothingToRun)));
            }
            Step::Run if findings.is_some() => answer = Some(not_run(tr(Msg::QueryGrammarStops))),
            Step::Run => {
                answer = Some(
                    match crate::queries_api::run_aql(aql.clone(), parameters.clone(), 0).await {
                        Ok(page) => results(&Answered {
                            page: &page,
                            aql: &aql,
                            parameters: &parameters,
                            chart,
                        }),
                        Err(error) => section(
                            Some(tr(Msg::ResultsTitle)),
                            None,
                            None,
                            refused(tr(Msg::QueryRunRefused), &error, None),
                        ),
                    },
                );
            }
            Step::Save if findings.is_some() => save_problem = Some(tr(Msg::QueryGrammarStops)),
            Step::Save => {
                if let Err(refused_save) = save(posted.as_ref(), &aql).await {
                    let (problem, refusal) = *refused_save;
                    save_problem = Some(problem);
                    top = refusal;
                }
            }
            Step::Edit => {}
        }
        let buildable = !aql.trim().is_empty()
            && findings.is_none()
            && crate::builder::lift::from_aql(&aql).is_ok();
        let to_build = buildable.then(|| {
            view! {
                <button type="submit" form=QUERY_FORM formaction="/query" name="intent" value="lift" class="fui-button fui-button-ghost fui-button-small">
                    {ferrohealth_ui::icon::icon(Glyph::LIST_CHECKS, "14")}
                    {tr(Msg::QueryOpenInBuild)}
                </button>
            }
        });
        let editor = view! {
            <CodeEditor
                setup=EditorSetup {
                    field: EditorField::Aql,
                    language: EditorLanguage::Aql,
                    text: aql.clone(),
                    opened: None,
                    marks: marks_of(findings.as_ref()),
                    label: tr(Msg::QueryTheAql),
                    lead: tr(Msg::QueryAql),
                    facts: vec![tr(Msg::EditorUtf8)],
                    findings: findings.clone(),
                }
                words=EditorWords::of_page()
                diagnostics=DiagnosticsWords::of_page()
            />
        };
        view! {
            <Title text=tr(Msg::QueryAqlTitle) />
            {heading(save_popover(posted.as_ref(), save_problem, loaded.as_ref().map(|(name, version)| (name.as_str(), version.as_str()))))}
            <div class="fui-stack">
                {top}
                <form id=QUERY_FORM method="post" action="/query/aql">
                    <input type="hidden" name="default-intent" value="run" />
                    <button type="submit" class="fui-sr-only" tabindex="-1">{tr(Msg::QueryRun)}</button>
                    {binder(tr(Msg::QueryWays), tabs(), "aql", panel(view! {
                        {plain_section(view! {
                            {note}
                            <div class="fui-cluster">{to_build}</div>
                            {editor}
                        })}
                        {section(Some(tr(Msg::QueryParameters)), u64::try_from(rows.len()).ok(), None, parameters_view(&rows))}
                        {plain_section(shown_choice(chart))}
                        {answer.unwrap_or_else(|| not_run(tr(Msg::ResultsNotRun)))}
                    }))}
                </form>
            </div>
        }
        .into_any()
    });
    view! { {page} }
}

#[cfg(test)]
mod tests {
    use super::{
        Draft, Leaf, UiCond, UiGroup, draft_of_query, kind_of, query_of, restructure, step, Step,
    };
    use crate::builder::model::{CriterionKind, CriterionNode};
    use crate::posted::Posted;

    /// The data points of the fixture template.
    fn leaves() -> Vec<Leaf> {
        vec![
            Leaf {
                path: "/content[openEHR-EHR-OBSERVATION.bp.v2]/data[at0001]/items[at0004]/value".to_owned(),
                label: "Blood pressure › Systolic".to_owned(),
                rm_type: "DV_QUANTITY".to_owned(),
                codes: Vec::new(),
                unit: Some("mm[Hg]".to_owned()),
            },
            Leaf {
                path: "/content[openEHR-EHR-OBSERVATION.bp.v2]/data[at0001]/items[at0008]/value".to_owned(),
                label: "Blood pressure › Position".to_owned(),
                rm_type: "DV_CODED_TEXT".to_owned(),
                codes: vec![("at1001".to_owned(), "Sitting".to_owned())],
                unit: None,
            },
        ]
    }

    fn cond(path_of: usize, op: &str, value: &str) -> UiCond {
        UiCond {
            path: leaves().get(path_of).map(|leaf| leaf.path.clone()).unwrap_or_default(),
            op: op.to_owned(),
            value: value.to_owned(),
        }
    }

    #[test]
    fn a_condition_takes_the_constraint_its_type_and_operator_name() {
        let (kind, negated) = kind_of(&cond(0, "ge", "140"), "DV_QUANTITY", "Systolic").expect("a number");
        assert!(!negated);
        assert_eq!(kind, CriterionKind::QuantityRange { min: Some(140.0), max: None, units: String::new() });
        let (kind, _) = kind_of(&cond(0, "eq", "120"), "DV_QUANTITY", "Systolic").expect("a number");
        assert_eq!(kind, CriterionKind::QuantityRange { min: Some(120.0), max: Some(120.0), units: String::new() });
        let (kind, negated) = kind_of(&cond(1, "ne", "at1001"), "DV_CODED_TEXT", "Position").expect("a code");
        assert!(negated);
        assert_eq!(kind, CriterionKind::CodedIn { codes: vec!["at1001".to_owned()], terminology: String::new() });
        assert!(kind_of(&cond(0, "ge", "high"), "DV_QUANTITY", "Systolic").is_err(), "a value that is no number is refused, never dropped");
    }

    #[test]
    fn a_draft_lowers_to_aql_and_lifts_back_to_the_same_draft() {
        let draft = Draft {
            template: "Vital signs.v1".to_owned(),
            shape: "data".to_owned(),
            columns: vec![(leaves().first().map(|leaf| leaf.path.clone()).unwrap_or_default(), "systolic".to_owned())],
            root: UiGroup {
                any: false,
                not: false,
                conditions: vec![cond(0, "ge", "140"), cond(1, "eq", "at1001")],
                groups: vec![UiGroup { any: true, not: true, conditions: vec![cond(0, "le", "90")], groups: Vec::new() }],
            },
            order: String::new(),
            descending: true,
            limit: "50".to_owned(),
        };
        let query = query_of(&draft, &leaves()).expect("the draft is whole");
        let Some(CriterionNode::Group { children, .. }) = &query.criteria else {
            panic!("the root is a group");
        };
        assert_eq!(children.len(), 3);
        let aql = crate::builder::lower::to_aql(&query).expect("the query lowers");
        assert!(aql.contains(">=140"), "{aql}");
        let lifted = crate::builder::lift::from_aql(&aql).expect("the builder's own AQL lifts");
        let back = draft_of_query(&lifted);
        assert_eq!(back.columns, draft.columns);
        assert_eq!(back.root.conditions.len(), 2);
        assert_eq!(back.root.groups.len(), 1);
    }

    #[test]
    fn an_add_or_remove_names_what_it_acts_on_and_is_never_a_run() {
        let mut root = UiGroup { conditions: vec![cond(0, "ge", "1")], ..UiGroup::default() };
        let posted = Posted::new(vec![("intent".to_owned(), "run".to_owned()), ("add-group".to_owned(), "g".to_owned())]);
        assert_eq!(step(Some(&posted)), Step::Edit);
        restructure(&mut root, &posted, "/x");
        assert_eq!(root.groups.len(), 1);
        let posted = Posted::new(vec![("remove".to_owned(), "g.c0".to_owned())]);
        restructure(&mut root, &posted, "/x");
        assert_eq!(root.conditions, Vec::new());
        let posted = Posted::new(vec![("remove-group".to_owned(), "g.0".to_owned())]);
        restructure(&mut root, &posted, "/x");
        assert_eq!(root.groups, Vec::new());
        let run = Posted::new(vec![("intent".to_owned(), "run".to_owned())]);
        assert_eq!(step(Some(&run)), Step::Run);
        assert_eq!(step(None), Step::Edit);
    }
}
