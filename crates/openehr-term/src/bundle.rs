// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! openEHR terminology loader + lookup API (TERM 3.1.0).
//!
//! The vendored terminology XML in `assets/` is embedded with `include_str!`
//! and parsed once into the generated TERM data model ([`Terminology`],
//! [`CodeSet`], [`TerminologyGroup`], …) plus lookup indexes, cached in a
//! [`LazyLock`]. It carries the RM-mandated terminology groups, the external
//! code sets (ISO country / language, IANA character-set / media-type), and
//! the property↔unit table.
//!
//! # Error model
//!
//! [`parse_terminology`] and [`parse_property_units`] are fallible, but the
//! public bundle exposes only infallible lookups: the assets are vendored and
//! embedded at compile time, so a parse failure is a corrupt build artifact and
//! [`openehr`] panics once on first access instead.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use openehr_base::containers::present;
use openehr_base::v1_3::prelude::Iso8601Date;

use crate::v3_1::terminology::code::Code;
use crate::v3_1::terminology::code_set::CodeSet;
use crate::v3_1::terminology::terminology::Terminology;
use crate::v3_1::terminology::terminology_concept::TerminologyConcept;
use crate::v3_1::terminology::terminology_group::TerminologyGroup;
use crate::v3_1::terminology::terminology_status::TerminologyStatus;

// Vendored openEHR terminology assets, embedded at compile time (no runtime I/O).
const EN_XML: &str = include_str!("../assets/en/openehr_terminology.xml");
const ES_XML: &str = include_str!("../assets/es/openehr_terminology.xml");
const JA_XML: &str = include_str!("../assets/ja/openehr_terminology.xml");
const PT_XML: &str = include_str!("../assets/pt/openehr_terminology.xml");
const ZH_XML: &str = include_str!("../assets/zh/openehr_terminology.xml");
const EXTERNAL_XML: &str = include_str!("../assets/openehr_external_terminologies.xml");
const PROPERTY_UNITS_XML: &str = include_str!("../assets/PropertyUnitData.xml");

/// ISO 639-1 code of the canonical (authoritative) terminology language.
///
/// Codes and group membership are language-independent — only rubrics differ —
/// so validity checks resolve against the English bundle.
const CANONICAL_LANG: &str = "en";

/// The languages whose `openehr_terminology.xml` is vendored, paired with the
/// embedded XML. `en` is the canonical bundle (see [`CANONICAL_LANG`]).
const LANGUAGE_ASSETS: &[(&str, &str)] = &[
    ("en", EN_XML),
    ("es", ES_XML),
    ("ja", JA_XML),
    ("pt", PT_XML),
    ("zh", ZH_XML),
];

/// An error parsing a vendored terminology asset.
#[derive(Debug, thiserror::Error)]
pub enum TerminologyError {
    /// The XML was not well-formed.
    #[error("terminology XML is not well-formed: {0}")]
    Xml(#[from] roxmltree::Error),
    /// The XML was well-formed but not the expected terminology shape.
    #[error("malformed terminology asset: {0}")]
    Malformed(String),
}

/// A property from `PropertyUnitData.xml` (an openEHR measurement property with
/// its allowed units), e.g. `Length` (openEHR `property`-group code `122`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// Local id within `PropertyUnitData.xml` (the `Unit.property_id` foreign key).
    pub id: String,
    /// Human-readable property name, e.g. `"Length"`.
    pub text: String,
    /// The concept code of this property in the openEHR `property` vocabulary
    /// (the `property`-group `id`, e.g. `"122"`), or empty if unmapped.
    pub openehr_code: String,
}

/// A unit of measure for a [`Property`], from `PropertyUnitData.xml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// The [`Property::id`] this unit belongs to.
    pub property_id: String,
    /// The unit symbol as displayed, e.g. `"cm"`.
    pub text: String,
    /// The unit's full name, e.g. `"centimeter"`.
    pub name: String,
    /// The UCUM code for this unit, if present.
    pub ucum: Option<String>,
    /// Whether this is the primary (base) unit for its property.
    pub primary: bool,
}

/// The property↔unit table parsed from `PropertyUnitData.xml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PropertyUnits {
    /// All measurement properties.
    pub properties: Vec<Property>,
    /// All units, each referencing a property via [`Unit::property_id`].
    pub units: Vec<Unit>,
}

/// One parsed language's terminology plus lookup indexes.
#[derive(Debug)]
struct LanguageBundle {
    terminology: Terminology,
    /// group `openehr_id` → index into `terminology.vocabularies`.
    group_by_id: HashMap<String, usize>,
    /// code-set `openehr_id` → index into `terminology.code_sets`.
    code_set_by_id: HashMap<String, usize>,
    /// group `openehr_id` → (concept `id` → index into that group's `concepts`).
    ///
    /// A `BTreeMap` because [`OpenehrTerminology::concept_rubric`] scans every
    /// group and hash order is implementation-defined.
    group_codes: BTreeMap<String, HashMap<String, usize>>,
}

