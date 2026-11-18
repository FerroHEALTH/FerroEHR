// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The by-size boxing decision for the RM: which single-valued fields and which
//! enum variants the emitter puts behind a `Box`, so the RM types a request
//! reads and moves stay small.
//!
//! No openEHR specification governs this: our own design (#6002, #5997). A
//! by-value RM node is moved several times while it is read (the visitor's
//! field buffer, the `Option`, the parent's buffer, the `Vec` push and its
//! growth), and every level of a nested read keeps its field buffers on the
//! stack. With every subtree inline, `CONTENT_ITEM` was 5,808 bytes and a
//! typed read spent a third of its time in `memmove`.
//!
//! The decision reads an estimate of each type's size, computed here from the
//! same rendered field types the struct emitter writes, so the estimate and
//! the emitted layout share one source. The estimate follows the Rust layout
//! rules closely enough for a threshold (`String`, `Vec` and `Box` sizes, the
//! `Option` niche, an enum's tag word); `openehr-rm`'s own layout test holds
//! the real sizes of the hot types under their budget.
//!
//! The policy applies to the classes of the RM schemas only. The other
//! components keep the by-value layout: an archetype or a BMM schema is read
//! once and held, never read per request.

use std::collections::{BTreeMap, BTreeSet};

use crate::analyze::{External, Model};
use crate::load::bmm::{BmmClass, BmmPropKind};
use crate::plan::overrides::class_binding;
use crate::plan::{Emission, decide};
use crate::render::naming;

/// The largest optional field payload kept inline, in bytes. Above it the
/// field is `Option<Box<T>>`: an absent field then costs one word, and a
/// present one one allocation.
pub(crate) const OPTIONAL_INLINE_MAX: usize = 64;

/// The largest mandatory field payload kept inline, in bytes. A mandatory
/// field is always present, so boxing it always costs an allocation; the bar
/// is higher than for an optional field.
pub(crate) const MANDATORY_INLINE_MAX: usize = 256;

/// The largest enum variant payload kept inline, in bytes. Above it the
/// variant is `Variant(Box<T>)`, so a list of the enum stays a list of small
/// values.
pub(crate) const VARIANT_INLINE_MAX: usize = 256;

/// The RM schema name (`BmmSchema::schema_name`) the policy applies to.
pub(crate) const RM_SCHEMA: &str = "rm";

/// An estimated layout: size and alignment in bytes, and whether the type has
/// a niche an `Option` can use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Sz {
    pub size: usize,
    pub align: usize,
    pub niche: bool,
}

impl Sz {
    const fn word(size: usize, niche: bool) -> Self {
        Sz {
            size,
            align: 8,
            niche,
        }
    }
}

/// The memo and the resolution tables of the estimate, built once per model.
#[derive(Default)]
pub(crate) struct LayoutMemo {
    sizes: std::sync::Mutex<BTreeMap<String, Sz>>,
    in_progress: std::sync::Mutex<BTreeSet<String>>,
    tables: std::sync::OnceLock<Tables>,
}

/// The rust-ident to spec-name index and the emission scope the estimate
/// renders field types in.
struct Tables {
    /// `DvText` → (`DV_TEXT`, false); `DvTextData` → (`DV_TEXT`, true).
    by_ident: BTreeMap<String, (String, bool)>,
    /// Every class the model emits.
    local: BTreeSet<String>,
    used: BTreeSet<String>,
}

fn tables(model: &Model) -> &Tables {
    model.layout_memo().tables.get_or_init(|| {
        let used = model.used_as_type();
        let mut by_ident = BTreeMap::new();
        let mut local = BTreeSet::new();
        for (name, class) in model.class_iter() {
            let emission = decide(model, class, &used);
            if matches!(emission, Emission::Skip) {
                continue;
            }
            local.insert(name.clone());
            by_ident.insert(naming::type_name(name), (name.clone(), false));
            if matches!(emission, Emission::PolyEnum(_)) {
                by_ident.insert(
                    format!("{}Data", naming::type_name(name)),
                    (name.clone(), true),
                );
            }
        }
        Tables {
            by_ident,
            local,
            used,
        }
    })
}

/// Whether an optional or mandatory single-valued field of `class` whose
/// rendered inner type is `inner` is boxed.
pub(crate) fn boxes_field(
    model: &Model,
    class: &BmmClass,
    mandatory: bool,
    inner: &str,
    generics: &[String],
) -> bool {
    // A field typed by a bare generic parameter has the size of each
    // instantiation, which the class does not know; it stays `T`.
    if !model.in_schema(&class.name, RM_SCHEMA) || generics.iter().any(|g| g == inner) {
        return false;
    }
    let limit = if mandatory {
        MANDATORY_INLINE_MAX
    } else {
        OPTIONAL_INLINE_MAX
    };
    size_of_rendered(model, inner, &bound_bindings(model, &class.name, generics)).size > limit
}

