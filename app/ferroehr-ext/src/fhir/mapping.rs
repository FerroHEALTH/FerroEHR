// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR-connector mapping-definition schema and the pure FHIR-to-FLAT
//! transform.
//!
//! No openEHR spec governs FHIR-to-openEHR mapping — our own design/extension:
//! master14's integration model maps integration archetypes to designed ones
//! via `GENERIC_ENTRY`, whereas this connector maps directly to designed
//! templates as data. The `FEEDER_AUDIT` builder, the one RM-typed part, lives
//! in [`super::feeder_audit`].
//!
//! A mapping definition binds one FHIR R4 resource profile to one openEHR
//! template. Each of its `entries` reads a value out of the incoming resource
//! through a `FHIRPath`-lite dot-path (see [`resolve`]) and writes it to a
//! template-relative openEHR FLAT path (the `id[:i]/…|suffix` key
//! `openehr_sdt::flat` consumes, `simplified_formats` master04 §Field
//! Identifiers). The resulting flat map goes to
//! [`composition_from_flat`](openehr_sdt::flat::convert::composition_from_flat)
//! with the template's `WebTemplate`, and the COMPOSITION it builds commits
//! through the platform's normal validated path. This module is protocol-free
//! and DB-free; the orchestration lives on `FerroEhrService`.
//!
//! The FHIR side is a deliberate subset of `FHIRPath`
//! (<https://hl7.org/fhirpath/>): object-field navigation, array indexing,
//! `first()`, and single-condition `where(path = literal)` filters — never the
//! full language. `FHIRPath` defines `where()` over collections; this
//! single-value subset takes the first matching element, which is what a flat
//! single-value leaf needs.
//!
//! `code_map` binds a FHIR system URL to an openEHR `terminology_id` and passes
//! the code through unchanged. A `coded` entry may also declare `translate`:
//! the orchestrator resolves each [`TranslationRequest`] (from
//! [`collect_translations`]) through the platform's terminology seam and hands
//! the answers back as [`CodeTranslations`], so this module never talks to a
//! server. The built COMPOSITION's own terminology validation remains the
//! authority on the result.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6): FHIR resources are an external standard \
              with no RM type (typed-FHIR evaluation tracked separately)"
)]

use std::collections::BTreeMap;

use crate::fhir::MappedSubject;
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// A validated FHIR→openEHR mapping definition (the `definition` JSON stored in
/// `fhir_mapping.definition`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FhirMappingDefinition {
    /// The FHIR resource type this mapping consumes (`Observation`, `Patient`, …).
    pub resource_type: String,
    /// The FHIR profile canonical URL this mapping binds (matched against the
    /// resource's `meta.profile`). Absent = the default mapping for the type.
    #[serde(default)]
    pub profile_url: Option<String>,
    /// The openEHR template (OPT) the built COMPOSITION targets.
    pub template_id: String,
    /// How to resolve the target EHR's subject from the FHIR resource.
    pub subject: SubjectMapping,
    /// Seed FLAT keys written before the entries (composition context defaults:
    /// `ctx/language`, `ctx/territory`, `ctx/composer_name`, `ctx/time`, …). An
    /// entry may overwrite a seed key.
    #[serde(default)]
    pub context: Map<String, Value>,
    /// The field-binding entries.
    pub entries: Vec<MappingEntry>,
}

/// How the connector resolves the target EHR's subject from the resource.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectMapping {
    /// `FHIRPath`-lite path to the subject identifier string (e.g.
    /// `subject.reference` on an Observation, or `id` on a Patient).
    pub reference_path: String,
    /// The openEHR subject-id namespace to record on the resolved EHR.
    pub namespace: String,
    /// An optional prefix to strip from the resolved reference (e.g.
    /// `Patient/` → the bare logical id).
    #[serde(default)]
    pub strip_prefix: Option<String>,
}

/// One field binding: read `fhir_path` (or a `constant`) out of the resource and
/// write it to `openehr_path` (a template-relative FLAT key), shaped by
/// `transform`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MappingEntry {
    /// The template-relative openEHR FLAT path (the `id[:i]/…` key; a
    /// value-shaping transform appends the `|suffix`).
    pub openehr_path: String,
    /// The `FHIRPath`-lite source path (omitted for a `constant`).
    #[serde(default)]
    pub fhir_path: Option<String>,
    /// A literal value to write verbatim to `openehr_path` (ignores `fhir_path`).
    #[serde(default)]
    pub constant: Option<Value>,
    /// The value-shaping transform (default: plain text/scalar).
    #[serde(default)]
    pub transform: Transform,
    /// For a `coded` transform: FHIR code-system URL → openEHR `terminology_id`.
    /// The key `*` is the fallback for any unmatched system.
    #[serde(default)]
    pub code_map: BTreeMap<String, String>,
    /// When true, an absent source value is an error (else the entry is skipped).
    #[serde(default)]
    pub required: bool,
}