/// The `TERMINOLOGY.vocabularies` list as a slice — the attribute is optional
/// (`0..1`), so `None` (the vendored asset declaring no `<group>`) reads as the
/// empty slice.
fn vocabularies(t: &Terminology) -> &[TerminologyGroup] {
    t.vocabularies.as_deref().unwrap_or_default()
}

/// The `TERMINOLOGY.code_sets` list as a slice (optional attribute, see
/// [`vocabularies`]).
fn code_sets(t: &Terminology) -> &[CodeSet] {
    t.code_sets.as_deref().unwrap_or_default()
}

/// The `TERMINOLOGY_GROUP.concepts` list as a slice (optional attribute, see
/// [`vocabularies`]).
fn concepts(g: &TerminologyGroup) -> &[TerminologyConcept] {
    g.concepts.as_deref().unwrap_or_default()
}

/// The `CODE_SET.codes` list as a slice (optional attribute, see
/// [`vocabularies`]).
fn code_list(cs: &CodeSet) -> &[Code] {
    cs.codes.as_deref().unwrap_or_default()
}

impl LanguageBundle {
    fn index(terminology: Terminology) -> Self {
        let mut group_by_id = HashMap::new();
        let mut group_codes: BTreeMap<String, HashMap<String, usize>> = BTreeMap::new();
        for (gi, group) in vocabularies(&terminology).iter().enumerate() {
            group_by_id.insert(group.openehr_id.clone(), gi);
            let codes = concepts(group)
                .iter()
                .enumerate()
                .map(|(ci, c)| (c.id.clone(), ci))
                .collect();
            group_codes.insert(group.openehr_id.clone(), codes);
        }
        let code_set_by_id = code_sets(&terminology)
            .iter()
            .enumerate()
            .map(|(i, cs)| (cs.openehr_id.clone(), i))
            .collect();
        Self {
            terminology,
            group_by_id,
            code_set_by_id,
            group_codes,
        }
    }
}

/// The parsed, indexed openEHR terminology bundle (TERM 3.1.0).
///
/// Obtain the shared, cached instance with [`openehr`]. All lookups are
/// infallible: an unknown group, code, or language yields `None`/`false`/an
/// empty slice rather than an error.
#[derive(Debug)]
pub struct OpenehrTerminology {
    /// Parsed terminology per language (keyed by ISO 639-1 code); `en` is canonical.
    languages: HashMap<String, Arc<LanguageBundle>>,
    /// The canonical (`en`) bundle, held directly so its presence is a
    /// construction guarantee rather than a key assumption.
    canonical: Arc<LanguageBundle>,
    /// External code sets (countries, languages, character sets, media types).
    external: Vec<CodeSet>,
    /// external code-set `openehr_id` → index into `external`.
    external_by_id: HashMap<String, usize>,
    /// external code-set `external_id` (e.g. `"ISO_639-1"`) → index into `external`.
    external_by_external_id: HashMap<String, usize>,
    /// external code-set `openehr_id` → set of valid code values (O(1) membership).
    external_codes: HashMap<String, HashSet<String>>,
    /// The property↔unit table.
    property_units: PropertyUnits,
}

impl OpenehrTerminology {
    /// Parse and index every vendored asset. Fails only on a corrupt asset.
    fn load() -> Result<Self, TerminologyError> {
        let mut languages = HashMap::new();
        let mut canonical = None;
        for &(lang, xml) in LANGUAGE_ASSETS {
            let bundle = Arc::new(LanguageBundle::index(parse_terminology(xml)?));
            if lang == CANONICAL_LANG {
                canonical = Some(Arc::clone(&bundle));
            }
            languages.insert(lang.to_owned(), bundle);
        }
        let Some(canonical) = canonical else {
            return Err(TerminologyError::Malformed(format!(
                "no vendored asset for the canonical language {CANONICAL_LANG}"
            )));
        };

        let external = parse_terminology(EXTERNAL_XML)?
            .code_sets
            .unwrap_or_default();
        let mut external_by_id = HashMap::new();
        let mut external_by_external_id = HashMap::new();
        let mut external_codes = HashMap::new();
        for (i, cs) in external.iter().enumerate() {
            external_by_id.insert(cs.openehr_id.clone(), i);
            if let Some(ext_id) = &cs.external_id {
                external_by_external_id.insert(ext_id.clone(), i);
            }
            external_codes.insert(
                cs.openehr_id.clone(),
                code_list(cs).iter().map(|c| c.value.clone()).collect(),
            );
        }

        let property_units = parse_property_units(PROPERTY_UNITS_XML)?;

        Ok(Self {
            languages,
            canonical,
            external,
            external_by_id,
            external_by_external_id,
            external_codes,
            property_units,
        })
    }

