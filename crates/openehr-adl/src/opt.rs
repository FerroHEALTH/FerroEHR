// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! OPT2 — operational-template generation (raw + profiled).
//!
//! Oracle: `docs/specs/openehr/AM/docs/OPT2/master02-overview.adoc` (the OPT
//! checklist), `master03-opt_raw.adoc` (§Artefact Structure grammar,
//! §Archetype References, §Flattening, §Terminology), `master04-opt_profiled.adoc`
//! (annotations removal, language filtering, binding filtering, node-level
//! substitution — spec TBD), and `docs/specs/openehr/AM/docs/ADL2/master10-templates.adoc`
//! (the worked template → OPT example).
//!
//! [`create_opt`] produces the *raw* OPT: it flattens the source template's
//! specialisation lineage ([`crate::flatten::flat_form`]) and then applies the
//! OPT-specific extras on top of the plain flat form (OPT2 master03 §Flattening):
//!
//! * all archetype references resolved to full 3-part-version ids
//!   (master03 §Archetype References);
//! * every `use_archetype` filler / external reference inlined as a
//!   `C_ARCHETYPE_ROOT` carrying the filler's flattened structure;
//! * every `use_node` internal reference (`C_COMPLEX_OBJECT_PROXY`) replaced by
//!   an inline copy of its target;
//! * all `closed` slots removed;
//! * every `existence matches {0}` attribute and `occurrences matches {0}`
//!   object removed;
//! * no sibling-order markers, no `specialise` section (a top-level standalone
//!   artefact, master02);
//! * the flat `terminology` of every constituent (other than the root) gathered
//!   into `component_terminologies` (master03 §Terminology).
//!
//! [`profile_opt`] produces a *profiled* OPT from a raw one (master04):
//! annotations removal, language filtering (≥1 language remains), and
//! terminology-binding filtering. Node-level terminology substitution is
//! explicitly TBD in the spec (master04 §Terminology Substitution) and is not
//! implemented — a request for it returns
//! [`OptError::NodeSubstitutionUnsupported`].
//!
//! NOTE: master02's "no specialisation statement" bullet is followed here (the
//! OPT is emitted as a top-level standalone artefact with no `specialize`
//! section); the master10 worked-example listing prints a residual `specialize`
//! line, which contradicts master02/master03 §Artefact Structure — the
//! normative checklist wins over the illustrative listing.

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;
use openehr_am::v2_4::aom2::constraint_model::c_archetype_root::CArchetypeRoot;
use openehr_am::v2_4::aom2::constraint_model::c_attribute::CAttribute;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object_proxy::CComplexObjectProxy;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::rm_overlay::rm_overlay::RmOverlay;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::beom::core::statement_set::StatementSet;
use openehr_am::v2_4::resource::resource_description::ResourceDescription;
use openehr_base::prelude::{
    MultiplicityInterval, ResourceAnnotations, TerminologyCode, TranslationDetails,
};

use crate::aom::access::{
    child_occurrences, common_mut, complex_attributes, object_node_id, strip_sibling_order,
};
use crate::artefact::{ArchetypeRepository, view};
use crate::flatten::{FlattenError, flat_form};
use crate::hrid::hrid_to_string;
use crate::odin::nil_uuid;
use crate::paths::parse_path;
use openehr_lang::nesting::Nesting;