/// How an entry's source value is shaped into FLAT leaf(s).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Transform {
    /// Plain scalar → the bare FLAT key (`DV_TEXT` value, or any bare leaf).
    #[default]
    Text,
    /// An ISO-8601 date/date-time string → the bare FLAT key (`DV_DATE_TIME` etc.).
    Date,
    /// A quantity: the source number → `{openehr_path}|magnitude`, the unit →
    /// `{openehr_path}|unit` (from `unit_path` in the resource, or the literal
    /// `unit`).
    Quantity {
        /// `FHIRPath`-lite path to the unit string.
        #[serde(default)]
        unit_path: Option<String>,
        /// A literal unit string (wins over `unit_path`).
        #[serde(default)]
        unit: Option<String>,
    },
    /// A coded value: the source code → `{openehr_path}|code`, the resolved
    /// `terminology_id` → `{openehr_path}|terminology`, and (optional) the
    /// display → `{openehr_path}|value`.
    Coded {
        /// `FHIRPath`-lite path to the code's system URL (looked up in `code_map`).
        #[serde(default)]
        system_path: Option<String>,
        /// `FHIRPath`-lite path to the human-readable display text.
        #[serde(default)]
        display_path: Option<String>,
        /// Cross-terminology translation of the code before it is written
        /// (resolved by the orchestrator through the terminology seam).
        #[serde(default)]
        translate: Option<CodeTranslate>,
    },
}

/// A `coded` entry's translation request: translate the source code into
/// `target_system` before writing it, via FHIR `ConceptMap/$translate`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeTranslate {
    /// The FHIR code-system URL to translate INTO (the `targetsystem`
    /// parameter; also the `code_map` key that names the leaf's
    /// `terminology_id`).
    pub target_system: String,
    /// An explicit `ConceptMap` canonical URL (the `url` parameter); absent =
    /// the terminology server picks from its registered maps.
    #[serde(default)]
    pub concept_map: Option<String>,
}

/// One code the orchestrator must translate before [`build_flat`] applies.
///
/// Carries the source coding, the target system, and the openEHR terminology
/// token the entry's `code_map` binds the target to (the routing key for the
/// terminology seam), for one `translate`-declaring entry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TranslationRequest {
    /// The source code's system URL (from the entry's `system_path`).
    pub system: String,
    /// The source code.
    pub code: String,
    /// The FHIR system URL to translate into.
    pub target_system: String,
    /// The explicit `ConceptMap` URL, when the entry pins one.
    pub concept_map: Option<String>,
    /// The openEHR `terminology_id` the entry's `code_map` resolves
    /// `target_system` to (`None` when the map has no binding — the
    /// orchestrator then routes to its default provider).
    pub route_terminology: Option<String>,
}

/// A resolved translation: the target code (and display, when the server
/// returned one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslatedCode {
    /// The translated code in the target system.
    pub code: String,
    /// The target concept's display text.
    pub display: Option<String>,
}

/// The resolved answers [`build_flat`] reads, keyed by
/// `(system, code, target_system)`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeTranslations(pub BTreeMap<(String, String, String), TranslatedCode>);

impl CodeTranslations {
    /// Looks up the translation for one source coding into one target system.
    #[must_use]
    pub fn get(&self, system: &str, code: &str, target_system: &str) -> Option<&TranslatedCode> {
        self.0
            .get(&(system.to_owned(), code.to_owned(), target_system.to_owned()))
    }
}

/// A FHIR→openEHR mapping transform failure.
#[derive(Debug, thiserror::Error)]
pub enum FhirMapError {
    /// A required source field was absent (or not a scalar where one was
    /// expected).
    #[error("required FHIR field '{fhir_path}' (→ '{openehr_path}') is absent")]
    MissingField {
        /// The `FHIRPath`-lite source path.
        fhir_path: String,
        /// The FLAT target path.
        openehr_path: String,
    },
    /// A source value was not the scalar kind the transform needs.
    #[error("FHIR field '{fhir_path}' (→ '{openehr_path}') is not a {expected}")]
    WrongType {
        /// The `FHIRPath`-lite source path.
        fhir_path: String,
        /// The FLAT target path.
        openehr_path: String,
        /// The expected scalar kind (`number`, `string`, …).
        expected: &'static str,
    },
    /// The resource carried no subject reference at the configured path.
    #[error("FHIR resource has no subject reference at '{0}'")]
    MissingSubject(String),
    /// A `coded` entry was declared without a `fhir_path`.
    #[error("coded mapping entry for '{0}' has no fhir_path")]
    CodedWithoutSource(String),
    /// A `translate`-declaring entry's source coding had no system to
    /// translate from (no `system_path`, or nothing at it).
    #[error("coded mapping entry for '{0}' declares translate but resolves no source system")]
    TranslateWithoutSystem(String),
    /// A required entry's code could not be translated into the target
    /// system (the terminology seam returned no equivalent concept).
    #[error(
        "required code '{code}' ({system}) has no translation into '{target_system}' \
         (→ '{openehr_path}')"
    )]
    Untranslatable {
        /// The source code's system URL.
        system: String,
        /// The source code.
        code: String,
        /// The FHIR system URL the entry translates into.
        target_system: String,
        /// The FLAT target path.
        openehr_path: String,
    },
    /// The reverse transform could not flatten the COMPOSITION (its
    /// `WebTemplate` walk failed) — a server-side error (the stored
    /// COMPOSITION should always flatten).
    #[error("could not flatten COMPOSITION for reverse mapping: {0}")]
    Reverse(String),
    /// The same failure with its cause intact (RFC 0201): the flattener's own
    /// error says WHICH node defeated the walk, which a string cannot be
    /// matched on.
    #[error("could not flatten COMPOSITION for reverse mapping: {0}")]
    ReverseFailed(String, #[source] openehr_sdt::flat::error::FlatError),
}

