// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 2, analyze: the include-closure [`Model`], polymorphic seams,
//! ownership graph and back-references, the constructibility proof and the
//! cross-schema re-emission closure. The Rust type text is in
//! [`crate::render::model_types`].

use crate::load::bmm::{BmmClass, BmmPropKind, BmmSchema, BmmType};
use crate::plan::overrides::{
    back_reference, is_mapped_class, primitive, subtype_extension_parents,
};
use crate::plan::{Emission, decide};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) mod invariants;
pub(crate) mod reemit;

/// A merged BMM model (e.g. BASE + RM) used for ancestor flattening and type
/// resolution across schema boundaries.
pub(crate) struct Model {
    pub(crate) classes: BTreeMap<String, BmmClass>,
    /// Lazy memo of [`nonempty_optional_lists`] (consulted per field).
    nonempty_memo: std::sync::OnceLock<BTreeSet<(String, String)>>,
    /// Each class's `schema_name` (the last schema defining it wins, as for
    /// [`Self::classes`]).
    schema_of: BTreeMap<String, String>,
    /// The size estimate behind the by-size boxing decision
    /// ([`crate::plan::layout`]).
    layout: crate::plan::layout::LayoutMemo,
}

/// Spec types provided by dependency crates, resolved to the paired dependency
/// generation's defining module by full path.
#[derive(Default)]
pub(crate) struct External {
    /// Spec class name → full module path of the dependency generation's defining
    /// module, first map wins.
    deps: Vec<BTreeMap<String, String>>,
    /// Path to the container-shape module that names `NonEmptyVec`.
    containers: String,
}

impl External {
    /// Points the container-shape path at the crate being emitted.
    pub(crate) fn in_crate(mut self, crate_name: &str) -> Self {
        self.containers = if crate_name == "openehr-base" {
            "crate::containers".to_string()
        } else {
            "openehr_base::containers".to_string()
        };
        self
    }

    /// The path the `NonEmptyVec` container shape is named from.
    pub(crate) fn containers_path(&self) -> &str {
        &self.containers
    }

    /// Register one dependency generation's exported spec names, each mapped
    /// to the full Rust path of its defining module.
    pub(crate) fn with(mut self, modules: BTreeMap<String, String>) -> Self {
        self.deps.push(modules);
        self
    }

    /// The full defining-module path a dependency generation exports `spec`
    /// from, if any (first registered match wins).
    pub(crate) fn module_of(&self, spec: &str) -> Option<&str> {
        self.deps
            .iter()
            .find_map(|modules| modules.get(spec))
            .map(String::as_str)
    }

    /// Whether a dependency generation exports `spec`.
    pub(crate) fn contains(&self, spec: &str) -> bool {
        self.module_of(spec).is_some()
    }
}

/// The spec class names a schema will actually emit (non-skipped), for building
/// the [`External`] index a downstream crate resolves against.
#[must_use]
pub(crate) fn emittable_specs(model: &Model, schema: &BmmSchema) -> BTreeSet<String> {
    let used = model.used_as_type();
    schema
        .classes
        .iter()
        .filter(|(_, c)| !matches!(decide(model, c, &used), Emission::Skip))
        .map(|(n, _)| n.clone())
        .collect()
}

/// A property resolved onto a concrete class, tracking which class it came from
/// (for the `// inherited: X` banner).
pub(crate) struct ResolvedProp<'a> {
    pub(crate) owner: String,
    pub(crate) prop: &'a crate::load::bmm::BmmProperty,
}

impl Model {
    /// Merge several schemas into one class map (later schemas override earlier
    /// on name collision — pass BASE before RM).
    #[must_use]
    pub(crate) fn merged(schemas: &[&BmmSchema]) -> Self {
        let mut classes = BTreeMap::new();
        let mut schema_of = BTreeMap::new();
        for s in schemas {
            for (name, class) in &s.classes {
                classes.insert(name.clone(), class.clone());
                schema_of.insert(name.clone(), s.schema_name.clone());
            }
        }
        Model {
            classes,
            nonempty_memo: std::sync::OnceLock::new(),
            schema_of,
            layout: crate::plan::layout::LayoutMemo::default(),
        }
    }

