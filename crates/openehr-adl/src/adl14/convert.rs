// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 → ADL 2 converter core: code planning, node coding, terminology
//! rebuild and constraint conversion.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion: our own design, its id-coded
//! numbering pinned by the `upgrade_from_14` corpus fixtures; a cited AOM2/ADL2
//! section pins the output form. The meta-data mapping follows `ADL1.4/masterAppB-extended_metadata.adoc`
//! §Standardised Items and `AOM2/master01-preface.adoc`, in
//! `crate::adl14::metadata`.

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;
use openehr_am::v2_4::aom2::terminology::archetype_term::ArchetypeTerm;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::aom2::terminology::value_set::ValueSet;

use crate::adl14::code_shift::{first_num, highest_ac_number, recode};
use crate::adl14::log::ConversionLog;
use crate::adl14::metadata::{ConversionRecord, convert_meta_data};
use crate::adl14::multiplicity::{elide_multiplicity, materialise_adl14_occurrences};
use crate::adl14::term_constraint::{CodeSpace, TermMinter, highest_at_number, rewrite_definition};
use crate::adl14::walk::{
    cco_data_mut, collect_local_value_codes, collect_node_codes, collect_slot_codes,
};
use crate::aom::access::{common_mut, tuple_primitive_node_ids_mut};
use crate::codes::{NodeCoding, is_new_at_level, specialisation_parent_from_code};
use crate::error::SyntaxError;
use crate::hrid::set_release_version;

/// Configuration for a 1.4→2 conversion (all spec-silent — our own design).
#[derive(Debug, Clone)]
pub struct ConvertConfig {
    /// The `adl_version` to stamp on the converted artefact.
    pub adl_version: String,
    /// The `rm_release` to stamp (1.4 sources carry no `rm_release`).
    pub rm_release: String,
    /// The code system of the converted archetype's nodes. At-coded ADL2 keeps
    /// every 1.4 code as written, so RM data recorded against the 1.4 archetype
    /// stays valid (`ADL2/master01-preface.adoc` §Backward Compatibility), and
    /// is the system openEHR systems must support
    /// (`org.openehr.am.aom2.adl_code_definitions` §Description); id-coded ADL2
    /// renumbers the nodes into id-codes (`ADL2/master07.04` §Node Identifier
    /// Codes), the form of the `upgrade_from_14` reference conversions.
    pub coding: NodeCoding,
    /// Per-terminology URI templates for synthesised term bindings; `{code}` is
    /// replaced by the external code. A terminology with no template gets a
    /// flagged fabricated fallback (`urn:adl14:<terminology>:<code>`).
    pub binding_uri_templates: BTreeMap<String, String>,
    /// Collapse specialised (dotted) codes to top level, producing a depth-0
    /// archetype, for a `-`-specialised root inlined standalone by a flattened OPT,
    /// whose lineage cannot be resolved (`ADL2/master09.02`; `AOM2/master03` VASID,
    /// VACSD). Off for plain source conversion. Our own design.
    pub collapse_specialised_codes: bool,
    /// The `"source_model"` the conversion record names; `None` names the
    /// source archetype's own identifier (an OPT root names its template).
    pub source_model: Option<String>,
    /// The `"tool"` the conversion record names: the converter's name and
    /// version.
    pub tool: String,
    /// The `"time"` the conversion record names, injectable so a conversion
    /// can be reproduced.
    pub time: jiff::Timestamp,
}

impl Default for ConvertConfig {
    fn default() -> Self {
        let mut binding_uri_templates = BTreeMap::new();
        // The `upgrade_from_14` fixtures bind `openehr` codes to
        // `http://openehr.org/id/{code}`.
        binding_uri_templates.insert(
            "openehr".to_owned(),
            "http://openehr.org/id/{code}".to_owned(),
        );
        Self {
            adl_version: "2.4.0".to_owned(),
            rm_release: "1.0.3".to_owned(),
            coding: NodeCoding::At,
            binding_uri_templates,
            collapse_specialised_codes: false,
            source_model: None,
            tool: format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")),
            time: jiff::Timestamp::now(),
        }
    }
}