/// Whether the variant of enum `class` whose rendered payload is `payload` is
/// boxed.
pub(crate) fn boxes_variant(
    model: &Model,
    class: &BmmClass,
    payload: &str,
    generics: &[String],
) -> bool {
    if !model.in_schema(&class.name, RM_SCHEMA) {
        return false;
    }
    size_of_rendered(
        model,
        payload,
        &bound_bindings(model, &class.name, generics),
    )
    .size
        > VARIANT_INLINE_MAX
}

/// The estimated size of a rendered type, for the layout report and the
/// tests.
pub(crate) fn estimated_size(model: &Model, rendered: &str) -> usize {
    size_of_rendered(model, rendered, &BTreeMap::new()).size
}

/// Each generic parameter of a class bound to its rendered bound, so a field
/// typed by the bare parameter is sized as its bound.
fn bound_bindings(model: &Model, class: &str, generics: &[String]) -> BTreeMap<String, String> {
    generics
        .iter()
        .map(|g| {
            let bound = model.resolved_param_bound(class, g).map_or_else(
                || "serde_json::Value".to_string(),
                |b| naming::type_name(&b),
            );
            (g.clone(), bound)
        })
        .collect()
}

/// A parsed rendered type: a path head and its generic arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ty {
    pub(crate) head: String,
    pub(crate) args: Vec<Ty>,
}

impl Ty {
    fn render(&self) -> String {
        if self.args.is_empty() {
            self.head.clone()
        } else {
            let args: Vec<String> = self.args.iter().map(Ty::render).collect();
            format!("{}<{}>", self.head, args.join(", "))
        }
    }
}

pub(crate) fn parse(s: &str) -> Option<Ty> {
    let mut chars = s.chars().filter(|c| !c.is_whitespace()).peekable();
    let ty = parse_ty(&mut chars)?;
    chars.peek().is_none().then_some(ty)
}

fn parse_ty(chars: &mut std::iter::Peekable<impl Iterator<Item = char>>) -> Option<Ty> {
    let mut head = String::new();
    while let Some(&c) = chars.peek() {
        if c.is_alphanumeric() || c == '_' || c == ':' {
            head.push(c);
            chars.next();
        } else {
            break;
        }
    }
    if head.is_empty() {
        return None;
    }
    let mut args = Vec::new();
    if chars.peek() == Some(&'<') {
        chars.next();
        loop {
            args.push(parse_ty(chars)?);
            match chars.next()? {
                ',' => {}
                '>' => break,
                _ => return None,
            }
        }
    }
    // The last path segment names the type.
    let head = head.rsplit("::").next().unwrap_or(&head).to_string();
    Some(Ty { head, args })
}

fn substitute(ty: &Ty, bindings: &BTreeMap<String, String>) -> Ty {
    if ty.args.is_empty()
        && let Some(bound) = bindings.get(&ty.head)
        && let Some(parsed) = parse(bound)
    {
        return parsed;
    }
    Ty {
        head: ty.head.clone(),
        args: ty.args.iter().map(|a| substitute(a, bindings)).collect(),
    }
}

fn size_of_rendered(model: &Model, rendered: &str, bindings: &BTreeMap<String, String>) -> Sz {
    match parse(rendered) {
        Some(ty) => size_of_ty(model, &substitute(&ty, bindings)),
        None => Sz::word(24, true),
    }
}

fn round_up(n: usize, align: usize) -> usize {
    n.div_ceil(align) * align
}

fn size_of_ty(model: &Model, ty: &Ty) -> Sz {
    match (ty.head.as_str(), ty.args.as_slice()) {
        ("Option", [inner]) => {
            let i = size_of_ty(model, inner);
            if i.niche {
                Sz { niche: false, ..i }
            } else {
                Sz {
                    size: round_up(i.size, i.align) + i.align,
                    align: i.align,
                    niche: false,
                }
            }
        }
        ("Box", _) => Sz::word(8, true),
        ("Vec" | "String" | "NonEmptyVec" | "BTreeMap" | "BTreeSet", _) => Sz::word(24, true),
        ("IndexMap", _) => Sz::word(72, true),
        ("Value", _) => Sz::word(80, true),
        ("bool", _) => Sz {
            size: 1,
            align: 1,
            niche: true,
        },
        ("u8" | "i8", _) => Sz {
            size: 1,
            align: 1,
            niche: false,
        },
        ("char", _) => Sz {
            size: 4,
            align: 4,
            niche: true,
        },
        ("i32" | "u32" | "f32", _) => Sz {
            size: 4,
            align: 4,
            niche: false,
        },
        ("i64" | "u64" | "f64" | "usize" | "isize", _) => Sz::word(8, false),
        _ => size_of_class(model, ty),
    }
}