/// A failure while generating or profiling an operational template.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OptError {
    /// Flattening the template's (or a constituent's) specialisation lineage
    /// failed.
    #[error("flattening failed while building the OPT: {0}")]
    Flatten(#[from] FlattenError),
    /// A `use_archetype` slot-filler / external reference did not resolve in the
    /// repository (OPT2 master03 §Archetype References — every reference must be
    /// resolvable to a full archetype id).
    #[error("archetype reference {0:?} could not be resolved to a constituent")]
    UnresolvedReference(String),
    /// Profiled-OPT node-level terminology substitution was requested. The spec
    /// explicitly leaves this TBD (`master04` §Terminology Substitution: "there
    /// is no way to do this in ADL2"), so it is not implemented.
    #[error(
        "node-level terminology substitution is not defined by the OPT2 spec (master04 §Terminology Substitution — TBD)"
    )]
    NodeSubstitutionUnsupported,
    /// Language filtering would leave the OPT with no languages; master04
    /// §Language Filtering permits removal only "up to the limit where only one
    /// remains".
    #[error("language filtering would remove every language (at least one must remain)")]
    NoLanguagesLeft,
    /// A filler inlines an archetype that is already being inlined above it —
    /// the constituents reference each other in a cycle, which no finite OPT
    /// can flatten (master03 §Flattening inlines every filler). Carries the
    /// chain of archetype references from the outermost to the repeated one.
    #[error("archetype references form a cycle: {}", .0.join(" -> "))]
    CyclicReference(Vec<String>),
    /// Inlining fillers and `use_node` targets composes objects deeper than
    /// the engine bound ([`openehr_lang::nesting::MAX_NESTING_DEPTH`]) — an
    /// implementation limit, not a spec rule.
    #[error("the operational template nests deeper than the limit of {limit} levels")]
    NestingTooDeep {
        /// The bound that was crossed.
        limit: usize,
    },
}

/// Generate the raw operational template from a source `root` template (or
/// archetype) and a `repo` supplying its specialisation parent and every
/// referenced constituent archetype.
///
/// The root's own `template_overlay` blocks are registered as local
/// constituents before flattening, so `use_archetype` references to them
/// resolve (master10 — overlays are local specialised archetypes used as slot
/// fillers).
///
/// # Errors
/// [`OptError::Flatten`] if the lineage cannot be flattened,
/// [`OptError::UnresolvedReference`] if a filler / external reference is absent
/// from `repo`, [`OptError::CyclicReference`] if the fillers reference each
/// other in a cycle, [`OptError::NestingTooDeep`] if inlining composes a tree
/// past the engine bound.
pub fn create_opt(
    root: &Archetype,
    repo: &ArchetypeRepository,
) -> Result<OperationalTemplate, OptError> {
    // A working repository seeded with the passed repo plus the root's local
    // overlays (master10: overlays are local specialised archetypes, referenced
    // as slot fillers and inlined here).
    let mut work = repo.clone();
    for overlay in root_overlays(root) {
        work.insert(Archetype::TemplateOverlay(Box::new(overlay)));
    }

    // Flatten the root's own specialisation lineage (against its `specialize`
    // parent) to the plain flat form (OPT2 master03 §Flattening builds on the
    // standard flattening).
    let flat_root = flat_form(root, &work)?;
    let root_id = hrid_to_string(view(&flat_root).archetype_id);
    let parts = RootParts::of(&flat_root);

    // Apply the OPT-specific transform to the flat definition, gathering the
    // constituent terminologies as fillers are inlined.
    let mut transform = Transform {
        work: &work,
        components: BTreeMap::new(),
        root_id: &root_id,
        inlining: vec![root_id.clone()],
    };
    let opt_def = transform.complex(parts.definition.clone(), Nesting::ROOT)?;
    let components = transform.components;

    Ok(OperationalTemplate {
        // A top-level standalone artefact — no specialisation parent
        // (master02: "no specialisation statement").
        parent_archetype_id: None,
        archetype_id: parts.archetype_id,
        is_differential: false,
        definition: opt_def,
        terminology: parts.terminology,
        rules: openehr_base::containers::present(parts.rules),
        rm_overlay: parts.rm_overlay,
        uid: None,
        original_language: parts.original_language,
        description: parts.description,
        is_controlled: None,
        annotations: parts.annotations,
        translations: parts.translations,
        adl_version: parts.adl_version,
        build_uid: nil_uuid(),
        rm_release: parts.rm_release,
        // A generated artefact (master07.05 `generated`).
        is_generated: true,
        other_meta_data: BTreeMap::new(),
        component_terminologies: (!components.is_empty()).then_some(components),
        terminology_extracts: None,
    })
}

// ── the OPT flattening transform (master03 §Flattening) ────────────────────

/// The state one OPT transform carries down the tree: the constituent
/// repository, the terminologies gathered as fillers are inlined, the root
/// template's id (whose terminology is not a component), and the chain of
/// archetypes currently being inlined, which detects reference cycles.
struct Transform<'a> {
    work: &'a ArchetypeRepository,
    components: BTreeMap<String, ArchetypeTerminology>,
    root_id: &'a str,
    inlining: Vec<String>,
}