/// A 1.4→2 conversion failure.
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    /// The 1.4 source did not parse.
    #[error("1.4 source did not parse: {0:?}")]
    Parse(Vec<SyntaxError>),
    /// The artefact kind is not a plain authored archetype (templates/OPTs are
    /// not yet converted here).
    #[error("1.4→2 conversion supports authored archetypes only: {0}")]
    Unsupported(String),
    /// A specialised 1.4 source introduces term codes that do not carry its
    /// specialisation depth (AOM 1.4 `master07-ontology_package.adoc`
    /// §Specialisation Depth), found by
    /// [`crate::adl14::specialised_codes::check_introduced_codes`].
    #[error(
        "the specialisation at depth {depth} introduces term codes its parent does not define \
         without {depth} '.' marker(s), which AOM 1.4 requires: {}",
        codes.join(", ")
    )]
    UndottedNewCodes {
        /// The child's specialisation depth.
        depth: usize,
        /// The offending codes, in code order.
        codes: Vec<String>,
    },
}

/// Parse a 1.4 `.adl` source and convert it to an ADL2 [`Archetype`].
///
/// # Errors
/// [`ConvertError::Parse`] if the source does not parse; [`ConvertError::Unsupported`]
/// for a non-archetype artefact kind.
pub fn parse_and_convert(
    src: &str,
    cfg: &ConvertConfig,
    log: &mut ConversionLog,
) -> Result<Archetype, ConvertError> {
    let art = crate::assemble::parse_artefact(src, crate::parse::Dialect::Adl14)
        .map_err(ConvertError::Parse)?;
    convert(&art, cfg, log)
}

/// Convert a 1.4-shaped [`Archetype`] into a spec-valid ADL2 archetype; a
/// specialised source is re-differentialised by
/// [`crate::adl14::differ::differentiate`].
///
/// # Errors
/// [`ConvertError::Unsupported`] for a non-archetype artefact kind.
pub fn convert(
    art: &Archetype,
    cfg: &ConvertConfig,
    log: &mut ConversionLog,
) -> Result<Archetype, ConvertError> {
    let data = match art {
        Archetype::AuthoredArchetype(b) => match b.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => d.clone(),
            AuthoredArchetype::Template(_) => {
                return Err(ConvertError::Unsupported("template".to_owned()));
            }
            AuthoredArchetype::OperationalTemplate(_) => {
                return Err(ConvertError::Unsupported("operational_template".to_owned()));
            }
        },
        Archetype::TemplateOverlay(_) => {
            return Err(ConvertError::Unsupported("template_overlay".to_owned()));
        }
    };

    let mut cx = Converter::new(cfg, log);
    let converted = cx.run(data);
    Ok(Archetype::AuthoredArchetype(Box::new(
        AuthoredArchetype::AuthoredArchetype(converted),
    )))
}

struct Converter<'a> {
    cfg: &'a ConvertConfig,
    /// The terminology-constraint minting state, which owns the log.
    minter: TermMinter<'a>,
    /// The ADL2 code space of the 1.4 value and value-set codes.
    codes: CollapseMaps,
    /// old node at-code → its ADL2 node code.
    node_map: BTreeMap<String, String>,
    /// Source at-codes referenced as a `local` *value* (they get an at-code
    /// term entry even when the same code is also a node id — a 1.4 at-code
    /// used as both an id and a value splits into an id-code + an at-code).
    value_at_codes: std::collections::BTreeSet<String>,
    /// New node ids already claimed by an occurrence — the VCOSU ground:
    /// ADL2 node ids are unique archetype-wide (`AOM2/master04.5` §Validity
    /// Rules: `C_OBJECT`), while 1.4 at-codes are only sibling-unique, so a
    /// reused code's second occurrence mints a fresh id.
    assigned_node_ids: std::collections::BTreeSet<String>,
    /// `(first-occurrence new id, freshly minted id)` per VCOSU re-mint — the
    /// terminology rebuild clones the definition/bindings of the first id
    /// under each minted one (VATDF: every used code must be defined).
    dup_mints: Vec<(String, String)>,
    /// Next free id-number for synthesised object nodes of an id-coded
    /// conversion (top-level id space); an at-coded one mints from the
    /// minter's at-code space.
    next_id: i64,
    /// The converted root node id (the terminology `concept_code`).
    root_id: String,
}

