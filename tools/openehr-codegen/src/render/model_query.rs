// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `model-query` report: for every class attribute the vendored BMM
//! declares, what the BMM states about it beside the Rust field shape the
//! emitter currently emits for it.
//!
//! Source of truth: the vendored BMM meta-model files under
//! `tools/openehr-codegen/vendor/bmm/components/<COMPONENT>/json/*.bmm.json` —
//! the same files [`crate::plan::composition`] feeds to the emitter, loaded
//! through the same LOAD → ANALYZE → PLAN stages. Nothing here re-derives an
//! emission decision: the field-shape column is produced by CALLING
//! [`crate::render::emit::field_type`], the per-class shape by calling
//! [`crate::plan::decide`], so the report cannot drift from what `emit` writes.
//!
//! What the BMM columns mean (the meta-model's own definitions):
//!
//! - **existence** — `BMM_PROPERTY.existence()` is "Interval form of `0..1`,
//!   `1..1` etc, generated from `_is_mandatory_`"
//!   (`LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_property.adoc`
//!   §`BMM_PROPERTY` Class), so a mandatory property reports `1..1` and an
//!   optional one `0..1`.
//! - **container + cardinality** — `BMM_CONTAINER_PROPERTY` "represents a
//!   container type based on one of the inbuilt types List <>, Set <>, Array
//!   <>" and carries an optional `cardinality: Multiplicity_interval`
//!   (`LANG/docs/UML/classes/org.openehr.lang.bmm.bmm_container_property.adoc`
//!   §`BMM_CONTAINER_PROPERTY` Class). A single-valued property has neither, and
//!   a container whose BMM node states no cardinality reports `-` rather than a
//!   synthesized bound.
//! - **abstract** — the vendored `*.bmm.json` carries `is_abstract` on class
//!   and function nodes only; its property nodes carry no abstract/effected
//!   facet (the `(effected)`/`(redefined)` markers in the spec's UML tables are
//!   documentation, not serialized meta-data). The report therefore states
//!   abstractness at the class level, where the BMM has it.
//!
//! The field-shape column is computed for the **declaring** class. A concrete
//! descendant flattens the same property through the same
//! [`crate::render::emit::field_type`] call with itself as the emitting class
//! (see [`crate::render::emit`]'s struct renderer), which can differ only where
//! the self-recursion boxing check resolves differently for that descendant.

#![expect(
    clippy::disallowed_types,
    reason = "dev tooling over JSON artifacts (vendored BMM/OAS bundles, emitter reports) — not the \
              application (#1694)"
)]
use crate::analyze::{
    External, Model, augment_with_reemit, class_paths, cross_schema_reemit, emittable_specs,
};
use crate::load::bmm::{BmmCardinality, BmmClass, BmmPropKind, BmmProperty, BmmSchema, BmmType};
use crate::plan::composition::{self, compose};
use crate::plan::overrides::{back_reference, class_binding};
use crate::plan::{Emission, decide};
use crate::render::emit::{field_type, struct_generics};
use std::collections::BTreeSet;

/// The report's error type (same boxed shape the rest of the CLI uses).
type Error = Box<dyn std::error::Error>;

/// The value reported in the emission column for a property of a class the plan
/// stage skips (a mapped/primitive class, or an abstract class with neither
/// concrete descendants nor any use as a field type): no Rust type is emitted,
/// so no field shape exists.
const NOT_EMITTED: &str = "(class not emitted)";

/// The value reported in the emission column for a designated owner/parent
/// back-reference: the emitter omits the field from the struct entirely (see
/// [`crate::plan::overrides::back_reference`]).
const BACK_REFERENCE: &str = "(omitted: back-reference)";

/// The placeholder for a column the BMM states nothing for.
const ABSENT: &str = "-";

/// The report's output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    /// Column-aligned text for reading in a terminal, with a row-count footer.
    Table,
    /// Tab-separated values with a header line — the machine-readable form.
    Tsv,
    /// A JSON array of one object per row.
    Json,
}

impl Format {
    /// The accepted `--format` values, for the usage/error text.
    pub(crate) const VALID: &str = "table, tsv, json";