    /// The canonical (English) bundle, held as its own field by [`Self::load`].
    fn canonical(&self) -> &LanguageBundle {
        &self.canonical
    }

    // ── Internal vocabularies (openehr_terminology.xml <group>) ──────────────

    /// The canonical (English) parsed terminology model.
    #[must_use]
    pub fn terminology(&self) -> &Terminology {
        &self.canonical().terminology
    }

    /// The vocabulary group with openEHR id `group_id` (e.g. `"null_flavours"`).
    #[must_use]
    pub fn group(&self, group_id: &str) -> Option<&TerminologyGroup> {
        let b = self.canonical();
        let &gi = b.group_by_id.get(group_id)?;
        vocabularies(&b.terminology).get(gi)
    }

    /// The openEHR id of the group whose display name is `name`
    /// (e.g. `"null flavours"` → `"null_flavours"`).
    #[must_use]
    pub fn group_id(&self, name: &str) -> Option<&str> {
        vocabularies(&self.canonical().terminology)
            .iter()
            .find(|g| g.name == name)
            .map(|g| g.openehr_id.as_str())
    }

    /// Every concept in `group_id` (canonical language); empty if unknown.
    #[must_use]
    pub fn concepts_in_group(&self, group_id: &str) -> &[TerminologyConcept] {
        match self.group(group_id) {
            Some(g) => concepts(g),
            None => &[],
        }
    }

    /// The rubric (display text) of concept `code` in `group_id`, in language
    /// `lang` (ISO 639-1). `None` if the language, group, or code is unknown.
    #[must_use]
    pub fn rubric(&self, group_id: &str, code: &str, lang: &str) -> Option<&str> {
        let b = self.languages.get(lang)?;
        let &gi = b.group_by_id.get(group_id)?;
        let &ci = b.group_codes.get(group_id)?.get(code)?;
        let concept = concepts(vocabularies(&b.terminology).get(gi)?).get(ci)?;
        Some(concept.rubric.as_str())
    }

    /// The rubric (display text) of concept `code` in language `lang`, searched
    /// across every group; `None` if the language or code is unknown.
    ///
    /// openEHR concept codes are globally unique integers, so the first match
    /// is the concept. A display helper for `DV_CODED_TEXT.value` (RM
    /// data_types §DV_CODED_TEXT); validation uses the group-scoped
    /// [`Self::rubric`] / [`Self::is_valid_code`].
    #[must_use]
    pub fn concept_rubric(&self, code: &str, lang: &str) -> Option<&str> {
        let b = self.languages.get(lang)?;
        for (group_id, codes) in &b.group_codes {
            if let Some(&ci) = codes.get(code)
                && let Some(&gi) = b.group_by_id.get(group_id)
                && let Some(concept) = vocabularies(&b.terminology)
                    .get(gi)
                    .and_then(|v| concepts(v).get(ci))
            {
                return Some(concept.rubric.as_str());
            }
        }
        None
    }

    /// Whether `code` is a valid concept in `group_id` (language-independent).
    #[must_use]
    pub fn is_valid_code(&self, group_id: &str, code: &str) -> bool {
        self.canonical()
            .group_codes
            .get(group_id)
            .is_some_and(|codes| codes.contains_key(code))
    }

    // ── Convenience validators for the RM-mandated groups ─────────────────────

    /// `COMPOSITION.category` — the `composition_category` group
    /// (`431` persistent, `451` episodic, `433` event, `815` report).
    #[must_use]
    pub fn is_valid_composition_category(&self, code: &str) -> bool {
        self.is_valid_code("composition_category", code)
    }

    /// Alias of [`Self::is_valid_composition_category`] (the RM attribute is `category`).
    #[must_use]
    pub fn is_valid_category(&self, code: &str) -> bool {
        self.is_valid_composition_category(code)
    }

    /// `EVENT_CONTEXT.setting` — the `setting` group.
    #[must_use]
    pub fn is_valid_setting(&self, code: &str) -> bool {
        self.is_valid_code("setting", code)
    }

    /// `ELEMENT.null_flavour` — the `null_flavours` group
    /// (`271` no information, `253` unknown, `272` masked, `273` not applicable).
    #[must_use]
    pub fn is_valid_null_flavour(&self, code: &str) -> bool {
        self.is_valid_code("null_flavours", code)
    }

    /// `ISM_TRANSITION.current_state` — the `instruction_states` group.
    #[must_use]
    pub fn is_valid_instruction_state(&self, code: &str) -> bool {
        self.is_valid_code("instruction_states", code)
    }

    /// `ISM_TRANSITION.transition` — the `instruction_transitions` group.
    #[must_use]
    pub fn is_valid_instruction_transition(&self, code: &str) -> bool {
        self.is_valid_code("instruction_transitions", code)
    }