impl Transform<'_> {
    /// OPT-transform a `C_COMPLEX_OBJECT` (or `C_ARCHETYPE_ROOT`) root at
    /// `level`: strip sibling markers and rewrite its attribute list. The
    /// object is its own proxy root — the flat artefact against which its
    /// `use_node` target paths resolve.
    fn complex(&mut self, def: CComplexObject, level: Nesting) -> Result<CComplexObject, OptError> {
        let proxy_root = def.clone();
        match def {
            CComplexObject::CComplexObject(mut d) => {
                d.sibling_order = None;
                d.attributes = openehr_base::containers::present(self.attributes(
                    d.attributes.unwrap_or_default(),
                    &proxy_root,
                    level,
                )?);
                Ok(CComplexObject::CComplexObject(d))
            }
            CComplexObject::CArchetypeRoot(mut r) => {
                r.sibling_order = None;
                r.attributes = openehr_base::containers::present(self.attributes(
                    r.attributes.unwrap_or_default(),
                    &proxy_root,
                    level,
                )?);
                Ok(CComplexObject::CArchetypeRoot(r))
            }
        }
    }

    /// OPT-transform an attribute list: drop `existence matches {0}` attributes
    /// (master03 §Flattening — deleted attributes removed), and rewrite each
    /// attribute's children one level deeper through [`Self::object`].
    fn attributes(
        &mut self,
        attrs: Vec<CAttribute>,
        proxy_root: &CComplexObject,
        level: Nesting,
    ) -> Result<Vec<CAttribute>, OptError> {
        let child_level = level
            .descend()
            .map_err(|e| OptError::NestingTooDeep { limit: e.limit })?;
        let mut out = Vec::new();
        for mut attr in attrs {
            // `existence matches {0}` — a logically-removed attribute (master03
            // §Flattening: "attribute nodes that have `existence matches {0}` … are
            // removed").
            if attr
                .existence
                .as_ref()
                .is_some_and(MultiplicityInterval::is_prohibited)
            {
                continue;
            }
            let mut kept = Vec::new();
            for child in std::mem::take(&mut attr.children).into_iter().flatten() {
                if let Some(node) = self.object(child, proxy_root, child_level)? {
                    kept.push(node);
                }
            }
            attr.children = openehr_base::containers::present(kept);
            // Differential paths do not survive into a flat/OPT form.
            attr.differential_path = None;
            out.push(attr);
        }
        Ok(out)
    }

    /// OPT-transform a single object node. Returns `None` when the node is
    /// removed (`occurrences matches {0}` object or a `closed` slot; master03
    /// §Flattening).
    fn object(
        &mut self,
        obj: CObject,
        proxy_root: &CComplexObject,
        level: Nesting,
    ) -> Result<Option<CObject>, OptError> {
        // `occurrences matches {0}` — a logically-removed object (master03
        // §Flattening: "object … nodes with `occurrences matches {0}` [are
        // removed]").
        if child_occurrences(&obj).is_some_and(MultiplicityInterval::is_prohibited) {
            return Ok(None);
        }
        match obj {
            // A `closed` slot is removed; an open slot is a valid runtime extension
            // point and is kept (master03 §Flattening: "all `closed` slots are
            // removed").
            CObject::ArchetypeSlot(mut s) => {
                if s.is_closed {
                    return Ok(None);
                }
                s.sibling_order = None;
                Ok(Some(CObject::ArchetypeSlot(s)))
            }
            // A `use_node` proxy is replaced by an inline copy of its target
            // (master03 §Flattening: "`use_node` internal references are replaced by
            // an inline copy of the target structure"). The copy re-enters one
            // level deeper: a target that contains the proxy itself would
            // otherwise re-inline without end, and the bound turns that into a
            // refusal.
            CObject::CComplexObjectProxy(p) => {
                let inlined = inline_proxy(&p, proxy_root)?;
                let deeper = level
                    .descend()
                    .map_err(|e| OptError::NestingTooDeep { limit: e.limit })?;
                self.object(inlined, proxy_root, deeper)
            }
            CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => {
                let node = self.inline_filler(*r, level)?;
                Ok(Some(node))
            }
            CObject::CComplexObject(CComplexObject::CComplexObject(mut d)) => {
                d.sibling_order = None;
                d.attributes = openehr_base::containers::present(self.attributes(
                    d.attributes.unwrap_or_default(),
                    proxy_root,
                    level,
                )?);
                Ok(Some(CObject::CComplexObject(
                    CComplexObject::CComplexObject(d),
                )))
            }
            // Primitive leaves survive as-is; strip any sibling marker.
            mut other => {
                strip_sibling_order(&mut other);
                Ok(Some(other))
            }
        }
    }

    /// Inline a `C_ARCHETYPE_ROOT` slot-filler / external reference: resolve its
    /// `archetype_ref` to a constituent, flatten that constituent, and populate
    /// the root with the constituent's flattened structure and full-version id
    /// (master03 §Archetype References + §Flattening). The constituent's flat
    /// terminology is gathered under `components` (master03 §Terminology).
    ///
    /// The constituent joins the inlining chain for the duration of its own
    /// transform: meeting it again below itself is a reference cycle.
    fn inline_filler(
        &mut self,
        mut r: CArchetypeRoot,
        level: Nesting,
    ) -> Result<CObject, OptError> {
        let Some(constituent) = self.work.get(&r.archetype_ref) else {
            return Err(OptError::UnresolvedReference(r.archetype_ref.clone()));
        };
        let full_id = hrid_to_string(view(constituent).archetype_id);
        if self.inlining.contains(&full_id) {
            let mut chain = self.inlining.clone();
            chain.push(full_id);
            return Err(OptError::CyclicReference(chain));
        }
        let flat = flat_form(constituent, self.work)?;
        let flat_view = view(&flat);

        // Gather the constituent's flat terminology (master03 §Terminology: the flat
        // terminology of every constituent except the root template). The
        // `component_terminologies` ODIN block carries only the term/binding/value-set
        // maps keyed by archetype id (master10), not the within-archetype
        // `concept_code`, so it is normalised away for serialisation fidelity.
        if full_id != self.root_id {
            self.components.entry(full_id.clone()).or_insert_with(|| {
                let mut term = flat_view.terminology.clone();
                term.concept_code = String::new();
                term
            });
        }

        // The filler's structure (recursively OPT-transformed, so nested fillers /
        // proxies / deletions inside the constituent are handled too), at the
        // level of the slot it fills.
        self.inlining.push(full_id.clone());
        let opt_def = self.complex(flat_view.definition.clone(), level);
        self.inlining.pop();
        let (rm_type, attributes, attribute_tuples) = match opt_def? {
            CComplexObject::CComplexObject(d) => (d.rm_type_name, d.attributes, d.attribute_tuples),
            CComplexObject::CArchetypeRoot(cr) => {
                (cr.rm_type_name, cr.attributes, cr.attribute_tuples)
            }
        };

        // The filler keeps the slot node id it was placed at; the RM type falls back
        // to the constituent's root type where the reference did not carry one.
        if r.rm_type_name.is_empty() {
            r.rm_type_name = rm_type;
        }
        r.archetype_ref = full_id;
        r.sibling_order = None;
        r.attributes = attributes;
        r.attribute_tuples = attribute_tuples;
        Ok(CObject::CComplexObject(CComplexObject::CArchetypeRoot(
            Box::new(r),
        )))
    }
}