    /// Parse a `--format` value.
    ///
    /// # Errors
    /// Returns an error naming every valid value if `s` is not one of them.
    pub(crate) fn parse(s: &str) -> Result<Self, Error> {
        match s {
            "table" => Ok(Self::Table),
            "tsv" => Ok(Self::Tsv),
            "json" => Ok(Self::Json),
            other => {
                Err(format!("unknown format {other:?}; valid formats: {}", Self::VALID).into())
            }
        }
    }
}

/// The report's filters; `None` everywhere reports the whole loaded model.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Query<'a> {
    /// Restrict to one composition key (`base`, `rm`, `lang`, `v1_4`, `v2_4`,
    /// `term`).
    pub component: Option<&'a str>,
    /// Restrict to one BMM class name (exact, as the BMM spells it).
    pub class: Option<&'a str>,
    /// Restrict to one attribute name (exact, as the BMM spells it).
    pub attribute: Option<&'a str>,
    /// Report one row per class × **flattened** attribute (every attribute the
    /// class carries, inherited ones included) instead of one row per class ×
    /// DECLARED attribute.
    ///
    /// This is the inheritance dimension: an attribute declared on an abstract
    /// class appears once in the default (declared) view and once per
    /// descendant here, each with its own emission computed by the same
    /// [`crate::render::emit::field_type`] call the struct renderer makes for
    /// that class — so a per-descendant divergence (the self-recursion boxing
    /// check resolving differently, a redeclared generic parameter) is visible
    /// instead of assumed absent.
    pub flattened: bool,
}

/// One reported attribute: the BMM facts plus the current emission decision.
#[derive(Debug, Clone)]
struct Row {
    /// The composition key the class was loaded under.
    component: &'static str,
    /// The vendored BMM file's stem — the generation the class belongs to (a
    /// crate composed of several generations declares some class names twice).
    bmm: String,
    /// The class's source package path, or [`ABSENT`] for a foundation class
    /// the schema declares outside its package tree.
    package: String,
    /// The BMM class name.
    class: String,
    /// The class's `is_abstract` marker.
    class_abstract: bool,
    /// The class's planned emission shape (`struct`, `enum`, `poly_enum`,
    /// `enum_literals`, `newtype`, `skip`).
    class_emission: &'static str,
    /// The attribute's 0-based position in the class's BMM declaration order
    /// (the canonical-JSON field order), preserved because the report itself is
    /// sorted by name.
    decl: usize,
    /// The class that DECLARES the attribute. Equal to `class` for an
    /// attribute the class declares itself; the ancestor's name for an
    /// inherited one (only ever different in the flattened view).
    declared_on: String,
    /// The BMM attribute name.
    attribute: String,
    /// The declared type exactly as the BMM states it (`List<OBJECT_REF>`,
    /// `DV_INTERVAL<DV_QUANTITY>`, `Hash<String, String>`).
    bmm_type: String,
    /// `1..1` or `0..1`, per `BMM_PROPERTY.existence()`.
    existence: &'static str,
    /// The container kind (`List`, `Set`, `Array`, `Hash`), or [`ABSENT`].
    container: String,
    /// The container's stated cardinality (`0..*`, `1..*`, `0..3`), or
    /// [`ABSENT`] when the BMM node states none.
    cardinality: String,
    /// The Rust field type the emitter currently emits, or one of
    /// [`NOT_EMITTED`] / [`BACK_REFERENCE`].
    emission: String,
}

/// The rows of a query's scope plus the names the filters validate against.
struct Scope {
    /// Every attribute row in the selected components.
    rows: Vec<Row>,
    /// Every class name in the selected components, including classes that
    /// declare no attribute (so `--class` on such a class is not an error).
    classes: BTreeSet<String>,
}

/// The column headers, in report order.
const HEADERS: [&str; 14] = [
    "component",
    "bmm",
    "package",
    "class",
    "abstract",
    "class_emission",
    "decl",
    "declared_on",
    "attribute",
    "bmm_type",
    "existence",
    "container",
    "cardinality",
    "emission",
];

impl Row {
    /// The row's cells, in [`HEADERS`] order.
    fn cells(&self) -> [String; 14] {
        [
            self.component.to_string(),
            self.bmm.clone(),
            self.package.clone(),
            self.class.clone(),
            if self.class_abstract {
                "abstract".to_string()
            } else {
                ABSENT.to_string()
            },
            self.class_emission.to_string(),
            self.decl.to_string(),
            self.declared_on.clone(),
            self.attribute.clone(),
            self.bmm_type.clone(),
            self.existence.to_string(),
            self.container.clone(),
            self.cardinality.clone(),
            self.emission.clone(),
        ]
    }
}