impl<'a> Converter<'a> {
    fn new(cfg: &'a ConvertConfig, log: &'a mut ConversionLog) -> Self {
        Self {
            cfg,
            minter: TermMinter::new(&cfg.binding_uri_templates, log),
            codes: CollapseMaps {
                coding: cfg.coding,
                collapse: cfg.collapse_specialised_codes,
                value_map: BTreeMap::new(),
                ac_map: BTreeMap::new(),
            },
            node_map: BTreeMap::new(),
            value_at_codes: std::collections::BTreeSet::new(),
            assigned_node_ids: std::collections::BTreeSet::new(),
            dup_mints: Vec::new(),
            next_id: 1,
            root_id: root_code(cfg.coding).to_owned(),
        }
    }

    fn run(&mut self, mut data: AuthoredArchetypeData) -> AuthoredArchetypeData {
        // 1. Build the node-id map + synthesis counters from the definition.
        crate::adl14::quantity_property::drop_quantity_properties(
            &mut data.definition,
            "",
            self.minter.log,
        );
        self.plan_codes(&data);
        // 2. Renumber node ids in document order: existing mapped, empty minted.
        self.renumber_nodes(&mut data.definition);
        // 3. Convert every terminology-code constraint (local/external/list).
        rewrite_definition(&mut data.definition, &mut self.minter, &mut self.codes, "");
        // 4. Write out the ADL 1.4 default occurrences where ADL 2 would infer a
        //    different one, then elide the RM-default cardinality/occurrences.
        //    Order matters: the materialisation reads the 1.4 cardinality, which
        //    the elision may drop.
        materialise_adl14_occurrences(&mut data.definition);
        elide_multiplicity(&mut data.definition);
        // 5. Rebuild the terminology (renumber keys, drop @internal, add synth).
        data.terminology = Box::new(self.rebuild_terminology(&data.terminology));
        // 6. Header/meta + description + version.
        self.stamp_meta(&mut data);
        data.is_differential = data.parent_archetype_id.is_some();
        data
    }

    // ── code planning ──────────────────────────────────────────────────────

    fn plan_codes(&mut self, data: &AuthoredArchetypeData) {
        let (max_id, deferred_nodes) = self.plan_node_codes(data);
        self.plan_redefined_slots(data);
        let (max_at, deferred_values) = self.plan_value_codes(data);
        self.next_id = max_id + 1;
        // NOTE: an at-coded archetype has one at-code space for nodes and
        // values (`AOM2/master07-terminology_package.adoc`), so a mint clears
        // every code it already uses.
        let max_at = match self.cfg.coding {
            NodeCoding::At => highest_at_number(&data.definition, &data.terminology),
            NodeCoding::Id => max_at,
        };
        self.minter.allocate_above(
            max_at,
            highest_ac_number(&data.definition, &data.terminology, self.cfg.coding),
        );
        for code in deferred_nodes {
            let fresh = self.alloc_id();
            self.minter.log.note(format!(
                "specialised node code {code} collapsed to {fresh} (standalone depth-0 emission)"
            ));
            self.node_map.insert(code, fresh);
        }
        for code in deferred_values {
            let fresh = self.minter.alloc_at();
            self.minter.log.note(format!(
                "specialised value code {code} collapsed to {fresh} (standalone depth-0 emission)"
            ));
            self.codes.value_map.insert(code, fresh);
        }
    }

    /// Maps every node at-code to its ADL2 node code, returning the highest
    /// id-number and the dotted codes the collapse defers (the root takes the
    /// root code).
    fn plan_node_codes(&mut self, data: &AuthoredArchetypeData) -> (i64, Vec<String>) {
        let collapse = self.cfg.collapse_specialised_codes;
        let source_root = match &data.definition {
            CComplexObject::CComplexObject(d) => d.node_id.clone(),
            CComplexObject::CArchetypeRoot(_) => String::new(),
        };
        let mut max_id = 1i64;
        let mut deferred: Vec<String> = Vec::new();
        collect_node_codes(&data.definition, &mut |code| {
            if code.is_empty() {
                return;
            }
            if collapse && code.contains('.') {
                if code == source_root {
                    self.node_map
                        .insert(code.to_owned(), root_code(self.cfg.coding).to_owned());
                } else if !self.node_map.contains_key(code) && !deferred.iter().any(|c| c == code) {
                    deferred.push(code.to_owned());
                }
                return;
            }
            let new = recode(self.cfg.coding, code, "id");
            if let Some(n) = first_num(&new) {
                max_id = max_id.max(n);
            }
            self.node_map.insert(code.to_owned(), new);
        });
        (max_id, deferred)
    }

