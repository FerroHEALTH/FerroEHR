// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! OPT2 operational-template generation, raw and profiled
//! (`AM/docs/OPT2/master02-overview.adoc`, `master03-opt_raw.adoc`,
//! `master04-opt_profiled.adoc`; `ADL2/master10-templates.adoc`).
//!
//! [`create_opt`] flattens the template's lineage and applies the OPT extras of
//! master03 §Flattening: full-version references, inlined fillers and
//! `use_node` targets, closed slots and `{0}` attributes and objects removed, the
//! parent kept as `parent_archetype_id` (AOM2 §3.6.6), the constituents'
//! terminologies under `component_terminologies`, and each overlay's parent
//! under [`OVERLAY_PARENTS_KEY`] (our own design).
//!
//! NOTE: master02 and master03 §Artefact Structure print no `specialize` section
//! for an OPT, and the normative checklist wins over master10's listing.

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

use crate::adl14::authored_flat::flattening_parent;
use crate::adl14::term_constraint::rewrite_as_authored;
use crate::aom::access::{
    child_occurrences, common_mut, complex_attributes, object_node_id, strip_sibling_order,
};
use crate::aom::build::complex_object;
use crate::artefact::{ArchetypeRepository, view};
use crate::flatten::{FlattenError, flat_form, overlay_node};
use crate::odin::nil_uuid;
use crate::paths::{child_path, parse_path};
use openehr_lang::nesting::Nesting;

mod profile;

/// A failure while generating or profiling an operational template.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum OptError {
    /// Flattening the template's (or a constituent's) specialisation lineage
    /// failed.
    #[error("flattening failed while building the OPT")]
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

/// The `other_meta_data` key under which [`create_opt`] records each inlined
/// overlay's parent, as `,`-separated `<overlay full id>=<parent full id>` pairs
/// in overlay order (our own design).
pub const OVERLAY_PARENTS_KEY: &str = "template_overlay_parents";

/// A raw operational template with the attributes its transform removed.
///
/// OPT2 `master03` §Flattening removes `existence matches {0}` attributes, so
/// the paths keep the prohibition beside the OPT (no specification governs this:
/// our own design).
#[derive(Debug, Clone, PartialEq)]
pub struct RawOpt {
    /// The raw operational template.
    pub template: OperationalTemplate,
    /// The path of every attribute removed for `existence matches {0}`, as
    /// the ADL path of its owning object in the OPT followed by
    /// `/<attribute>` (`/context` for one of the root), in walk order.
    pub prohibited_attributes: Vec<String>,
}

/// Generate the raw operational template of `root` with the attributes it
/// removed ([`RawOpt`]); `repo` supplies the parent and constituents.
///
/// The root's `template_overlay` blocks resolve as local constituents (master10).
/// An ADL 1.4 constituent keeps its node codes, with each term constraint
/// rewritten to the ADL2 form (`ADL2/master04.5` §Formal Terminology Constraint).
///
/// # Errors
/// [`OptError::Flatten`] if the lineage cannot be flattened,
/// [`OptError::UnresolvedReference`] if a filler / external reference is absent
/// from `repo`, [`OptError::CyclicReference`] if the fillers reference each
/// other in a cycle, [`OptError::NestingTooDeep`] if inlining composes a tree
/// past the engine bound.
pub fn create_opt(root: &Archetype, repo: &ArchetypeRepository) -> Result<RawOpt, OptError> {
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
    let root_id = view(&flat_root).archetype_id.to_string();
    let mut parts = RootParts::of(&flat_root);
    rewrite_as_authored(&mut parts.definition, &mut parts.terminology);

    // Apply the OPT-specific transform to the flat definition, gathering the
    // constituent terminologies as fillers are inlined.
    let mut transform = Transform {
        work: &work,
        components: BTreeMap::new(),
        overlay_parents: BTreeMap::new(),
        root_id: &root_id,
        inlining: vec![root_id.clone()],
        prohibited: Vec::new(),
    };
    let opt_def = transform.complex(parts.definition.clone(), Nesting::ROOT, "")?;
    let components = transform.components;
    let mut other_meta_data = BTreeMap::new();
    if !transform.overlay_parents.is_empty() {
        let pairs: Vec<String> = transform
            .overlay_parents
            .iter()
            .map(|(overlay, parent)| format!("{overlay}={parent}"))
            .collect();
        other_meta_data.insert(OVERLAY_PARENTS_KEY.to_owned(), pairs.join(","));
    }

    let template = OperationalTemplate {
        parent_archetype_id: root_parent(root, &work),
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
        other_meta_data,
        component_terminologies: (!components.is_empty()).then_some(components),
        terminology_extracts: None,
    };
    Ok(RawOpt {
        template,
        prohibited_attributes: transform.prohibited,
    })
}

/// The specialisation parent of `root` as its full id (AOM2 §3.6.6; OPT2
/// `master02`); an unresolvable reference stays as written.
fn root_parent(root: &Archetype, work: &ArchetypeRepository) -> Option<String> {
    let root_view = view(root);
    let parent_id = root_view.parent_archetype_id?;
    Some(
        work.resolve(parent_id, root_view.archetype_id.namespace.as_deref())
            .map_or_else(
                || parent_id.to_owned(),
                |parent| view(parent).archetype_id.to_string(),
            ),
    )
}