fn size_of_class(model: &Model, ty: &Ty) -> Sz {
    let key = ty.render();
    let memo = model.layout_memo();
    if let Some(sz) = lock(&memo.sizes).get(&key) {
        return *sz;
    }
    let t = tables(model);
    let Some((spec, data)) = t.by_ident.get(&ty.head).cloned() else {
        return Sz::word(24, true);
    };
    if !lock(&memo.in_progress).insert(key.clone()) {
        // A cycle the emitter breaks with a `Box` reaches back here.
        return Sz::word(8, true);
    }
    let sz = compute_class(model, t, &spec, data, &ty.args);
    lock(&memo.in_progress).remove(&key);
    lock(&memo.sizes).insert(key, sz);
    sz
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn compute_class(model: &Model, t: &Tables, spec: &str, data: bool, args: &[Ty]) -> Sz {
    let Some(class) = model.get(spec) else {
        return Sz::word(24, true);
    };
    let emission = decide(model, class, &t.used);
    let poly = matches!(emission, Emission::PolyEnum(_));
    match emission {
        Emission::PolyEnum(_) if data => struct_size(model, t, class, args),
        Emission::Struct => struct_size(model, t, class, args),
        Emission::Enum(variants) | Emission::PolyEnum(variants) => {
            let generics: Vec<String> = model
                .used_generic_params(&class.name)
                .into_iter()
                .map(|(n, _)| n)
                .collect();
            let bindings: BTreeMap<String, String> = generics
                .iter()
                .cloned()
                .zip(args.iter().map(Ty::render))
                .collect();
            let mut payloads: Vec<Sz> = variants
                .iter()
                .map(|v| {
                    let (_, payload, _) = crate::render::emit::enum_variant_payload(
                        model,
                        class,
                        v,
                        &generics,
                        &t.local,
                        &External::default(),
                    );
                    size_of_rendered(model, &payload, &bindings)
                })
                .collect();
            if poly {
                payloads.push(struct_size(model, t, class, args));
            }
            let max = payloads.iter().map(|p| p.size).max().unwrap_or(0);
            Sz::word(round_up(max, 8) + 8, true)
        }
        Emission::EnumLiterals(e) => {
            if e.underlying_type == "INTEGER" {
                Sz::word(8, true)
            } else {
                Sz::word(24, true)
            }
        }
        Emission::Newtype(prim) => size_of_rendered(model, prim, &BTreeMap::new()),
        Emission::Skip => Sz::word(24, true),
    }
}

fn struct_size(model: &Model, t: &Tables, class: &BmmClass, args: &[Ty]) -> Sz {
    let generics = crate::render::emit::struct_generics(model, class);
    let subst = class_binding(&class.name);
    let bindings: BTreeMap<String, String> = generics
        .iter()
        .cloned()
        .zip(args.iter().map(Ty::render))
        .collect();
    let external = External::default();
    let mut size = 0;
    let mut align = 1;
    let mut niche = false;
    for rp in model.flattened_props(class) {
        let p = rp.prop;
        if crate::plan::overrides::back_reference(&rp.owner, &p.name).is_some() {
            continue;
        }
        let rendered = match &p.kind {
            BmmPropKind::Single(_) | BmmPropKind::Container { .. } => {
                crate::render::emit::field_type(
                    model, class, p, &generics, &subst, &t.local, &external,
                )
            }
        };
        let f = size_of_rendered(model, &rendered, &bindings);
        size = round_up(size, f.align) + f.size;
        align = align.max(f.align);
        niche |= f.niche;
    }
    Sz {
        size: round_up(size.max(1), align),
        align,
        niche,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_paths_and_generics() {
        let t = parse("Option<openehr_base::containers::NonEmptyVec<Link>>").expect("parses");
        assert_eq!(t.head, "Option");
        assert_eq!(t.args[0].head, "NonEmptyVec");
        assert_eq!(t.args[0].args[0].head, "Link");
        assert_eq!(t.render(), "Option<NonEmptyVec<Link>>");
    }

    #[test]
    fn substitutes_a_bare_parameter() {
        let mut b = BTreeMap::new();
        b.insert("T".to_string(), "ItemStructure".to_string());
        let t = substitute(&parse("Option<T>").expect("parses"), &b);
        assert_eq!(t.render(), "Option<ItemStructure>");
    }

    #[test]
    fn option_takes_a_tag_only_without_a_niche() {
        let m = Model::merged(&[]);
        assert_eq!(estimated_size(&m, "Option<String>"), 24);
        assert_eq!(estimated_size(&m, "Option<f64>"), 16);
        assert_eq!(estimated_size(&m, "Option<i32>"), 8);
        assert_eq!(estimated_size(&m, "Option<Box<X>>"), 8);
    }
}
