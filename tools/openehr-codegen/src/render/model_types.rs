// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Rust type text for a BMM type ([`Model::render_type`]) and the spec class
//! names it embeds ([`Model::referenced_specs`]).

use crate::analyze::{External, Model};
use crate::load::bmm::{BmmClass, BmmPropKind, BmmType};
use crate::plan::overrides::{back_reference, open_extension_point, primitive};
use crate::render::naming;
use std::collections::{BTreeMap, BTreeSet};

impl Model {
    /// Renders a BMM type to Rust. `local` holds the class names emitted here,
    /// `external` the dependency types, `subst` the ancestor-generic bindings; any
    /// other class becomes `serde_json::Value`.
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

    /// Renders a simple BMM type reference; a bare generic class is filled with its
    /// parameter bounds, and an unbounded parameter is threaded from scope.
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

    /// Renders a generic BMM type over its rendered arguments: containers to Rust
    /// collections by arity, a monomorphized class without its args, anything
    /// else to free-form JSON.
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

    /// The content types in scope for an unbounded generic slot: the free
    /// parameter names, then the `subst` values.
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

    /// The emittable spec class names a class's flattened fields refer to.
    pub(crate) fn referenced_specs(
        &self,
        class: &BmmClass,
        generics: &[String],
        subst: &BTreeMap<String, String>,
    ) -> BTreeSet<String> {
        let mut roots = BTreeSet::new();
        for rp in self.flattened_props(class) {
            // A back-reference field is omitted, so it needs no import.
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