    /// Gives a 1.4 slot that redefines a parent slot the node id of that slot
    /// (`AOM2/master04.5` VDSSID: "must have an identical node id").
    fn plan_redefined_slots(&mut self, data: &AuthoredArchetypeData) {
        if self.cfg.collapse_specialised_codes {
            return;
        }
        let mut redefined = Vec::new();
        collect_slot_codes(&data.definition, &mut |code| {
            if code.contains('.') && !is_new_at_level(code) {
                redefined.push(code.to_owned());
            }
        });
        for code in redefined {
            let mut parent = specialisation_parent_from_code(&code);
            // `at0004.0.1` redefines the top parent's `at0004` past a `.0` filler.
            while let Some(p) = parent.as_deref().filter(|p| p.ends_with(".0")) {
                parent = specialisation_parent_from_code(p);
            }
            let Some(parent) = parent else { continue };
            let id = recode(self.cfg.coding, &parent, "id");
            self.minter.log.note(format!(
                "specialised slot {code} takes the node id {id} of the slot it redefines (VDSSID)"
            ));
            self.node_map.insert(code, id);
        }
    }

    /// Records the value at-codes referenced in constraints (local single or
    /// list), returning the highest at-number reached and the codes deferred
    /// by the collapse.
    fn plan_value_codes(&mut self, data: &AuthoredArchetypeData) -> (i64, Vec<String>) {
        let collapse = self.cfg.collapse_specialised_codes;
        let mut max_at = 0i64;
        let mut value_at_codes = std::collections::BTreeSet::new();
        let mut deferred: Vec<String> = Vec::new();
        collect_local_value_codes(&data.definition, &mut |code| {
            if collapse && code.contains('.') {
                if !deferred.iter().any(|c| c == code) {
                    deferred.push(code.to_owned());
                }
                value_at_codes.insert(code.to_owned());
                return;
            }
            let new = recode(self.cfg.coding, code, "at");
            if let Some(n) = first_num(&new) {
                max_at = max_at.max(n);
            }
            value_at_codes.insert(code.to_owned());
        });
        self.value_at_codes = value_at_codes;
        (max_at, deferred)
    }

    /// A fresh node code: the next id-code, or for an at-coded conversion the
    /// minter's next at-code.
    fn alloc_id(&mut self) -> String {
        match self.cfg.coding {
            NodeCoding::At => self.minter.alloc_at(),
            NodeCoding::Id => {
                let c = format!("id{}", self.next_id);
                self.next_id += 1;
                c
            }
        }
    }

    // ── node-id renumbering ──────────────────────────────────────────────────

    fn renumber_nodes(&mut self, def: &mut CComplexObject) {
        renumber_cco(def, self);
        if let CComplexObject::CComplexObject(d) = def {
            self.root_id = d.node_id.clone();
        }
    }

    fn new_node_id(&mut self, old: &str) -> String {
        if old.is_empty() {
            let fresh = self.alloc_id();
            self.assigned_node_ids.insert(fresh.clone());
            return fresh;
        }
        let mapped = self
            .node_map
            .get(old)
            .cloned()
            .unwrap_or_else(|| recode(self.cfg.coding, old, "id"));
        if self.assigned_node_ids.insert(mapped.clone()) {
            return mapped;
        }
        // A second occurrence of a reused 1.4 code: 1.4 node ids are only
        // sibling-unique, ADL2 requires archetype-wide uniqueness (VCOSU,
        // `AOM2/master04.5` §Validity Rules: C_OBJECT) — mint a fresh id and
        // clone the first id's terminology in the rebuild.
        let fresh = self.alloc_id();
        self.assigned_node_ids.insert(fresh.clone());
        self.minter.log.note(format!(
            "reused 1.4 node code {old} re-minted as {fresh} (archetype-wide id uniqueness)"
        ));
        self.dup_mints.push((mapped, fresh.clone()));
        fresh
    }

    // ── terminology rebuild ──────────────────────────────────────────────────