    /// Whether `class` is defined by a schema named `schema_name`.
    pub(crate) fn in_schema(&self, class: &str, schema_name: &str) -> bool {
        self.schema_of.get(class).is_some_and(|s| s == schema_name)
    }

    /// The size-estimate memo of [`crate::plan::layout`].
    pub(crate) fn layout_memo(&self) -> &crate::plan::layout::LayoutMemo {
        &self.layout
    }

    pub(crate) fn get(&self, name: &str) -> Option<&BmmClass> {
        self.classes.get(name)
    }

    /// Iterate every class in the merged model, in name order.
    pub(crate) fn class_iter(&self) -> impl Iterator<Item = (&String, &BmmClass)> {
        self.classes.iter()
    }

    /// Whether `name` is a generic parameter declared on `class` or an ancestor.
    pub(crate) fn is_generic_param(&self, class: &str, name: &str) -> bool {
        let Some(c) = self.get(class) else {
            return false;
        };
        c.generic_params.iter().any(|g| g.name == name)
            || c.ancestors.iter().any(|a| self.is_generic_param(a, name))
    }

    /// Is `name` mapped to Rust rather than emitted (a [`primitive`] or a
    /// [`crate::plan::overrides::MAPPED_CLASSES`] entry)?
    pub(crate) fn is_mapped(name: &str) -> bool {
        primitive(name).is_some() || is_mapped_class(name)
    }

    /// Whether `class` inherits from `target` transitively, through the declared
    /// `ancestors` and [`crate::plan::overrides::SUBTYPE_EXTENSIONS`].
    pub(crate) fn inherits(&self, class: &str, target: &str) -> bool {
        let Some(c) = self.get(class) else {
            return false;
        };
        for a in c.ancestors.iter().map(String::as_str) {
            if a == target || self.inherits(a, target) {
                return true;
            }
        }
        // A separate loop: the `&'static str` items would force `self`'s borrow to
        // `'static`.
        for a in subtype_extension_parents(class) {
            if a == target || self.inherits(a, target) {
                return true;
            }
        }
        false
    }

    /// Whether `from` reaches `target` through single-valued field types, a cycle
    /// that needs boxing.
    pub(crate) fn reaches(&self, from: &str, target: &str, seen: &mut BTreeSet<String>) -> bool {
        if from == target {
            return true;
        }
        if !seen.insert(from.to_string()) {
            return false;
        }
        let Some(class) = self.get(from) else {
            return false;
        };
        // An abstract class is an enum, so a cycle can run through its variants.
        if class.is_abstract {
            return self
                .enum_variants(from)
                .iter()
                .any(|d| d == target || self.reaches(d, target, seen));
        }
        self.flattened_props(class).iter().any(|rp| {
            let BmmPropKind::Single(t) = &rp.prop.kind else {
                return false;
            };
            let root = t.root_name();
            !Self::is_mapped(root) && (root == target || self.reaches(root, target, seen))
        })
    }

    /// The immediate concrete subtypes of `name`, the variants of its enum.
    ///
    /// A descendant below another concrete descendant is covered by that one's
    /// enum, so the enums nest (`DATA_VALUE ⊇ DvText ⊇ DvCodedText`).
    pub(crate) fn enum_variants(&self, name: &str) -> Vec<String> {
        let all: Vec<String> = self
            .classes
            .values()
            .filter(|c| {
                !c.is_abstract
                    && c.name != name
                    && !Self::is_mapped(&c.name)
                    && self.inherits(&c.name, name)
            })
            .map(|c| c.name.clone())
            .collect();
        all.iter()
            .filter(|d| {
                !all.iter()
                    .any(|c| c.as_str() != d.as_str() && self.inherits(d, c))
            })
            .cloned()
            .collect()
    }

    /// The class names this class's flattened field types reference.
    fn field_roots(&self, name: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        if let Some(class) = self.get(name) {
            for rp in self.flattened_props(class) {
                match &rp.prop.kind {
                    BmmPropKind::Single(t) => collect_roots(t, &mut out),
                    BmmPropKind::Container { item, .. } => collect_roots(item, &mut out),
                }
            }
        }
        out
    }