/// Render the report for `query` in `format`.
///
/// # Errors
/// Returns an error if a vendored BMM file cannot be loaded, or if a filter
/// names a component/class/attribute the loaded model does not have (the error
/// lists the valid values).
pub(crate) fn render(query: &Query<'_>, format: Format) -> Result<String, Error> {
    let scope = collect(query.component, query.flattened)?;
    let mut rows = scope.rows;

    if let Some(class) = query.class {
        if !scope.classes.contains(class) {
            return Err(unknown_value("class", class, &scope.classes));
        }
        rows.retain(|r| r.class == class);
    }
    if let Some(attribute) = query.attribute {
        let available: BTreeSet<String> = rows.iter().map(|r| r.attribute.clone()).collect();
        if !available.contains(attribute) {
            return Err(unknown_value("attribute", attribute, &available));
        }
        rows.retain(|r| r.attribute == attribute);
    }

    match format {
        Format::Table => Ok(render_table(&rows)),
        Format::Tsv => Ok(render_tsv(&rows)),
        Format::Json => render_json(&rows),
    }
}

/// The composition keys a `--component` filter selects.
///
/// # Errors
/// Returns an error listing every valid key if `component` names none.
fn select_keys(component: Option<&str>) -> Result<Vec<&'static str>, Error> {
    let all: Vec<&'static str> = composition::COMPOSITIONS.iter().map(|c| c.key).collect();
    match component {
        None => Ok(all),
        Some(key) => match all.iter().find(|k| **k == key) {
            Some(found) => Ok(vec![*found]),
            None => Err(format!(
                "unknown component {key:?}; valid components: {}",
                all.join(", ")
            )
            .into()),
        },
    }
}

/// Load every selected component and project its attributes into rows, sorted
/// by (component, generation, class, attribute) for byte-stable output.
fn collect(component: Option<&str>, flattened: bool) -> Result<Scope, Error> {
    let mut out = Scope {
        rows: Vec::new(),
        classes: BTreeSet::new(),
    };
    for key in select_keys(component)? {
        let composed = compose(key)?;
        for generation in &composed.generations {
            for unit in &generation.units {
                // Project the schema the emitter actually RENDERS this unit
                // from — its own schema, augmented with the cross-schema
                // re-emission closure exactly as `cli::cmd_emit` augments
                // every unit (a unit with an empty closure is unchanged).
                // Projecting a differently-composed schema would attribute
                // field shapes to a crate that emits no such class.
                let deps: Vec<&BmmSchema> = generation.dep_schemas.iter().collect();
                let reemit = cross_schema_reemit(&unit.model, &unit.schema);
                let schema = augment_with_reemit(&unit.schema, &unit.model, &reemit, &deps);
                collect_generation(
                    key,
                    unit.spec.file,
                    &unit.model,
                    &schema,
                    &generation.external,
                    flattened,
                    &mut out,
                );
            }
        }
    }
    out.rows.sort_by(|a, b| {
        (a.component, &a.bmm, &a.class, &a.attribute).cmp(&(
            b.component,
            &b.bmm,
            &b.class,
            &b.attribute,
        ))
    });
    Ok(out)
}