/// Every archetype the operational template of `root` needs and `repo` does not
/// hold, sorted and deduplicated (OPT2 `master03` §Archetype References; VARXR).
#[must_use]
pub fn unresolved_constituents(root: &Archetype, repo: &ArchetypeRepository) -> Vec<String> {
    let mut work = repo.clone();
    for overlay in root_overlays(root) {
        work.insert(Archetype::TemplateOverlay(Box::new(overlay)));
    }
    let mut missing = std::collections::BTreeSet::new();
    let mut seen = std::collections::BTreeSet::new();
    let mut queue = vec![root];
    seen.insert(view(root).archetype_id.to_string());
    while let Some(artefact) = queue.pop() {
        let mut lineage = artefact;
        while let Some(parent_id) = flattening_parent(&view(lineage)) {
            match work.resolve(parent_id, view(lineage).archetype_id.namespace.as_deref()) {
                Some(parent) if seen.insert(view(parent).archetype_id.to_string()) => {
                    lineage = parent;
                }
                Some(_) => break,
                None => {
                    missing.insert(parent_id.to_owned());
                    break;
                }
            }
        }
        let flat = flat_form(artefact, &work).ok();
        let definition = flat
            .as_ref()
            .map_or_else(|| view(artefact).definition, |f| view(f).definition);
        let mut roots = Vec::new();
        crate::validate::slots::collect_roots(definition, &mut roots);
        for reference in roots {
            if reference.archetype_ref.is_empty() {
                continue;
            }
            let namespace = view(artefact).archetype_id.namespace.as_deref();
            match work.resolve(&reference.archetype_ref, namespace) {
                Some(constituent) => {
                    if seen.insert(view(constituent).archetype_id.to_string()) {
                        queue.push(constituent);
                    }
                }
                None => {
                    missing.insert(reference.archetype_ref.clone());
                }
            }
        }
    }
    missing.into_iter().collect()
}

// ── the OPT flattening transform (master03 §Flattening) ────────────────────

/// The state one OPT transform carries down the tree: the constituent
/// repository, the terminologies gathered as fillers are inlined, the root
/// template's id (whose terminology is not a component), and the chain of
/// archetypes currently being inlined, which detects reference cycles.
struct Transform<'a> {
    work: &'a ArchetypeRepository,
    components: BTreeMap<String, ArchetypeTerminology>,
    /// The full id of each inlined template overlay's specialisation parent,
    /// keyed by the overlay's full id.
    overlay_parents: BTreeMap<String, String>,
    root_id: &'a str,
    inlining: Vec<String>,
    /// The path of every attribute removed for `existence matches {0}`.
    prohibited: Vec<String>,
}