    /// `PARTICIPATION.function` — the `participation_function` group.
    #[must_use]
    pub fn is_valid_participation_function(&self, code: &str) -> bool {
        self.is_valid_code("participation_function", code)
    }

    /// `PARTICIPATION.mode` — the `participation_mode` group.
    #[must_use]
    pub fn is_valid_participation_mode(&self, code: &str) -> bool {
        self.is_valid_code("participation_mode", code)
    }

    /// `ATTESTATION.reason` — the `attestation_reason` group
    /// (`240` signed, `648` witnessed).
    #[must_use]
    pub fn is_valid_attestation_reason(&self, code: &str) -> bool {
        self.is_valid_code("attestation_reason", code)
    }

    /// `VERSION.lifecycle_state` — the `version_lifecycle_state` group.
    ///
    /// NOTE: SPECPR-51 — code `532` is `complete` here and `completed` in
    /// `instruction_states`.
    #[must_use]
    pub fn is_valid_version_lifecycle_state(&self, code: &str) -> bool {
        self.is_valid_code("version_lifecycle_state", code)
    }

    /// `AUDIT_DETAILS.change_type` — the `audit_change_type` group.
    #[must_use]
    pub fn is_valid_audit_change_type(&self, code: &str) -> bool {
        self.is_valid_code("audit_change_type", code)
    }

    /// `PARTY_RELATED.relationship` — the `subject_relationship` group.
    #[must_use]
    pub fn is_valid_subject_relationship(&self, code: &str) -> bool {
        self.is_valid_code("subject_relationship", code)
    }

    /// `TERM_MAPPING.purpose` — the `term_mapping_purpose` group.
    #[must_use]
    pub fn is_valid_term_mapping_purpose(&self, code: &str) -> bool {
        self.is_valid_code("term_mapping_purpose", code)
    }

    /// `EVENT.math_function` — the `event_math_function` group.
    #[must_use]
    pub fn is_valid_event_math_function(&self, code: &str) -> bool {
        self.is_valid_code("event_math_function", code)
    }

    /// `DV_QUANTITY.property` (and related) — the `property` group.
    #[must_use]
    pub fn is_valid_property(&self, code: &str) -> bool {
        self.is_valid_code("property", code)
    }

    // ── Internal code sets (openehr_terminology.xml <codeset>) ────────────────

    /// An internal code set by openEHR id (e.g. `"normal_statuses"`,
    /// `"compression_algorithms"`, `"integrity_check_algorithms"`).
    #[must_use]
    pub fn code_set(&self, openehr_id: &str) -> Option<&CodeSet> {
        let b = self.canonical();
        let &i = b.code_set_by_id.get(openehr_id)?;
        code_sets(&b.terminology).get(i)
    }

    /// `DV_ORDERED.normal_status` — the `normal_statuses` code set
    /// (`HHH`/`HH`/`H`/`N`/`L`/`LL`/`LLL`).
    #[must_use]
    pub fn is_valid_normal_status(&self, code: &str) -> bool {
        self.code_set("normal_statuses")
            .is_some_and(|cs| code_list(cs).iter().any(|c| c.value == code))
    }

    // ── External code sets (openehr_external_terminologies.xml) ──────────────

    /// Every external code set (`"countries"`, `"languages"`, `"character_sets"`,
    /// `"media_types"`), in vendored order.
    #[must_use]
    pub fn external_code_sets(&self) -> &[CodeSet] {
        &self.external
    }

    /// An external code set by its openEHR id (`"countries"`, `"languages"`,
    /// `"character_sets"`, `"media_types"`).
    #[must_use]
    pub fn external_code_set(&self, openehr_id: &str) -> Option<&CodeSet> {
        let &i = self.external_by_id.get(openehr_id)?;
        self.external.get(i)
    }

    /// The external code set published under `external_id` (e.g. `"ISO_639-1"`),
    /// if one exists.
    #[must_use]
    pub fn external_terminology(&self, external_id: &str) -> Option<&CodeSet> {
        let &i = self.external_by_external_id.get(external_id)?;
        self.external.get(i)
    }

    /// Whether an external code system with the given id exists — matched against
    /// both the openEHR id (`"languages"`) and the published id (`"ISO_639-1"`).
    #[must_use]
    pub fn has_external_terminology(&self, id: &str) -> bool {
        self.external_by_external_id.contains_key(id) || self.external_by_id.contains_key(id)
    }

    /// Whether `code` is a member of the external code set `openehr_id`.
    #[must_use]
    pub fn is_valid_external_code(&self, openehr_id: &str, code: &str) -> bool {
        self.external_codes
            .get(openehr_id)
            .is_some_and(|codes| codes.contains(code))
    }