    /// Class names used anywhere as a property type — enum-slot candidates.
    pub(crate) fn used_as_type(&self) -> BTreeSet<String> {
        let mut used = BTreeSet::new();
        for c in self.classes.values() {
            for p in &c.properties {
                match &p.kind {
                    BmmPropKind::Single(t) => collect_roots(t, &mut used),
                    BmmPropKind::Container { item, .. } => collect_roots(item, &mut used),
                }
            }
        }
        used
    }

    /// Flatten a class's properties, ancestor-first, with child redefinitions
    /// overriding the inherited type in place.
    pub(crate) fn flattened_props(&self, class: &BmmClass) -> Vec<ResolvedProp<'_>> {
        let mut order: Vec<String> = Vec::new();
        let mut map: BTreeMap<String, ResolvedProp<'_>> = BTreeMap::new();
        self.gather(&class.name, &mut order, &mut map);
        order.into_iter().filter_map(|n| map.remove(&n)).collect()
    }

    fn gather<'a>(
        &'a self,
        class_name: &str,
        order: &mut Vec<String>,
        map: &mut BTreeMap<String, ResolvedProp<'a>>,
    ) {
        let Some(class) = self.get(class_name) else {
            return;
        };
        for anc in &class.ancestors {
            self.gather(anc, order, map);
        }
        for p in &class.properties {
            if !map.contains_key(&p.name) {
                order.push(p.name.clone());
            }
            map.insert(
                p.name.clone(),
                ResolvedProp {
                    owner: class.name.clone(),
                    prop: p,
                },
            );
        }
    }

    // ── generic-parameter + type-graph resolution ───────────────────────────

    /// The generic params a class's flattened fields use as a bare param type, as
    /// `(name, bound)` in declaration order.
    pub(crate) fn used_generic_params(&self, name: &str) -> Vec<(String, Option<String>)> {
        let Some(class) = self.get(name) else {
            return Vec::new();
        };
        if class.generic_params.is_empty() {
            return Vec::new();
        }
        // An abstract class's enum is generic over the params any concrete variant
        // uses (`ORIGINAL_VERSION.data: T`).
        if class.is_abstract {
            let variants = self.enum_variants(name);
            return class
                .generic_params
                .iter()
                .filter(|g| {
                    variants.iter().any(|d| {
                        self.used_generic_params(d)
                            .iter()
                            .any(|(n, _)| *n == g.name)
                    })
                })
                .map(|g| (g.name.clone(), self.resolved_param_bound(name, &g.name)))
                .collect();
        }
        let declared: BTreeSet<&str> = class
            .generic_params
            .iter()
            .map(|g| g.name.as_str())
            .collect();
        // NOTE: `plan::elided_arguments::ELIDED_ARGUMENTS` restores the arguments the
        // BMM drops, so a parameter counts only where it appears literally.
        let mut used = BTreeSet::new();
        for rp in self.flattened_props(class) {
            match &rp.prop.kind {
                BmmPropKind::Single(t) | BmmPropKind::Container { item: t, .. } => {
                    collect_param_uses(t, &declared, &mut used);
                }
            }
        }
        class
            .generic_params
            .iter()
            .filter(|g| used.contains(g.name.as_str()))
            .map(|g| (g.name.clone(), self.resolved_param_bound(name, &g.name)))
            .collect()
    }

    /// The `conforms_to` bound of `param` on `class`, inherited by parameter name
    /// up the ancestor chain.
    pub(crate) fn resolved_param_bound(&self, class_name: &str, param: &str) -> Option<String> {
        let class = self.get(class_name)?;
        if let Some(g) = class.generic_params.iter().find(|g| g.name == param)
            && let Some(bound) = &g.conforms_to
        {
            return Some(bound.clone());
        }
        class
            .ancestors
            .iter()
            .find_map(|anc| self.resolved_param_bound(anc, param))
    }

    /// The bounds to auto-fill for a bare reference to a generic class, or `None`
    /// when it uses no generic params.
    pub(crate) fn generic_param_bounds(&self, name: &str) -> Option<Vec<Option<String>>> {
        let used = self.used_generic_params(name);
        if used.is_empty() {
            None
        } else {
            Some(used.into_iter().map(|(_, bound)| bound).collect())
        }
    }

    /// Every spec class name the rendered type of `t` embeds by value, as
    /// [`Self::render_type`] renders it.
    pub(crate) fn effective_roots(&self, t: &BmmType, out: &mut BTreeSet<String>) {
        match t {
            BmmType::Simple(n) => {
                out.insert(n.clone());
                if let Some(bounds) = self.generic_param_bounds(n) {
                    for b in bounds.into_iter().flatten() {
                        self.effective_roots(&BmmType::Simple(b), out);
                    }
                }
            }
            BmmType::Generic { root, params } => {
                out.insert(root.clone());
                // A monomorphized spec class drops its args, as `render_type` does.
                let keeps_args = matches!(root.as_str(), "List" | "Array" | "Set" | "Hash")
                    || self.generic_param_bounds(root).is_some();
                if keeps_args {
                    for p in params {
                        self.effective_roots(p, out);
                    }
                }
            }
        }
    }

    /// The constructible class names, as a least fixpoint: an abstract class if
    /// any variant is, a concrete class if every mandatory single-valued field
    /// type is.
    fn constructible_classes(&self) -> BTreeSet<String> {
        let mut ok: BTreeSet<String> = BTreeSet::new();
        while self.mark_constructible_round(&mut ok) {}
        ok
    }

    /// Runs one round of the constructibility fixpoint, returning whether `ok`
    /// grew.
    fn mark_constructible_round(&self, ok: &mut BTreeSet<String>) -> bool {
        let mut changed = false;
        for (name, class) in &self.classes {
            if ok.contains(name) || Self::is_mapped(name) {
                continue;
            }
            if self.class_is_constructible(name, class, ok) {
                ok.insert(name.clone());
                changed = true;
            }
        }
        changed
    }

    /// Whether one class is constructible given the classes already known to be.
    fn class_is_constructible(&self, name: &str, class: &BmmClass, ok: &BTreeSet<String>) -> bool {
        if class.is_abstract {
            return self
                .enum_variants(name)
                .iter()
                .any(|v| ok.contains(v) || Self::is_mapped(v));
        }
        self.flattened_props(class)
            .iter()
            .all(|rp| self.prop_allows_construction(rp, ok))
    }

    /// Whether one property leaves its owner constructible: only a mandatory,
    /// single-valued, non-back-reference field of a defined model class not yet
    /// known constructible blocks.
    fn prop_allows_construction(&self, rp: &ResolvedProp<'_>, ok: &BTreeSet<String>) -> bool {
        if !rp.prop.is_mandatory || back_reference(&rp.owner, &rp.prop.name).is_some() {
            return true;
        }
        let BmmPropKind::Single(t) = &rp.prop.kind else {
            return true;
        };
        if let BmmType::Generic { root, .. } = t
            && matches!(root.as_str(), "List" | "Array" | "Set" | "Hash")
        {
            return true;
        }
        let mut roots = BTreeSet::new();
        self.effective_roots(t, &mut roots);
        roots
            .iter()
            .all(|r| Self::is_mapped(r) || self.get(r).is_none() || ok.contains(r))
    }

    /// The concrete emittable classes of `schema` that are not constructible,
    /// sorted; each must be broken at a spec-cited [`back_reference`].
    #[must_use]
    pub(crate) fn constructibility_violations(&self, schema: &BmmSchema) -> Vec<String> {
        let ok = self.constructible_classes();
        let mut bad: Vec<String> = schema
            .classes
            .values()
            .filter(|c| !c.is_abstract && !Self::is_mapped(&c.name))
            .map(|c| c.name.clone())
            .filter(|n| !ok.contains(n))
            .collect();
        bad.sort_unstable();
        bad
    }

    /// Panics unless every concrete emittable class in `schema` is constructible.
    pub(crate) fn assert_constructible(&self, schema: &BmmSchema) {
        let bad = self.constructibility_violations(schema);
        assert!(
            bad.is_empty(),
            "openehr-codegen: non-constructible type(s) would be emitted — an unbroken \
             mandatory single-valued construction cycle: {bad:?}. Break each cycle at its \
             owner/parent back-reference edge by adding a spec-cited `back_reference` entry; \
             never relax a forward/mandatory data field to `Option`."
        );
    }
}