/// Inline a `use_node` proxy: copy the object at `proxy.target_path` in
/// `proxy_root`, restamping the proxy's node id / occurrences (master03
/// §Flattening; `C_COMPLEX_OBJECT_PROXY` semantics, master04.5). The local
/// occurrences of the proxy win over the target's where set.
fn inline_proxy(
    proxy: &CComplexObjectProxy,
    proxy_root: &CComplexObject,
) -> Result<CObject, OptError> {
    let Some(mut target) = find_object(proxy_root, &proxy.target_path) else {
        // A `use_node` whose target does not resolve is a VUNP defect; here we
        // cannot inline it — surface it as an unresolved reference so the OPT is
        // not silently wrong.
        return Err(OptError::UnresolvedReference(proxy.target_path.clone()));
    };
    // Restamp identity from the proxy.
    {
        let (_, nid, occ, sib) = common_mut(&mut target);
        proxy.node_id.clone_into(nid);
        if proxy.occurrences.is_some() {
            occ.clone_from(&proxy.occurrences);
        }
        *sib = None;
    }
    Ok(target)
}

/// Find (a clone of) the object node at `path` within `root`.
fn find_object(root: &CComplexObject, path: &str) -> Option<CObject> {
    let segments = parse_path(path);
    if segments.is_empty() {
        return None;
    }
    let mut current: &CComplexObject = root;
    for (idx, seg) in segments.iter().enumerate() {
        let attr = complex_attributes(current)
            .iter()
            .find(|a| a.rm_attribute_name == seg.attribute)?;
        let child = match &seg.node_id {
            Some(nid) => attr
                .children
                .iter()
                .flatten()
                .find(|c| object_node_id(c) == nid)?,
            None if attr.children.as_ref().map_or(0, Vec::len) == 1 => {
                attr.children.iter().flatten().next()?
            }
            None => return None,
        };
        if idx + 1 == segments.len() {
            return Some(child.clone());
        }
        match child {
            CObject::CComplexObject(cco) => current = cco,
            _ => return None,
        }
    }
    None
}