/// Resolve a **`FHIRPath`-lite** dot-path against a JSON value.
///
/// Grammar (a deliberate subset of `FHIRPath`,
/// <https://hl7.org/fhirpath/> §5.1 `first()` / §5.2 `where()`):
///
/// ```text
/// path    := segment ('.' segment)*
/// segment := name index? | index | 'first()' | 'where(' path '=' literal ')'
/// name    := [A-Za-z_][A-Za-z0-9_]*
/// index   := '[' digits ']'
/// literal := '\'' chars '\'' | digits ('.' digits)? | 'true' | 'false'
/// ```
///
/// A `name` selects an object field; an `index` selects an array element;
/// `first()` selects an array's first element; `where(path = literal)` on an
/// array selects the FIRST element whose `path` equals the literal (on an
/// object, the object itself when it matches). `FHIRPath` proper evaluates
/// `where()` over collections — this single-value subset takes the first
/// match, because a FLAT leaf is one value. So
/// `component.where(code.coding[0].code = '8480-6').valueQuantity.value`,
/// `code.coding.where(system = 'http://loinc.org').code`, and
/// `meta.profile[0]` all resolve. Anything richer (other functions, unions,
/// arithmetic) is out of scope and yields `None`.
#[must_use]
pub fn resolve<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in split_steps(path)? {
        if seg == "first()" {
            cur = cur.as_array()?.first()?;
        } else if let Some(inner) = seg.strip_prefix("where(").and_then(|r| r.strip_suffix(')')) {
            let (lhs, rhs) = parse_condition(inner)?;
            cur = match cur {
                Value::Array(items) => items
                    .iter()
                    .find(|item| resolve(item, lhs).is_some_and(|v| *v == rhs))?,
                other if resolve(other, lhs).is_some_and(|v| *v == rhs) => other,
                _ => return None,
            };
        } else {
            let (name, index) = parse_segment(seg)?;
            if !name.is_empty() {
                cur = cur.get(name)?;
            }
            if let Some(i) = index {
                cur = cur.get(i)?;
            }
        }
    }
    Some(cur)
}