impl Transform<'_> {
    /// OPT-transform a `C_COMPLEX_OBJECT` (or `C_ARCHETYPE_ROOT`) root at
    /// `level` and OPT path `path`: strip sibling markers and rewrite its
    /// attribute list. The object is its own proxy root — the flat artefact
    /// against which its `use_node` target paths resolve.
    fn complex(
        &mut self,
        def: CComplexObject,
        level: Nesting,
        path: &str,
    ) -> Result<CComplexObject, OptError> {
        let proxy_root = def.clone();
        match def {
            CComplexObject::CComplexObject(mut d) => {
                d.sibling_order = None;
                d.attributes = openehr_base::containers::present(self.attributes(
                    d.attributes.unwrap_or_default(),
                    &proxy_root,
                    level,
                    path,
                )?);
                Ok(CComplexObject::CComplexObject(d))
            }
            CComplexObject::CArchetypeRoot(mut r) => {
                r.sibling_order = None;
                r.attributes = openehr_base::containers::present(self.attributes(
                    r.attributes.unwrap_or_default(),
                    &proxy_root,
                    level,
                    path,
                )?);
                Ok(CComplexObject::CArchetypeRoot(r))
            }
        }
    }

    /// OPT-transform an attribute list: drop `existence matches {0}` attributes
    /// (master03 §Flattening — deleted attributes removed) and record their
    /// paths under the owning object's `path`, and rewrite each attribute's
    /// children one level deeper through [`Self::object`].
    fn attributes(
        &mut self,
        attrs: Vec<CAttribute>,
        proxy_root: &CComplexObject,
        level: Nesting,
        path: &str,
    ) -> Result<Vec<CAttribute>, OptError> {
        let child_level = level
            .descend()
            .map_err(|e| OptError::NestingTooDeep { limit: e.limit })?;
        let mut out = Vec::new();
        for mut attr in attrs {
            let attr_path = format!("{path}/{}", attr.rm_attribute_name);
            // `existence matches {0}` — a logically-removed attribute (master03
            // §Flattening: "attribute nodes that have `existence matches {0}` … are
            // removed").
            if attr
                .existence
                .as_ref()
                .is_some_and(MultiplicityInterval::is_prohibited)
            {
                self.prohibited.push(attr_path);
                continue;
            }
            let mut kept = Vec::new();
            for child in std::mem::take(&mut attr.children).into_iter().flatten() {
                if let Some(node) = self.object(child, proxy_root, child_level, &attr_path)? {
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

    /// OPT-transform a single object node under the attribute at `attr_path`.
    /// Returns `None` when the node is removed (`occurrences matches {0}`
    /// object or a `closed` slot; master03 §Flattening).
    fn object(
        &mut self,
        obj: CObject,
        proxy_root: &CComplexObject,
        level: Nesting,
        attr_path: &str,
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
            // master03 §Flattening: "`use_node` internal references are replaced by an
            // inline copy of the target structure"; the depth bound stops a self-cycle.
            CObject::CComplexObjectProxy(p) => {
                let inlined = inline_proxy(&p, proxy_root)?;
                let deeper = level
                    .descend()
                    .map_err(|e| OptError::NestingTooDeep { limit: e.limit })?;
                self.object(inlined, proxy_root, deeper, attr_path)
            }
            CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => {
                let path = child_path(attr_path, &r.node_id);
                let node = self.inline_filler(*r, level, &path)?;
                Ok(Some(node))
            }
            CObject::CComplexObject(CComplexObject::CComplexObject(mut d)) => {
                d.sibling_order = None;
                let path = child_path(attr_path, &d.node_id);
                d.attributes = openehr_base::containers::present(self.attributes(
                    d.attributes.unwrap_or_default(),
                    proxy_root,
                    level,
                    &path,
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

    /// Inline a `C_ARCHETYPE_ROOT` filler with its constituent's flattened structure
    /// and full-version id, gathering its terminology (master03 §Archetype References,
    /// §Flattening, §Terminology); meeting it again below itself is a cycle.
    fn inline_filler(
        &mut self,
        mut r: CArchetypeRoot,
        level: Nesting,
        path: &str,
    ) -> Result<CObject, OptError> {
        let work = self.work;
        let referrer = self.inlining.last().and_then(|id| id.rsplit_once("::"));
        let Some(constituent) = work.resolve(&r.archetype_ref, referrer.map(|(ns, _)| ns)) else {
            return Err(OptError::UnresolvedReference(r.archetype_ref.clone()));
        };
        let full_id = view(constituent).archetype_id.to_string();
        if self.inlining.contains(&full_id) {
            let mut chain = self.inlining.clone();
            chain.push(full_id);
            return Err(OptError::CyclicReference(chain));
        }
        if let Archetype::TemplateOverlay(overlay) = constituent
            && let Some(parent) = overlay.parent_archetype_id.as_deref()
        {
            let parent_id = work
                .resolve(parent, overlay.archetype_id.namespace.as_deref())
                .map_or_else(
                    || parent.to_owned(),
                    |found| view(found).archetype_id.to_string(),
                );
            self.overlay_parents.insert(full_id.clone(), parent_id);
        }
        let flat = flat_form(constituent, self.work)?;
        let flat_view = view(&flat);
        let mut definition = flat_view.definition.clone();
        let mut terminology = flat_view.terminology.clone();
        rewrite_as_authored(&mut definition, &mut terminology);
        // NOTE: a `use_archetype` body redefines the referenced archetype at this node
        // (`ADL2/master10-templates.adoc` OPT2 example), so it overlays the flat form.
        if let CComplexObject::CComplexObject(base) = &definition
            && !(r.attributes.as_ref().is_none_or(Vec::is_empty)
                && r.attribute_tuples.as_ref().is_none_or(Vec::is_empty))
        {
            let body = complex_object(
                String::new(),
                base.node_id.clone(),
                r.attributes.take().unwrap_or_default(),
                r.attribute_tuples.take().unwrap_or_default(),
                None,
            );
            let base = CObject::CComplexObject(definition.clone());
            if let CObject::CComplexObject(overlaid) = overlay_node(&base, &body)? {
                definition = overlaid;
            }
        }

        // Every constituent's flat terminology but the root's (master03 §Terminology),
        // without `concept_code` (master10).
        if full_id != self.root_id {
            let mut component = terminology;
            component.concept_code = String::new();
            self.components.entry(full_id.clone()).or_insert(component);
        }

        // The filler's structure (recursively OPT-transformed, so nested fillers /
        // proxies / deletions inside the constituent are handled too), at the
        // level of the slot it fills.
        self.inlining.push(full_id.clone());
        let opt_def = self.complex(definition, level, path);
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

/// The profiling choices applied to a raw OPT (master04); `substitute_nodes` is
/// refused with [`OptError::NodeSubstitutionUnsupported`].
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
        profile::filter_languages(&mut out, keep)?;
    }

    // Terminology binding filtering (master04 §Terminology Binding Filtering).
    profile::filter_bindings(&mut out.terminology, &spec.bindings);
    if let Some(components) = out.component_terminologies.as_mut() {
        for term in components.values_mut() {
            profile::filter_bindings(term, &spec.bindings);
        }
    }

    Ok(out)
}

// ── root-field extraction ──────────────────────────────────────────────────

/// The overlays a root template carries (empty for any other artefact kind).
pub(crate) fn root_overlays(
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