// ── profiled OPT (master04) ────────────────────────────────────────────────

/// Which terminology bindings a profiled OPT removes (master04 §Terminology
/// Binding Filtering).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum BindingFilter {
    /// Keep every binding.
    #[default]
    KeepAll,
    /// Remove the bindings of exactly these terminologies.
    Remove(Vec<String>),
    /// Remove every terminology binding.
    RemoveAll,
}

/// The profiling choices applied to a raw OPT (master04).
///
/// Node-level terminology substitution (`substitute_nodes`) is not
/// implemented — the spec leaves it TBD — and a `true` value returns
/// [`OptError::NodeSubstitutionUnsupported`].
#[derive(Debug, Clone, Default)]
pub struct ProfileSpec {
    /// Languages to keep. `None` keeps every language; `Some(list)` keeps only
    /// the listed languages (≥1 must remain, master04 §Language Filtering).
    pub keep_languages: Option<Vec<String>>,
    /// Remove the whole `annotations` section (master04 §Annotations Removal).
    pub remove_annotations: bool,
    /// Which terminology bindings to remove (master04 §Terminology Binding
    /// Filtering).
    pub bindings: BindingFilter,
    /// Request node-level terminology substitution — unsupported (master04
    /// §Terminology Substitution, TBD).
    pub substitute_nodes: bool,
}

/// Produce a profiled OPT from a raw one, applying `spec` (master04).
///
/// # Errors
/// [`OptError::NodeSubstitutionUnsupported`] if `spec.substitute_nodes` is set,
/// or [`OptError::NoLanguagesLeft`] if language filtering would remove every
/// language.
pub fn profile_opt(
    opt: &OperationalTemplate,
    spec: &ProfileSpec,
) -> Result<OperationalTemplate, OptError> {
    if spec.substitute_nodes {
        return Err(OptError::NodeSubstitutionUnsupported);
    }
    let mut out = opt.clone();

    // Annotations removal (master04 §Annotations Removal).
    if spec.remove_annotations {
        out.annotations = None;
    }

    // Language filtering (master04 §Language Filtering).
    if let Some(keep) = &spec.keep_languages {
        filter_languages(&mut out, keep)?;
    }

    // Terminology binding filtering (master04 §Terminology Binding Filtering).
    filter_bindings(&mut out.terminology, &spec.bindings);
    if let Some(components) = out.component_terminologies.as_mut() {
        for term in components.values_mut() {
            filter_bindings(term, &spec.bindings);
        }
    }

    Ok(out)
}

