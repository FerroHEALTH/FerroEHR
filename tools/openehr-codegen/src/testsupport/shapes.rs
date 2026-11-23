// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The emitted shapes: the decided shape of every class, the size estimate
//! behind the by-size boxing decision, and the mandatory containers a reader
//! fills when absent.

use std::collections::{BTreeMap, BTreeSet};

use super::Error;
use crate::load::bmm::BmmPropKind;
use crate::plan::composition::compose;
use crate::plan::{Emission, decide};
use crate::render::naming;

/// The decided shape of every class of every generation of a crate, keyed
/// `"<bmm file>::<CLASS>"`.
///
/// # Errors
/// Returns an error if the crate's BMM files cannot be loaded.
pub fn plan_shapes(key: &str) -> Result<BTreeMap<String, String>, Error> {
    let c = compose(key)?;
    let mut out = BTreeMap::new();
    for g in &c.generations {
        for u in &g.units {
            let used = u.model.used_as_type();
            for (name, class) in &u.schema.classes {
                out.insert(
                    format!("{}::{name}", u.spec.file),
                    shape_name(&decide(&u.model, class, &used)).to_string(),
                );
            }
        }
    }
    Ok(out)
}

fn shape_name(e: &Emission) -> &'static str {
    match e {
        Emission::Struct => "Struct",
        Emission::Enum(_) => "Enum",
        Emission::PolyEnum(_) => "PolyEnum",
        Emission::EnumLiterals(_) => "EnumLiterals",
        Emission::Newtype(_) => "Newtype",
        Emission::Skip => "Skip",
    }
}

/// The estimated size of every type the current generation of `key` emits
/// (`plan::layout`).
///
/// # Errors
/// Returns an error if the composition's BMM files cannot be loaded.
pub fn estimated_sizes(key: &str) -> Result<BTreeMap<String, usize>, Error> {
    let c = compose(key)?;
    let g = c.current();
    let mut out = BTreeMap::new();
    for u in &g.units {
        let used = u.model.used_as_type();
        for (name, class) in &u.schema.classes {
            let e = decide(&u.model, class, &used);
            if matches!(e, Emission::Skip) {
                continue;
            }
            let ty = naming::type_name(name);
            let generics = crate::render::emit::struct_generics(&u.model, class);
            let rendered = if generics.is_empty() {
                ty.clone()
            } else {
                let args: Vec<String> = generics
                    .iter()
                    .map(|p| {
                        u.model.resolved_param_bound(name, p).map_or_else(
                            || "serde_json::Value".to_string(),
                            |b| naming::type_name(&b),
                        )
                    })
                    .collect();
                format!("{ty}<{}>", args.join(", "))
            };
            out.insert(ty, crate::plan::layout::estimated_size(&u.model, &rendered));
            if matches!(e, Emission::PolyEnum(_)) {
                let data = format!("{}Data", naming::type_name(name));
                out.insert(
                    data.clone(),
                    crate::plan::layout::estimated_size(&u.model, &data),
                );
            }
        }
    }
    Ok(out)
}

/// Every mandatory `0..*` container reachable from `roots` in the current RM
/// generation, as `CLASS.attribute`.
///
/// # Errors
/// Returns an error if the RM composition's BMM files cannot be loaded.
pub fn reachable_mandatory_containers(roots: &[&str]) -> Result<Vec<String>, Error> {
    let c = compose("rm")?;
    let g = c.current();
    let mut out = Vec::new();
    for u in &g.units {
        let model = &u.model;
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut stack: Vec<String> = roots.iter().map(|r| (*r).to_string()).collect();
        while let Some(name) = stack.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(class) = model.get(&name) else {
                continue;
            };
            stack.extend(model.enum_variants(&name));
            for f in model.json_fields(&name) {
                if matches!(f.kind, crate::plan::JsonFieldKind::Container) {
                    out.push(format!("{name}.{}", f.wire_name));
                }
            }
            for rp in model.flattened_props(class) {
                let mut roots = BTreeSet::new();
                match &rp.prop.kind {
                    BmmPropKind::Single(t) | BmmPropKind::Container { item: t, .. } => {
                        model.effective_roots(t, &mut roots);
                    }
                }
                stack.extend(roots);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}
