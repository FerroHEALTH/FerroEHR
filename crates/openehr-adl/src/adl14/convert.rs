// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 → ADL 2 converter core.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion — the whole `adl14` module is
//! our own design (see the [`crate::adl14`] flag). The conversion HEURISTICS
//! are pinned by the paired `upgrade_from_14` corpus fixtures, never by a
//! spec clause; where a comment cites an AOM2/ADL2 section, it pins the
//! validity of the ADL2 OUTPUT form (which is spec-governed), not the
//! conversion procedure.
//!
//! The one exception is the standardised `description/other_details` meta-data
//! mapping (`build_uid` + the `RESOURCE_DESCRIPTION` governance items), which
//! `ADL1.4/masterAppB-extended_metadata.adoc` §Standardised Items governs
//! directly — those items are "intended to be implemented by any ADL 1.4 =>
//! ADL 2 conversion tool". It is applied by `crate::adl14::metadata`.
//!
//! This module owns the code spaces (planning, renumbering, terminology
//! rebuild) and the constraint conversion; the three stages that need no
//! converter state are siblings — `crate::adl14::walk` (the read-only
//! definition traversals + the shared complex-object accessor),
//! `crate::adl14::multiplicity` (the occurrences/cardinality reconciliation)
//! and `crate::adl14::metadata` (description / meta-data / version).

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};
use openehr_am::v2_4::aom2::constraint_model::c_attribute_tuple::CAttributeTuple;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;
use openehr_am::v2_4::aom2::constraint_model::primitive::c_terminology_code::CTerminologyCode;
use openehr_am::v2_4::aom2::definitions::adl_code_definitions::AdlCodeDefinitionsData;
use openehr_am::v2_4::aom2::terminology::archetype_term::ArchetypeTerm;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::aom2::terminology::value_set::ValueSet;

use crate::adl14::log::ConversionLog;
use crate::adl14::metadata::{set_release_version, transform_description};
use crate::adl14::multiplicity::{elide_multiplicity, materialise_adl14_occurrences};
use crate::adl14::walk::{
    cco_data_mut, collect_local_value_codes, collect_node_codes, walk_constraints,
};
use crate::error::SyntaxError;

