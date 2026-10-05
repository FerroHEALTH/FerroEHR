// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Rust type text for a BMM type, plus the import set that must agree with it.
//!
//! A second `impl` block on [`crate::analyze::Model`]: the analysed model
//! carries the type graph, and these methods turn a node of it into the Rust
//! type *string* an emitter writes ([`Model::render_type`]) and the spec class
//! names that string embeds by value ([`Model::referenced_specs`], the `use`
//! set). They live here because producing text is stage-4 work; the graph facts
//! they consult ([`Model::generic_param_bounds`], [`Model::effective_roots`])
//! stay in [`crate::analyze`], which ANALYZE-only code (the constructibility
//! proof) shares.

use crate::analyze::{External, Model};
use crate::load::bmm::{BmmClass, BmmPropKind, BmmType};
use crate::plan::overrides::{back_reference, open_extension_point, primitive};
use crate::render::naming;
use std::collections::{BTreeMap, BTreeSet};

impl Model {
    /// Render a BMM type to Rust. `local` is the set of spec class names emitted
    /// in the current crate; `external` maps types provided by dependency
    /// crates. `subst` binds a generic parameter name to a concrete spec type
    /// (an ancestor-generic binding the BMM drops, e.g. `Multiplicity_interval`'s
    /// `T` → `Integer`). A referenced class in neither `local` nor `external`
    /// (or a malformed container) degrades to `serde_json::Value`. Local and
    /// external class types render as the bare ident (the import machinery adds
    /// the right `use`).
    pub(crate) fn render_type(
        &self,
        t: &BmmType,
        generics: &[String],
        subst: &BTreeMap<String, String>,
        local: &BTreeSet<String>,
        external: &External,
    ) -> String {
        match t {
            BmmType::Simple(n) => self.render_simple(n, generics, subst, local, external),
            BmmType::Generic { root, params } => {
                let ps: Vec<String> = params
                    .iter()
                    .map(|p| self.render_type(p, generics, subst, local, external))
                    .collect();
                self.render_container(root, &ps, local, external)
            }
        }
    }

    /// Render a simple (non-container) BMM type reference.
    ///
    /// A bare reference to a *generic* class (the BMM omits the args, e.g.
    /// `normal_range: DV_INTERVAL`) is filled with each type parameter's bound
    /// (`DV_INTERVAL` → `DvInterval<DvOrdered>`). An *unbounded* parameter (the
    /// versioned-content `T` of the VERSION family) is threaded from the
    /// enclosing scope's type params (`generics`, then bound `subst` values),
    /// so it stays strongly typed instead of degrading to `serde_json::Value`.
    fn render_simple(
        &self,
        n: &str,
        generics: &[String],
        subst: &BTreeMap<String, String>,
        local: &BTreeSet<String>,
        external: &External,
    ) -> String {
        if let Some(concrete) = subst.get(n) {
            // A bound ancestor-generic parameter: render the concrete.
            return self.render_type(
                &BmmType::Simple(concrete.clone()),
                generics,
                subst,
                local,
                external,
            );
        }
        if let Some(p) = primitive(n) {
            return p.to_string();
        }
        if generics.iter().any(|g| g == n) {
            return n.to_owned();
        }
        if n == "Any" {
            return "serde_json::Value".to_string();
        }
        if let Some(carrier) = open_extension_point(n) {
            return carrier.to_string();
        }
        if !local.contains(n) && !external.contains(n) {
            return "serde_json::Value".to_string();
        }
        let Some(bounds) = self.generic_param_bounds(n) else {
            return naming::type_name(n);
        };
        let mut content = Self::scope_content_types(generics, subst);
        let args: Vec<String> = bounds
            .iter()
            .map(|b| match b {
                Some(bound) => self.render_type(
                    &BmmType::Simple(bound.clone()),
                    generics,
                    subst,
                    local,
                    external,
                ),
                None => content
                    .next()
                    .unwrap_or_else(|| "serde_json::Value".to_string()),
            })
            .collect();
        format!("{}<{}>", naming::type_name(n), args.join(", "))
    }

    /// Render a generic BMM type reference over its already-rendered arguments.
    ///
    /// Foundation container generics map to Rust collections, matched
    /// structurally by arity (slice patterns), never by index. A container of
    /// the wrong arity (e.g. the deeply-nested free-form `Hash` in
    /// `RESOURCE_ANNOTATIONS`) or a type neither emitted here nor by a dependency
    /// degrades to free-form JSON. A non-container respects the class's
    /// *effective* arity: a class whose only param was unused is monomorphized
    /// (emitted non-generic), so a reference must drop the explicit args
    /// (`REFERENCE_RANGE<X>` → `ReferenceRange`).
    fn render_container(
        &self,
        root: &str,
        ps: &[String],
        local: &BTreeSet<String>,
        external: &External,
    ) -> String {
        match (root, ps) {
            ("Hash", [key, value]) => {
                format!("std::collections::BTreeMap<{key}, {value}>")
            }
            ("List" | "Array", [item]) => format!("Vec<{item}>"),
            ("Set", [item]) => format!("std::collections::BTreeSet<{item}>"),
            ("Hash" | "List" | "Array" | "Set", _) => "serde_json::Value".to_string(),
            (r, _) if !local.contains(r) && !external.contains(r) => {
                "serde_json::Value".to_string()
            }
            (r, _) => match self.generic_param_bounds(r) {
                None => naming::type_name(r),
                Some(_) => format!("{}<{}>", naming::type_name(r), ps.join(", ")),
            },
        }
    }

    /// The concrete content types available in the current emission scope, in
    /// priority order, for threading into an *unbounded* generic slot: the free
    /// generic-parameter names first (a generic class threads its own `T`), then
    /// the rendered values of any ancestor-generic `subst` bindings (a
    /// monomorphized class threads its bound content type).
    fn scope_content_types(
        generics: &[String],
        subst: &BTreeMap<String, String>,
    ) -> std::vec::IntoIter<String> {
        let mut v: Vec<String> = generics.to_vec();
        for val in subst.values() {
            v.push(primitive(val).map_or_else(|| naming::type_name(val), str::to_string));
        }
        v.into_iter()
    }

    /// The emittable spec class names a class refers to in its (flattened)
    /// fields — for computing precise `use` imports. Excludes primitives,
    /// generic params, mapped/skip types, and `Any`.
    pub(crate) fn referenced_specs(
        &self,
        class: &BmmClass,
        generics: &[String],
        subst: &BTreeMap<String, String>,
    ) -> BTreeSet<String> {
        let mut roots = BTreeSet::new();
        for rp in self.flattened_props(class) {
            // A back-reference field is omitted from the emitted struct
            // (see `back_reference` / `render_struct_def`), so it contributes no
            // import — including its target here would emit an unused `use`.
            if back_reference(&rp.owner, &rp.prop.name).is_some() {
                continue;
            }
            match &rp.prop.kind {
                BmmPropKind::Single(t) => self.effective_roots(t, &mut roots),
                BmmPropKind::Container { item, .. } => self.effective_roots(item, &mut roots),
            }
        }
        roots
            .into_iter()
            // Substitute a bound generic parameter (`T` → `COMPOSITION`) so the
            // concrete content type is imported, not the parameter name.
            .map(|n| subst.get(&n).cloned().unwrap_or(n))
            .filter(|n| !Self::is_mapped(n) && n != "Any" && !generics.iter().any(|g| g == n))
            .collect()
    }
}