    fn rebuild_terminology(&mut self, old: &ArchetypeTerminology) -> ArchetypeTerminology {
        let term_definitions = self.rebuild_term_definitions(old);
        let term_bindings = self.rebuild_term_bindings(old);
        let value_sets = self.rebuild_value_sets(old);
        ArchetypeTerminology {
            is_differential: old.is_differential,
            original_language: old.original_language.clone(),
            concept_code: self.root_id.clone(),
            term_definitions,
            term_bindings: if term_bindings.is_empty() {
                None
            } else {
                Some(term_bindings)
            },
            value_sets: if value_sets.is_empty() {
                None
            } else {
                Some(value_sets)
            },
            terminology_extracts: old.terminology_extracts.clone(),
        }
    }

    /// The renumbered `term_definitions`, per language; `@ internal @` node terms,
    /// found in the original language, are dropped in every language.
    fn rebuild_term_definitions(
        &mut self,
        old: &ArchetypeTerminology,
    ) -> BTreeMap<String, BTreeMap<String, ArchetypeTerm>> {
        let internal_nodes: std::collections::BTreeSet<String> = old
            .term_definitions
            .get(&old.original_language)
            .into_iter()
            .flatten()
            .filter(|(code, term)| {
                self.node_map.contains_key(*code) && term.description.trim() == "@ internal @"
            })
            .map(|(code, _)| code.clone())
            .collect();

        let mut term_definitions: BTreeMap<String, BTreeMap<String, ArchetypeTerm>> =
            BTreeMap::new();
        for (lang, terms) in &old.term_definitions {
            let mut out: BTreeMap<String, ArchetypeTerm> = BTreeMap::new();
            for (code, term) in terms {
                self.rebuild_term(code, term, &internal_nodes, &mut out);
            }
            // VCOSU re-mints: each freshly minted node id reuses the first
            // occurrence's rubric (the 1.4 source defined ONE term for the
            // shared code; both ADL2 nodes must be defined — VATDF).
            for (first, fresh) in &self.dup_mints {
                if let Some(term) = out.get(first) {
                    out.insert(fresh.clone(), term_with_code(term, fresh.clone()));
                }
            }
            self.minter.add_terms(&mut out);
            term_definitions.insert(lang.clone(), out);
        }
        term_definitions
    }

    /// Renumbers one 1.4 term definition into its ADL2 entries.
    ///
    /// An ac-code keeps its prefix (`master07.13` §Terminology section); a value
    /// at-code yields an at-code term and a node at-code a node-code term, one
    /// entry when both are at-codes. An `@ internal @` node term is dropped.
    fn rebuild_term(
        &mut self,
        code: &str,
        term: &ArchetypeTerm,
        internal_nodes: &std::collections::BTreeSet<String>,
        out: &mut BTreeMap<String, ArchetypeTerm>,
    ) {
        if AdlCodeDefinitionsData::is_value_set_code(code) {
            let ac = self.codes.value_set_code(&mut self.minter, code);
            out.insert(ac.clone(), term_with_code(term, ac));
            return;
        }
        let is_node = self.node_map.contains_key(code);
        if self.value_at_codes.contains(code) || !is_node {
            let at = self.codes.value_code(&mut self.minter, code);
            out.insert(at.clone(), term_with_code(term, at));
        }
        if !is_node || internal_nodes.contains(code) {
            return;
        }
        // The planned mapping, never a re-shift: under the specialisation
        // collapse a dotted code maps to a fresh flat id that a plain shift
        // would miss.
        let id = self
            .node_map
            .get(code)
            .cloned()
            .unwrap_or_else(|| recode(self.cfg.coding, code, "id"));
        out.insert(id.clone(), term_with_code(term, id));
    }