/// Configuration for a 1.4→2 conversion (all spec-silent — our own design).
#[derive(Debug, Clone)]
pub struct ConvertConfig {
    /// The `adl_version` to stamp on the converted artefact.
    pub adl_version: String,
    /// The `rm_release` to stamp (1.4 sources carry no `rm_release`).
    pub rm_release: String,
    /// Per-terminology URI templates for synthesised term bindings; `{code}` is
    /// replaced by the external code. A terminology with no template gets a
    /// flagged fabricated fallback (`urn:adl14:<terminology>:<code>`).
    pub binding_uri_templates: BTreeMap<String, String>,
    /// Collapse specialised (dotted) codes to top level, producing a depth-0
    /// archetype. For a `-`-specialised root inlined standalone by a
    /// flattened OPT the differential lineage is unresolvable — the spec
    /// defines differential-form semantics only relative to a resolvable
    /// parent (`ADL2/master09.02` §Specialisation concepts), a specialised
    /// archetype must declare that parent (`AOM2/master03` §Validity Rules
    /// VASID/VACSD), and deriving the parent id from the 1.4 `-` naming is
    /// endorsed nowhere — so the standalone source is emitted UNSPECIALISED:
    /// every dotted node/value/constraint code is renumbered into the flat
    /// code space (terminology keys follow), satisfying VARCN/VATCD at depth
    /// 0. Off for plain ADL-source conversion, where the `specialise` clause
    /// carries the lineage. No openEHR spec governs 1.4→2 conversion — our
    /// own design/extension.
    pub collapse_specialised_codes: bool,
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
            adl_version: "2.0.6".to_owned(),
            rm_release: "1.0.3".to_owned(),
            binding_uri_templates,
            collapse_specialised_codes: false,
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

/// Convert a 1.4-shaped [`Archetype`] (from
/// [`crate::assemble::parse_artefact`] in [`crate::parse::Dialect::Adl14`])
/// into a spec-valid ADL2 archetype.
///
/// For a specialised source, this performs the base conversion (renumbering
/// against the child's own codes); re-differentialisation against the
/// converted+flattened parent is [`crate::adl14::differ::differentiate`].
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

/// A synthesised terminology entry the converter minted.
struct Synth {
    code: String,
    text: String,
    description: String,
    binding: Option<(String, String)>, // (terminology, uri)
    value_set_members: Option<Vec<String>>,
}

struct Converter<'a> {
    cfg: &'a ConvertConfig,
    log: &'a mut ConversionLog,
    /// old node at-code → new node id-code.
    node_map: BTreeMap<String, String>,
    /// Source at-codes referenced as a `local` *value* (they get an at-code
    /// term entry even when the same code is also a node id — a 1.4 at-code
    /// used as both an id and a value splits into an id-code + an at-code).
    value_at_codes: std::collections::BTreeSet<String>,
    /// Collapsed-value remapping (dotted `local` value at-codes → fresh flat
    /// at-codes) — populated only under `collapse_specialised_codes`.
    value_map: BTreeMap<String, String>,
    /// Collapsed-constraint remapping (dotted ac-codes → fresh flat
    /// ac-codes) — populated only under `collapse_specialised_codes`.
    ac_map: BTreeMap<String, String>,
    /// New node ids already claimed by an occurrence — the VCOSU ground:
    /// ADL2 node ids are unique archetype-wide (`AOM2/master04.5` §Validity
    /// Rules: `C_OBJECT`), while 1.4 at-codes are only sibling-unique, so a
    /// reused code's second occurrence mints a fresh id.
    assigned_node_ids: std::collections::BTreeSet<String>,
    /// `(first-occurrence new id, freshly minted id)` per VCOSU re-mint — the
    /// terminology rebuild clones the definition/bindings of the first id
    /// under each minted one (VATDF: every used code must be defined).
    dup_mints: Vec<(String, String)>,
    /// Next free id-number for synthesised object nodes (top-level id space).
    next_id: i64,
    /// Next free at-number for synthesised external at-codes.
    next_at: i64,
    /// Next free ac-number for synthesised value sets.
    next_ac: i64,
    /// Synthesised terminology entries, in mint order.
    synth: Vec<Synth>,
    /// The converted root node id (the terminology `concept_code`).
    root_id: String,
}

impl<'a> Converter<'a> {
    fn new(cfg: &'a ConvertConfig, log: &'a mut ConversionLog) -> Self {
        Self {
            cfg,
            log,
            node_map: BTreeMap::new(),
            value_at_codes: std::collections::BTreeSet::new(),
            value_map: BTreeMap::new(),
            ac_map: BTreeMap::new(),
            assigned_node_ids: std::collections::BTreeSet::new(),
            dup_mints: Vec::new(),
            next_id: 1,
            next_at: 1,
            next_ac: 1,
            synth: Vec::new(),
            root_id: "id1".to_owned(),
        }
    }

    fn run(&mut self, mut data: AuthoredArchetypeData) -> AuthoredArchetypeData {
        // 1. Build the node-id map + synthesis counters from the definition.
        self.plan_codes(&data);
        // 2. Renumber node ids in document order (existing → mapped, empty →
        //    synthesised).
        self.renumber_nodes(&mut data.definition);
        // 3. Convert every terminology-code constraint (local/external/list).
        self.convert_constraints(&mut data.definition);
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
        let (max_at, deferred_values) = self.plan_value_codes(data);
        self.next_id = max_id + 1;
        self.next_at = max_at + 1;
        self.next_ac = highest_ac_number(data) + 1;
        for code in deferred_nodes {
            let fresh = self.alloc_id();
            self.log.note(format!(
                "specialised node code {code} collapsed to {fresh} (standalone depth-0 emission)"
            ));
            self.node_map.insert(code, fresh);
        }
        for code in deferred_values {
            let fresh = self.alloc_at();
            self.log.note(format!(
                "specialised value code {code} collapsed to {fresh} (standalone depth-0 emission)"
            ));
            self.value_map.insert(code, fresh);
        }
    }

