// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reflective RM accessor (`openehr-rm` `access/generated.rs`, #6002):
//! `RmObject`, `AsRm` and `FromRm`, and the attribute table `construct` lays
//! a `Partial` out by. A class with a validating constructor is read-only.
//! No openEHR specification governs this access: our own design.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::analyze::{External, Model};
use crate::load::bmm::BmmClass;
use crate::plan::layout::{Ty, parse};
use crate::plan::overrides::{back_reference, class_binding, type_override};
use crate::plan::{Emission, JsonFieldKind, construction, decide};
use crate::render::emit::{GenFile, field_type, struct_generics};
use crate::render::emit_walk::{WalkClass, WalkSchema, for_each_class};
use crate::render::generics::generic_header;
use crate::render::naming;

/// What a field holds.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Holds {
    Object,
    Primitive(Prim),
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Prim {
    String,
    Integer,
    Integer64,
    Real,
    Real32,
    Boolean,
    Character,
}

pub(crate) struct Field {
    pub(crate) wire: String,
    pub(crate) ident: String,
    pub(crate) holds: Holds,
    pub(crate) optional: bool,
    pub(crate) list: bool,
    pub(crate) non_empty: bool,
    pub(crate) default: Option<String>,
}

fn prim(head: &str) -> Option<Prim> {
    match head {
        "String" => Some(Prim::String),
        "i32" => Some(Prim::Integer),
        "i64" => Some(Prim::Integer64),
        "f64" => Some(Prim::Real),
        "f32" => Some(Prim::Real32),
        "bool" => Some(Prim::Boolean),
        "char" => Some(Prim::Character),
        _ => None,
    }
}

/// The heads of the rendered types that hold an RM or BASE object.
pub(crate) fn walkable(model: &Model) -> BTreeSet<String> {
    let used = model.used_as_type();
    let mut out = BTreeSet::new();
    for (name, class) in model.class_iter() {
        match decide(model, class, &used) {
            Emission::Struct | Emission::Enum(_) => {
                out.insert(naming::type_name(name));
            }
            Emission::PolyEnum(_) => {
                out.insert(naming::type_name(name));
                out.insert(format!("{}Data", naming::type_name(name)));
            }
            _ => {}
        }
    }
    out
}

pub(crate) fn fields(
    model: &Model,
    class: &BmmClass,
    generics: &[String],
    walk: &BTreeSet<String>,
    local: &BTreeSet<String>,
    external: &External,
) -> Vec<Field> {
    let subst: BTreeMap<String, String> = class_binding(&class.name);
    let json = model.json_fields(&class.name);
    let props: Vec<_> = model
        .flattened_props(class)
        .into_iter()
        .filter(|rp| back_reference(&rp.owner, &rp.prop.name).is_none())
        .collect();
    props
        .iter()
        .zip(json)
        .map(|(rp, jf)| {
            let p = rp.prop;
            let rendered = field_type(model, class, p, generics, &subst, local, external);
            let parsed = parse(&rendered);
            let (mut head, optional, list, _boxed) = parsed
                .as_ref()
                .map_or((String::new(), false, false, false), strip);
            if type_override(&class.name, &p.name).is_some() && prim(&head).is_none() {
                head.clear();
            }
            let holds = if let Some(pr) = prim(&head) {
                if list {
                    Holds::Other
                } else {
                    Holds::Primitive(pr)
                }
            } else if (walk.contains(&head) || generics.contains(&head))
                && parsed
                    .as_ref()
                    .is_some_and(|t| args_hold_objects(t, walk, generics))
            {
                Holds::Object
            } else {
                Holds::Other
            };
            Field {
                wire: jf.wire_name,
                ident: jf.rust_name,
                holds,
                optional,
                list,
                non_empty: matches!(jf.kind, JsonFieldKind::NonEmptyContainer)
                    || (optional && rendered.contains("NonEmptyVec")),
                default: jf.default.filter(|d| !d.starts_with('@')),
            }
        })
        .collect()
}