/// Project one BMM generation's classes into rows.
fn collect_generation(
    component: &'static str,
    bmm_file: &str,
    model: &Model,
    schema: &BmmSchema,
    external: &External,
    flattened: bool,
    out: &mut Scope,
) {
    let used = model.used_as_type();
    // The same set `emit_version` calls `local`: the spec names this version
    // emits, against which a field type resolves crate-locally.
    let local = emittable_specs(model, schema);
    let packages = class_paths(schema);
    let bmm = bmm_stem(bmm_file);

    for (name, class) in &schema.classes {
        out.classes.insert(name.clone());
        let emission = decide(model, class, &used);
        let class_emission = emission_label(&emission);
        let skipped = matches!(emission, Emission::Skip);
        let generics = struct_generics(model, class);
        let subst = class_binding(name);
        let props = view_props(model, class, name, flattened);
        let package = packages
            .get(name)
            .cloned()
            .unwrap_or_else(|| ABSENT.to_string());
        for (decl, (owner, prop)) in props.into_iter().enumerate() {
            let shape = if skipped {
                NOT_EMITTED.to_string()
            } else if back_reference(&owner, &prop.name).is_some() {
                BACK_REFERENCE.to_string()
            } else {
                field_type(model, class, prop, &generics, &subst, &local, external)
            };
            let (container, cardinality) = container_columns(prop);
            out.rows.push(Row {
                component,
                bmm: bmm.clone(),
                package: package.clone(),
                class: name.clone(),
                class_abstract: class.is_abstract,
                class_emission,
                decl,
                declared_on: owner,
                attribute: prop.name.clone(),
                bmm_type: bmm_type_text(prop),
                existence: existence_text(prop),
                container,
                cardinality,
                emission: shape,
            });
        }
    }
}

/// The properties one class contributes to the report, paired with the class
/// each is declared on.
///
/// The DECLARED view is the class's own properties. The FLATTENED view is every
/// property the class CARRIES, resolved through the same
/// [`Model::flattened_props`] the struct renderer walks — so the emission column
/// is computed for the class that actually emits the field.
fn view_props<'a>(
    model: &'a Model,
    class: &'a BmmClass,
    name: &str,
    flattened: bool,
) -> Vec<(String, &'a BmmProperty)> {
    if flattened {
        return model
            .flattened_props(class)
            .into_iter()
            .map(|rp| (rp.owner, rp.prop))
            .collect();
    }
    class
        .properties
        .iter()
        .map(|p| (name.to_owned(), p))
        .collect()
}

/// The `existence` column: one property's BMM existence range as spec text.
fn existence_text(prop: &BmmProperty) -> &'static str {
    if prop.is_mandatory { "1..1" } else { "0..1" }
}