    /// Maps every existing node at-code to its shifted id-code, returning the
    /// highest id-number reached and the codes deferred by the collapse.
    ///
    /// Under collapse, dotted (specialised) codes are deferred: they get fresh
    /// flat ids above the max, in document order (the root always takes `id1`
    /// — VARCN's depth-0 root form).
    fn plan_node_codes(&mut self, data: &AuthoredArchetypeData) -> (i64, Vec<String>) {
        let collapse = self.cfg.collapse_specialised_codes;
        let root_code = match &data.definition {
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
                if code == root_code {
                    self.node_map.insert(code.to_owned(), "id1".to_owned());
                } else if !self.node_map.contains_key(code) && !deferred.iter().any(|c| c == code) {
                    deferred.push(code.to_owned());
                }
                return;
            }
            let new = shift_code(code, "id");
            if let Some(n) = first_num(&new) {
                max_id = max_id.max(n);
            }
            self.node_map.insert(code.to_owned(), new);
        });
        (max_id, deferred)
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
            let new = shift_code(code, "at");
            if let Some(n) = first_num(&new) {
                max_at = max_at.max(n);
            }
            value_at_codes.insert(code.to_owned());
        });
        self.value_at_codes = value_at_codes;
        (max_at, deferred)
    }

    fn alloc_id(&mut self) -> String {
        let c = format!("id{}", self.next_id);
        self.next_id += 1;
        c
    }

    fn alloc_at(&mut self) -> String {
        let c = format!("at{}", self.next_at);
        self.next_at += 1;
        c
    }

    fn alloc_ac(&mut self) -> String {
        let c = format!("ac{}", self.next_ac);
        self.next_ac += 1;
        c
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
            .unwrap_or_else(|| shift_code(old, "id"));
        if self.assigned_node_ids.insert(mapped.clone()) {
            return mapped;
        }
        // A second occurrence of a reused 1.4 code: 1.4 node ids are only
        // sibling-unique, ADL2 requires archetype-wide uniqueness (VCOSU,
        // `AOM2/master04.5` §Validity Rules: C_OBJECT) — mint a fresh id and
        // clone the first id's terminology in the rebuild.
        let fresh = self.alloc_id();
        self.assigned_node_ids.insert(fresh.clone());
        self.log.note(format!(
            "reused 1.4 node code {old} re-minted as {fresh} (archetype-wide id uniqueness)"
        ));
        self.dup_mints.push((mapped, fresh.clone()));
        fresh
    }

    /// The ADL2 at-code of a `local` value code: the collapse remap when one
    /// exists, else the ordinary shift.
    fn value_at(&self, code: &str) -> String {
        self.value_map
            .get(code)
            .cloned()
            .unwrap_or_else(|| shift_code(code, "at"))
    }

    /// The at-code for a terminology entry under the collapse: a dotted code
    /// with no planned remap yet (an UNUSED flattened-ontology entry — VTSD
    /// covers defined codes, not only used ones) mints a fresh flat at-code
    /// on first sight; everything else follows [`Self::value_at`].
    fn collapsed_value_at(&mut self, code: &str) -> String {
        if self.cfg.collapse_specialised_codes
            && code.contains('.')
            && !self.value_map.contains_key(code)
        {
            let fresh = self.alloc_at();
            self.log.note(format!(
                "specialised terminology code {code} collapsed to {fresh} (standalone depth-0 emission)"
            ));
            self.value_map.insert(code.to_owned(), fresh);
        }
        self.value_at(code)
    }

    /// The ac-code under the collapse: a dotted ac (specialised constraint
    /// definition) mints a fresh flat ac on first sight; else the ordinary
    /// shift.
    fn collapsed_ac(&mut self, code: &str) -> String {
        if self.cfg.collapse_specialised_codes && code.contains('.') {
            if let Some(existing) = self.ac_map.get(code) {
                return existing.clone();
            }
            let fresh = self.alloc_ac();
            self.log.note(format!(
                "specialised constraint code {code} collapsed to {fresh} (standalone depth-0 emission)"
            ));
            self.ac_map.insert(code.to_owned(), fresh.clone());
            return fresh;
        }
        shift_code(code, "ac")
    }

    // ── terminology-constraint conversion ────────────────────────────────────

    fn convert_constraints(&mut self, def: &mut CComplexObject) {
        convert_constraints_cco(def, self, "");
    }

    /// Convert one 1.4 `C_TERMINOLOGY_CODE.constraint` encoding
    /// (`terminology::code[,code…][;assumed]`, or an already-ADL2 `at/ac` code)
    /// into an ADL2 constraint, minting synthesised codes as needed.
    /// `owner_text` is the enclosing element's rubric (for the `(synthesised)`
    /// value-set label). Returns `(constraint, assumed)`.
    fn convert_constraint(&mut self, raw: &str, owner_text: &str) -> (String, Option<String>) {
        // Split off an assumed value (`…;code`).
        let (body, assumed_raw) = match raw.split_once(';') {
            Some((b, a)) => (b, Some(a)),
            None => (raw, None),
        };
        // Not a qualified 1.4 code: a bare ac-code (a 1.4 `CONSTRAINT_REF`
        // reference or a reference-set ac minted by the OPT front end) is
        // shifted like every other code so it stays aligned with its shifted
        // terminology entry (`ac0001`→`ac2`; the `ac0.K` specialisation form
        // keeps its number — see `shift_code`). Anything else (`at5`) is
        // already ADL2; pass through.
        let Some((terminology, codes_str)) = body.split_once("::") else {
            if AdlCodeDefinitionsData::is_value_set_code(body) {
                let ac = self.collapsed_ac(body);
                let assumed = assumed_raw.map(|a| self.collapsed_value_at(a));
                return (ac, assumed);
            }
            return (body.to_owned(), assumed_raw.map(str::to_owned));
        };
        let codes: Vec<&str> = codes_str
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let is_local = terminology == "local";

        // Map each source code to its ADL2 at-code (shift a local at-code; mint
        // for an external code).
        let mut at_codes: Vec<String> = Vec::new();
        for code in &codes {
            let at = if is_local {
                self.collapsed_value_at(code)
            } else {
                self.external_at_code(terminology, code)
            };
            at_codes.push(at);
        }

        let assumed = assumed_raw.map(|a| {
            if is_local {
                self.collapsed_value_at(a)
            } else {
                self.external_at_code(terminology, a)
            }
        });

        if at_codes.len() == 1 {
            return (at_codes.into_iter().next().unwrap_or_default(), assumed);
        }
        (self.value_set_ac(&at_codes, owner_text), assumed)
    }

    /// The synthesised ac-code value set for a code LIST, minted on first sight
    /// and reused for an identical member signature (idempotent via the log).
    fn value_set_ac(&mut self, at_codes: &[String], owner_text: &str) -> String {
        let signature = at_codes.join(",");
        if let Some(existing) = self.log.value_set(&signature) {
            return existing.to_owned();
        }
        let ac = self.alloc_ac();
        self.log.record_value_set(&signature, &ac);
        let (text, description) = synth_value_set_rubric(owner_text);
        self.synth.push(Synth {
            code: ac.clone(),
            text,
            description,
            binding: None,
            value_set_members: Some(at_codes.to_vec()),
        });
        ac
    }

    /// The synthesised at-code for an external `terminology::code`, minting +
    /// recording a binding on first sight (idempotent via the log).
    fn external_at_code(&mut self, terminology: &str, code: &str) -> String {
        let key = format!("{terminology}::{code}");
        if let Some(existing) = self.log.external_at_code(&key) {
            return existing.to_owned();
        }
        let at = self.alloc_at();
        self.log.record_external_at_code(&key, &at);
        let uri = match self.cfg.binding_uri_templates.get(terminology) {
            Some(tmpl) => tmpl.replace("{code}", code),
            // Fabricated fallback URI (flagged) where no template is configured.
            None => format!("urn:adl14:{terminology}:{code}"),
        };
        self.synth.push(Synth {
            code: at.clone(),
            // NOTE: no openEHR spec governs 1.4→2 conversion — our own
            // design/extension; the synthesised rubric reuses the external code
            // as its text, this converter resolving no external terminology.
            text: code.to_owned(),
            description: code.to_owned(),
            binding: Some((terminology.to_owned(), uri)),
            value_set_members: None,
        });
        at
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

    /// The renumbered `term_definitions`, per language.
    ///
    /// `@ internal @` node terms are dropped in every language (the marker is
    /// authored in the original language; translations carry the openEHR
    /// untranslated form `*@ internal @(<lang>)`), so the internal node set is
    /// determined once, from the original language.
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
            // Add synthesised terms to every language.
            //
            // NOTE: no openEHR spec governs 1.4→2 conversion — our own
            // design/extension; one rubric text is minted for all languages, a
            // converter having no translator for per-language rubrics.
            for s in &self.synth {
                out.insert(
                    s.code.clone(),
                    ArchetypeTerm {
                        code: s.code.clone(),
                        text: s.text.clone(),
                        description: s.description.clone(),
                        other_items: None,
                    },
                );
            }
            term_definitions.insert(lang.clone(), out);
        }
        term_definitions
    }

    /// Renumbers one 1.4 term definition into its ADL2 entries.
    ///
    /// An ac-code (a merged 1.4 `constraint_definitions` entry — ADL2 merges
    /// that section into `term_definitions`, `master07.13` §Terminology
    /// section) keeps its ac prefix, shifted like every other code so the
    /// converted `C_TERMINOLOGY_CODE` ac constraints still resolve (VACDF). A
    /// 1.4 at-code used as a value (`[local::atX]`) always yields an at-code
    /// term; used as a node id it yields an id-code term, and a code that is
    /// both splits into both entries. An `@ internal @` node term is dropped
    /// (reference-converter behaviour; validates clean).
    fn rebuild_term(
        &mut self,
        code: &str,
        term: &ArchetypeTerm,
        internal_nodes: &std::collections::BTreeSet<String>,
        out: &mut BTreeMap<String, ArchetypeTerm>,
    ) {
        if AdlCodeDefinitionsData::is_value_set_code(code) {
            let ac = self.collapsed_ac(code);
            out.insert(ac.clone(), term_with_code(term, ac));
            return;
        }
        let is_node = self.node_map.contains_key(code);
        if self.value_at_codes.contains(code) || !is_node {
            let at = self.collapsed_value_at(code);
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
            .unwrap_or_else(|| shift_code(code, "id"));
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
        for s in &self.synth {
            if let Some((terminology, uri)) = &s.binding {
                term_bindings
                    .entry(terminology.clone())
                    .or_default()
                    .insert(s.code.clone(), uri.clone());
            }
        }
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
        for s in &self.synth {
            if let Some(members) = &s.value_set_members
                && let Ok(members) = openehr_base::containers::NonEmptyVec::new(members.clone())
            {
                value_sets.insert(
                    s.code.clone(),
                    ValueSet {
                        id: s.code.clone(),
                        members,
                    },
                );
            }
        }
        value_sets
    }

    fn rename_binding_key(&self, key: &str) -> String {
        // Binding keys are codes or code-terminated paths; rename a leading code.
        if let Some(id) = self.node_map.get(key) {
            id.clone()
        } else if AdlCodeDefinitionsData::is_at_code(key) {
            self.value_at(key)
        } else if AdlCodeDefinitionsData::is_value_set_code(key) {
            // A merged 1.4 `constraint_bindings` key (ADL2 folds that section
            // into `term_bindings`, `master07.13` §Terminology section)
            // follows its definitions-rebuild remap (collapse map first,
            // else the ordinary ac shift), matching VTCBK.
            self.ac_map
                .get(key)
                .cloned()
                .unwrap_or_else(|| shift_code(key, "ac"))
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
        set_release_version(&mut data.archetype_id, &version);

        // `other_details["build_uid"]` → the archetype's `build_uid`
        // (`ADL1.4/masterAppB-extended_metadata.adoc` §Standardised Items:
        // "Guid string … See AOM2 spec, Machine Identifiers section"). Only a
        // well-formed GUID is consumed — a value violating the stated syntax
        // stays in `other_details` verbatim rather than being guessed at — and
        // an already-populated `build_uid` (a 1.4 header `build_uid=` meta
        // item) is never overwritten by the meta-data section.
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

        if let Some(desc) = data.description.as_mut() {
            transform_description(desc);
            // Surface the conversion's non-mechanical decisions (collapse
            // remaps, VCOSU re-mints) as conversion provenance — the AOM2
            // home for it (`RESOURCE_DESCRIPTION.conversion_details`).
            if !self.log.notes.is_empty() {
                let details = desc.conversion_details.get_or_insert_with(BTreeMap::new);
                for (index, note) in self.log.notes.iter().enumerate() {
                    details.insert(format!("code_remap_{index:03}"), note.clone());
                }
            }
        }
    }
}

// ── free-function tree walks ─────────────────────────────────────────────────

fn renumber_cco(cco: &mut CComplexObject, cx: &mut Converter<'_>) {
    let Some(d) = cco_data_mut(cco) else { return };
    d.node_id = cx.new_node_id(&d.node_id);
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
        CObject::ArchetypeSlot(s) => s.node_id = cx.new_node_id(&s.node_id),
        _ => {}
    }
}