    /// The renumbered `term_bindings`: existing keys renamed, synthesised ones
    /// added, and each VCOSU re-mint inheriting the first occurrence's binding.
    fn rebuild_term_bindings(
        &self,
        old: &ArchetypeTerminology,
    ) -> BTreeMap<String, BTreeMap<String, String>> {
        let mut term_bindings: BTreeMap<String, BTreeMap<String, String>> = old
            .term_bindings
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|(t, m)| {
                let m2 = m
                    .into_iter()
                    .map(|(k, v)| (self.rename_binding_key(&k), v))
                    .collect();
                (t, m2)
            })
            .collect();
        self.minter.add_bindings(&mut term_bindings);
        for bindings in term_bindings.values_mut() {
            for (first, fresh) in &self.dup_mints {
                if let Some(uri) = bindings.get(first).cloned() {
                    bindings.insert(fresh.clone(), uri);
                }
            }
        }
        term_bindings
    }

    /// The value sets: the 1.4 ones verbatim plus one per synthesised ac-code
    /// that carries members.
    fn rebuild_value_sets(&self, old: &ArchetypeTerminology) -> BTreeMap<String, ValueSet> {
        let mut value_sets: BTreeMap<String, ValueSet> = old.value_sets.clone().unwrap_or_default();
        self.minter.add_value_sets(&mut value_sets);
        value_sets
    }

    fn rename_binding_key(&self, key: &str) -> String {
        // Binding keys are codes or code-terminated paths; rename a leading code.
        if let Some(id) = self.node_map.get(key) {
            id.clone()
        } else if AdlCodeDefinitionsData::is_at_code(key) {
            self.codes.value_at(key)
        } else if AdlCodeDefinitionsData::is_value_set_code(key) {
            // A merged 1.4 `constraint_bindings` key (ADL2 folds that section
            // into `term_bindings`, `master07.13` §Terminology section)
            // follows its definitions-rebuild remap (collapse map first,
            // else the ordinary ac shift), matching VTCBK.
            self.codes
                .ac_map
                .get(key)
                .cloned()
                .unwrap_or_else(|| recode(self.cfg.coding, key, "ac"))
        } else {
            key.to_owned()
        }
    }

    // ── meta / description / version ─────────────────────────────────────────

    fn stamp_meta(&mut self, data: &mut AuthoredArchetypeData) {
        data.adl_version = Some(self.cfg.adl_version.clone());
        self.cfg.rm_release.clone_into(&mut data.rm_release);
        data.is_generated = true;

        // Version from `other_details["revision"]` if present, else 1.0.0.
        let revision = data
            .description
            .as_ref()
            .and_then(|d| d.other_details.as_ref())
            .and_then(|o| o.get("revision"))
            .cloned();
        let version = revision.clone().unwrap_or_else(|| "1.0.0".to_owned());
        let source_model = self
            .cfg
            .source_model
            .clone()
            .unwrap_or_else(|| data.archetype_id.semantic_id());
        set_release_version(&mut data.archetype_id, &version);

        // `other_details["build_uid"]` → `build_uid` (`ADL1.4/masterAppB-extended_metadata.adoc`
        // §Standardised Items), only for a well-formed GUID and an unset `build_uid`.
        if data.build_uid.value().is_nil()
            && let Some(other) = data
                .description
                .as_mut()
                .and_then(|d| d.other_details.as_mut())
            && let Some(value) = other
                .get("build_uid")
                .and_then(|raw| uuid::Uuid::parse_str(raw.trim()).ok())
        {
            data.build_uid = openehr_base::prelude::Uuid::new(value);
            other.remove("build_uid");
        }

        let record = ConversionRecord {
            source_model: &source_model,
            tool: &self.cfg.tool,
            time: self.cfg.time,
        };
        convert_meta_data(data, &record, &self.minter.log.notes);
    }
}

// ── free-function tree walks ─────────────────────────────────────────────────

fn renumber_cco(cco: &mut CComplexObject, cx: &mut Converter<'_>) {
    let Some(d) = cco_data_mut(cco) else { return };
    d.node_id = cx.new_node_id(&d.node_id);
    let coding = cx.cfg.coding;
    tuple_primitive_node_ids_mut(d).for_each(|id| coding.recode_primitive_node_id(id));
    for attr in d.attributes.iter_mut().flatten() {
        for child in attr.children.iter_mut().flatten() {
            renumber_obj(child, cx);
        }
    }
    for tuple in d.attribute_tuples.iter_mut().flatten() {
        for member in tuple.members.iter_mut().flatten() {
            for child in member.children.iter_mut().flatten() {
                renumber_obj(child, cx);
            }
        }
    }
}

fn renumber_obj(obj: &mut CObject, cx: &mut Converter<'_>) {
    match obj {
        CObject::CComplexObject(cco) => renumber_cco(cco, cx),
        CObject::CComplexObjectProxy(p) => {
            p.node_id = cx.new_node_id(&p.node_id);
            p.target_path = rewrite_path(&p.target_path, cx);
        }
        CObject::ArchetypeSlot(s) => {
            s.node_id = cx.new_node_id(&s.node_id);
            let assertions = s.includes.iter_mut().chain(s.excludes.iter_mut()).flatten();
            assertions.for_each(|a| crate::rules::recode_slot_matcher(a, cx.cfg.coding));
        }
        primitive => cx
            .cfg
            .coding
            .recode_primitive_node_id(common_mut(primitive).1),
    }
}