/// The planned shape of a class, as the label the report prints.
fn emission_label(emission: &Emission<'_>) -> &'static str {
    match emission {
        Emission::Struct => "struct",
        Emission::Enum(_) => "enum",
        Emission::PolyEnum(_) => "poly_enum",
        Emission::EnumLiterals(_) => "enum_literals",
        Emission::Newtype(_) => "newtype",
        Emission::Skip => "skip",
    }
}

/// The vendored BMM file's stem — `components/RM/json/openehr_rm_1.2.0.bmm.json`
/// → `openehr_rm_1.2.0`.
fn bmm_stem(file: &str) -> String {
    let name = file.rsplit('/').next().unwrap_or(file);
    name.strip_suffix(".bmm.json").unwrap_or(name).to_string()
}

/// The declared type exactly as the BMM states it: a container property renders
/// as `<container_type><item>`, a single property as its (possibly generic)
/// type reference.
fn bmm_type_text(prop: &BmmProperty) -> String {
    match &prop.kind {
        BmmPropKind::Single(t) => type_text(t),
        BmmPropKind::Container {
            container_type,
            item,
            ..
        } => format!("{container_type}<{}>", type_text(item)),
    }
}

/// A BMM type reference as text (`DV_TEXT`, `DV_INTERVAL<DV_QUANTITY>`).
fn type_text(t: &BmmType) -> String {
    match t {
        BmmType::Simple(s) => s.clone(),
        BmmType::Generic { root, params } => format!(
            "{root}<{}>",
            params.iter().map(type_text).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// The container-kind and cardinality columns of a property.
fn container_columns(prop: &BmmProperty) -> (String, String) {
    match &prop.kind {
        BmmPropKind::Single(_) => (ABSENT.to_string(), ABSENT.to_string()),
        BmmPropKind::Container {
            container_type,
            cardinality,
            ..
        } => (
            container_type.clone(),
            cardinality
                .as_ref()
                .map_or_else(|| ABSENT.to_string(), cardinality_text),
        ),
    }
}

/// A `Multiplicity_interval` cardinality as `lower..upper` / `lower..*`.
fn cardinality_text(c: &BmmCardinality) -> String {
    c.upper.map_or_else(
        || format!("{}..*", c.lower),
        |upper| format!("{}..{upper}", c.lower),
    )
}

/// A loud "no such value" error naming the valid values — every one of them
/// when nothing resembles the input, else the ones whose name contains it.
fn unknown_value(kind: &str, value: &str, valid: &BTreeSet<String>) -> Error {
    let needle = value.to_lowercase();
    let near: Vec<&str> = valid
        .iter()
        .filter(|v| v.to_lowercase().contains(&needle))
        .map(String::as_str)
        .collect();
    let (listed, scope) = if near.is_empty() {
        (
            valid.iter().map(String::as_str).collect::<Vec<_>>(),
            String::new(),
        )
    } else {
        (
            near,
            format!(" of {} loaded, matching by name", valid.len()),
        )
    };
    format!(
        "unknown {kind} {value:?}; valid {kind} values ({}{scope}): {}",
        listed.len(),
        listed.join(", ")
    )
    .into()
}

/// Column-aligned text with a header, a rule, and a row-count footer.
fn render_table(rows: &[Row]) -> String {
    let cells: Vec<[String; 14]> = rows.iter().map(Row::cells).collect();
    let mut widths: [usize; 13] = [0; 13];
    for (w, h) in widths.iter_mut().zip(HEADERS) {
        *w = h.len();
    }
    for row in &cells {
        for (w, c) in widths.iter_mut().zip(row) {
            *w = (*w).max(c.chars().count());
        }
    }

    let mut out = String::new();
    push_padded(&mut out, &widths, HEADERS.iter().copied());
    push_padded(&mut out, &widths, widths.iter().map(|w| "-".repeat(*w)));
    for row in &cells {
        push_padded(&mut out, &widths, row.iter().map(String::as_str));
    }
    out.push_str(&format!("\n{} attribute rows\n", rows.len()));
    out
}

/// Append one padded table line (trailing padding trimmed).
fn push_padded<S: AsRef<str>>(
    out: &mut String,
    widths: &[usize; 13],
    cells: impl IntoIterator<Item = S>,
) {
    let mut line = String::new();
    for (cell, width) in cells.into_iter().zip(widths) {
        let text = cell.as_ref();
        line.push_str(text);
        line.push_str(&" ".repeat(width.saturating_sub(text.chars().count())));
        line.push_str("  ");
    }
    out.push_str(line.trim_end());
    out.push('\n');
}

/// Tab-separated values with a header line.
fn render_tsv(rows: &[Row]) -> String {
    let mut out = String::new();
    out.push_str(&HEADERS.join("\t"));
    out.push('\n');
    for row in rows {
        out.push_str(&row.cells().join("\t"));
        out.push('\n');
    }
    out
}

/// A JSON array of one object per row (keys in [`HEADERS`] order).
///
/// # Errors
/// Returns the `serde_json` error if the rows cannot be serialized.
fn render_json(rows: &[Row]) -> Result<String, Error> {
    let values: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "component": r.component,
                "bmm": r.bmm,
                "package": r.package,
                "class": r.class,
                "abstract": r.class_abstract,
                "class_emission": r.class_emission,
                "decl": r.decl,
                "attribute": r.attribute,
                "bmm_type": r.bmm_type,
                "existence": r.existence,
                "container": r.container,
                "cardinality": r.cardinality,
                "emission": r.emission,
            })
        })
        .collect();
    let mut out = serde_json::to_string_pretty(&values)?;
    out.push('\n');
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_parse_rejects_an_unknown_value_loudly() {
        let err = Format::parse("yaml").expect_err("yaml is not a report format");
        let text = err.to_string();
        assert!(text.contains("yaml"), "{text}");
        assert!(text.contains("table, tsv, json"), "{text}");
    }

    #[test]
    fn bmm_stem_strips_the_vendored_path_and_suffix() {
        assert_eq!(
            bmm_stem("components/RM/json/openehr_rm_1.2.0.bmm.json"),
            "openehr_rm_1.2.0"
        );
    }

    #[test]
    fn cardinality_renders_bounded_and_unbounded_intervals() {
        assert_eq!(
            cardinality_text(&BmmCardinality {
                lower: 0,
                upper: None
            }),
            "0..*"
        );
        assert_eq!(
            cardinality_text(&BmmCardinality {
                lower: 1,
                upper: Some(3)
            }),
            "1..3"
        );
    }
}