fn convert_constraints_cco(cco: &mut CComplexObject, cx: &mut Converter<'_>, owner_text: &str) {
    let Some(d) = cco_data_mut(cco) else { return };
    let node_text = owner_text.to_owned();
    for attr in d.attributes.iter_mut().flatten() {
        for child in attr.children.iter_mut().flatten() {
            convert_constraints_obj(child, cx, &node_text);
        }
    }
    for tuple in d.attribute_tuples.iter_mut().flatten() {
        convert_constraints_tuple(tuple, cx, &node_text);
    }
}

/// Converts the terminology codes inside one `C_ATTRIBUTE_TUPLE` in place.
///
/// Both halves of the tuple carry constraints: the tuple MEMBERS' child
/// objects, and the tuple ROWS, whose primitive members hold the actual
/// terminology codes (ordinal symbols and the like).
fn convert_constraints_tuple(
    tuple: &mut CAttributeTuple,
    cx: &mut Converter<'_>,
    owner_text: &str,
) {
    for member in tuple.members.iter_mut().flatten() {
        for child in member.children.iter_mut().flatten() {
            convert_constraints_obj(child, cx, owner_text);
        }
    }
    for row in tuple.tuples.iter_mut().flatten() {
        for m in &mut row.members {
            if let CPrimitiveObject::CTerminologyCode(tc) = m {
                convert_terminology_code(tc, cx, owner_text);
            }
        }
    }
}