fn collect_roots(t: &BmmType, out: &mut BTreeSet<String>) {
    match t {
        BmmType::Simple(n) => {
            out.insert(n.clone());
        }
        BmmType::Generic { root, params } => {
            out.insert(root.clone());
            for p in params {
                collect_roots(p, out);
            }
        }
    }
}

/// Record which of `declared` generic-parameter names appear explicitly in `t`
/// (as the bare param type or an explicit generic argument).
fn collect_param_uses(t: &BmmType, declared: &BTreeSet<&str>, out: &mut BTreeSet<String>) {
    match t {
        BmmType::Simple(n) => {
            if declared.contains(n.as_str()) {
                out.insert(n.clone());
            }
        }
        BmmType::Generic { root, params } => {
            if declared.contains(root.as_str()) {
                out.insert(root.clone());
            }
            for p in params {
                collect_param_uses(p, declared, out);
            }
        }
    }
}

/// Build a class → nested directory path map from the package tree, e.g.
/// `DV_QUANTITY` → `data_types/quantity`.
pub(crate) fn class_paths(schema: &BmmSchema) -> BTreeMap<String, String> {
    fn walk(p: &crate::load::bmm::BmmPackage, prefix: &str, out: &mut BTreeMap<String, String>) {
        let seg = p.name.rsplit('.').next().unwrap_or(&p.name);
        let path = if prefix.is_empty() {
            seg.to_string()
        } else {
            format!("{prefix}/{seg}")
        };
        for c in &p.classes {
            out.insert(c.clone(), path.clone());
        }
        for sub in &p.packages {
            walk(sub, &path, out);
        }
    }
    let mut out = BTreeMap::new();
    for p in &schema.packages {
        walk(p, "", &mut out);
    }
    out
}