/// Keep only the listed languages across the OPT's language / terminology /
/// description sections (master04 §Language Filtering). At least one language
/// must remain.
fn filter_languages(opt: &mut OperationalTemplate, keep: &[String]) -> Result<(), OptError> {
    let keep_set: std::collections::BTreeSet<&str> = keep.iter().map(String::as_str).collect();

    // The resulting language set must be non-empty.
    let root_lang = opt.original_language.code_string.clone();
    let surviving: Vec<&String> = opt
        .terminology
        .term_definitions
        .keys()
        .filter(|l| keep_set.contains(l.as_str()))
        .collect();
    if surviving.is_empty() && !keep_set.contains(root_lang.as_str()) {
        return Err(OptError::NoLanguagesLeft);
    }

    retain_languages_terminology(&mut opt.terminology, &keep_set);
    if let Some(components) = opt.component_terminologies.as_mut() {
        for term in components.values_mut() {
            retain_languages_terminology(term, &keep_set);
        }
    }
    if let Some(translations) = opt.translations.as_mut() {
        translations.retain(|lang, _| keep_set.contains(lang.as_str()));
    }
    if let Some(description) = opt.description.as_mut()
        && let Some(details) = description.details.as_mut()
    {
        details.retain(|lang, _| keep_set.contains(lang.as_str()));
    }
    Ok(())
}

fn retain_languages_terminology(
    term: &mut ArchetypeTerminology,
    keep: &std::collections::BTreeSet<&str>,
) {
    term.term_definitions
        .retain(|lang, _| keep.contains(lang.as_str()));
}

/// Remove terminology bindings per `filter` (master04 §Terminology Binding
/// Filtering).
fn filter_bindings(term: &mut ArchetypeTerminology, filter: &BindingFilter) {
    match filter {
        BindingFilter::KeepAll => {}
        BindingFilter::RemoveAll => term.term_bindings = None,
        BindingFilter::Remove(terminologies) => {
            if let Some(bindings) = term.term_bindings.as_mut() {
                for t in terminologies {
                    bindings.remove(t);
                }
                if bindings.is_empty() {
                    term.term_bindings = None;
                }
            }
        }
    }
}

// ── root-field extraction ──────────────────────────────────────────────────

/// The overlays a root template carries (empty for any other artefact kind).
fn root_overlays(
    root: &Archetype,
) -> Vec<openehr_am::v2_4::aom2::archetype::template_overlay::TemplateOverlay> {
    match root {
        Archetype::AuthoredArchetype(a) => match a.as_ref() {
            AuthoredArchetype::Template(t) => t.overlays.clone().unwrap_or_default(),
            AuthoredArchetype::AuthoredArchetype(_) | AuthoredArchetype::OperationalTemplate(_) => {
                Vec::new()
            }
        },
        Archetype::TemplateOverlay(_) => Vec::new(),
    }
}

/// The root fields the OPT header carries, cloned from the flat root artefact.
struct RootParts {
    archetype_id: openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid,
    definition: CComplexObject,
    terminology: ArchetypeTerminology,
    rules: Vec<StatementSet>,
    rm_overlay: Option<RmOverlay>,
    original_language: TerminologyCode,
    description: Option<ResourceDescription>,
    annotations: Option<ResourceAnnotations>,
    translations: Option<BTreeMap<String, TranslationDetails>>,
    adl_version: Option<String>,
    rm_release: String,
}

impl RootParts {
    fn of(flat_root: &Archetype) -> Self {
        let v = view(flat_root);
        Self {
            archetype_id: v.archetype_id.clone(),
            definition: v.definition.clone(),
            terminology: v.terminology.clone(),
            rules: rules_of(flat_root),
            rm_overlay: v.rm_overlay.cloned(),
            original_language: v
                .original_language
                .cloned()
                .unwrap_or_else(fallback_language),
            description: v.description.cloned(),
            annotations: v.annotations.cloned(),
            translations: v.translations.cloned(),
            adl_version: v.adl_version.map(str::to_owned),
            rm_release: v.rm_release.to_owned(),
        }
    }
}

fn rules_of(archetype: &Archetype) -> Vec<StatementSet> {
    match archetype {
        Archetype::AuthoredArchetype(a) => match a.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => d.rules.clone().unwrap_or_default(),
            AuthoredArchetype::Template(t) => t.rules.clone().unwrap_or_default(),
            AuthoredArchetype::OperationalTemplate(o) => o.rules.clone().unwrap_or_default(),
        },
        Archetype::TemplateOverlay(t) => t.rules.clone().unwrap_or_default(),
    }
}

fn fallback_language() -> TerminologyCode {
    TerminologyCode {
        terminology_id: "ISO_639-1".to_owned(),
        terminology_version: None,
        code_string: "en".to_owned(),
        uri: None,
    }
}