    /// A valid ISO 639-1 language code (e.g. `"en"`). Used by `DV_TEXT.language`,
    /// `COMPOSITION.language`, `EHR_STATUS`, etc.
    #[must_use]
    pub fn is_valid_language(&self, code: &str) -> bool {
        self.is_valid_external_code("languages", code)
    }

    /// A valid ISO 3166-1 country code (e.g. `"US"`).
    #[must_use]
    pub fn is_valid_country(&self, code: &str) -> bool {
        self.is_valid_external_code("countries", code)
    }

    /// A valid IANA character-set name (`DV_TEXT.encoding`).
    #[must_use]
    pub fn is_valid_character_set(&self, code: &str) -> bool {
        self.is_valid_external_code("character_sets", code)
    }

    /// A valid IANA media type (`DV_MULTIMEDIA.media_type`).
    #[must_use]
    pub fn is_valid_media_type(&self, code: &str) -> bool {
        self.is_valid_external_code("media_types", code)
    }

    // ── Property ↔ unit data (PropertyUnitData.xml) ───────────────────────────

    /// The parsed property↔unit table.
    #[must_use]
    pub fn property_units(&self) -> &PropertyUnits {
        &self.property_units
    }

    /// The measurement property whose openEHR `property`-group code is
    /// `openehr_code` (e.g. `"122"` → `Length`).
    #[must_use]
    pub fn property_by_openehr_code(&self, openehr_code: &str) -> Option<&Property> {
        self.property_units
            .properties
            .iter()
            .find(|p| p.openehr_code == openehr_code)
    }

    /// The units defined for the property whose openEHR `property`-group code is
    /// `openehr_code`; empty if the property is unknown.
    #[must_use]
    pub fn units_for_property(&self, openehr_code: &str) -> Vec<&Unit> {
        match self.property_by_openehr_code(openehr_code) {
            Some(p) => self
                .property_units
                .units
                .iter()
                .filter(|u| u.property_id == p.id)
                .collect(),
            None => Vec::new(),
        }
    }
}

/// The shared, cached openEHR terminology bundle (TERM 3.1.0).
static OPENEHR: LazyLock<OpenehrTerminology> = LazyLock::new(|| {
    // NOTE: the assets are vendored and embedded at compile time, so a parse
    // failure is a corrupt build artifact, which this panic reports.
    #[expect(
        clippy::expect_used,
        reason = "the assets are `include_str!`-embedded at compile time and parsed by the crate's own tests, so an Err here is a corrupt build artifact, not a runtime condition; the message is should-phrased per the Book ch9 shape"
    )]
    OpenehrTerminology::load()
        .expect("vendored openEHR terminology assets should parse (build-time invariant)")
});

/// The shared openEHR terminology bundle (TERM 3.1.0).
///
/// Parsed and indexed once from the compile-time-embedded vendored assets on
/// first call, then cached for the process lifetime.
#[must_use]
pub fn openehr() -> &'static OpenehrTerminology {
    &OPENEHR
}

/// Parse an `openehr_terminology.xml` / `openehr_external_terminologies.xml`
/// document (a `<terminology>` root of `<codeset>` and `<group>` elements) into
/// the generated [`Terminology`] model.
///
/// # Errors
///
/// Returns [`TerminologyError`] if the XML is not well-formed or its root is not
/// `<terminology>`.
pub fn parse_terminology(xml: &str) -> Result<Terminology, TerminologyError> {
    let doc = roxmltree::Document::parse(xml)?;
    let root = doc.root_element();
    if root.tag_name().name() != "terminology" {
        return Err(TerminologyError::Malformed(format!(
            "expected <terminology> root, found <{}>",
            root.tag_name().name()
        )));
    }

    let mut code_sets = Vec::new();
    let mut vocabularies = Vec::new();
    for child in root.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "codeset" => code_sets.push(parse_code_set(&child)),
            "group" => vocabularies.push(parse_group(&child)),
            // Ignore any future/unknown element kinds rather than fail hard.
            _ => {}
        }
    }

    Ok(Terminology {
        name: attr(&root, "name").unwrap_or_default().to_owned(),
        language: attr(&root, "language").unwrap_or_default().to_owned(),
        code_sets: present(code_sets),
        vocabularies: present(vocabularies),
        version: attr(&root, "version").map(ToOwned::to_owned),
        date: attr(&root, "date").map(|d| Iso8601Date {
            value: d.to_owned(),
        }),
    })
}