fn rewrite_path(path: &str, cx: &Converter<'_>) -> String {
    // Rewrite `[atNNNN]` predicates in an ADL path to their converted codes.
    let mut out = String::with_capacity(path.len());
    let mut chars = path.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '[' {
            // Read to the matching ']'.
            let start = i + 1;
            let rest = path.get(start..).unwrap_or_default();
            if let Some(end) = rest.find(']')
                && let Some(code) = rest.get(..end)
            {
                out.push('[');
                out.push_str(&converted_predicate_code(code, cx));
                out.push(']');
                for _ in 0..=end {
                    chars.next();
                }
                continue;
            }
        }
        out.push(c);
    }
    out
}

/// The converted spelling of one path-predicate code.
fn converted_predicate_code(code: &str, cx: &Converter<'_>) -> String {
    if let Some(id) = cx.node_map.get(code) {
        return id.clone();
    }
    if AdlCodeDefinitionsData::is_at_code(code) {
        return recode(cx.cfg.coding, code, "id");
    }
    code.to_owned()
}

/// The root node code of a depth-0 archetype in `coding`
/// (`Root_code_regex_pattern`, `org.openehr.am.aom2.adl_code_definitions`
/// §Constants).
fn root_code(coding: NodeCoding) -> &'static str {
    match coding {
        NodeCoding::At => "at0000",
        NodeCoding::Id => "id1",
    }
}

fn term_with_code(term: &ArchetypeTerm, code: String) -> ArchetypeTerm {
    ArchetypeTerm {
        code,
        text: term.text.clone(),
        description: term.description.clone(),
        other_items: term.other_items.clone(),
    }
}

/// The converter's code space: every 1.4 value and value-set code spelt in
/// the ADL2 code system ([`recode`]), and under `collapse_specialised_codes`
/// each dotted one remapped to a fresh flat code.
struct CollapseMaps {
    coding: NodeCoding,
    collapse: bool,
    /// Collapsed-value remapping (dotted `local` value at-codes → fresh flat
    /// at-codes) — populated only under `collapse_specialised_codes`.
    value_map: BTreeMap<String, String>,
    /// Collapsed-constraint remapping (dotted ac-codes → fresh flat
    /// ac-codes) — populated only under `collapse_specialised_codes`.
    ac_map: BTreeMap<String, String>,
}

impl CollapseMaps {
    /// The ADL2 at-code of a `local` value code: the collapse remap when one
    /// exists, else its ADL2 spelling.
    fn value_at(&self, code: &str) -> String {
        self.value_map
            .get(code)
            .cloned()
            .unwrap_or_else(|| recode(self.coding, code, "at"))
    }
}

impl CodeSpace for CollapseMaps {
    /// The at-code for a value or terminology entry under the collapse: an unused
    /// dotted code mints a fresh flat at-code, everything else follows
    /// [`CollapseMaps::value_at`].
    fn value_code(&mut self, minter: &mut TermMinter<'_>, code: &str) -> String {
        if self.collapse && code.contains('.') && !self.value_map.contains_key(code) {
            let fresh = minter.alloc_at();
            minter.log.note(format!(
                "specialised terminology code {code} collapsed to {fresh} (standalone depth-0 emission)"
            ));
            self.value_map.insert(code.to_owned(), fresh);
        }
        self.value_at(code)
    }

    /// The ac-code under the collapse: a dotted ac (specialised constraint
    /// definition) mints a fresh flat ac on first sight; else its ADL2
    /// spelling.
    fn value_set_code(&mut self, minter: &mut TermMinter<'_>, code: &str) -> String {
        if self.collapse && code.contains('.') {
            if let Some(existing) = self.ac_map.get(code) {
                return existing.clone();
            }
            let fresh = minter.alloc_ac();
            minter.log.note(format!(
                "specialised constraint code {code} collapsed to {fresh} (standalone depth-0 emission)"
            ));
            self.ac_map.insert(code.to_owned(), fresh.clone());
            return fresh;
        }
        recode(self.coding, code, "ac")
    }
}