/// Converts one `C_TERMINOLOGY_CODE`'s constraint and assumed value in place.
fn convert_terminology_code(tc: &mut CTerminologyCode, cx: &mut Converter<'_>, owner_text: &str) {
    let (constraint, assumed) = cx.convert_constraint(&tc.constraint, owner_text);
    tc.constraint = constraint;
    if let Some(a) = assumed {
        tc.assumed_value = Some(openehr_base::prelude::TerminologyCode {
            terminology_id: "local".to_owned(),
            terminology_version: None,
            code_string: a,
            uri: None,
        });
    }
}

fn convert_constraints_obj(obj: &mut CObject, cx: &mut Converter<'_>, owner_text: &str) {
    match obj {
        CObject::CTerminologyCode(tc) => convert_terminology_code(tc, cx, owner_text),
        CObject::CComplexObject(cco) => convert_constraints_cco(cco, cx, owner_text),
        _ => {}
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
///
/// A planned node code takes its planned id; any other at-code is shifted into
/// the `id` space; anything else passes through.
fn converted_predicate_code(code: &str, cx: &Converter<'_>) -> String {
    if let Some(id) = cx.node_map.get(code) {
        return id.clone();
    }
    if AdlCodeDefinitionsData::is_at_code(code) {
        return shift_code(code, "id");
    }
    code.to_owned()
}

// ── code shifting ────────────────────────────────────────────────────────────

/// The highest ac-number the source already uses, across its terminology
/// entries and its bare constraint refs.
///
/// Synthesised and minted acs must allocate ABOVE the shifted range, so a
/// fresh `acN` never collides with an existing `ac000(N-1)` → `acN`.
fn highest_ac_number(data: &AuthoredArchetypeData) -> i64 {
    let mut highest = 0i64;
    let mut track = |code: &str| {
        if AdlCodeDefinitionsData::is_value_set_code(code)
            && !code.contains('.')
            && let Some(n) = first_num(&shift_code(code, "ac"))
        {
            highest = highest.max(n);
        }
    };
    for terms in data.terminology.term_definitions.values() {
        for code in terms.keys() {
            track(code);
        }
    }
    walk_constraints(&data.definition, &mut |raw, _| {
        let body = raw.split_once(';').map_or(raw, |(b, _)| b);
        if !body.contains("::") {
            track(body.trim());
        }
    });
    highest
}

/// Shift a 1.4 code to an ADL2 code with the given `prefix` (`"id"`/`"at"`).
/// The first segment's number is incremented by one (`at0000`→`id1`,
/// `at0003.1`→`id4.1`), except the specialisation new-code prefix `at0`
/// (`at0.89`→`id0.89` — number kept). No openEHR spec governs this — fixture-pinned.
fn shift_code(code: &str, prefix: &str) -> String {
    let bare = code
        .trim_start_matches("at")
        .trim_start_matches("id")
        .trim_start_matches("ac");
    let mut segs = bare.split('.');
    let Some(first) = segs.next() else {
        return code.to_owned();
    };
    let rest: Vec<&str> = segs.collect();
    let has_suffix = !rest.is_empty();
    let shifted_first = if first == "0" && has_suffix {
        // `at0.K` new-at-level code: keep the 0.
        "0".to_owned()
    } else {
        match first.parse::<i64>() {
            Ok(n) => (n + 1).to_string(),
            Err(_) => first.to_owned(),
        }
    };
    if has_suffix {
        format!("{prefix}{shifted_first}.{}", rest.join("."))
    } else {
        format!("{prefix}{shifted_first}")
    }
}

fn first_num(code: &str) -> Option<i64> {
    let bare = code
        .trim_start_matches("at")
        .trim_start_matches("id")
        .trim_start_matches("ac");
    bare.split('.').next()?.parse::<i64>().ok()
}

fn term_with_code(term: &ArchetypeTerm, code: String) -> ArchetypeTerm {
    ArchetypeTerm {
        code,
        text: term.text.clone(),
        description: term.description.clone(),
        other_items: term.other_items.clone(),
    }
}

fn synth_value_set_rubric(owner_text: &str) -> (String, String) {
    // The fixtures label a synthesised value set with the owning element's
    // rubric + " (synthesised)".
    let base = if owner_text.is_empty() {
        "Value set".to_owned()
    } else {
        owner_text.to_owned()
    };
    (
        format!("{base} (synthesised)"),
        format!("{base} (synthesised)"),
    )
}