/// `(innermost head, optional, list, boxed)` of a rendered field type.
fn strip(ty: &Ty) -> (String, bool, bool, bool) {
    let mut t = ty;
    let (mut optional, mut list, mut boxed) = (false, false, false);
    if t.head == "Option"
        && let Some(a) = t.args.first()
    {
        optional = true;
        t = a;
    }
    if t.head == "Box"
        && let Some(a) = t.args.first()
    {
        boxed = true;
        t = a;
    }
    if (t.head == "Vec" || t.head == "NonEmptyVec")
        && let Some(a) = t.args.first()
    {
        list = true;
        t = a;
        if t.head == "Box"
            && let Some(a) = t.args.first()
        {
            t = a;
        }
    }
    (t.head.clone(), optional, list, boxed)
}

/// The generic parameters the enum `name` uses and the idents of its
/// `variants`, with `own` (the enum's own data struct) last when it has one.
pub(crate) fn enum_parts(
    model: &Model,
    name: &str,
    variants: &[String],
    own: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let generics = model
        .used_generic_params(name)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let mut idents: Vec<String> = variants.iter().map(|v| naming::type_name(v)).collect();
    idents.extend(own.map(str::to_owned));
    (generics, idents)
}

/// Emit the accessor file for the current RM generation.
#[must_use]
pub(crate) fn emit_files(model: &Model, schemas: &[WalkSchema<'_>]) -> Vec<GenFile> {
    let walk = walkable(model);
    let mut impls = String::new();
    let mut table = String::new();
    for_each_class(model, schemas, |c| {
        let WalkClass {
            name,
            class,
            path,
            rust,
            emission,
            local,
            external,
        } = c;
        match emission {
            Emission::Struct => {
                let g = struct_generics(model, class);
                let f = fields(model, class, &g, &walk, local, external);
                emit_struct(&mut impls, &mut table, class, &path, &rust, &g, &f);
            }
            Emission::PolyEnum(variants) => {
                let g = struct_generics(model, class);
                let f = fields(model, class, &g, &walk, local, external);
                emit_struct(
                    &mut impls,
                    &mut table,
                    class,
                    &path,
                    &format!("{rust}Data"),
                    &g,
                    &f,
                );
                let (eg, idents) = enum_parts(model, name, &variants, Some(&rust));
                let arms = model.xsi_dispatch(name, &variants);
                emit_enum(&mut impls, &path, &rust, &eg, &idents, &arms);
            }
            Emission::Enum(variants) => {
                let (eg, idents) = enum_parts(model, name, &variants, None);
                let arms = model.xsi_dispatch(name, &variants);
                emit_enum(&mut impls, &path, &rust, &eg, &idents, &arms);
            }
            _ => {}
        }
    });
    let mut body = String::from(
        "// @generated by openehr-codegen (emit-validate) — DO NOT EDIT.\n\
         //! The generated half of [`super`]: the reflective view of every RM and BASE\n\
         //! class of the release model, and the attribute layout of a [`Partial`].\n\n\
         #![allow(\n    \
         clippy::too_many_lines,\n    \
         clippy::match_same_arms,\n    \
         clippy::match_single_binding,\n    \
         clippy::single_match_else,\n    \
         clippy::needless_return,\n    \
         clippy::vec_init_then_push,\n    \
         clippy::match_like_matches_macro,\n    \
         clippy::semicolon_if_nothing_returned,\n    \
         clippy::explicit_iter_loop,\n    \
         clippy::default_trait_access,\n    \
         unused_variables,\n    \
         unused_mut,\n    \
         unreachable_code,\n    \
         reason = \"one impl per class of the RM and BASE models; a generated accessor\"\n)]\n\n\
         use std::any::Any;\n\n\
         use super::{\n    AccessError, AsRm, Children, ChildrenMut, FromRm, Items, ItemsMut, Partial, RmObject,\n    RmRef, RmValue, Site,\n};\n\n\
         /// The class name and the attribute layout of a concrete class a\n\
         /// [`Partial`] can be built for.\n\
         pub(super) fn attributes(rm_type: &str) -> Option<(&'static str, &'static [&'static str])> {\n\
         match rm_type {\n",
    );
    body.push_str(&table);
    body.push_str("_ => None,\n}\n}\n\n");
    body.push_str(&impls);
    vec![GenFile {
        path: "access/generated.rs".to_string(),
        body,
    }]
}

fn emit_enum(
    b: &mut String,
    path: &str,
    rust: &str,
    generics: &[String],
    idents: &[String],
    arms: &[(String, String)],
) {
    let (hdr, args) = generic_header(
        generics,
        "AsRm + FromRm + Clone + std::fmt::Debug + Send + Sync + 'static",
    );
    let _ = write!(
        b,
        "impl{hdr} AsRm for {path}::{rust}{args} {{\n\
         fn as_rm(&self) -> &dyn RmObject {{\nmatch self {{\n"
    );
    for v in idents {
        let _ = writeln!(b, "Self::{v}(x) => x.as_rm(),");
    }
    b.push_str("}\n}\nfn as_rm_mut(&mut self) -> &mut dyn RmObject {\nmatch self {\n");
    for v in idents {
        let _ = writeln!(b, "Self::{v}(x) => x.as_rm_mut(),");
    }
    let _ = write!(
        b,
        "}}\n}}\n}}\n\n\
         impl{hdr} FromRm for {path}::{rust}{args} {{\n\
         fn from_rm(object: Box<dyn RmObject>, at: Site) -> Result<Self, AccessError> {{\n\
         match object.rm_type() {{\n"
    );
    for (target, ident) in arms {
        let _ = writeln!(
            b,
            "{target:?} => Ok(Self::{ident}(FromRm::from_rm(object, at)?)),"
        );
    }
    b.push_str("other => Err(at.refuse(other)),\n}\n}\n}\n\n");
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm family per RmObject method, each over the same field view"
)]
fn emit_struct(
    b: &mut String,
    table: &mut String,
    class: &BmmClass,
    path: &str,
    rust: &str,
    generics: &[String],
    fields: &[Field],
) {
    let spec = &class.name;
    let (hdr, args) = generic_header(
        generics,
        "AsRm + FromRm + Clone + std::fmt::Debug + Send + Sync + 'static",
    );
    let validated = construction::is_validated(spec);
    // A validated class keeps its fields `pub(crate)` in its own crate, so it
    // reads through its generated accessors and is never built by name.
    let read = |f: &Field| -> String {
        if validated {
            format!("self.{}()", f.ident)
        } else {
            format!("self.{}", f.ident)
        }
    };
    let names: Vec<String> = fields.iter().map(|f| format!("{:?}", f.wire)).collect();
    let names = names.join(", ");
    if !class.is_abstract && !validated {
        let _ = writeln!(table, "{spec:?} => Some(({spec:?}, &[{names}])),");
    }

    let mut is_set = String::new();
    let mut attribute = String::new();
    let mut object_mut = String::new();
    let mut set = String::new();
    let mut push = String::new();
    let mut items = String::new();
    let mut items_mut = String::new();
    let mut children = String::new();
    let mut children_mut = String::new();
    let mut build = String::new();
    for (i, f) in fields.iter().enumerate() {
        let w = &f.wire;
        let x = read(f);
        // is_set
        let set_expr = match (f.list, f.optional) {
            (false, false) => "true".to_string(),
            (false, true) => format!("{x}.is_some()"),
            (true, false) if f.non_empty => "true".to_string(),
            (true, false) => format!("!{x}.is_empty()"),
            (true, true) => format!("{x}.as_ref().is_some_and(|l| !l.is_empty())"),
        };
        let _ = writeln!(is_set, "{w:?} => {set_expr},");
        // attribute
        if !f.list {
            // `v` is a reference to the field value.
            let one = |v: &str| -> String {
                match f.holds {
                    Holds::Object => format!("RmRef::Object({v}.as_rm())"),
                    Holds::Primitive(Prim::String) => format!("RmRef::String({v})"),
                    Holds::Primitive(Prim::Integer) => format!("RmRef::Integer(i64::from(*{v}))"),
                    Holds::Primitive(Prim::Integer64) => format!("RmRef::Integer(*{v})"),
                    Holds::Primitive(Prim::Real) => format!("RmRef::Real(*{v})"),
                    Holds::Primitive(Prim::Real32) => format!("RmRef::Real(f64::from(*{v}))"),
                    Holds::Primitive(Prim::Boolean) => format!("RmRef::Boolean(*{v})"),
                    Holds::Primitive(Prim::Character) => format!("RmRef::Character(*{v})"),
                    Holds::Other => String::new(),
                }
            };
            if f.holds != Holds::Other {
                let expr = if validated {
                    // Accessors give `&str`/`&T`, and `Option<&str>`/`Option<&T>`.
                    match (f.optional, f.holds) {
                        (false, Holds::Primitive(Prim::String)) => {
                            format!("Some(RmRef::String({x}))")
                        }
                        (true, Holds::Primitive(Prim::String)) => format!("{x}.map(RmRef::String)"),
                        (false, _) => format!("Some({})", one(&x)),
                        (true, _) => format!("{x}.map(|v| {})", one("v")),
                    }
                } else if f.optional {
                    format!("{x}.as_ref().map(|v| {})", one("v"))
                } else {
                    let v = match f.holds {
                        Holds::Object => format!("RmRef::Object({x}.as_rm())"),
                        Holds::Primitive(Prim::String) => format!("RmRef::String(&{x})"),
                        Holds::Primitive(Prim::Integer) => {
                            format!("RmRef::Integer(i64::from({x}))")
                        }
                        Holds::Primitive(Prim::Integer64) => format!("RmRef::Integer({x})"),
                        Holds::Primitive(Prim::Real) => format!("RmRef::Real({x})"),
                        Holds::Primitive(Prim::Real32) => format!("RmRef::Real(f64::from({x}))"),
                        Holds::Primitive(Prim::Boolean) => format!("RmRef::Boolean({x})"),
                        Holds::Primitive(Prim::Character) => format!("RmRef::Character({x})"),
                        Holds::Other => String::new(),
                    };
                    format!("Some({v})")
                };
                let _ = writeln!(attribute, "{w:?} => {expr},");
            }
        }
        if f.holds == Holds::Object && !validated {
            if f.list {
                let (it, itm) = if f.optional {
                    (
                        format!("{x}.iter().flatten()"),
                        format!("{x}.iter_mut().flatten()"),
                    )
                } else {
                    (format!("{x}.iter()"), format!("{x}.iter_mut()"))
                };
                let _ = writeln!(items, "{w:?} => Some(Box::new({it}.map(AsRm::as_rm))),");
                let _ = writeln!(
                    items_mut,
                    "{w:?} => Some(Box::new({itm}.map(AsRm::as_rm_mut))),"
                );
                let _ = writeln!(
                    children,
                    "for (i, v) in {it}.enumerate() {{ out.push(({w:?}, Some(i), v.as_rm())); }}"
                );
                let _ = writeln!(
                    children_mut,
                    "for v in {itm} {{ out.push(({w:?}, v.as_rm_mut())); }}"
                );
                let target = if f.optional {
                    if f.non_empty {
                        format!(
                            "match &mut {x} {{ Some(l) => l.push(member), None => {x} = Some(openehr_base::containers::NonEmptyVec::of(member)), }}"
                        )
                    } else {
                        format!("{x}.get_or_insert_with(Vec::new).push(member);")
                    }
                } else {
                    format!("{x}.push(member);")
                };
                let _ = writeln!(
                    push,
                    "{w:?} => {{ let member = FromRm::from_rm(value, super::site({spec:?}, {w:?}))?; {target} Ok(()) }}"
                );
            } else {
                if f.optional {
                    let _ = writeln!(object_mut, "{w:?} => {x}.as_mut().map(AsRm::as_rm_mut),");
                    let _ = writeln!(
                        children,
                        "if let Some(v) = &{x} {{ out.push(({w:?}, None, v.as_rm())); }}"
                    );
                    let _ = writeln!(
                        children_mut,
                        "if let Some(v) = &mut {x} {{ out.push(({w:?}, v.as_rm_mut())); }}"
                    );
                } else {
                    let _ = writeln!(object_mut, "{w:?} => Some({x}.as_rm_mut()),");
                    let _ = writeln!(children, "out.push(({w:?}, None, {x}.as_rm()));");
                    let _ = writeln!(children_mut, "out.push(({w:?}, {x}.as_rm_mut()));");
                }
                let wrap = if f.optional { "Some(v)" } else { "v" };
                let _ = writeln!(
                    set,
                    "{w:?} => match value {{ RmValue::Object(o) => {{ let v = FromRm::from_rm(o, super::site({spec:?}, {w:?}))?; {x} = {wrap}; Ok(()) }} other => Err(super::site({spec:?}, {w:?}).refuse(&other.kind())) }},"
                );
            }
        } else if f.holds == Holds::Object && validated && !f.list {
            if f.optional {
                let _ = writeln!(
                    children,
                    "if let Some(v) = {x} {{ out.push(({w:?}, None, v.as_rm())); }}"
                );
            } else {
                let _ = writeln!(children, "out.push(({w:?}, None, {x}.as_rm()));");
            }
        }
        if let Holds::Primitive(_) = f.holds
            && !validated
            && !f.list
        {
            let wrap = if f.optional { "Some(v)" } else { "v" };
            let _ = writeln!(
                set,
                "{w:?} => {{ let v = super::FromPrimitive::from_value(value, super::site({spec:?}, {w:?}))?; {x} = {wrap}; Ok(()) }}"
            );
        }
        if f.list && f.holds != Holds::Object && !validated {
            let _ = writeln!(
                push,
                "{w:?} => Err(AccessError::Unsupported {{ rm_type: {spec:?}, attribute: {w:?} }}),"
            );
        }
        if !f.list && f.holds == Holds::Other && !validated {
            let _ = writeln!(
                set,
                "{w:?} => Err(AccessError::Unsupported {{ rm_type: {spec:?}, attribute: {w:?} }}),"
            );
        }
        if f.list && !validated {
            let _ = writeln!(
                set,
                "{w:?} => Err(AccessError::NotSingle {{ rm_type: {spec:?}, attribute: attr.to_owned() }}),"
            );
        }
        if !f.list && !validated {
            let _ = writeln!(
                push,
                "{w:?} => Err(AccessError::NotMultiple {{ rm_type: {spec:?}, attribute: attr.to_owned() }}),"
            );
        }
        // finish from a Partial
        let ident = &f.ident;
        let value = match (f.holds, f.list, f.optional) {
            (Holds::Object, false, false) => format!("p.object({i}, {w:?})?"),
            (Holds::Object, false, true) => format!("p.opt_object({i}, {w:?})?"),
            (Holds::Object, true, false) if f.non_empty => format!("p.non_empty({i}, {w:?})?"),
            (Holds::Object, true, false) => format!("p.vec({i}, {w:?})?"),
            (Holds::Object, true, true) if f.non_empty => format!("p.opt_non_empty({i}, {w:?})?"),
            (Holds::Object, true, true) => format!("p.objects({i}, {w:?})?"),
            (Holds::Primitive(_), false, false) => match &f.default {
                Some(d) => format!("p.opt_primitive({i}, {w:?})?.unwrap_or({d})"),
                None => format!("p.primitive({i}, {w:?})?"),
            },
            (Holds::Primitive(_), false, true) => format!("p.opt_primitive({i}, {w:?})?"),
            (_, _, true) => "None".to_string(),
            (_, true, false) if f.non_empty => {
                format!(
                    "return Err(AccessError::Unsupported {{ rm_type: {spec:?}, attribute: {w:?} }})"
                )
            }
            (_, _, false) => "Default::default()".to_string(),
        };
        let _ = writeln!(build, "{ident}: {value},");
    }
    let (set_body, push_body, object_mut_body, items_body, items_mut_body, children_mut_body) =
        if validated {
            (
                format!(
                    "drop(value);\nErr(if [{decl}].contains(&attr) {{ AccessError::NotBuildable({spec:?}) }} else {{ super::undeclared({spec:?}, attr) }})",
                    decl = declared_list(fields)
                ),
                format!(
                    "drop(value);\nErr(if [{decl}].contains(&attr) {{ AccessError::NotBuildable({spec:?}) }} else {{ super::undeclared({spec:?}, attr) }})",
                    decl = declared_list(fields)
                ),
                "let _ = attr; None".to_string(),
                "let _ = attr; None".to_string(),
                "let _ = attr; None".to_string(),
                "Box::new(std::iter::empty())".to_string(),
            )
        } else {
            (
                format!("match attr {{\n{set}_ => Err(super::undeclared({spec:?}, attr)),\n}}"),
                format!("match attr {{\n{push}_ => Err(super::undeclared({spec:?}, attr)),\n}}"),
                format!("match attr {{\n{object_mut}_ => None,\n}}"),
                format!("match attr {{\n{items}_ => None,\n}}"),
                format!("match attr {{\n{items_mut}_ => None,\n}}"),
                format!(
                    "let mut out: Vec<(&'static str, &mut dyn RmObject)> = Vec::new();\n{children_mut}Box::new(out.into_iter())"
                ),
            )
        };
    let mut presence = String::new();
    for f in fields {
        let local = format!("__present_{}", f.wire);
        if fields
            .iter()
            .any(|g| g.default.as_deref().is_some_and(|d| d.contains(&local)))
        {
            let _ = writeln!(
                presence,
                "let {local} = RmObject::is_set(&p, {:?});",
                f.wire
            );
        }
    }
    let partial = if validated {
        format!("Err(AccessError::NotBuildable({spec:?}))")
    } else {
        format!(
            "match object.downcast::<Partial>() {{\nOk(p) if p.is_of({spec:?}) => {{\nlet mut p = *p;\n{presence}Ok(Self {{\n{build}}})\n}}\n_ => Err(at.refuse(given)),\n}}"
        )
    };
    let _ = write!(
        b,
        "impl{hdr} RmObject for {path}::{rust}{args} {{\n\
         fn rm_type(&self) -> &'static str {{ {spec:?} }}\n\
         fn attributes(&self) -> &'static [&'static str] {{ &[{names}] }}\n\
         fn clone_box(&self) -> Box<dyn RmObject> {{ Box::new(self.clone()) }}\n\
         fn as_any(&self) -> &dyn Any {{ self }}\n\
         fn as_any_mut(&mut self) -> &mut dyn Any {{ self }}\n\
         fn into_any(self: Box<Self>) -> Box<dyn Any> {{ self }}\n\
         fn is_set(&self, attr: &str) -> bool {{\nmatch attr {{\n{is_set}_ => false,\n}}\n}}\n\
         fn attribute(&self, attr: &str) -> Option<RmRef<'_>> {{\nmatch attr {{\n{attribute}_ => None,\n}}\n}}\n\
         fn object_mut(&mut self, attr: &str) -> Option<&mut dyn RmObject> {{\n{object_mut_body}\n}}\n\
         fn set(&mut self, attr: &str, value: RmValue) -> Result<(), AccessError> {{\n{set_body}\n}}\n\
         fn push(&mut self, attr: &str, value: Box<dyn RmObject>) -> Result<(), AccessError> {{\n{push_body}\n}}\n\
         fn items(&self, attr: &str) -> Option<Items<'_>> {{\n{items_body}\n}}\n\
         fn items_mut(&mut self, attr: &str) -> Option<ItemsMut<'_>> {{\n{items_mut_body}\n}}\n\
         fn children(&self) -> Children<'_> {{\n\
         let mut out: Vec<(&'static str, Option<usize>, &dyn RmObject)> = Vec::new();\n{children}Box::new(out.into_iter())\n}}\n\
         fn children_mut(&mut self) -> ChildrenMut<'_> {{\n{children_mut_body}\n}}\n\
         }}\n\n\
         impl{hdr} AsRm for {path}::{rust}{args} {{\n\
         fn as_rm(&self) -> &dyn RmObject {{ self }}\n\
         fn as_rm_mut(&mut self) -> &mut dyn RmObject {{ self }}\n\
         }}\n\n\
         impl{hdr} FromRm for {path}::{rust}{args} {{\n\
         fn from_rm(object: Box<dyn RmObject>, at: Site) -> Result<Self, AccessError> {{\n\
         let given = object.rm_type();\n\
         let object = match object.into_any().downcast::<Self>() {{\nOk(s) => return Ok(*s),\nErr(o) => o,\n}};\n\
         {partial}\n\
         }}\n}}\n\n",
    );
}

fn declared_list(fields: &[Field]) -> String {
    fields
        .iter()
        .map(|f| format!("{:?}", f.wire))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether every generic argument of a type is an RM object, a wrapper of one
/// or a parameter.
fn args_hold_objects(ty: &Ty, walk: &BTreeSet<String>, generics: &[String]) -> bool {
    ty.args.iter().all(|a| {
        matches!(a.head.as_str(), "Option" | "Box" | "Vec" | "NonEmptyVec")
            && args_hold_objects(a, walk, generics)
            || (walk.contains(&a.head) || generics.contains(&a.head))
                && args_hold_objects(a, walk, generics)
    })
}