/// The attribute named by a `x /= Void implies not x.is_empty` assertion,
/// accepting `void`, `.empty` and a leading conjunct
/// (`PARTY.Relationships_validity`).
pub(crate) fn nonempty_list_attribute(expression: &str) -> Option<String> {
    let normalized = expression.split_whitespace().collect::<Vec<_>>().join(" ");
    let (lhs, rhs) = normalized.split_once(" implies ")?;
    let attribute = lhs
        .strip_suffix(" /= Void")
        .or_else(|| lhs.strip_suffix(" /= void"))?;
    if attribute.is_empty()
        || !attribute
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '_')
    {
        return None;
    }
    let is_empty = format!("not {attribute}.is_empty");
    let empty = format!("not {attribute}.empty");
    let matches_predicate = rhs == is_empty
        || rhs == empty
        || rhs.starts_with(&format!("({is_empty} and "))
        || rhs.starts_with(&format!("({empty} and "));
    if !matches_predicate {
        return None;
    }
    Some(attribute.to_owned())
}

/// The `(declaring class, attribute)` pairs of every optional container
/// under a present-implies-non-empty invariant.
pub(crate) fn nonempty_optional_lists_cached(model: &Model) -> &BTreeSet<(String, String)> {
    model
        .nonempty_memo
        .get_or_init(|| nonempty_optional_lists(model))
}

fn nonempty_optional_lists(model: &Model) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for (class_name, class) in &model.classes {
        for expression in class.invariants.values() {
            let Some(attribute) = nonempty_list_attribute(expression) else {
                continue;
            };
            let Some(p) = class.properties.iter().find(|p| p.name == attribute) else {
                continue;
            };
            if !p.is_mandatory && matches!(p.kind, BmmPropKind::Container { .. }) {
                out.insert((class_name.clone(), attribute));
            }
        }
    }
    out
}