fn parse_code_set(node: &roxmltree::Node) -> CodeSet {
    let codes = node
        .children()
        .filter(roxmltree::Node::is_element)
        .filter(|c| c.tag_name().name() == "code")
        .map(|c| Code {
            value: attr(&c, "value").unwrap_or_default().to_owned(),
            description: attr(&c, "description").map(ToOwned::to_owned),
            status: status_attr(&c),
        })
        .collect();
    CodeSet {
        name: attr(node, "name").unwrap_or_default().to_owned(),
        openehr_id: attr(node, "openehr_id").unwrap_or_default().to_owned(),
        issuer: attr(node, "issuer").unwrap_or_default().to_owned(),
        codes: present(codes),
        external_id: attr(node, "external_id").map(ToOwned::to_owned),
        status: status_attr(node),
    }
}

fn parse_group(node: &roxmltree::Node) -> TerminologyGroup {
    let concepts = node
        .children()
        .filter(roxmltree::Node::is_element)
        .filter(|c| c.tag_name().name() == "concept")
        .map(|c| TerminologyConcept {
            id: attr(&c, "id").unwrap_or_default().to_owned(),
            rubric: attr(&c, "rubric").unwrap_or_default().to_owned(),
            status: status_attr(&c),
        })
        .collect();
    TerminologyGroup {
        name: attr(node, "name").unwrap_or_default().to_owned(),
        concepts: present(concepts),
        openehr_id: attr(node, "openehr_id").unwrap_or_default().to_owned(),
        status: status_attr(node),
    }
}

/// Returns the optional `status` attribute the terminology XSDs declare on
/// `codeset`/`code`/`group`/`concept`.
///
/// [`TerminologyStatus::from_wire`] is total, so an out-of-set token survives
/// as [`TerminologyStatus::Other`]. The pinned 3.1.0 assets declare no `status`
/// attributes, so this reads `None` there.
fn status_attr(node: &roxmltree::Node) -> Option<TerminologyStatus> {
    attr(node, "status").map(TerminologyStatus::from_wire)
}

/// Parse `PropertyUnitData.xml` (a `<PropertyUnits>` root of `<Property>` and
/// `<Unit>` elements) into a [`PropertyUnits`] table.
///
/// # Errors
///
/// Returns [`TerminologyError`] if the XML is not well-formed or its root is not
/// `<PropertyUnits>`.
pub fn parse_property_units(xml: &str) -> Result<PropertyUnits, TerminologyError> {
    let doc = roxmltree::Document::parse(xml)?;
    let root = doc.root_element();
    if root.tag_name().name() != "PropertyUnits" {
        return Err(TerminologyError::Malformed(format!(
            "expected <PropertyUnits> root, found <{}>",
            root.tag_name().name()
        )));
    }

    let mut properties = Vec::new();
    let mut units = Vec::new();
    for child in root.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "Property" => properties.push(Property {
                id: attr(&child, "id").unwrap_or_default().to_owned(),
                text: attr(&child, "Text").unwrap_or_default().to_owned(),
                openehr_code: attr(&child, "openEHR").unwrap_or_default().to_owned(),
            }),
            "Unit" => units.push(Unit {
                property_id: attr(&child, "property_id").unwrap_or_default().to_owned(),
                text: attr(&child, "Text").unwrap_or_default().to_owned(),
                name: attr(&child, "name").unwrap_or_default().to_owned(),
                ucum: attr(&child, "UCUM").map(ToOwned::to_owned),
                primary: attr(&child, "primary") == Some("true"),
            }),
            _ => {}
        }
    }

    Ok(PropertyUnits { properties, units })
}