/// Splits a path on `.` at depth zero — dots inside `where(…)` parentheses or
/// single-quoted literals do not split. `None` on unbalanced parens/quotes.
fn split_steps(path: &str) -> Option<Vec<&str>> {
    let mut steps = Vec::new();
    let mut depth = 0_u32;
    let mut in_quote = false;
    let mut start = 0;
    for (i, c) in path.char_indices() {
        match c {
            '\'' => in_quote = !in_quote,
            '(' if !in_quote => depth += 1,
            ')' if !in_quote => depth = depth.checked_sub(1)?,
            '.' if !in_quote && depth == 0 => {
                steps.push(path.get(start..i)?);
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 || in_quote {
        return None;
    }
    steps.push(path.get(start..)?);
    Some(steps)
}

/// Parses a `where()` condition body — `path = literal` — into the comparison
/// path and the literal's JSON value.
fn parse_condition(inner: &str) -> Option<(&str, Value)> {
    // The `=` split must ignore any `=` inside the quoted literal, so scan at
    // quote depth zero.
    let mut in_quote = false;
    let eq = inner.char_indices().find_map(|(i, c)| match c {
        '\'' => {
            in_quote = !in_quote;
            None
        }
        '=' if !in_quote => Some(i),
        _ => None,
    })?;
    let lhs = inner.get(..eq)?.trim();
    let raw = inner.get(eq + 1..)?.trim();
    if lhs.is_empty() || raw.is_empty() {
        return None;
    }
    let rhs = if let Some(s) = raw.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        Value::String(s.to_owned())
    } else if raw == "true" || raw == "false" {
        Value::Bool(raw == "true")
    } else {
        serde_json::from_str::<serde_json::Number>(raw)
            .ok()
            .map(Value::Number)?
    };
    Some((lhs, rhs))
}

/// Splits one path segment into its `(name, index)` parts.
///
/// `component[0]` → `("component", Some(0))`, `reference` → `("reference",
/// None)`; `None` for a malformed segment (unbalanced/broken index). Shared
/// with the reverse transform's writer ([`super::reverse`]).
#[must_use]
pub fn parse_segment(seg: &str) -> Option<(&str, Option<usize>)> {
    match seg.split_once('[') {
        None => Some((seg, None)),
        Some((name, rest)) => {
            let digits = rest.strip_suffix(']')?;
            let idx = digits.parse::<usize>().ok()?;
            Some((name, Some(idx)))
        }
    }
}

/// Builds the FLAT map for a resource under a mapping definition.
///
/// Seeds the `context` keys, then each entry's produced leaf(s).
/// `translations` carries the orchestrator's resolved [`TranslationRequest`]
/// answers (empty when no entry declares `translate`).
///
/// # Errors
/// Returns [`FhirMapError`] when a mapping entry cannot be applied to the
/// resource (missing/mistyped member, unresolvable path, an untranslatable
/// required code).
pub fn build_flat(
    resource: &Value,
    def: &FhirMappingDefinition,
    translations: &CodeTranslations,
) -> Result<Map<String, Value>, FhirMapError> {
    let mut flat = Map::new();
    for (key, value) in &def.context {
        flat.insert(key.clone(), value.clone());
    }
    for entry in &def.entries {
        apply_entry(resource, entry, translations, &mut flat)?;
    }
    Ok(flat)
}

/// Enumerates the translations `def` needs for `resource`.
///
/// One request per `translate`-declaring coded entry whose source code +
/// system resolve, deduplicated and ordered; the orchestrator resolves each
/// through the terminology seam and hands the answers to [`build_flat`].
///
/// # Errors
/// Returns [`FhirMapError::TranslateWithoutSystem`] when a `translate` entry
/// resolves a code but no source system, and
/// [`FhirMapError::MissingField`] when a required entry's code is absent —
/// the same refusals [`build_flat`] would reach, surfaced before any
/// terminology call.
pub fn collect_translations(
    resource: &Value,
    def: &FhirMappingDefinition,
) -> Result<Vec<TranslationRequest>, FhirMapError> {
    let mut requests = std::collections::BTreeSet::new();
    for entry in &def.entries {
        let Transform::Coded {
            system_path,
            translate: Some(translate),
            ..
        } = &entry.transform
        else {
            continue;
        };
        if entry.constant.is_some() {
            continue;
        }
        let fhir_path = entry
            .fhir_path
            .as_deref()
            .ok_or_else(|| FhirMapError::CodedWithoutSource(entry.openehr_path.clone()))?;
        let Some(code) = resolve(resource, fhir_path).and_then(Value::as_str) else {
            if entry.required {
                return Err(FhirMapError::MissingField {
                    fhir_path: fhir_path.to_owned(),
                    openehr_path: entry.openehr_path.clone(),
                });
            }
            continue;
        };
        let system = system_path
            .as_deref()
            .and_then(|p| resolve(resource, p))
            .and_then(Value::as_str)
            .ok_or_else(|| FhirMapError::TranslateWithoutSystem(entry.openehr_path.clone()))?;
        requests.insert(TranslationRequest {
            system: system.to_owned(),
            code: code.to_owned(),
            target_system: translate.target_system.clone(),
            concept_map: translate.concept_map.clone(),
            route_terminology: entry.code_map.get(&translate.target_system).cloned(),
        });
    }
    Ok(requests.into_iter().collect())
}

/// Apply one entry, writing zero or more FLAT leaves into `flat`.
fn apply_entry(
    resource: &Value,
    entry: &MappingEntry,
    translations: &CodeTranslations,
    flat: &mut Map<String, Value>,
) -> Result<(), FhirMapError> {
    // A constant short-circuits any source read (any transform).
    if let Some(c) = &entry.constant {
        flat.insert(entry.openehr_path.clone(), c.clone());
        return Ok(());
    }
    match &entry.transform {
        Transform::Text | Transform::Date => {
            if let Some(v) = source(resource, entry)? {
                flat.insert(entry.openehr_path.clone(), scalar(v, entry)?);
            }
            Ok(())
        }
        Transform::Quantity { unit_path, unit } => {
            apply_quantity(resource, entry, unit_path.as_deref(), unit.as_deref(), flat)
        }
        Transform::Coded {
            system_path,
            display_path,
            translate,
        } => {
            let coded = CodedTransform {
                system_path: system_path.as_deref(),
                display_path: display_path.as_deref(),
                translate: translate.as_ref(),
            };
            apply_coded(resource, entry, &coded, translations, flat)
        }
    }
}

/// The `Coded` transform's own three fields, as a borrowed view.
struct CodedTransform<'a> {
    system_path: Option<&'a str>,
    display_path: Option<&'a str>,
    translate: Option<&'a CodeTranslate>,
}

/// Writes a `DV_QUANTITY`'s `|magnitude` and `|unit` leaves.
///
/// A literal `unit` wins over a `unit_path` read.
///
/// # Errors
/// The source-read and numeric-conversion rejections of the entry.
fn apply_quantity(
    resource: &Value,
    entry: &MappingEntry,
    unit_path: Option<&str>,
    unit: Option<&str>,
    flat: &mut Map<String, Value>,
) -> Result<(), FhirMapError> {
    if let Some(v) = source(resource, entry)? {
        let magnitude = number(v, entry)?;
        flat.insert(format!("{}|magnitude", entry.openehr_path), magnitude);
    }
    let unit_val = unit.map(str::to_owned).or_else(|| {
        unit_path
            .and_then(|p| resolve(resource, p))
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    if let Some(u) = unit_val {
        flat.insert(format!("{}|unit", entry.openehr_path), Value::String(u));
    }
    Ok(())
}

/// Writes a coded leaf's `|code`, `|terminology` and `|value` pairs, applying
/// a configured concept translation first.
///
/// # Errors
/// [`FhirMapError::CodedWithoutSource`] for an entry with no FHIR path,
/// [`FhirMapError::MissingField`] for a required entry whose source is absent,
/// and the translation rejections of [`apply_translated_code`].
fn apply_coded(
    resource: &Value,
    entry: &MappingEntry,
    coded: &CodedTransform<'_>,
    translations: &CodeTranslations,
    flat: &mut Map<String, Value>,
) -> Result<(), FhirMapError> {
    let fhir_path = entry
        .fhir_path
        .as_deref()
        .ok_or_else(|| FhirMapError::CodedWithoutSource(entry.openehr_path.clone()))?;
    let Some(code) = resolve(resource, fhir_path).and_then(Value::as_str) else {
        if entry.required {
            return Err(FhirMapError::MissingField {
                fhir_path: fhir_path.to_owned(),
                openehr_path: entry.openehr_path.clone(),
            });
        }
        return Ok(());
    };
    let system = coded
        .system_path
        .and_then(|p| resolve(resource, p))
        .and_then(Value::as_str);
    if let Some(translate) = coded.translate {
        return apply_translated_code(entry, translate, system, code, translations, flat);
    }
    flat.insert(format!("{}|code", entry.openehr_path), json!(code));
    let terminology = system
        .and_then(|s| entry.code_map.get(s))
        .or_else(|| entry.code_map.get("*"))
        .map_or("local", String::as_str);
    flat.insert(
        format!("{}|terminology", entry.openehr_path),
        json!(terminology),
    );
    if let Some(dp) = coded.display_path
        && let Some(display) = resolve(resource, dp).and_then(Value::as_str)
    {
        flat.insert(format!("{}|value", entry.openehr_path), json!(display));
    }
    Ok(())
}

/// Writes the TRANSLATED concept for a coded leaf.
///
/// No equivalent concept means writing the SOURCE code under the TARGET
/// terminology, which would be a silently wrong clinical value — so a required
/// entry refuses and an optional one writes nothing. The translated concept's
/// own display wins, because the source resource's `display_path` text names
/// the SOURCE concept.
///
/// # Errors
/// [`FhirMapError::TranslateWithoutSystem`] when the source coding names no
/// system, and [`FhirMapError::Untranslatable`] for a required entry with no
/// equivalent concept.
fn apply_translated_code(
    entry: &MappingEntry,
    translate: &CodeTranslate,
    system: Option<&str>,
    code: &str,
    translations: &CodeTranslations,
    flat: &mut Map<String, Value>,
) -> Result<(), FhirMapError> {
    let system =
        system.ok_or_else(|| FhirMapError::TranslateWithoutSystem(entry.openehr_path.clone()))?;
    let Some(translated) = translations.get(system, code, &translate.target_system) else {
        if entry.required {
            return Err(FhirMapError::Untranslatable {
                system: system.to_owned(),
                code: code.to_owned(),
                target_system: translate.target_system.clone(),
                openehr_path: entry.openehr_path.clone(),
            });
        }
        return Ok(());
    };
    flat.insert(
        format!("{}|code", entry.openehr_path),
        json!(translated.code),
    );
    let terminology = entry
        .code_map
        .get(&translate.target_system)
        .or_else(|| entry.code_map.get("*"))
        .map_or("local", String::as_str);
    flat.insert(
        format!("{}|terminology", entry.openehr_path),
        json!(terminology),
    );
    if let Some(display) = &translated.display {
        flat.insert(format!("{}|value", entry.openehr_path), json!(display));
    }
    Ok(())
}

/// Resolve an entry's source value, honouring `required`.
fn source<'a>(
    resource: &'a Value,
    entry: &MappingEntry,
) -> Result<Option<&'a Value>, FhirMapError> {
    let Some(fhir_path) = entry.fhir_path.as_deref() else {
        return Ok(None);
    };
    match resolve(resource, fhir_path) {
        Some(v) => Ok(Some(v)),
        None if entry.required => Err(FhirMapError::MissingField {
            fhir_path: fhir_path.to_owned(),
            openehr_path: entry.openehr_path.clone(),
        }),
        None => Ok(None),
    }
}

/// Coerce a source value to a scalar FLAT value (string / number / bool); reject
/// objects and arrays (a FLAT leaf is a single value).
fn scalar(v: &Value, entry: &MappingEntry) -> Result<Value, FhirMapError> {
    match v {
        Value::String(_) | Value::Number(_) | Value::Bool(_) => Ok(v.clone()),
        _ => Err(FhirMapError::WrongType {
            fhir_path: entry.fhir_path.clone().unwrap_or_default(),
            openehr_path: entry.openehr_path.clone(),
            expected: "scalar",
        }),
    }
}

/// Coerce a source value to a JSON number (FLAT `|magnitude`); accept a numeric
/// string too (FHIR sometimes serialises decimals as strings).
fn number(v: &Value, entry: &MappingEntry) -> Result<Value, FhirMapError> {
    if v.is_number() {
        return Ok(v.clone());
    }
    if let Some(s) = v.as_str()
        && let Ok(n) = s.parse::<f64>()
        && let Some(num) = serde_json::Number::from_f64(n)
    {
        return Ok(Value::Number(num));
    }
    Err(FhirMapError::WrongType {
        fhir_path: entry.fhir_path.clone().unwrap_or_default(),
        openehr_path: entry.openehr_path.clone(),
        expected: "number",
    })
}

/// Extract the target EHR subject from the resource per the mapping's
/// `subject` rule.
/// # Errors
/// Returns [`FhirMapError::MissingSubject`] when the mapping's subject
/// reference path resolves to nothing usable.
pub fn extract_subject(
    resource: &Value,
    def: &FhirMappingDefinition,
) -> Result<MappedSubject, FhirMapError> {
    let raw = resolve(resource, &def.subject.reference_path)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| FhirMapError::MissingSubject(def.subject.reference_path.clone()))?;
    let id = def
        .subject
        .strip_prefix
        .as_deref()
        .and_then(|p| raw.strip_prefix(p))
        .unwrap_or(raw);
    Ok(MappedSubject {
        id: id.to_owned(),
        namespace: def.subject.namespace.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use openehr_its::opt14;
    use openehr_sdt::flat::convert::composition_from_flat;
    use openehr_sdt::flat::webtemplate::builder::build_web_template;
    use openehr_sdt::flat::webtemplate::model::WebTemplate;
    use serde_json::json;

    use super::super::feeder_audit::{
        ORIGINATING_SYSTEM, feeder_audit, inject_feeder_audit, resource_id, resource_version,
    };
    use super::*;

    fn def(v: Value) -> FhirMappingDefinition {
        serde_json::from_value(v).expect("valid mapping definition")
    }

    // ── `FHIRPath`-lite resolver ───────────────────────────────────────────────
    #[test]
    fn resolve_navigates_fields_and_indices() {
        let r = json!({
            "resourceType": "Observation",
            "code": { "coding": [{ "system": "http://loinc.org", "code": "8480-6" }] },
            "component": [
                { "valueQuantity": { "value": 120, "unit": "mm[Hg]" } },
                { "valueQuantity": { "value": 80, "unit": "mm[Hg]" } }
            ],
            "meta": { "profile": ["http://example.org/bp"] }
        });
        assert_eq!(
            resolve(&r, "code.coding[0].code").and_then(Value::as_str),
            Some("8480-6")
        );
        assert_eq!(
            resolve(&r, "component[1].valueQuantity.value").and_then(Value::as_i64),
            Some(80)
        );
        assert_eq!(
            resolve(&r, "meta.profile[0]").and_then(Value::as_str),
            Some("http://example.org/bp")
        );
        assert!(resolve(&r, "component[5].valueQuantity").is_none());
        assert!(resolve(&r, "missing.field").is_none());
    }

    // ── build_flat: quantity + constant + context ────────────────────────────
    #[test]
    fn build_flat_observation_quantities() {
        let d = def(json!({
            "resource_type": "Observation",
            "template_id": "ehrbase_blood_pressure_simple.de.v0",
            "subject": { "reference_path": "subject.reference", "namespace": "fhir", "strip_prefix": "Patient/" },
            "context": { "ctx/language": "en", "ctx/territory": "US", "ctx/composer_name": "fhir-connector" },
            "entries": [
                { "openehr_path": "encounter_training_sample/blood_pressure_training_sample:0/systolic",
                  "fhir_path": "component[0].valueQuantity.value",
                  "transform": { "kind": "quantity", "unit_path": "component[0].valueQuantity.unit" } },
                { "openehr_path": "encounter_training_sample/blood_pressure_training_sample:0/diastolic",
                  "fhir_path": "component[1].valueQuantity.value",
                  "transform": { "kind": "quantity", "unit": "mm[Hg]" } },
                { "openehr_path": "ctx/time", "fhir_path": "effectiveDateTime", "transform": { "kind": "date" } }
            ]
        }));
        let resource = json!({
            "resourceType": "Observation",
            "subject": { "reference": "Patient/p-1" },
            "effectiveDateTime": "2026-02-03T04:05:06Z",
            "component": [
                { "valueQuantity": { "value": 120, "unit": "mm[Hg]" } },
                { "valueQuantity": { "value": 80 } }
            ]
        });
        let flat = build_flat(&resource, &d, &CodeTranslations::default()).expect("build_flat");
        assert_eq!(
            flat["encounter_training_sample/blood_pressure_training_sample:0/systolic|magnitude"],
            json!(120)
        );
        assert_eq!(
            flat["encounter_training_sample/blood_pressure_training_sample:0/systolic|unit"],
            json!("mm[Hg]")
        );
        assert_eq!(
            flat["encounter_training_sample/blood_pressure_training_sample:0/diastolic|magnitude"],
            json!(80)
        );
        assert_eq!(
            flat["encounter_training_sample/blood_pressure_training_sample:0/diastolic|unit"],
            json!("mm[Hg]")
        );
        assert_eq!(flat["ctx/time"], json!("2026-02-03T04:05:06Z"));
        assert_eq!(flat["ctx/language"], json!("en"));
    }

    // ── build_flat: coded value via code_map (terminology seam) ──────────────
    #[test]
    fn build_flat_coded_uses_code_map() {
        let d = def(json!({
            "resource_type": "Condition",
            "template_id": "t",
            "subject": { "reference_path": "subject.reference", "namespace": "fhir" },
            "entries": [
                { "openehr_path": "x/problem",
                  "fhir_path": "code.coding[0].code",
                  "transform": { "kind": "coded", "system_path": "code.coding[0].system", "display_path": "code.coding[0].display" },
                  "code_map": { "http://snomed.info/sct": "SNOMED-CT" } }
            ]
        }));
        let resource = json!({
            "resourceType": "Condition",
            "code": { "coding": [{ "system": "http://snomed.info/sct", "code": "73211009", "display": "Diabetes mellitus" }] }
        });
        let flat = build_flat(&resource, &d, &CodeTranslations::default()).expect("build_flat");
        assert_eq!(flat["x/problem|code"], json!("73211009"));
        assert_eq!(flat["x/problem|terminology"], json!("SNOMED-CT"));
        assert_eq!(flat["x/problem|value"], json!("Diabetes mellitus"));
    }

    #[test]
    fn build_flat_coded_unmapped_system_defaults_local() {
        let d = def(json!({
            "resource_type": "Condition", "template_id": "t",
            "subject": { "reference_path": "id", "namespace": "fhir" },
            "entries": [
                { "openehr_path": "x/problem", "fhir_path": "code.coding[0].code",
                  "transform": { "kind": "coded", "system_path": "code.coding[0].system" } }
            ]
        }));
        let resource =
            json!({ "code": { "coding": [{ "system": "http://unknown", "code": "z" }] } });
        let flat = build_flat(&resource, &d, &CodeTranslations::default()).expect("build_flat");
        assert_eq!(flat["x/problem|terminology"], json!("local"));
    }

    #[test]
    fn required_missing_field_errors() {
        let d = def(json!({
            "resource_type": "Observation", "template_id": "t",
            "subject": { "reference_path": "id", "namespace": "fhir" },
            "entries": [ { "openehr_path": "x/y", "fhir_path": "absent.here", "required": true } ]
        }));
        let err = build_flat(&json!({}), &d, &CodeTranslations::default()).unwrap_err();
        assert!(matches!(err, FhirMapError::MissingField { .. }));
    }

    #[test]
    fn extract_subject_strips_prefix() {
        let d = def(json!({
            "resource_type": "Observation", "template_id": "t",
            "subject": { "reference_path": "subject.reference", "namespace": "fhir", "strip_prefix": "Patient/" },
            "entries": []
        }));
        let s = extract_subject(&json!({ "subject": { "reference": "Patient/abc" } }), &d)
            .expect("subject");
        assert_eq!(s.id, "abc");
        assert_eq!(s.namespace, "fhir");
    }

    #[test]
    fn extract_subject_absent_errors() {
        let d = def(json!({
            "resource_type": "Observation", "template_id": "t",
            "subject": { "reference_path": "subject.reference", "namespace": "fhir" },
            "entries": []
        }));
        assert!(matches!(
            extract_subject(&json!({}), &d),
            Err(FhirMapError::MissingSubject(_))
        ));
    }

    // ── End-to-end: FHIR → FLAT → canonical COMPOSITION ──────────────────────
    // Justification for the corpus template: `ehrbase_blood_pressure_simple.de.v0`
    // (vendored under corpus/fixtures/service) is a small, single-OBSERVATION
    // blood-pressure template whose flat json-ids (systolic/diastolic
    // magnitude+unit) map cleanly to a FHIR BP Observation's components — the
    // canonical starter mapping. The FLAT keys used here were taken from
    // `to_flat(example_composition(wt))` for this OPT, so they are exactly the
    // template's committable json-id leaves.
    fn bp_web_template() -> WebTemplate {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../corpus/fixtures/service/knowledge/opt/ehrbase_blood_pressure_simple.de.v0.opt",
        );
        let xml = std::fs::read_to_string(path).expect("read opt");
        let opt = opt14::from_xml(&xml).expect("parse opt");
        build_web_template(&opt).expect("build wt")
    }

    #[test]
    fn observation_maps_to_canonical_composition_with_feeder_audit() {
        let wt = bp_web_template();
        let d = def(json!({
            "resource_type": "Observation",
            "template_id": "ehrbase_blood_pressure_simple.de.v0",
            "subject": { "reference_path": "subject.reference", "namespace": "fhir", "strip_prefix": "Patient/" },
            "context": {
                "ctx/language": "en", "ctx/territory": "US",
                "ctx/composer_name": "fhir-connector", "ctx/time": "2026-02-03T04:05:06Z"
            },
            "entries": [
                { "openehr_path": "encounter_training_sample/blood_pressure_training_sample:0/systolic",
                  "fhir_path": "component[0].valueQuantity.value",
                  "transform": { "kind": "quantity", "unit_path": "component[0].valueQuantity.unit" } },
                { "openehr_path": "encounter_training_sample/blood_pressure_training_sample:0/diastolic",
                  "fhir_path": "component[1].valueQuantity.value",
                  "transform": { "kind": "quantity", "unit_path": "component[1].valueQuantity.unit" } }
            ]
        }));
        let resource = json!({
            "resourceType": "Observation",
            "id": "bp-1",
            "meta": { "versionId": "3" },
            "subject": { "reference": "Patient/p-1" },
            "component": [
                { "valueQuantity": { "value": 118, "unit": "mm[Hg]" } },
                { "valueQuantity": { "value": 76, "unit": "mm[Hg]" } }
            ]
        });

        let flat = build_flat(&resource, &d, &CodeTranslations::default()).expect("build_flat");
        // Fixed ctx/time (ITS-REST simplified_formats master04 §Context) so the
        // built composition is deterministic under test.
        let mut comp = composition_from_flat(&flat, &wt, "2024-01-01T00:00:00Z")
            .expect("from_flat builds a composition");
        inject_feeder_audit(
            &mut comp,
            feeder_audit(
                "Observation",
                &resource_id(&resource, "Observation"),
                resource_version(&resource).as_deref(),
                "2026-02-03T04:05:06Z",
            ),
        );

        // The result is a canonical COMPOSITION that deserialises as openehr-rm.
        assert_eq!(comp["_type"], json!("COMPOSITION"));
        let parsed: openehr_rm::v1_2::composition::composition::Composition =
            openehr_its::json::from_canonical_value(&comp).expect("deserialises as RM Composition");
        assert!(
            parsed.feeder_audit.is_some(),
            "FEEDER_AUDIT present on the composition"
        );
        assert_eq!(
            comp["feeder_audit"]["originating_system_audit"]["system_id"],
            json!(ORIGINATING_SYSTEM)
        );
        // The mapped systolic magnitude survives the FLAT→RM build.
        let s = comp.to_string();
        assert!(s.contains("118"), "systolic magnitude present");
        assert!(s.contains("76"), "diastolic magnitude present");
    }

    // ── `FHIRPath`-lite: where() + first() ───────────────────────────────────
    #[test]
    fn resolve_where_filters_and_first_selects() {
        let r = json!({
            "code": { "coding": [
                { "system": "http://loinc.org", "code": "8480-6" },
                { "system": "http://snomed.info/sct", "code": "271649006" }
            ]},
            "component": [
                { "code": { "coding": [{ "code": "8480-6" }] },
                  "valueQuantity": { "value": 120 } },
                { "code": { "coding": [{ "code": "8462-4" }] },
                  "valueQuantity": { "value": 80 } }
            ]
        });
        assert_eq!(
            resolve(
                &r,
                "code.coding.where(system = 'http://snomed.info/sct').code"
            )
            .and_then(Value::as_str),
            Some("271649006")
        );
        assert_eq!(
            resolve(
                &r,
                "component.where(code.coding[0].code = '8462-4').valueQuantity.value"
            )
            .and_then(Value::as_i64),
            Some(80),
            "the filter picks by content, not position"
        );
        assert_eq!(
            resolve(&r, "code.coding.first().code").and_then(Value::as_str),
            Some("8480-6")
        );
        assert!(
            resolve(
                &r,
                "component.where(code.coding[0].code = 'absent').valueQuantity"
            )
            .is_none()
        );
        // A numeric literal compares as a number, not a string.
        assert!(
            resolve(
                &r,
                "component.where(valueQuantity.value = 120).code.coding[0].code"
            )
            .is_some()
        );
        // Malformed paths yield None, never a panic.
        assert!(resolve(&r, "code.where(system = 'unterminated").is_none());
        assert!(resolve(&r, "code.where(system 'no-eq')").is_none());
        assert!(resolve(&r, "code.coding.where()").is_none());
    }

    // ── translate: collect + apply through CodeTranslations ─────────────────
    fn translate_def(required: bool) -> FhirMappingDefinition {
        def(json!({
            "resource_type": "Condition", "template_id": "t",
            "subject": { "reference_path": "id", "namespace": "fhir" },
            "entries": [
                { "openehr_path": "x/problem",
                  "fhir_path": "code.coding[0].code",
                  "required": required,
                  "transform": { "kind": "coded",
                    "system_path": "code.coding[0].system",
                    "display_path": "code.coding[0].display",
                    "translate": { "target_system": "http://snomed.info/sct" } },
                  "code_map": { "http://snomed.info/sct": "SNOMED-CT" } }
            ]
        }))
    }

    fn loinc_condition() -> Value {
        json!({ "code": { "coding": [{
            "system": "http://loinc.org", "code": "8480-6", "display": "Systolic BP"
        }] } })
    }

    #[test]
    fn collect_translations_enumerates_the_route() {
        let requests =
            collect_translations(&loinc_condition(), &translate_def(false)).expect("collect");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].system, "http://loinc.org");
        assert_eq!(requests[0].code, "8480-6");
        assert_eq!(requests[0].target_system, "http://snomed.info/sct");
        assert_eq!(requests[0].route_terminology.as_deref(), Some("SNOMED-CT"));
    }

    #[test]
    fn build_flat_translated_code_wins_over_source() {
        let mut translations = CodeTranslations::default();
        translations.0.insert(
            (
                "http://loinc.org".to_owned(),
                "8480-6".to_owned(),
                "http://snomed.info/sct".to_owned(),
            ),
            TranslatedCode {
                code: "271649006".to_owned(),
                display: Some("Systolic blood pressure".to_owned()),
            },
        );
        let flat =
            build_flat(&loinc_condition(), &translate_def(false), &translations).expect("build");
        assert_eq!(flat["x/problem|code"], json!("271649006"));
        assert_eq!(flat["x/problem|terminology"], json!("SNOMED-CT"));
        assert_eq!(
            flat["x/problem|value"],
            json!("Systolic blood pressure"),
            "the translated concept's display wins over display_path"
        );
    }

    #[test]
    fn build_flat_untranslated_required_refuses_and_optional_writes_nothing() {
        let err = build_flat(
            &loinc_condition(),
            &translate_def(true),
            &CodeTranslations::default(),
        )
        .unwrap_err();
        assert!(matches!(err, FhirMapError::Untranslatable { .. }));

        let flat = build_flat(
            &loinc_condition(),
            &translate_def(false),
            &CodeTranslations::default(),
        )
        .expect("build");
        assert!(
            !flat.contains_key("x/problem|code"),
            "an untranslatable optional entry writes NO leaf — never the source code \
             under the target terminology"
        );
    }

    #[test]
    fn translate_without_source_system_refuses() {
        let d = def(json!({
            "resource_type": "Condition", "template_id": "t",
            "subject": { "reference_path": "id", "namespace": "fhir" },
            "entries": [
                { "openehr_path": "x/problem", "fhir_path": "code.coding[0].code",
                  "transform": { "kind": "coded",
                    "translate": { "target_system": "http://snomed.info/sct" } } }
            ]
        }));
        let resource = json!({ "code": { "coding": [{ "code": "8480-6" }] } });
        assert!(matches!(
            collect_translations(&resource, &d).unwrap_err(),
            FhirMapError::TranslateWithoutSystem(_)
        ));
        assert!(matches!(
            build_flat(&resource, &d, &CodeTranslations::default()).unwrap_err(),
            FhirMapError::TranslateWithoutSystem(_)
        ));
    }
}