/// An attribute value by local name (namespace-insensitive).
fn attr<'a>(node: &roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == name)
        .map(|a| a.value())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── RM-mandated group validators (codes pulled from the vendored XML) ─────

    #[test]
    fn composition_category_codes() {
        let t = openehr();
        // 431 persistent, 451 episodic, 433 event, 815 report.
        assert!(t.is_valid_composition_category("431"));
        assert!(t.is_valid_category("451")); // alias
        assert!(t.is_valid_composition_category("433"));
        assert!(t.is_valid_composition_category("815"));
        assert!(!t.is_valid_composition_category("999"));
    }

    #[test]
    fn null_flavour_codes_and_rubric() {
        let t = openehr();
        assert!(t.is_valid_null_flavour("271")); // no information
        assert!(t.is_valid_null_flavour("253")); // unknown
        assert!(t.is_valid_null_flavour("273")); // not applicable
        assert!(!t.is_valid_null_flavour("999"));
        assert_eq!(
            t.rubric("null_flavours", "271", "en"),
            Some("no information")
        );
        assert_eq!(
            t.rubric("null_flavours", "273", "en"),
            Some("not applicable")
        );
    }

    #[test]
    fn setting_codes_and_rubric() {
        let t = openehr();
        assert!(t.is_valid_setting("225")); // home
        assert!(t.is_valid_setting("238")); // other care
        assert!(!t.is_valid_setting("999"));
        assert_eq!(t.rubric("setting", "225", "en"), Some("home"));
    }

    #[test]
    fn instruction_states_and_transitions() {
        let t = openehr();
        assert!(t.is_valid_instruction_state("245")); // active
        assert!(t.is_valid_instruction_state("532")); // completed
        assert!(!t.is_valid_instruction_state("999"));
        assert!(t.is_valid_instruction_transition("535")); // initiate
        assert!(t.is_valid_instruction_transition("548")); // finish
        assert!(!t.is_valid_instruction_transition("999"));
    }

    #[test]
    fn participation_function_and_mode() {
        let t = openehr();
        assert!(t.is_valid_participation_function("253")); // unknown (only member)
        assert!(!t.is_valid_participation_function("216"));
        assert!(t.is_valid_participation_mode("216")); // face-to-face communication
        assert_eq!(
            t.rubric("participation_mode", "216", "en"),
            Some("face-to-face communication")
        );
        assert!(!t.is_valid_participation_mode("999"));
    }

    #[test]
    fn attestation_reason_and_audit_change_type() {
        let t = openehr();
        assert!(t.is_valid_attestation_reason("240")); // signed
        assert!(t.is_valid_attestation_reason("648")); // witnessed
        assert!(!t.is_valid_attestation_reason("999"));
        assert!(t.is_valid_audit_change_type("249")); // creation
        assert!(t.is_valid_audit_change_type("250")); // amendment
    }

    #[test]
    fn version_lifecycle_state_specpr51_quirk() {
        let t = openehr();
        assert!(t.is_valid_version_lifecycle_state("532"));
        assert!(t.is_valid_version_lifecycle_state("553")); // incomplete
        // SPECPR-51: code 532 is "complete" in version_lifecycle_state but
        // "completed" in instruction_states — group-scoped rubrics prove it.
        assert_eq!(
            t.rubric("version_lifecycle_state", "532", "en"),
            Some("complete")
        );
        assert_eq!(
            t.rubric("instruction_states", "532", "en"),
            Some("completed")
        );
    }

    // ── General lookups ──────────────────────────────────────────────────────

    #[test]
    fn general_is_valid_code() {
        let t = openehr();
        assert!(t.is_valid_code("subject_relationship", "0")); // self
        assert!(t.is_valid_code("event_math_function", "146")); // mean
        assert!(t.is_valid_property("122")); // Length
        assert!(!t.is_valid_code("event_math_function", "999"));
        assert!(!t.is_valid_code("no_such_group", "146"));
    }

    #[test]
    fn group_id_by_name_and_group_by_id() {
        let t = openehr();
        assert_eq!(t.group_id("null flavours"), Some("null_flavours"));
        assert_eq!(
            t.group_id("composition category"),
            Some("composition_category")
        );
        assert_eq!(t.group_id("no such group"), None);
        assert!(t.group("null_flavours").is_some());
        assert!(t.group("no_such_group").is_none());
    }

    #[test]
    fn concepts_in_group_counts() {
        let t = openehr();
        assert_eq!(t.concepts_in_group("composition_category").len(), 4);
        assert_eq!(t.concepts_in_group("null_flavours").len(), 4);
        assert!(t.concepts_in_group("no_such_group").is_empty());
    }

    #[test]
    fn rubric_multi_language_and_unknowns() {
        let t = openehr();
        assert_eq!(
            t.rubric("null_flavours", "271", "es"),
            Some("sin información")
        );
        assert_eq!(t.rubric("null_flavours", "273", "ja"), Some("該当なし"));
        assert_eq!(
            t.rubric("null_flavours", "271", "pt"),
            Some("sem informação")
        );
        // Unknown language / unknown code / unknown group → None.
        assert_eq!(t.rubric("null_flavours", "271", "de"), None);
        assert_eq!(t.rubric("null_flavours", "999", "en"), None);
        assert_eq!(t.rubric("no_such_group", "271", "en"), None);
    }

    // ── Internal + external code sets ─────────────────────────────────────────

    #[test]
    fn normal_status_code_set() {
        let t = openehr();
        assert!(t.is_valid_normal_status("N"));
        assert!(t.is_valid_normal_status("HHH"));
        assert!(!t.is_valid_normal_status("X"));
        assert!(t.code_set("normal_statuses").is_some());
    }

    #[test]
    fn external_languages_countries_charsets_media() {
        let t = openehr();
        assert!(t.is_valid_language("en"));
        assert!(t.is_valid_language("es"));
        assert!(!t.is_valid_language("zz"));
        assert!(t.is_valid_country("US"));
        assert!(t.is_valid_country("AF"));
        assert!(!t.is_valid_country("ZZ"));
        assert!(t.is_valid_character_set("UTF-8"));
        assert!(!t.is_valid_character_set("NOT-A-CHARSET"));
        assert!(t.is_valid_media_type("audio/G722"));
    }

    #[test]
    fn external_terminology_existence() {
        let t = openehr();
        assert!(t.has_external_terminology("ISO_639-1")); // by external_id
        assert!(t.has_external_terminology("ISO_3166-1"));
        assert!(t.has_external_terminology("languages")); // by openehr_id
        assert!(!t.has_external_terminology("NO_SUCH_SYSTEM"));
        assert!(t.external_code_set("languages").is_some());
        assert_eq!(
            t.external_terminology("ISO_639-1")
                .map(|cs| cs.openehr_id.as_str()),
            Some("languages")
        );
    }

    // ── Property ↔ unit data ──────────────────────────────────────────────────

    #[test]
    fn property_units_lookup() {
        let t = openehr();
        let length = t.property_by_openehr_code("122").expect("Length property");
        assert_eq!(length.text, "Length");
        let units = t.units_for_property("122");
        assert!(!units.is_empty());
        // The primary SI unit for Length is the meter (symbol "m", UCUM "m").
        let meter = units
            .iter()
            .find(|u| u.text == "m")
            .expect("meter unit present");
        assert!(meter.primary);
        assert_eq!(meter.ucum.as_deref(), Some("m"));
        assert!(t.property_by_openehr_code("does-not-exist").is_none());
        assert!(t.units_for_property("does-not-exist").is_empty());
    }

    #[test]
    fn energy_dose_units_display_their_own_symbol() {
        // Before specifications-TERM 78edd7f59 the milligray row displayed "cGy".
        let units = openehr().units_for_property("508");
        let milligray = units
            .iter()
            .find(|u| u.name == "milligray")
            .expect("milligray unit present");
        assert_eq!(milligray.text, "mGy");
        assert_eq!(milligray.ucum.as_deref(), Some("mGy"));
        assert_eq!(units.iter().filter(|u| u.text == "cGy").count(), 1);
    }

    // ── Fallible parser (corrupt-asset path) ──────────────────────────────────

    #[test]
    fn status_attributes_parse_when_present_and_none_when_absent() {
        // The XSDs declare an optional `status` on codeset/code/group/concept
        // (assets/schema/openehr_terminology.xsd); the pinned assets omit it.
        let t = parse_terminology(
            r#"<terminology name="openehr" language="en">
                 <codeset issuer="x" openehr_id="cs" name="cs" status="active">
                   <code value="A" status="retired"/>
                   <code value="B"/>
                 </codeset>
                 <group name="g" openehr_id="g" status="trial">
                   <concept id="1" rubric="one" status="experimental"/>
                   <concept id="2" rubric="two"/>
                 </group>
               </terminology>"#,
        )
        .expect("synthetic terminology parses");
        let cs = &code_sets(&t)[0];
        assert_eq!(cs.status, Some(TerminologyStatus::Active));
        assert_eq!(code_list(cs)[0].status, Some(TerminologyStatus::Retired));
        assert_eq!(code_list(cs)[1].status, None);
        let g = &vocabularies(&t)[0];
        assert_eq!(g.status, Some(TerminologyStatus::Trial));
        // An out-of-set token is preserved, never dropped (from_wire is total).
        assert_eq!(
            concepts(g)[0].status,
            Some(TerminologyStatus::Other("experimental".to_owned()))
        );
        assert_eq!(concepts(g)[1].status, None);
    }

    #[test]
    fn vendored_assets_carry_no_status_attributes() {
        // Every parsed status is None at the 3.1.0 pin.
        let t = openehr();
        let no_status = |term: &Terminology| {
            code_sets(term)
                .iter()
                .all(|cs| cs.status.is_none() && code_list(cs).iter().all(|c| c.status.is_none()))
                && vocabularies(term)
                    .iter()
                    .all(|g| g.status.is_none() && concepts(g).iter().all(|c| c.status.is_none()))
        };
        assert!(no_status(t.terminology()));
        assert!(
            t.external_code_sets()
                .iter()
                .all(|cs| cs.status.is_none() && code_list(cs).iter().all(|c| c.status.is_none()))
        );
    }

    #[test]
    fn parser_rejects_wrong_root() {
        let err = parse_terminology("<wrong>text</wrong>").unwrap_err();
        assert!(matches!(err, TerminologyError::Malformed(_)));
        let err = parse_property_units("<terminology/>").unwrap_err();
        assert!(matches!(err, TerminologyError::Malformed(_)));
    }

    #[test]
    fn parser_rejects_malformed_xml() {
        assert!(matches!(
            parse_terminology("<terminology><group>").unwrap_err(),
            TerminologyError::Xml(_)
        ));
    }

    #[test]
    fn all_vendored_assets_load() {
        // Exercises the LazyLock init: every vendored asset parses.
        let t = openehr();
        assert_eq!(t.terminology().language, "en");
        assert!(t.languages.contains_key("zh"));
        assert!(!t.property_units().properties.is_empty());
    }
}
