// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! REST emitter: the vendored `-codegen` OAS → the Rust contract for one API
//! group, into `openehr-its/src/rest/generated/`.
//!
//! Per group it emits the transport DTOs, the param and headers structs, the
//! route table, the `server` half (an `#[async_trait]` trait and an axum
//! `router`) and the `client` half, both from one OAS read. RM payload schemas
//! resolve to the generated `openehr_rm`/`openehr_base` types.

#![expect(
    clippy::disallowed_types,
    reason = "dev tooling over JSON artifacts (vendored BMM/OAS bundles, emitter reports) — not the \
              application (#1694)"
)]
use crate::load::oas::{Oas, Operation, Param, Response};
use crate::plan::overrides::{
    oas_monomorphization, rest_default_member, rest_docs_text_header, rest_docs_text_headers,
};
use crate::render::emit_rest_headers;
use crate::render::naming;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// Map non-`[A-Za-z0-9_]` chars to `_` (a valid-ident base).
fn clean(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The `snake_case`, keyword-escaped Rust field ident for an OAS property or
/// param name (`minOp` → `min_op`, `Content-Type` → `content_type`); a leading
/// `_` is kept.
pub(super) fn field_id(raw: &str) -> String {
    let leading_us = raw.starts_with('_');
    let mut out = String::new();
    let mut prev_alnum_lower = false;
    for c in raw.chars() {
        if c.is_ascii_uppercase() {
            if prev_alnum_lower {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            prev_alnum_lower = false;
        } else if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_alnum_lower = true;
        } else {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            prev_alnum_lower = false;
        }
    }
    let mut s = out.trim_matches('_').to_string();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    if leading_us {
        s.insert(0, '_');
    }
    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    naming::field_ident(&s)
}

/// A Rust type ident for an OAS operation name (`operationId`s are snake, so
/// `type_name` gives `PascalCase`). Non-ident chars (`adl1.4`) map to `_` first.
fn type_id(raw: &str) -> String {
    naming::type_name(&clean(raw))
}

/// The Rust type name for an OAS DTO schema key; an all-uppercase acronym is
/// title-cased (`AQL` → `Aql`).
fn dto_type(raw: &str) -> String {
    let c = clean(raw);
    let alpha: String = c.chars().filter(char::is_ascii_alphabetic).collect();
    if !alpha.is_empty() && alpha.chars().all(|ch| ch.is_ascii_uppercase()) {
        naming::type_name(&c)
    } else {
        c
    }
}

/// The serde attribute every `Option` field of a generated REST DTO or param
/// struct carries.
///
/// No vendored component schema sets `nullable`, so an absent optional
/// property is omitted on the wire, never `null`
/// (<https://spec.openapis.org/oas/v3.0.3#schema-object>).
const SKIP_NONE_ATTR: &str = "    #[serde(skip_serializing_if = \"Option::is_none\")]\n";

/// The generated field name for an OAS `additionalProperties` extension map.
const ADDITIONAL_PROPERTIES_FIELD: &str = "additional_properties";

/// Emits the flattened extension map for a DTO whose OAS schema leaves the
/// object open, or nothing when it declares `additionalProperties: false`.
///
/// An absent keyword is an open object: "`additionalProperties` defaults to
/// `true`" (OpenAPI 3.0.3 §Schema Object).
fn emit_additional_properties(b: &mut String, name: &str, schema: &Value, ctx: &Ctx) {
    let value_ty = match schema.get("additionalProperties") {
        None | Some(Value::Bool(true)) => "serde_json::Value".to_string(),
        Some(v @ Value::Object(_)) => ctx.rust_type(v),
        // `false` closes the object, which gets `deny_unknown_fields` instead.
        Some(_) => return,
    };
    let declared = if schema.get("additionalProperties").is_some() {
        "declares as an extension point"
    } else {
        "leaves open (no `additionalProperties`, which defaults to `true`)"
    };
    let _ = write!(
        b,
        "    /// The undeclared (`additionalProperties`) members of `{name}`, which\n\
         \x20   /// its ITS-REST OAS component schema {declared}.\n\
         \x20   #[serde(flatten)]\n\
         \x20   pub {ADDITIONAL_PROPERTIES_FIELD}: std::collections::BTreeMap<String, {value_ty}>,\n"
    );
}

/// Types emitted by the RM and BASE crates — `PascalCase` Rust name → the full
/// generation-module type path an OAS `$ref` resolves to (never a prelude).
pub(crate) struct RmNames {
    pub rm: BTreeMap<String, String>,
    pub base: BTreeMap<String, String>,
}

/// Component schemas the OAS declares once per group, identical but for one
/// discriminator-typed field; the shared module emits them generic over it.
///
/// ITS-REST `schemas/demographic/UpdateVersion.yaml` is a "copy of same schema
/// from `ehr`, as we need to redefine to use only demographic types".
const GENERIC_OVER: &[(&str, &str)] = &[ENVELOPE];

/// The one [`GENERIC_OVER`] envelope and its generic field.
const ENVELOPE: (&str, &str) = ("UpdateVersion", "data");

/// Component schemas whose list field carries the [`GENERIC_OVER`] envelope,
/// and so take its type parameter.
///
/// "the CONTRIBUTION envelope itself remains canonical JSON" and only "each
/// `versions[i].data`" is FLAT or STRUCTURED (ITS-REST
/// `operations/contribution_create.yaml` §Simplified Formats).
const GENERIC_CARRIERS: &[(&str, &str)] = &[("NewContribution", "versions")];

/// How a DTO struct carries the [`GENERIC_OVER`] type parameter.
enum Generic {
    /// Not generic.
    None,
    /// The envelope itself: `field` is typed `T`.
    Param(&'static str),
    /// A carrier: `field` holds envelopes over `T`, which defaults to `default`.
    Carrier {
        /// The carrying list field.
        field: &'static str,
        /// The group's own content union, the parameter's default.
        default: String,
    },
}

/// The group's type argument for the [`GENERIC_OVER`] envelope `name`: the
/// flattened `field` ref (this group's `Versionable`).
fn generic_arg(ctx: &Ctx, name: &str, field: &str) -> String {
    ctx.oas
        .schemas()
        .into_iter()
        .find(|(n, _)| n == name)
        .and_then(|(_, schema)| {
            ctx.oas
                .resolve(schema)
                .pointer(&format!("/properties/{field}"))
                .map(|s| ctx.rust_type(s))
        })
        .unwrap_or_else(|| "serde_json::Value".to_string())
}

/// The `$ref` names a schema reaches, skipping a genericized field's subtree.
fn ref_names_of(name: &str, schema: &Value, out: &mut BTreeSet<String>) {
    let skip = GENERIC_OVER
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f);
    walk_refs(schema, skip, out);
}

/// Walks a schema value collecting every `$ref` target name, skipping
/// `skip_field` one level below `properties`.
fn walk_refs(v: &Value, skip_field: Option<&str>, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            if let Some(r) = m.get("$ref").and_then(Value::as_str)
                && let Some(n) = r.rsplit('/').next()
            {
                out.insert(n.to_string());
            }
            for (k, val) in m {
                if skip_field == Some(k.as_str()) {
                    continue;
                }
                match val {
                    Value::Object(props) if k == "properties" => {
                        walk_property_refs(props, skip_field, out);
                    }
                    _ => walk_refs(val, None, out),
                }
            }
        }
        Value::Array(a) => {
            for item in a {
                walk_refs(item, skip_field, out);
            }
        }
        _ => {}
    }
}

/// Walks a `properties` map, skipping the genericized field's subtree.
fn walk_property_refs(
    props: &serde_json::Map<String, Value>,
    skip_field: Option<&str>,
    out: &mut BTreeSet<String>,
) {
    for (pk, pv) in props {
        if skip_field != Some(pk.as_str()) {
            walk_refs(pv, None, out);
        }
    }
}

/// A canonical (key-sorted) representation for schema-identity comparison.
fn stable_repr(v: &Value) -> String {
    fn sort(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.iter()
                    .map(|(k, val)| (k.clone(), sort(val)))
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(sort).collect()),
            other => other.clone(),
        }
    }
    sort(v).to_string()
}

/// The component schemas shared byte-identically by more than one group, not
/// RM/BASE-resolved, whose `$ref` closure stays inside RM, BASE and this set.
pub(crate) fn hoist_set(bundles: &[(&str, Oas)], names: &RmNames) -> BTreeSet<String> {
    use std::collections::BTreeMap;
    let mut reprs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut refs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (_, oas) in bundles {
        for (name, schema) in oas.schemas() {
            *counts.entry(name.clone()).or_default() += 1;
            reprs
                .entry(name.clone())
                .or_default()
                .insert(stable_repr(schema));
            let mut r = BTreeSet::new();
            ref_names_of(&name, schema, &mut r);
            refs.entry(name).or_default().extend(r);
        }
    }
    let mut hoisted: BTreeSet<String> = counts
        .iter()
        .filter(|(n, c)| {
            **c > 1
                && reprs.get(*n).is_some_and(|r| r.len() == 1)
                && !names.rm.contains_key(*n)
                && !names.base.contains_key(*n)
                && oas_monomorphization(n).is_none()
        })
        .map(|(n, _)| n.clone())
        .collect();
    // Fixpoint: drop any candidate whose refs leave {RM, BASE, hoisted}.
    loop {
        let snapshot = hoisted.clone();
        hoisted.retain(|n| {
            refs.get(n).is_some_and(|rs| {
                rs.iter().all(|r| {
                    names.rm.contains_key(r)
                        || names.base.contains_key(r)
                        || oas_monomorphization(r).is_some()
                        || snapshot.contains(r)
                })
            })
        });
        if hoisted.len() == snapshot.len() {
            break;
        }
    }
    hoisted
}

/// Emits the shared `common` module: each [`hoist_set`] DTO once, the
/// [`GENERIC_OVER`] schema with its type parameter.
#[must_use]
pub(crate) fn emit_common(oas: &Oas, names: &RmNames, hoisted: &BTreeSet<String>) -> String {
    let no_unions = BTreeMap::new();
    let ctx = Ctx {
        oas,
        names,
        dtos: hoisted,
        hoisted,
        in_common: true,
        unions: &no_unions,
    };
    let mut b = String::new();
    let _ = write!(
        b,
        "// @generated by openehr-codegen (emit-rest) — DO NOT EDIT.\n\
         //! ITS-REST contract, shared component schemas: DTOs that appear in more\n\
         //! than one API group's OAS bundle with identical definitions, hoisted so\n\
         //! one Rust type serves every group (the per-group bundles duplicate\n\
         //! shared schemas verbatim).\n\n\
         #![allow(\n    \
         clippy::all,\n    \
         clippy::pedantic,\n    \
         clippy::nursery,\n    \
         dead_code,\n    \
         unused_variables,\n    \
         reason = \"mechanically generated contract text: the OAS is emitted in \
         full (every DTO, param struct and route, whether or not this workspace \
         consumes it yet), so style and dead-code lints do not apply — the \
         hand-written runtime and the implementing adapter carry the lint bar\"\n\
         )]\n\
         use serde::{{Deserialize, Serialize}};\n\n"
    );
    for (name, schema) in oas.schemas() {
        if hoisted.contains(&name) {
            emit_dto(&mut b, &name, schema, &ctx);
        }
    }
    b
}

/// Emit the generated module for one API group. `dtos` is the set of component
/// schema names this group defines that are *not* RM types (i.e. real DTOs).
#[must_use]
pub(crate) fn emit_group(
    oas: &Oas,
    group: &str,
    names: &RmNames,
    hoisted: &BTreeSet<String>,
) -> String {
    // Component schemas split into RM-resolved vs local DTOs.
    let dtos: BTreeSet<String> = oas
        .schemas()
        .iter()
        .map(|(n, _)| n.clone())
        .filter(|n| {
            !names.rm.contains_key(n)
                && !names.base.contains_key(n)
                && oas_monomorphization(n).is_none()
        })
        .collect();
    let mut ops = oas.operations();
    for op in &mut ops {
        append_docs_text_headers(op);
    }
    let no_unions = BTreeMap::new();
    let unions = response_unions(
        &ops,
        &Ctx {
            oas,
            names,
            dtos: &dtos,
            hoisted,
            in_common: false,
            unions: &no_unions,
        },
    );
    let union_names: BTreeMap<String, String> = unions
        .iter()
        .map(|u| (u.repr.clone(), u.ty_name.clone()))
        .collect();
    let ctx = Ctx {
        oas,
        names,
        dtos: &dtos,
        hoisted,
        in_common: false,
        unions: &union_names,
    };

    let mut b = String::new();
    let _ = write!(
        b,
        "// @generated by openehr-codegen (emit-rest) — DO NOT EDIT.\n\
         //! ITS-REST contract for the `{group}` API group: DTOs, per-operation\n\
         //! param structs, per-response headers structs, the `server` and\n\
         //! `client` halves, and the route table.\n\n\
         #![allow(\n    \
         clippy::all,\n    \
         clippy::pedantic,\n    \
         clippy::nursery,\n    \
         dead_code,\n    \
         unused_imports,\n    \
         unused_variables,\n    \
         reason = \"mechanically generated contract text: the OAS is emitted in \
         full (every DTO, param struct and route, whether or not this workspace \
         consumes it yet), so style and dead-code lints do not apply — the \
         hand-written runtime and the implementing adapter carry the lint bar\"\n\
         )]\n\
         use serde::{{Deserialize, Serialize}};\n\n"
    );

    // ── DTOs ──
    for (name, schema) in oas.schemas() {
        if !ctx.dtos.contains(&name) {
            continue;
        }
        if ctx.hoisted.contains(&name) {
            // Hoisted into `common`; a generic schema gets a group-local alias at the
            // group's own type argument.
            if let Some((_, field)) = GENERIC_OVER.iter().find(|(n, _)| *n == name) {
                let arg = ctx
                    .oas
                    .resolve(schema)
                    .pointer(&format!("/properties/{field}"))
                    .map_or_else(|| "serde_json::Value".to_string(), |s| ctx.rust_type(s));
                let ty = dto_type(&name);
                let _ = writeln!(
                    b,
                    "/// This group's instantiation of the shared generic `{name}` envelope\n\
                     /// (`super::common::{ty}`), bound at this group's own content union.\n\
                     pub type {ty} = super::common::{ty}<{arg}>;\n"
                );
            }
            continue;
        }
        emit_dto(&mut b, &name, schema, &ctx);
    }

    // ── response unions (inline `oneOf` response bodies) ──
    for union in &unions {
        emit_tagged_enum(
            &mut b,
            &TaggedEnum {
                label: &union.component,
                ty_name: &union.ty_name,
                summary: format!(
                    "/// The `{}` ITS-REST OAS response body: an inline `oneOf` over\n\
                     /// canonical-JSON RM classes, dispatched on each member's own `_type`.\n",
                    union.component
                ),
                variants: &union.variants,
                aliases: &[],
                base: None,
            },
        );
    }

    // ── per-operation param structs ──
    for op in &ops {
        emit_params_struct(&mut b, op, &ctx);
        emit_request_body_decoder(&mut b, op, &ctx);
    }

    // ── per-response headers structs (shared by both halves) ──
    for op in &ops {
        emit_response_headers(&mut b, op);
    }

    // ── server (the `rest-server` half) ──
    emit_server_module(&mut b, group, &ops, &ctx);

    // ── client (the `rest-client` half) ──
    emit_client_module(&mut b, group, &ops, &ctx);

    // ── route table ──
    let _ = write!(
        b,
        "/// The operations of this group as `(method, path, operation_id)`, in OAS\n\
         /// document order: `server::router` binds each to its trait method, and\n\
         /// `crate::rest::routes::lookup` matches a request path against them.\n\
         pub const ROUTES: &[(&str, &str, &str)] = &[\n"
    );
    for op in &ops {
        let _ = writeln!(
            b,
            "    (\"{}\", \"{}\", \"{}\"),",
            op.method.to_uppercase(),
            op.path,
            op.operation_id
        );
    }
    b.push_str("];\n\n");
    let _ = write!(
        b,
        "/// The declared parameters of each operation, index-aligned with [`ROUTES`]:\n\
         /// the operation's `*Params::PARAMS`, empty when it declares none.\n\
         pub const ROUTE_PARAMS: &[&[crate::rest::routes::Param]] = &[\n"
    );
    for op in &ops {
        if has_params_struct(op) {
            let _ = writeln!(b, "    {}::PARAMS,", param_struct_name(op));
        } else {
            b.push_str("    &[],\n");
        }
    }
    b.push_str(
        "];\n\n\
         const _: () = assert!(\n    \
         ROUTE_PARAMS.len() == ROUTES.len(),\n    \
         \"ROUTE_PARAMS carries one row per ROUTES entry\"\n\
         );\n\n",
    );
    let _ = write!(
        b,
        "/// The request-body media types of each operation, index-aligned with\n\
         /// [`ROUTES`]: the `requestBody.content` keys of the OAS, in document order,\n\
         /// empty when the operation takes no body.\n\
         pub const ROUTE_REQUEST_MEDIA: &[&[&str]] = &[\n"
    );
    for op in &ops {
        let media: Vec<String> = admitted_request_media(op, oas)
            .iter()
            .map(|m| format!("{m:?}"))
            .collect();
        let _ = writeln!(b, "    &[{}],", media.join(", "));
    }
    b.push_str(
        "];\n\n\
         const _: () = assert!(\n    \
         ROUTE_REQUEST_MEDIA.len() == ROUTES.len(),\n    \
         \"ROUTE_REQUEST_MEDIA carries one row per ROUTES entry\"\n\
         );\n",
    );
    b
}

struct Ctx<'a> {
    oas: &'a Oas,
    names: &'a RmNames,
    dtos: &'a BTreeSet<String>,
    /// The cross-group hoisted schema names ([`hoist_set`]).
    hoisted: &'a BTreeSet<String>,
    /// Whether this renders the shared `common` module, where hoisted names render
    /// bare, or a group module, where they render `super::common::…`.
    in_common: bool,
    /// The group's response unions ([`response_unions`]): the stable form of
    /// each typed inline `oneOf` schema → its emitted enum name.
    unions: &'a BTreeMap<String, String>,
}

impl Ctx<'_> {
    /// Maps a `$ref` schema to its Rust type.
    ///
    /// A key that differs from its class (`Clstr`, generic instantiations) is
    /// resolved through `plan::overrides::OAS_MONOMORPHIZATIONS`. RM/BASE types
    /// resolve to the typed spec structs, whose serde impls are the strict
    /// canonical-JSON reader.
    fn ref_type(&self, name: &str, schema: &Value) -> String {
        if let Some(spec) = oas_monomorphization(name) {
            return spec.to_string();
        }
        if let Some(path) = self.names.rm.get(name) {
            return path.clone();
        }
        if let Some(path) = self.names.base.get(name) {
            return path.clone();
        }
        if self.hoisted.contains(name) && !self.in_common {
            if GENERIC_OVER.iter().any(|(n, _)| *n == name) {
                return dto_type(name);
            }
            return format!("super::common::{}", dto_type(name));
        }
        if self.dtos.contains(name) {
            return dto_type(name);
        }
        self.rust_type(self.oas.resolve(schema))
    }

    /// Maps an OAS schema to a Rust type; an unknown or complex shape is
    /// `serde_json::Value`.
    fn rust_type(&self, schema: &Value) -> String {
        if let Some(name) = Oas::ref_name(schema) {
            return self.ref_type(&name, schema);
        }
        if let Some(union) = self.unions.get(&stable_repr(schema)) {
            return union.clone();
        }
        // `oneOf` a resource and `Identifier`: the full and the identifier form
        // a `Prefer` header selects, read by the runtime's `Representation`.
        if let Some([first, second]) = schema
            .get("oneOf")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
        {
            let full = match (Oas::ref_name(first), Oas::ref_name(second)) {
                (Some(_), Some(id)) if id == "Identifier" => Some(first),
                (Some(id), Some(_)) if id == "Identifier" => Some(second),
                _ => None,
            };
            if let Some(full) = full {
                return format!(
                    "crate::rest::runtime::Representation<{}>",
                    self.rust_type(full)
                );
            }
        }
        // A single-`$ref` `allOf` with no own `properties` is an alias for its
        // referent (<https://spec.openapis.org/oas/v3.0.3#composition-and-inheritance-polymorphism>).
        if schema.get("properties").is_none()
            && let Some(members) = schema.get("allOf").and_then(Value::as_array)
        {
            match members.as_slice() {
                [only] => return self.rust_type(only),
                // A multi-member `allOf` needs a merged struct; the vendored OAS has none, so
                // the payload stays untyped rather than losing members.
                _ => return "serde_json::Value".to_string(),
            }
        }
        match schema.get("type").and_then(Value::as_str) {
            Some("string") => "String".to_string(),
            Some("integer") => "i64".to_string(),
            Some("number") => "f64".to_string(),
            Some("boolean") => "bool".to_string(),
            Some("array") => {
                let item = schema.get("items").map_or_else(
                    || "serde_json::Value".to_string(),
                    |i| {
                        if i.as_object().is_some_and(serde_json::Map::is_empty) {
                            "serde_json::Value".to_string()
                        } else {
                            // The item schema keeps its `$ref` name so it emits `Vec<ResultSetColumn>`.
                            self.rust_type(i)
                        }
                    },
                );
                format!("Vec<{item}>")
            }
            Some("object") => {
                if schema.get("properties").is_some() {
                    // An inline (anonymous) object — not named; carry as JSON.
                    "serde_json::Value".to_string()
                } else {
                    // A free map (`additionalProperties`).
                    "std::collections::BTreeMap<String, serde_json::Value>".to_string()
                }
            }
            // oneOf/anyOf and untyped → free-form JSON (single-`$ref` `allOf`
            // composition is resolved above).
            _ => "serde_json::Value".to_string(),
        }
    }

    /// The `allOf`-flattened object shape of a component schema: its members,
    /// ancestors first, and the union of every `required` list on the chain.
    ///
    /// OAS 3.0 models inheritance as `allOf` composition
    /// (<https://spec.openapis.org/oas/v3.0.3#composition-and-inheritance-polymorphism>).
    /// A member a descendant redeclares keeps the ancestor's position.
    fn merged_object(&self, schema: &Value) -> (Vec<(String, Value)>, BTreeSet<String>) {
        let mut props: Vec<(String, Value)> = Vec::new();
        let mut required: BTreeSet<String> = BTreeSet::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        self.collect_object(schema, &mut props, &mut required, &mut seen);
        (props, required)
    }

    /// Accumulates one link of the [`Ctx::merged_object`] chain, ancestors first;
    /// `seen` breaks a cyclic or diamond `$ref` chain.
    fn collect_object(
        &self,
        schema: &Value,
        props: &mut Vec<(String, Value)>,
        required: &mut BTreeSet<String>,
        seen: &mut BTreeSet<String>,
    ) {
        if let Some(members) = schema.get("allOf").and_then(Value::as_array) {
            for member in members {
                if let Some(name) = Oas::ref_name(member)
                    && !seen.insert(name)
                {
                    continue;
                }
                self.collect_object(self.oas.resolve(member), props, required, seen);
            }
        }
        if let Some(own) = schema.get("properties").and_then(Value::as_object) {
            for (pname, pschema) in own {
                if let Some(slot) = props.iter_mut().find(|(n, _)| n == pname) {
                    slot.1 = pschema.clone();
                } else {
                    props.push((pname.clone(), pschema.clone()));
                }
            }
        }
        required.extend(
            schema
                .get("required")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }

    /// Maps an OAS parameter to a Rust type: the grammar type
    /// [`crate::plan::rest_text_types`] records for its component, else its schema's.
    ///
    /// A parameter is text under its `style`/`explode` rules
    /// (<https://spec.openapis.org/oas/v3.0.3#style-values>), so an array of
    /// structured items carries one raw `String` per occurrence.
    fn param_rust_type(&self, op: &Operation, p: &Param) -> String {
        let schema = &p.schema;
        if let Some(typed) =
            crate::plan::rest_text_types::parameter_type(p.component.as_deref(), &op.operation_id)
        {
            return typed.rust_type.to_owned();
        }
        if schema.get("type").and_then(Value::as_str) == Some("array") {
            let item = schema.get("items").map(|i| self.rust_type(i));
            let inner = item
                .as_deref()
                .filter(|mapped| matches!(*mapped, "String" | "i64" | "f64" | "bool"))
                .unwrap_or("String");
            return format!("Vec<{inner}>");
        }
        self.rust_type(schema)
    }
}

fn emit_dto(b: &mut String, name: &str, schema: &Value, ctx: &Ctx) {
    let schema = ctx.oas.resolve(schema);
    // A `discriminator.mapping` over `_type` emits an enum over its targets.
    if let Some(mapping) = schema
        .get("discriminator")
        .and_then(|d| d.get("mapping"))
        .and_then(Value::as_object)
    {
        // A base carrying `x-discriminator-value` is instantiable: it emits as a
        // `…Data` struct and joins the enum as one more variant.
        let base_tag = schema
            .get("x-discriminator-value")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (props, required) = ctx.merged_object(schema);
        let base = match (base_tag, props.is_empty()) {
            (Some(tag), false) => {
                let data_ty = format!("{}Data", dto_type(name));
                emit_struct(
                    b,
                    name,
                    &data_ty,
                    &Generic::None,
                    &props,
                    &required,
                    schema,
                    ctx,
                );
                Some((tag, data_ty))
            }
            _ => None,
        };
        let ty_name = dto_type(name);
        let variants: Vec<(String, String, String)> = mapping
            .iter()
            .filter_map(|(tag, target)| {
                let target = target.as_str()?;
                let ref_name = target.rsplit('/').next()?.to_string();
                let rust_ty = ctx.rust_type(&serde_json::json!({ "$ref": target }));
                Some((tag.clone(), ref_name, rust_ty))
            })
            .collect();
        // Docs-text aliases read as the instantiable base, keeping their own tag.
        let aliases: Vec<&str> = crate::plan::overrides::rest_discriminator_aliases(name)
            .map(|a| a.tag)
            .collect();
        emit_tagged_enum(
            b,
            &TaggedEnum {
                label: name,
                ty_name: &ty_name,
                summary: format!(
                    "/// The `{name}` ITS-REST OAS component schema: `_type`-discriminated\n\
                     /// polymorphism over its OAS `discriminator.mapping` targets.\n"
                ),
                variants: &variants,
                aliases: &aliases,
                base: base.as_ref(),
            },
        );
        return;
    }
    // object with named properties → struct; everything else → an alias.
    if schema.get("type").and_then(Value::as_str) == Some("object")
        && schema.get("properties").is_some()
    {
        let ty_name = dto_type(name);
        let (props, required) = ctx.merged_object(schema);
        // The generic hoisted schema types its flattened field as `T`.
        let generic = if ctx.in_common {
            GENERIC_OVER
                .iter()
                .find(|(n, _)| *n == name)
                .map_or(Generic::None, |(_, f)| Generic::Param(f))
        } else if let Some((_, field)) = GENERIC_CARRIERS.iter().find(|(n, _)| *n == name) {
            let (envelope, data) = ENVELOPE;
            Generic::Carrier {
                field,
                default: generic_arg(ctx, envelope, data),
            }
        } else {
            Generic::None
        };
        emit_struct(b, name, &ty_name, &generic, &props, &required, schema, ctx);
    } else {
        // string/array/map/ref alias.
        let typed = crate::plan::rest_text_types::schema_type(name);
        let _ = writeln!(
            b,
            "/// The `{name}` ITS-REST OAS component schema (a non-object shape, so\n\
             /// it is an alias rather than a struct).{}\n\
             pub type {} = {};\n",
            typed.map_or_else(String::new, |t| format!(
                "\n///\n/// Read by its grammar, {}.",
                t.citation
            )),
            dto_type(name),
            typed.map_or_else(|| ctx.rust_type(schema), |t| t.rust_type.to_owned())
        );
    }
}

/// Emits one transport-DTO struct from its `allOf`-flattened shape.
#[expect(
    clippy::too_many_arguments,
    reason = "one emission site each for the schema's identity, its Rust name, the generic binding, the flattened shape and the declaring schema — bundling them into a struct would only rename the same arguments"
)]
fn emit_struct(
    b: &mut String,
    name: &str,
    ty_name: &str,
    generic: &Generic,
    props: &[(String, Value)],
    all_required: &BTreeSet<String>,
    schema: &Value,
    ctx: &Ctx,
) {
    // The OAS `required` list minus `plan::overrides::REST_OPTIONAL_OVERRIDES`.
    let required: BTreeSet<&str> = all_required
        .iter()
        .map(String::as_str)
        .filter(|f| crate::plan::overrides::rest_optional_override(ty_name, f).is_none())
        .collect();
    // `additionalProperties: false` is a closed object: `deny_unknown_fields`.
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    let (deny_doc, deny_attr) = if closed {
        (
            "///\n\
             /// The OAS declares this schema `additionalProperties: false`, so an\n\
             /// undeclared member is refused rather than silently ignored.\n",
            "#[serde(deny_unknown_fields)]\n",
        )
    } else {
        ("", "")
    };
    let generics = match generic {
        Generic::None => String::new(),
        Generic::Param(_) => "<T>".to_string(),
        Generic::Carrier { default, .. } => format!("<T = {default}>"),
    };
    let _ = write!(
        b,
        "/// The `{name}` transport DTO of this API group (an ITS-REST OAS\n\
         /// component schema).\n\
         {deny_doc}#[derive(Debug, Clone, Serialize, Deserialize)]\n\
         {deny_attr}pub struct {ty_name}{generics} {{\n"
    );
    for (pname, pschema) in props {
        emit_struct_field(
            b,
            &StructField {
                owner: name,
                ty_name,
                pname,
                pschema,
                ty: match generic {
                    Generic::Param(field) if *field == pname.as_str() => Some("T".to_string()),
                    Generic::Carrier { field, .. } if *field == pname.as_str() => {
                        Some(carrier_field_type(&ctx.rust_type(pschema)))
                    }
                    _ => None,
                },
                is_required: required.contains(pname.as_str()),
            },
            ctx,
        );
    }
    emit_additional_properties(b, name, schema, ctx);
    b.push_str("}\n\n");
}

/// One DTO field's emission inputs.
#[derive(Clone)]
struct StructField<'a> {
    /// The OAS component schema name the field belongs to.
    owner: &'a str,
    /// The Rust type name of the emitted DTO.
    ty_name: &'a str,
    /// The OAS property name.
    pname: &'a str,
    /// The OAS property schema.
    pschema: &'a Value,
    /// The field's type where the DTO's generic parameter sets it.
    ty: Option<String>,
    /// Whether the field is required after the docs-text-wins corrections.
    is_required: bool,
}

/// The type of a [`GENERIC_CARRIERS`] field: its group-alias envelope items
/// (`Vec<UpdateVersion>`) rebound to the shared envelope over `T`.
fn carrier_field_type(ty: &str) -> String {
    let envelope = dto_type(ENVELOPE.0);
    let rebound = ty.replacen(
        &format!("<{envelope}>"),
        &format!("<super::common::{envelope}<T>>"),
        1,
    );
    if rebound == ty {
        format!("compile_error!(\"the carrier field type `{ty}` holds no `{envelope}` list\")")
    } else {
        rebound
    }
}

/// Emits one DTO field: its doc line, serde attributes and typed declaration.
fn emit_struct_field(b: &mut String, f: &StructField<'_>, ctx: &Ctx) {
    let ident = field_id(f.pname);
    let mut ty = f.ty.clone().unwrap_or_else(|| ctx.rust_type(f.pschema));
    if !f.is_required {
        ty = format!("Option<{ty}>");
    }
    // `missing_docs` checks a field; the OAS property name is its summary.
    let _ = writeln!(b, "    /// The `{}` property of `{}`.", f.pname, f.owner);
    // A docs-text-wins correction carries its citation into the generated code.
    if let Some(ov) = crate::plan::overrides::rest_optional_override(f.ty_name, f.pname) {
        let _ = writeln!(b, "    /// OPTIONAL by the docs text — {}", ov.citation);
        let _ = writeln!(b, "    /// ({})", ov.reason);
    }
    // A default-when-absent member keeps its type and carries its citation.
    let default_member = rest_default_member(f.ty_name, f.pname);
    if let Some(dm) = default_member {
        let _ = writeln!(b, "    /// DEFAULT when absent — {}", dm.citation);
        let _ = writeln!(b, "    /// ({})", dm.reason);
    }
    let tagged = crate::plan::rest_tagged::tagged_field(f.owner, f.pname, f.is_required);
    b.push_str(tagged.as_ref().map_or("", |t| t.doc.as_str()));
    if let Some(rename) = naming::serde_rename(f.pname, &ident) {
        let _ = writeln!(b, "    #[serde(rename = \"{rename}\")]");
    }
    if !f.is_required {
        b.push_str(SKIP_NONE_ATTR);
    }
    if default_member.is_some() {
        b.push_str("    #[serde(default)]\n");
    }
    b.push_str(tagged.as_ref().map_or("", |t| t.attr));
    let _ = writeln!(b, "    pub {ident}: {ty},");
}

/// One `_type`-dispatched enum to emit: an OAS `discriminator.mapping`
/// schema ([`emit_dto`]) or an inline response `oneOf` ([`response_unions`]).
struct TaggedEnum<'a> {
    /// The OAS name the decode error messages carry (the component schema or
    /// the response component).
    label: &'a str,
    /// The emitted Rust type name.
    ty_name: &'a str,
    /// The type's doc comment, `///` lines ending in a newline.
    summary: String,
    /// `(tag, variant, rust type)` per member, in document order.
    variants: &'a [(String, String, String)],
    /// Docs-text alias tags that read as the instantiable base.
    aliases: &'a [&'a str],
    /// `(tag, data type)` of an instantiable base, if any.
    base: Option<&'a (String, String)>,
}

/// Emits a `_type`-dispatched enum, one variant per member in document order.
///
/// An unknown `_type` is refused naming the legal set. With `base`, the
/// schema's own `x-discriminator-value` form is a final variant and an object
/// with no `_type` reads as it; without one, a missing `_type` is an error.
fn emit_tagged_enum(b: &mut String, e: &TaggedEnum<'_>) {
    let name = e.label;
    let ty_name = e.ty_name;
    let base = e.base;
    let aliases = e.aliases;
    let mut variants: Vec<(String, String, String)> = e.variants.to_vec();
    if let Some((tag, data_ty)) = base {
        variants.push((tag.clone(), ty_name.to_owned(), data_ty.clone()));
    }
    let variants = variants;
    let tag_list = variants
        .iter()
        .map(|(t, _, _)| t.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let _ = write!(
        b,
        "{}#[derive(Debug, Clone)]\npub enum {ty_name} {{\n",
        e.summary
    );
    for (tag, ref_name, rust_ty) in &variants {
        let _ = writeln!(b, "    /// `_type: \"{tag}\"`\n    {ref_name}({rust_ty}),");
    }
    b.push_str("}\n\n");
    // Serialize: delegate to the inner value (its impl writes `_type`).
    let _ = write!(
        b,
        "impl ::serde::Serialize for {ty_name} {{\n    \
         fn serialize<__S: ::serde::Serializer>(&self, __serializer: __S) \
         -> ::core::result::Result<__S::Ok, __S::Error> {{\n        match self {{\n"
    );
    for (_, ref_name, _) in &variants {
        let _ = writeln!(
            b,
            "            Self::{ref_name}(__x) => ::serde::Serialize::serialize(__x, __serializer),"
        );
    }
    b.push_str("        }\n    }\n}\n\n");
    // Deserialize: strict tag-anywhere dispatch on the mapping keys.
    let _ = write!(
        b,
        "impl<'de> ::serde::Deserialize<'de> for {ty_name} {{\n    \
         fn deserialize<__D: ::serde::Deserializer<'de>>(__deserializer: __D) \
         -> ::core::result::Result<Self, __D::Error> {{\n        \
         const __TAGS: &[&str] = &[{tags}];\n        \
         struct __Visitor;\n        \
         impl<'de> ::serde::de::Visitor<'de> for __Visitor {{\n            \
         type Value = {ty_name};\n            \
         fn expecting(&self, __f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {{\n                \
         __f.write_str(\"an ITS-REST `{name}` object\")\n            \
         }}\n            \
         fn visit_map<__A: ::serde::de::MapAccess<'de>>(\n                \
         self,\n                mut __map: __A,\n            \
         ) -> ::core::result::Result<Self::Value, __A::Error> {{\n                \
         let (__tag, __buffered) =\n                    \
         ::openehr_base::serde_support::read_slot_tag(&mut __map, __TAGS)?;\n                \
         match __tag {{\n                    \
         Some(::openehr_base::serde_support::TagMatch::Known(__t)) => {{\n                        \
         let __rest = ::openehr_base::serde_support::TaggedRest::new(\n                            \
         Some(__t),\n                            __buffered,\n                            __map,\n                        \
         );\n                        match __t {{\n",
        tags = variants
            .iter()
            .map(|(t, _, _)| t.as_str())
            .chain(aliases.iter().copied())
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for (tag, ref_name, _) in &variants {
        let _ = write!(
            b,
            "                            \"{tag}\" => ::core::result::Result::Ok({ty_name}::{ref_name}(\n                                \
             ::serde::Deserialize::deserialize(__rest)?,\n                            )),\n"
        );
    }
    for tag in aliases {
        if base.is_some() {
            let _ = write!(
                b,
                "                            \"{tag}\" => ::core::result::Result::Ok({ty_name}::{ty_name}(\n                                \
                 ::serde::Deserialize::deserialize(__rest)?,\n                            )),\n"
            );
        } else {
            let _ = writeln!(
                b,
                "                            \"{tag}\" => compile_error!(\"the alias `{tag}` needs an instantiable `{name}` base\"),"
            );
        }
    }
    // No `_type`: the concrete base's form when declared, otherwise a refusal.
    let none_arm = base.map_or_else(
        || {
            format!(
                "                        \
                 ::core::result::Result::Err(::openehr_base::serde_support::missing_type(\n                            \
                 \"{name}\",\n                            \"{tag_list}\",\n                        ))\n"
            )
        },
        |(_, _)| {
            format!(
                "                        \
                 let __rest =\n                            \
                 ::openehr_base::serde_support::TaggedRest::new(None, __buffered, __map);\n                        \
                 ::core::result::Result::Ok({ty_name}::{ty_name}(\n                            \
                 ::serde::Deserialize::deserialize(__rest)?,\n                        ))\n"
            )
        },
    );
    let _ = write!(
        b,
        "                            __other => ::core::result::Result::Err(\n                                \
         ::openehr_base::serde_support::unexpected_type(\n                                    \
         \"{name}\",\n                                    __other,\n                                    \
         \"{tag_list}\",\n                                ),\n                            ),\n                        \
         }}\n                    }}\n                    \
         Some(::openehr_base::serde_support::TagMatch::Unknown(__other)) => {{\n                        \
         ::core::result::Result::Err(::openehr_base::serde_support::unexpected_type(\n                            \
         \"{name}\",\n                            &__other,\n                            \"{tag_list}\",\n                        \
         ))\n                    }}\n                    \
         None => {{\n{none_arm}                    \
         }}\n                }}\n            }}\n        }}\n        \
         ::serde::Deserializer::deserialize_map(__deserializer, __Visitor)\n    }}\n}}\n\n"
    );
}

/// One inline response `oneOf` the group emits as a `_type`-dispatched enum.
struct ResponseUnion {
    /// The `#/components/responses` entry declaring the body.
    component: String,
    /// The emitted Rust type name.
    ty_name: String,
    /// The stable form of the `oneOf` schema ([`stable_repr`]).
    repr: String,
    /// `(tag, variant, rust type)` per member, in document order.
    variants: Vec<(String, String, String)>,
}

/// The inline response-body `oneOf`s of `ops` that are `_type` unions, one per
/// response component, in first-use order.
///
/// A `oneOf` without a `discriminator` qualifies when every member is a `$ref`
/// with its own distinct single-valued `_type` enum, which makes the members
/// mutually exclusive (<https://spec.openapis.org/oas/v3.0.3#discriminator-object>).
fn response_unions(ops: &[Operation], ctx: &Ctx) -> Vec<ResponseUnion> {
    let mut out: Vec<ResponseUnion> = Vec::new();
    for resp in ops.iter().flat_map(|op| op.responses.iter()) {
        let (Some(component), [(mt, schema)]) = (&resp.component, resp.media.as_slice()) else {
            continue;
        };
        if mt != "application/json" || out.iter().any(|u| &u.component == component) {
            continue;
        }
        if let Some(variants) = union_variants(schema, ctx) {
            let stem = component.trim_start_matches(|c: char| c.is_ascii_digit() || c == '_');
            out.push(ResponseUnion {
                component: component.clone(),
                ty_name: format!("{}Body", type_id(stem)),
                repr: stable_repr(schema),
                variants,
            });
        }
    }
    out
}

/// The `(tag, variant, rust type)` members of a [`response_unions`] `oneOf`,
/// or `None` when the schema is not one.
fn union_variants(schema: &Value, ctx: &Ctx) -> Option<Vec<(String, String, String)>> {
    if schema.get("$ref").is_some() || schema.get("discriminator").is_some() {
        return None;
    }
    let members = schema.get("oneOf")?.as_array()?;
    if members.len() < 2 {
        return None;
    }
    let mut variants: Vec<(String, String, String)> = Vec::new();
    for member in members {
        let ref_name = Oas::ref_name(member)?;
        if ref_name == "Identifier" {
            return None;
        }
        let tag = match ctx
            .oas
            .resolve(member)
            .pointer("/properties/_type/enum")?
            .as_array()?
            .as_slice()
        {
            [only] => only.as_str()?.to_owned(),
            _ => return None,
        };
        if variants.iter().any(|(t, _, _)| *t == tag) {
            return None;
        }
        variants.push((tag, ref_name, ctx.rust_type(member)));
    }
    Some(variants)
}

fn param_struct_name(op: &Operation) -> String {
    format!("{}Params", type_id(&op.operation_id))
}

/// Whether `op` gets a param struct: it declares at least one parameter.
fn has_params_struct(op: &Operation) -> bool {
    !op.parameters.is_empty()
}

/// Emits the param struct of `op` and, from the same parameter list, its
/// `PARAMS` table — one `crate::rest::routes::Param` per field, in field order.
fn emit_params_struct(b: &mut String, op: &Operation, ctx: &Ctx) {
    if !has_params_struct(op) {
        return;
    }
    let mut table = String::new();
    let sname = param_struct_name(op);
    let _ = write!(
        b,
        "/// Parameters for `{}` (path/query/header).\n\
         #[derive(Debug, Clone, Serialize, Deserialize)]\npub struct {sname} {{\n",
        op.operation_id
    );
    for p in &op.parameters {
        let ident = field_id(&p.name);
        let mut ty = ctx.param_rust_type(op, p);
        if !p.required {
            ty = format!("Option<{ty}>");
        }
        if let Some(rename) = naming::serde_rename(&p.name, &ident) {
            let _ = writeln!(b, "    #[serde(rename = \"{rename}\")]");
        }
        if !p.required {
            b.push_str(SKIP_NONE_ATTR);
        }
        let _ = writeln!(b, "    /// `{}` ({})", p.name, p.location);
        let op_id = &op.operation_id;
        if let Some(t) = crate::plan::rest_text_types::parameter_type(p.component.as_deref(), op_id)
        {
            let _ = writeln!(b, "    ///\n    /// Read by its grammar, {}.", t.citation);
        }
        // A docs-text header carries its citation into the generated code (the
        // OAS declares no parameter for it; the ITS-REST docs text defines it).
        if let Some(h) = rest_docs_text_header(&op.operation_id, &p.name) {
            let _ = writeln!(b, "    /// DEFINED by the docs text — {}", h.citation);
            let _ = writeln!(b, "    /// ({})", h.reason);
        }
        let _ = writeln!(b, "    pub {ident}: {ty},");
        let _ = writeln!(table, "        {},", param_entry(ctx.oas, p));
    }
    b.push_str("}\n\n");
    let _ = write!(
        b,
        "impl {sname} {{\n    \
         /// The parameters of `{}`, one per field, in field order.\n    \
         pub const PARAMS: &'static [crate::rest::routes::Param] = &[\n{table}    ];\n\n",
        op.operation_id
    );
    emit_params_decoders(b, op, ctx);
    b.push_str("}\n\n");
}

/// Emits `{operation}_request_body`, the public request-body decoder the
/// generated handler runs.
fn emit_request_body_decoder(b: &mut String, op: &Operation, ctx: &Ctx) {
    let Some((schema, required)) = &op.request_body else {
        return;
    };
    let json = json_request(op);
    let simplified = simplified_request(op, ctx);
    let ty = if json {
        server_body_type(op, schema, ctx)
    } else {
        "String".to_string()
    };
    let (ty, call) = match (json, *required) {
        (true, true) if simplified => (ty, "payload_body(content_type, body)"),
        (true, false) if simplified => (
            format!("Option<{ty}>"),
            "payload_body_optional(content_type, body)",
        ),
        (true, true) => (ty, "json_body(content_type, body)"),
        (true, false) => (
            format!("Option<{ty}>"),
            "json_body_optional(content_type, body)",
        ),
        (false, true) => (ty, "text_body(body)"),
        (false, false) => (format!("Option<{ty}>"), "text_body_optional(body)"),
    };
    let content_type = if json {
        "content_type"
    } else {
        "_content_type"
    };
    let _ = write!(
        b,
        "/// Decodes the request body of `{op_id}` from the request's `Content-Type`\n\
         /// and body bytes, exactly as the generated router decodes it.\n\
         ///\n\
         /// # Errors\n\
         /// Returns [`crate::rest::runtime::ApiError::UnsupportedMediaType`] for a\n\
         /// `Content-Type` the operation does not read, and\n\
         /// [`crate::rest::runtime::ApiError::BadRequest`] for a body that is absent where\n\
         /// required or not the documented shape.\n\
         pub fn {method}_request_body(\n    \
         {content_type}: Option<&http::HeaderValue>,\n    \
         body: &[u8],\n\
         ) -> Result<{ty}, crate::rest::runtime::ApiError> {{\n    \
         crate::rest::decode::{call}\n\
         }}\n\n",
        op_id = op.operation_id,
        method = field_id(&op.operation_id),
    );
}

/// Emits the `from_request` and `from_parts` decoders of `op`'s param struct.
fn emit_params_decoders(b: &mut String, op: &Operation, ctx: &Ctx) {
    let has = |location: &str| op.parameters.iter().any(|p| p.location == location);
    let (path, query, header) = (has("path"), has("query"), has("header"));
    let unused = |used: bool, name: &str| {
        if used {
            name.to_string()
        } else {
            format!("_{name}")
        }
    };
    let mut parts: Vec<&str> = Vec::new();
    let mut params: Vec<&str> = Vec::new();
    let mut prelude = String::new();
    if path {
        prelude.push_str(
            "        let path = crate::rest::decode::PathValues::from_route(matched)?;\n",
        );
        parts.push("&path");
        params.push("path: &crate::rest::decode::PathValues");
    }
    if query {
        prelude.push_str("        let query = crate::rest::decode::QueryPairs::parse(query)?;\n");
        parts.push("&query");
        params.push("query: &crate::rest::decode::QueryPairs");
    }
    if header {
        parts.push("headers");
        params.push("headers: &http::HeaderMap");
    }
    let _ = write!(
        b,
        "    /// Decodes the parameters of `{op_id}` from a request [`crate::rest::routes::lookup`]\n    \
         /// matched to it: its path parameters, its query string without the `?`, and its\n    \
         /// headers, exactly as the generated router decodes them.\n    \
         ///\n    \
         /// # Errors\n    \
         /// Returns [`crate::rest::runtime::ApiError::BadRequest`] naming the parameter that is\n    \
         /// missing, repeated where a single value is declared, not text, or not a valid value.\n    \
         pub fn from_request(\n        \
         {matched}: &crate::rest::routes::RouteMatch,\n        \
         {query_arg}: Option<&str>,\n        \
         {headers_arg}: &http::HeaderMap,\n    \
         ) -> Result<Self, crate::rest::runtime::ApiError> {{\n\
         {prelude}        Self::from_parts({parts})\n    \
         }}\n\n    \
         /// Decodes the parameters of `{op_id}` from the request's decoded parts.\n    \
         pub(crate) fn from_parts({params}) -> Result<Self, crate::rest::runtime::ApiError> {{\n        \
         Ok(Self {{\n",
        op_id = op.operation_id,
        matched = unused(path, "matched"),
        query_arg = unused(query, "query"),
        headers_arg = unused(header, "headers"),
        parts = parts.join(", "),
        params = params.join(", "),
    );
    for p in &op.parameters {
        let _ = writeln!(
            b,
            "            {}: {},",
            field_id(&p.name),
            param_extraction(op, p, ctx)
        );
    }
    b.push_str("        })\n    }\n");
}

/// The path of the hand-written parameter model the route tables carry.
const ROUTES_MODULE: &str = "crate::rest::routes";

/// The `crate::rest::routes::Param` literal of `p`.
fn param_entry(oas: &Oas, p: &Param) -> String {
    let location = match p.location.as_str() {
        "path" => format!("{ROUTES_MODULE}::ParamLocation::Path"),
        "query" => format!("{ROUTES_MODULE}::ParamLocation::Query"),
        "header" => format!("{ROUTES_MODULE}::ParamLocation::Header"),
        "cookie" => format!("{ROUTES_MODULE}::ParamLocation::Cookie"),
        other => {
            format!("compile_error!(\"parameter location `{other}` is not an OAS 3.0 location\")")
        }
    };
    format!(
        "{ROUTES_MODULE}::Param {{ name: {:?}, location: {location}, required: {}, explode: {}, kind: {}, identifier: {} }}",
        p.name,
        p.required,
        param_explode(p),
        param_kind(oas, &p.schema),
        param_identifier(p)
    )
}

/// The `Option<crate::rest::routes::IdentifierClass>` expression of `p`, from
/// [`crate::plan::overrides::REST_PATH_IDENTIFIERS`] for a path parameter.
fn param_identifier(p: &Param) -> String {
    if p.location != "path" {
        return "None".to_string();
    }
    let Some(entry) = p
        .component
        .as_deref()
        .and_then(crate::plan::overrides::rest_path_identifier)
    else {
        return format!(
            "compile_error!(\"path parameter `{}` has no REST_PATH_IDENTIFIERS entry\")",
            p.name
        );
    };
    match entry.class {
        None => "None".to_string(),
        Some("HIER_OBJECT_ID") => format!("Some({ROUTES_MODULE}::IdentifierClass::HierObject)"),
        Some("OBJECT_VERSION_ID") => {
            format!("Some({ROUTES_MODULE}::IdentifierClass::ObjectVersion)")
        }
        Some("UID_BASED_ID") => format!("Some({ROUTES_MODULE}::IdentifierClass::UidBased)"),
        Some(other) => format!("compile_error!(\"no IdentifierClass for `{other}`\")"),
    }
}

/// Whether `p` explodes: its declared `explode`, else `true` under the `form`
/// style (OAS 3.0.3 §Parameter Object).
fn param_explode(p: &Param) -> bool {
    let default_style = match p.location.as_str() {
        "query" | "cookie" => "form",
        _ => "simple",
    };
    let style = p.style.as_deref().unwrap_or(default_style);
    p.explode.unwrap_or(style == "form")
}

/// The `crate::rest::routes::ParamKind` expression of a parameter `schema`,
/// derived only from the `enum`, `type`, `format` and `items` it states.
fn param_kind(oas: &Oas, schema: &Value) -> String {
    let schema = oas.resolve(schema);
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let values: Vec<String> = values
            .iter()
            .map(|v| {
                format!(
                    "{:?}",
                    v.as_str().map_or_else(|| v.to_string(), str::to_string)
                )
            })
            .collect();
        return format!("{ROUTES_MODULE}::ParamKind::Enum(&[{}])", values.join(", "));
    }
    if schema.get("type").is_none()
        && let Some([only]) = schema
            .get("allOf")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
    {
        return param_kind(oas, only);
    }
    let kind = match schema.get("type").and_then(Value::as_str) {
        None => "Unspecified".to_string(),
        Some("string") => match schema.get("format").and_then(Value::as_str) {
            None => "Text".to_string(),
            Some("uuid") => "Uuid".to_string(),
            Some("date") => "Date".to_string(),
            // NOTE: the bundles spell `version_at_time` as `format: datetime`; OAS
            // 3.0.3 §Data Types registers `date-time`, and both name one instant.
            Some("date-time" | "datetime") => "DateTime".to_string(),
            Some(other) => format!("Formatted({other:?})"),
        },
        Some("integer") => "Integer".to_string(),
        Some("number") => "Number".to_string(),
        Some("boolean") => "Boolean".to_string(),
        Some("object") => "Object".to_string(),
        Some("array") => {
            let item = schema.get("items").map_or_else(
                || format!("{ROUTES_MODULE}::ParamKind::Unspecified"),
                |items| param_kind(oas, items),
            );
            format!("Array(&{item})")
        }
        Some(other) => {
            return format!("compile_error!(\"schema type `{other}` is not an OAS 3.0 type\")");
        }
    };
    format!("{ROUTES_MODULE}::ParamKind::{kind}")
}

/// Appends `op`'s `plan::overrides::REST_DOCS_TEXT_HEADERS` as optional header
/// parameters.
fn append_docs_text_headers(op: &mut Operation<'_>) {
    for h in rest_docs_text_headers(&op.operation_id) {
        if op.parameters.iter().any(|p| p.name == h.name) {
            continue;
        }
        let schema = if h.list {
            serde_json::json!({ "type": "array", "items": { "type": "string" } })
        } else {
            serde_json::json!({ "type": "string" })
        };
        op.parameters.push(Param {
            name: h.name.to_string(),
            location: "header".to_string(),
            component: None,
            required: false,
            style: None,
            explode: None,
            schema,
        });
    }
}

/// Whether `op`'s request body is canonical JSON (its content declares
/// `application/json`); any other body travels as text.
fn json_request(op: &Operation) -> bool {
    op.request_media.iter().any(|m| m == "application/json")
}

/// The media types `op`'s request body is admitted in: its `requestBody.content`
/// keys, then any further type its `Content-Type` parameter enum declares.
///
/// The docs text admits XML and the Simplified Formats where the OAS lists
/// only JSON (ITS-REST `operations/contribution_create.yaml` §Simplified
/// Formats).
fn admitted_request_media(op: &Operation, oas: &Oas) -> Vec<String> {
    let mut media = op.request_media.clone();
    let declared = op
        .parameters
        .iter()
        .filter(|p| p.location == "header" && p.name.eq_ignore_ascii_case("content-type"))
        .filter_map(|p| oas.resolve(&p.schema).get("enum").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str);
    for m in declared {
        if !op.request_media.is_empty() && !media.iter().any(|known| known == m) {
            media.push(m.to_string());
        }
    }
    media
}

/// The Simplified Formats media types (ITS-REST overview `Resources.md`
/// §Simplified Formats).
const SIMPLIFIED_MEDIA: [&str; 2] = [
    "application/openehr.wt.flat+json",
    "application/openehr.wt.structured+json",
];

/// Whether `op` takes a canonical-JSON body whose `Content-Type` parameter
/// admits both Simplified Formats.
fn simplified_request(op: &Operation, ctx: &Ctx) -> bool {
    json_request(op)
        && op.parameters.iter().any(|p| {
            p.location == "header"
                && p.name.eq_ignore_ascii_case("content-type")
                && ctx
                    .oas
                    .resolve(&p.schema)
                    .get("enum")
                    .and_then(Value::as_array)
                    .is_some_and(|values| {
                        SIMPLIFIED_MEDIA
                            .iter()
                            .all(|m| values.iter().any(|v| v.as_str() == Some(m)))
                    })
        })
}

/// The server body type of `op`: its canonical type, or a `Payload` over it
/// and its Simplified Formats form.
fn server_body_type(op: &Operation, schema: &Value, ctx: &Ctx) -> String {
    let canonical = ctx.rust_type(schema);
    if !simplified_request(op, ctx) {
        return canonical;
    }
    let carrier =
        Oas::ref_name(schema).is_some_and(|name| GENERIC_CARRIERS.iter().any(|(n, _)| *n == name));
    let simplified = if carrier {
        format!("{canonical}<serde_json::Value>")
    } else {
        "serde_json::Value".to_string()
    };
    format!("crate::rest::runtime::Payload<{canonical}, {simplified}>")
}

/// The headers struct of every documented response of `op` that declares
/// headers, at group level so both halves name the same type.
fn emit_response_headers(b: &mut String, op: &Operation) {
    for resp in &client_responses(op) {
        if resp.headers.is_empty() {
            continue;
        }
        let (variant, _) = status_variant(resp.status);
        let _ = write!(
            b,
            "/// The response headers the OAS declares for the `{}` answer of\n\
             /// `{} {}`: each value the answer carries, `None` (or an empty list)\n\
             /// when it carries none.\n\
             #[derive(Debug, Clone, Default)]\n\
             pub struct {} {{\n",
            resp.status.as_u16(),
            op.method.to_uppercase(),
            op.path,
            headers_name(op, &variant)
        );
        for h in &resp.headers {
            if LIST_HEADERS.contains(&h.as_str()) {
                let _ = writeln!(
                    b,
                    "    /// Every value of the `{h}` response header, one per field line.\n    pub {}: Vec<String>,",
                    field_id(h)
                );
            } else {
                let _ = writeln!(
                    b,
                    "    /// The `{h}` response header.\n    pub {}: Option<String>,",
                    field_id(h)
                );
            }
        }
        b.push_str("}\n\n");
        emit_rest_headers::emit_response_headers_impl(
            b,
            &headers_name(op, &variant),
            &resp.headers,
        );
    }
}

/// The success answers of `op` the server half answers with: the documented
/// `2xx` and `3xx` statuses. An error status is a `Refusal`.
fn success_responses(op: &Operation) -> Vec<Response> {
    client_responses(op)
        .into_iter()
        .filter(|r| r.status.is_success() || r.status.is_redirection())
        .collect()
}

fn server_response_name(op: &Operation) -> String {
    format!("{}Response", type_id(&op.operation_id))
}

/// The axum path for `op` plus the positional capture name of every path
/// parameter.
///
/// The RFC 6570 query expansion is dropped and each `{name}` becomes
/// `{p<segment index>}`: axum refuses two routes naming different captures at
/// one position.
fn axum_path(op: &Operation) -> (String, BTreeMap<String, String>) {
    let template = op.path.split("{?").next().unwrap_or(op.path);
    let mut captures = BTreeMap::new();
    let segments: Vec<String> = template
        .split('/')
        .enumerate()
        .map(|(index, segment)| {
            match segment
                .strip_prefix('{')
                .and_then(|rest| rest.strip_suffix('}'))
            {
                Some(name) => {
                    let capture = format!("p{index}");
                    captures.insert(name.to_string(), capture.clone());
                    format!("{{{capture}}}")
                }
                None => segment.to_string(),
            }
        })
        .collect();
    (segments.join("/"), captures)
}

/// Emits the `server` module of one API group: per operation the success
/// enum, trait method and handler, plus the `router`.
fn emit_server_module(out: &mut String, group: &str, ops: &[Operation], ctx: &Ctx) {
    let trait_name = format!("{}Api", naming::type_name(group));
    // Rendered into its own buffer, one module level below the group, like
    // the client module (see `emit_client_module`).
    let mut b = String::new();
    let b = &mut b;
    let _ = write!(
        b,
        "/// The server half of the `{group}` API group (ITS-REST): the `{trait_name}`\n\
         /// trait an implementation provides, one success-answer enum per operation,\n\
         /// and `router`, which binds every operation of the route table to its\n\
         /// trait method over axum.\n\
         #[cfg(feature = \"rest-server\")]\n\
         pub mod server {{\n    \
         use super::*;\n\n"
    );
    for op in ops {
        emit_server_response(b, op, ctx);
    }
    let _ = write!(
        b,
        "    /// Server contract for the `{group}` API group (ITS-REST).\n    \
         ///\n    \
         /// Every method defaults to refusing with `ApiError::NotImplemented` (`501`),\n    \
         /// so an implementor overrides only the operations it supports; `router`\n    \
         /// serves an implementation over axum. A method refuses with a\n    \
         /// [`crate::rest::runtime::Refusal`]: `?` turns an `ApiError` into one,\n    \
         /// and `Refusal::with_headers` adds the headers the OAS declares for the\n    \
         /// answer (the `ETag` of a `412`, for one).\n    \
         #[async_trait::async_trait]\n    \
         pub trait {trait_name} {{\n"
    );
    for op in ops {
        emit_trait_method(b, op, ctx);
    }
    b.push_str("    }\n\n");
    let _ = write!(
        b,
        "    /// The axum router serving every operation of the `{group}` group over `api`.\n    \
         ///\n    \
         /// Each route is bound at its OAS path relative to the API base (an RFC 6570\n    \
         /// query expansion dropped, path captures named by segment position). A\n    \
         /// handler decodes the request into the operation's params struct and body\n    \
         /// — a missing or unparseable parameter answers `400` naming it, a\n    \
         /// canonical-JSON body sent as another `Content-Type` answers `415` — and\n    \
         /// encodes the trait method's answer, or its `Refusal` as the ITS-REST\n    \
         /// `Error` body with the refusal's headers. Mount it under the base path\n    \
         /// with `axum::Router::nest`.\n    \
         ///\n    \
         /// The router carries no fallback, so group routers merge freely (axum\n    \
         /// refuses to merge two routers that both carry one); finish the merged\n    \
         /// router with `crate::rest::server::with_fallbacks` for the `404` and `405`\n    \
         /// answers, or take `crate::rest::server::router`, which does both.\n    \
         ///\n    \
         /// The typed bodies are canonical JSON only: a server that also serves\n    \
         /// canonical XML or a Simplified Format routes those requests itself, and\n    \
         /// the `accept` parameter reaches the trait method, which answers\n    \
         /// `ApiError::NotAcceptable` for a representation it does not serve.\n    \
         pub fn router<S>(api: std::sync::Arc<S>) -> axum::Router\n    \
         where\n        \
         S: {trait_name} + Send + Sync + 'static,\n    \
         {{\n        \
         axum::Router::new()\n"
    );
    for op in ops {
        let (path, _) = axum_path(op);
        let _ = writeln!(
            b,
            "            .route(\n                \"{path}\",\n                \
             axum::routing::on(axum::routing::MethodFilter::{}, handle_{}::<S>),\n            )",
            op.method.to_uppercase(),
            field_id(&op.operation_id)
        );
    }
    b.push_str("            .with_state(api)\n    }\n\n");
    for op in ops {
        emit_server_handler(b, op, &trait_name, ctx);
    }
    b.push_str("}\n\n");
    out.push_str(&b.replace("super::common::", "super::super::common::"));
}

/// Emits the success-answer enum of one operation, one variant per documented
/// `2xx` or `3xx` status.
fn emit_server_response(b: &mut String, op: &Operation, ctx: &Ctx) {
    let name = server_response_name(op);
    let _ = write!(
        b,
        "    /// The answers `{} {}` succeeds with: one variant per `2xx`/`3xx` status\n    \
         /// the OAS documents (an error is a [`crate::rest::runtime::Refusal`]).\n    \
         #[derive(Debug, Clone)]\n    \
         pub enum {name} {{\n",
        op.method.to_uppercase(),
        op.path
    );
    for resp in &success_responses(op) {
        let (variant, _) = status_variant(resp.status);
        let _ = writeln!(b, "        /// The `{}` answer.", resp.status.as_u16());
        let mut fields: Vec<(&str, String)> = Vec::new();
        match client_body(op, resp, ctx) {
            ClientBody::None | ClientBody::Error => {}
            ClientBody::Json { ty, optional } => fields.push(if optional {
                (
                    "The body, sent as canonical JSON; `None` sends no body.",
                    format!("body: Option<{ty}>"),
                )
            } else {
                ("The body, sent as canonical JSON.", format!("body: {ty}"))
            }),
            ClientBody::Raw => fields.push((
                "The body, sent as given, in the representation the request `Accept` \
                 selected (the `Content-Type` response header names it).",
                "body: Vec<u8>".to_string(),
            )),
        }
        if !resp.headers.is_empty() {
            fields.push((
                "The response headers the OAS declares for this answer.",
                format!("headers: {}", headers_name(op, &variant)),
            ));
        }
        if fields.is_empty() {
            let _ = writeln!(b, "        {variant},");
        } else {
            let _ = writeln!(b, "        {variant} {{");
            for (doc, field) in &fields {
                let _ = writeln!(b, "            /// {doc}\n            {field},");
            }
            b.push_str("        },\n");
        }
    }
    b.push_str("    }\n\n");
}

/// Emits the trait method of one operation, defaulting to `NotImplemented`.
fn emit_trait_method(b: &mut String, op: &Operation, ctx: &Ctx) {
    let method = field_id(&op.operation_id);
    let mut args = String::from("&self");
    if !op.parameters.is_empty() {
        let _ = write!(args, ", params: {}", param_struct_name(op));
    }
    if let Some((schema, required)) = &op.request_body {
        let ty = if json_request(op) {
            server_body_type(op, schema, ctx)
        } else {
            "String".to_string()
        };
        if *required {
            let _ = write!(args, ", body: {ty}");
        } else {
            let _ = write!(args, ", body: Option<{ty}>");
        }
    }
    let _ = writeln!(
        b,
        "        /// `{} {}`\n        \
         async fn {method}({args}) -> Result<{}, crate::rest::runtime::Refusal> {{\n            \
         Err(crate::rest::runtime::ApiError::NotImplemented.into())\n        \
         }}",
        op.method.to_uppercase(),
        op.path,
        server_response_name(op)
    );
}

/// How a parameter of type `ty` is carried: a list (`Vec`), a form-exploded
/// object (a free map), or a scalar.
enum ParamShape {
    Scalar,
    List,
    Object,
}

fn param_shape(ty: &str) -> ParamShape {
    let inner = ty
        .strip_prefix("Option<")
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(ty);
    if inner.starts_with("Vec<") {
        ParamShape::List
    } else if inner.starts_with("std::collections::BTreeMap<") || inner == "QueryParameters" {
        ParamShape::Object
    } else {
        ParamShape::Scalar
    }
}

/// The extraction expression for one parameter, the inverse of what the
/// client writes for it (`emit_query_param`, `emit_header_param`).
fn param_extraction(op: &Operation, p: &Param, ctx: &Ctx) -> String {
    let shape = param_shape(&ctx.param_rust_type(op, p));
    let name = &p.name;
    match p.location.as_str() {
        "path" => format!("path.value(\"{name}\")?"),
        "query" => match (shape, p.required) {
            (ParamShape::Scalar, true) => format!("query.required(\"{name}\")?"),
            (ParamShape::Scalar, false) => format!("query.optional(\"{name}\")?"),
            (ParamShape::List, true) => format!("query.list_required(\"{name}\")?"),
            (ParamShape::List, false) => format!("query.list(\"{name}\")?"),
            (ParamShape::Object, required) => {
                let declared: Vec<String> = op
                    .parameters
                    .iter()
                    .filter(|q| q.location == "query" && q.name != p.name)
                    .map(|q| format!("\"{}\"", q.name))
                    .collect();
                let call = if required {
                    "members_required"
                } else {
                    "members"
                };
                format!("query.{call}(\"{name}\", &[{}])?", declared.join(", "))
            }
        },
        _ => match (shape, p.required) {
            (ParamShape::List, true) => {
                format!("crate::rest::decode::header_list_required(headers, \"{name}\")?")
            }
            (ParamShape::List, false) => {
                format!("crate::rest::decode::header_list(headers, \"{name}\")?")
            }
            (_, true) => format!("crate::rest::decode::header_required(headers, \"{name}\")?"),
            (_, false) => format!("crate::rest::decode::header_optional(headers, \"{name}\")?"),
        },
    }
}

/// Emits the axum handler of one operation.
fn emit_server_handler(b: &mut String, op: &Operation, trait_name: &str, ctx: &Ctx) {
    let method = field_id(&op.operation_id);
    let response = server_response_name(op);
    let (_, captures) = axum_path(op);
    let has = |location: &str| op.parameters.iter().any(|p| p.location == location);
    let mut extractors = String::from(
        "        axum::extract::State(api): axum::extract::State<std::sync::Arc<S>>,\n",
    );
    if has("header") || op.request_body.is_some() {
        extractors.push_str("        headers: http::HeaderMap,\n");
    }
    if has("query") {
        extractors.push_str("        axum::extract::RawQuery(query): axum::extract::RawQuery,\n");
    }
    if has("path") {
        extractors.push_str(
            "        path: Result<\n            axum::extract::RawPathParams,\n            \
             axum::extract::rejection::RawPathParamsRejection,\n        >,\n",
        );
    }
    if op.request_body.is_some() {
        extractors.push_str("        body: axum::body::Bytes,\n");
    }
    let _ = write!(
        b,
        "    /// Serves `{} {}` through [`{trait_name}::{method}`].\n    \
         async fn handle_{method}<S>(\n{extractors}    ) -> axum::response::Response\n    \
         where\n        \
         S: {trait_name} + Send + Sync + 'static,\n    \
         {{\n        \
         let served: Result<axum::response::Response, crate::rest::runtime::Refusal> = async {{\n",
        op.method.to_uppercase(),
        op.path
    );
    let mut parts: Vec<&str> = Vec::new();
    if has("path") {
        let names: Vec<String> = captures
            .iter()
            .map(|(name, capture)| format!("(\"{capture}\", \"{name}\")"))
            .collect();
        let _ = writeln!(
            b,
            "            let path = crate::rest::server::path_captures(path, &[{}])?;",
            names.join(", ")
        );
        parts.push("&path");
    }
    if has("query") {
        b.push_str(
            "            let query = crate::rest::decode::QueryPairs::parse(query.as_deref())?;\n",
        );
        parts.push("&query");
    }
    if has("header") {
        parts.push("&headers");
    }
    let mut call_args: Vec<&str> = Vec::new();
    if !op.parameters.is_empty() {
        let _ = writeln!(
            b,
            "            let params = {}::from_parts({})?;",
            param_struct_name(op),
            parts.join(", ")
        );
        call_args.push("params");
    }
    if op.request_body.is_some() {
        let _ = writeln!(
            b,
            "            let body = {method}_request_body(headers.get(http::header::CONTENT_TYPE), &body)?;"
        );
        call_args.push("body");
    }
    let _ = writeln!(
        b,
        "            let reply: crate::rest::server::Reply = match api.{method}({}).await? {{",
        call_args.join(", ")
    );
    for resp in &success_responses(op) {
        let (variant, constant) = status_variant(resp.status);
        let body = client_body(op, resp, ctx);
        let has_body = matches!(body, ClientBody::Json { .. } | ClientBody::Raw);
        let mut bindings: Vec<&str> = Vec::new();
        if has_body {
            bindings.push("body");
        }
        if !resp.headers.is_empty() {
            bindings.push("headers");
        }
        let pattern = if bindings.is_empty() {
            format!("{response}::{variant}")
        } else {
            format!("{response}::{variant} {{ {} }}", bindings.join(", "))
        };
        let mut lines: Vec<String> = Vec::new();
        match body {
            ClientBody::Json { optional: true, .. } => {
                lines.push("if let Some(body) = body.as_ref() {\n                        reply.json(body)?;\n                    }".to_string());
            }
            ClientBody::Json {
                optional: false, ..
            } => lines.push("reply.json(&body)?;".to_string()),
            ClientBody::Raw => lines.push("reply.raw(body);".to_string()),
            ClientBody::None | ClientBody::Error => {}
        }
        if !resp.headers.is_empty() {
            lines.push("reply.headers(headers)?;".to_string());
        }
        if lines.is_empty() {
            let _ = writeln!(
                b,
                "                {pattern} => crate::rest::server::Reply::new({constant}),"
            );
        } else {
            let _ = writeln!(
                b,
                "                {pattern} => {{\n                    \
                 let mut reply = crate::rest::server::Reply::new({constant});"
            );
            for line in &lines {
                let _ = writeln!(b, "                    {line}");
            }
            b.push_str("                    reply\n                }\n");
        }
    }
    b.push_str(
        "            };\n            \
         Ok(reply.finish())\n        \
         }\n        \
         .await;\n        \
         crate::rest::server::respond(served)\n    \
         }\n\n",
    );
}

/// Emits the `client` module of one API group: per operation the outcome
/// enum, the headers structs and one client method.
///
/// A status the OAS does not document is a typed error, never an outcome.
fn emit_client_module(out: &mut String, group: &str, ops: &[Operation], ctx: &Ctx) {
    let client_ty = format!("{}Client", naming::type_name(group));
    // Its own buffer: one module deeper, so a hoisted reference needs one more
    // `super`.
    let mut b = String::new();
    let b = &mut b;
    let _ = write!(
        b,
        "/// The client half of the `{group}` API group (ITS-REST): one method per\n\
         /// operation over a [`crate::rest::client::Client`], answering an outcome\n\
         /// enum with one variant per status the OAS documents for it.\n\
         #[cfg(feature = \"rest-client\")]\n\
         pub mod client {{\n    \
         use super::*;\n\n"
    );
    for op in ops {
        emit_outcome(b, op, ctx);
    }
    let _ = write!(
        b,
        "    /// The `{group}` API group over one configured CDR.\n    \
         #[derive(Debug, Clone)]\n    \
         pub struct {client_ty}<'c, T> {{\n        \
         client: &'c crate::rest::client::Client<T>,\n        \
         options: crate::rest::client::CallOptions,\n    \
         }}\n\n    \
         impl<'c, T: crate::rest::client::Transport> {client_ty}<'c, T> {{\n        \
         /// The `{group}` API group over `client`.\n        \
         #[must_use]\n        \
         pub fn new(client: &'c crate::rest::client::Client<T>) -> Self {{\n            \
         Self {{\n                \
         client,\n                \
         options: crate::rest::client::CallOptions::default(),\n            \
         }}\n        \
         }}\n\n        \
         /// This group client applying `options` (a deadline, extra headers) to\n        \
         /// every call it makes.\n        \
         #[must_use]\n        \
         pub fn with_options(mut self, options: crate::rest::client::CallOptions) -> Self {{\n            \
         self.options = options;\n            \
         self\n        \
         }}\n\n"
    );
    for op in ops {
        emit_client_method(b, op, ctx);
    }
    b.push_str("    }\n}\n");
    out.push_str(&b.replace("super::common::", "super::super::common::"));
}

/// The `PascalCase` variant name and `http::StatusCode` constant path for one
/// status, from `canonical_reason`; an unnamed status fails the generated
/// crate's compile.
fn status_variant(status: http::StatusCode) -> (String, String) {
    let Some(reason) = status.canonical_reason() else {
        return (
            format!("UnregisteredStatus{}", status.as_u16()),
            format!("http::StatusCode::UNREGISTERED_{}", status.as_u16()),
        );
    };
    // `IM_A_TEAPOT` is the one constant whose spelling drops the apostrophe.
    let words: Vec<String> = reason
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    let name: String = words
        .iter()
        .map(|w| {
            let mut cs = w.chars();
            cs.next()
                .map(|c| c.to_ascii_uppercase())
                .into_iter()
                .chain(cs)
                .collect::<String>()
        })
        .collect();
    let constant = words
        .iter()
        .map(|w| w.to_ascii_uppercase())
        .collect::<Vec<_>>()
        .join("_");
    (name, format!("http::StatusCode::{constant}"))
}

/// The responses the client matches for `op`: the OAS set, plus `204` on every
/// create operation.
///
/// NOTE: ITS-REST `overview/Requests_and_responses.md` §Prefer header: "If no
/// response body is returned, the service SHOULD use `204 No Content`".
fn client_responses(op: &Operation) -> Vec<Response> {
    let has_prefer = op.parameters.iter().any(|p| p.name == "Prefer");
    let mut out: Vec<Response> = op
        .responses
        .iter()
        .map(|r| Response {
            status: r.status,
            component: r.component.clone(),
            media: r.media.clone(),
            headers: r.headers.clone(),
        })
        .collect();
    let created = out.iter().find(|r| r.status == http::StatusCode::CREATED);
    if has_prefer
        && let Some(created) = created
        && !out.iter().any(|r| r.status == http::StatusCode::NO_CONTENT)
    {
        let headers = created.headers.clone();
        out.push(Response {
            status: http::StatusCode::NO_CONTENT,
            component: None,
            media: Vec::new(),
            headers,
        });
        out.sort_by_key(|r| r.status);
    }
    out
}

/// The response headers carried as lists.
///
/// NOTE: ITS-REST `overview/Requests_and_responses.md` §openehr-item-tag: "The
/// list of all `ITEM_TAG`".
pub(super) const LIST_HEADERS: &[&str] = &["openehr-item-tag", "openehr-version-item-tag"];

/// The client's typing of one documented response body.
enum ClientBody {
    /// No content declared: the variant carries no body.
    None,
    /// A `4xx` answer: the error body as received, decoded as the ITS-REST
    /// `Error` when it is one (`crate::rest::client::ErrorBody`).
    Error,
    /// Exactly one media type, `application/json`: decoded into the mapped
    /// Rust type; `optional` when the OAS lets the body be empty.
    Json { ty: String, optional: bool },
    /// Several media types, or a non-JSON one: the body as received.
    Raw,
}

/// How the client types the body of `resp` for `op`.
///
/// A `201` of an operation taking `Prefer` is `Option` ("If the `Prefer` header
/// is missing or set to `return=minimal`, the body is empty"); every `4xx`
/// carries the error body as received.
fn client_body(op: &Operation, resp: &Response, ctx: &Ctx) -> ClientBody {
    if resp.status.is_client_error() {
        return ClientBody::Error;
    }
    match resp.media.as_slice() {
        [] => ClientBody::None,
        [(mt, schema)] if mt == "application/json" => {
            let has_prefer = op.parameters.iter().any(|p| p.name == "Prefer");
            let optional = resp.status == http::StatusCode::CREATED && has_prefer;
            ClientBody::Json {
                ty: ctx.rust_type(schema),
                optional,
            }
        }
        _ => ClientBody::Raw,
    }
}

fn outcome_name(op: &Operation) -> String {
    format!("{}Outcome", type_id(&op.operation_id))
}

fn headers_name(op: &Operation, variant: &str) -> String {
    format!("{}{variant}Headers", type_id(&op.operation_id))
}

/// The outcome enum of one operation; the headers struct of a response that
/// declares headers is emitted at group level (`emit_response_headers`).
fn emit_outcome(b: &mut String, op: &Operation, ctx: &Ctx) {
    let outcome = outcome_name(op);
    let responses = client_responses(op);
    let _ = write!(
        b,
        "    /// The outcome of `{} {}`: one variant per status the OAS documents.\n    \
         /// A status outside this set is a [`crate::rest::client::ClientError`].\n    \
         #[derive(Debug, Clone)]\n    \
         pub enum {outcome} {{\n",
        op.method.to_uppercase(),
        op.path
    );
    for resp in &responses {
        let (variant, _) = status_variant(resp.status);
        let _ = writeln!(b, "        /// The `{}` answer.", resp.status.as_u16());
        // Each field with its own doc line (`missing_docs` reaches variant
        // fields too).
        let mut fields: Vec<(String, String)> = Vec::new();
        match client_body(op, resp, ctx) {
            ClientBody::None => {}
            ClientBody::Error => fields.push((
                "The error body as received, decoded as the ITS-REST `Error` when it is one."
                    .to_string(),
                "body: crate::rest::client::ErrorBody".to_string(),
            )),
            ClientBody::Json { ty, optional } => {
                fields.push(if optional {
                    (
                        "The body, decoded from canonical JSON; `None` when the service sent none."
                            .to_string(),
                        format!("body: Option<{ty}>"),
                    )
                } else {
                    (
                        "The body, decoded from canonical JSON.".to_string(),
                        format!("body: {ty}"),
                    )
                });
            }
            ClientBody::Raw => fields.push((
                "The body as received; the request `Accept` selected its form and the \
                 `Content-Type` response header names it."
                    .to_string(),
                "body: Vec<u8>".to_string(),
            )),
        }
        if !resp.headers.is_empty() {
            fields.push((
                "The response headers the OAS declares for this answer.".to_string(),
                format!("headers: {}", headers_name(op, &variant)),
            ));
        }
        if fields.is_empty() {
            let _ = writeln!(b, "        {variant},");
        } else {
            let _ = writeln!(b, "        {variant} {{");
            for (doc, field) in &fields {
                let _ = writeln!(b, "            /// {doc}\n            {field},");
            }
            b.push_str("        },\n");
        }
    }
    b.push_str("    }\n\n");
}

/// The request-path expression for `op`, each `{name}` percent-encoded and the
/// RFC 6570 query expansion dropped.
fn path_format(op: &Operation) -> String {
    let mut template = String::new();
    let mut args: Vec<String> = Vec::new();
    let mut chunks = op.path.split('{');
    template.push_str(chunks.next().unwrap_or_default());
    for chunk in chunks {
        if let Some((name, tail)) = chunk.split_once('}') {
            if !name.starts_with('?') {
                template.push_str("{}");
                args.push(format!(
                    "crate::rest::client::path_segment(&params.{})",
                    field_id(name)
                ));
            }
            template.push_str(tail);
        } else {
            template.push('{');
            template.push_str(chunk);
        }
    }
    if args.is_empty() {
        format!("String::from(\"{template}\")")
    } else {
        format!("format!(\"{template}\", {})", args.join(", "))
    }
}

/// One client method: build the request from the params and body, execute
/// it, and match the answer's status against the documented set.
fn emit_client_method(b: &mut String, op: &Operation, ctx: &Ctx) {
    let method = field_id(&op.operation_id);
    let outcome = outcome_name(op);
    let mut args = String::from("&self");
    if !op.parameters.is_empty() {
        let _ = write!(args, ", params: &{}", param_struct_name(op));
    }
    let json_request = json_request(op);
    let mut body_ty = None;
    if let Some((schema, required)) = &op.request_body {
        let ty = if json_request {
            format!("&{}", ctx.rust_type(schema))
        } else {
            "&str".to_string()
        };
        if *required {
            let _ = write!(args, ", body: {ty}");
        } else {
            let _ = write!(args, ", body: Option<{ty}>");
        }
        body_ty = Some(*required);
    }
    let _ = write!(
        b,
        "        /// `{} {}`\n        \
         ///\n        \
         /// # Errors\n        \
         /// A status the OAS does not document for this operation, a refused\n        \
         /// credential, a service failure, an undecodable body, or a request\n        \
         /// that could not be sent — see [`crate::rest::client::ClientError`].\n        \
         pub async fn {method}({args}) -> Result<{outcome}, crate::rest::client::ClientError> {{\n            \
         let mut request = crate::rest::client::Request::new(http::Method::{}, {});\n",
        op.method.to_uppercase(),
        op.path,
        op.method.to_uppercase(),
        path_format(op)
    );
    // Parameters, in declaration order: query → query string, header → headers.
    for p in &op.parameters {
        let ident = field_id(&p.name);
        let ty = ctx.param_rust_type(op, p);
        match p.location.as_str() {
            "query" => emit_query_param(b, &p.name, &ident, &ty, p.required),
            "header" => emit_header_param(b, &p.name, &ident, &ty, p.required),
            _ => {}
        }
    }
    if let Some(required) = body_ty {
        emit_client_body(b, op, required, json_request);
    }
    let _ = write!(
        b,
        "            request.apply_options(&self.options);\n            \
         let answer = self.client.execute(request).await?;\n            \
         match answer.status() {{\n"
    );
    for resp in &client_responses(op) {
        emit_client_arm(b, op, resp, ctx, &outcome);
    }
    b.push_str(
        "                _ => Err(answer.into_undocumented()),\n            \
         }\n        \
         }\n\n",
    );
}

/// The request body onto the request: JSON through the typed body, any other
/// media as text, and an optional body only when the caller passed one.
fn emit_client_body(b: &mut String, op: &Operation, required: bool, json_request: bool) {
    let content_type_expr = if op.parameters.iter().any(|p| p.name == "Content-Type") {
        "params.content_type.as_deref()".to_string()
    } else {
        "None".to_string()
    };
    let send = if json_request {
        format!("request.json_body(body, {content_type_expr})?;")
    } else {
        let default_media = op
            .request_media
            .first()
            .map_or("application/octet-stream", String::as_str);
        format!("request.text_body(body, {content_type_expr}.unwrap_or(\"{default_media}\"))?;")
    };
    if required {
        let _ = writeln!(b, "            {send}");
    } else {
        let _ = writeln!(
            b,
            "            if let Some(body) = body {{\n                {send}\n            }}"
        );
    }
}

/// One match arm of a client method: the documented status onto its outcome
/// variant, with the body and headers the response declares.
fn emit_client_arm(b: &mut String, op: &Operation, resp: &Response, ctx: &Ctx, outcome: &str) {
    let (variant, constant) = status_variant(resp.status);
    let mut fields: Vec<String> = Vec::new();
    match client_body(op, resp, ctx) {
        ClientBody::None => {}
        ClientBody::Error => fields.push("body: answer.error_body()".to_string()),
        ClientBody::Json { optional, .. } => fields.push(if optional {
            "body: answer.optional_json()?".to_string()
        } else {
            "body: answer.json()?".to_string()
        }),
        ClientBody::Raw => fields.push("body: answer.body().to_vec()".to_string()),
    }
    if !resp.headers.is_empty() {
        let members: Vec<String> = resp
            .headers
            .iter()
            .map(|h| {
                if LIST_HEADERS.contains(&h.as_str()) {
                    format!("{}: answer.header_all(\"{h}\")", field_id(h))
                } else {
                    format!("{}: answer.header(\"{h}\")", field_id(h))
                }
            })
            .collect();
        fields.push(format!(
            "headers: {} {{ {} }}",
            headers_name(op, &variant),
            members.join(", ")
        ));
    }
    if fields.is_empty() {
        let _ = writeln!(b, "                {constant} => Ok({outcome}::{variant}),");
    } else {
        let _ = writeln!(
            b,
            "                {constant} => Ok({outcome}::{variant} {{ {} }}),",
            fields.join(", ")
        );
    }
}

/// Emits one query parameter: a scalar as one pair, an array as one pair per
/// item, an object as one pair per member.
fn emit_query_param(b: &mut String, name: &str, ident: &str, ty: &str, required: bool) {
    let inner = ty
        .strip_prefix("Option<")
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(ty);
    let push = if inner.starts_with("Vec<") {
        format!(
            "for item in value {{\n                    request.query(\"{name}\", item);\n                }}"
        )
    } else if inner.starts_with("std::collections::BTreeMap<") || inner == "QueryParameters" {
        // A JSON string goes bare; any other value as its JSON text (a number
        // or boolean reads as itself, an object or array as JSON).
        "for (member, item) in value {\n                    \
         match item {\n                        \
         serde_json::Value::String(text) => request.query(member, text),\n                        \
         other => request.query(member, other),\n                    \
         }\n                }"
            .to_string()
    } else {
        format!("request.query(\"{name}\", value);")
    };
    if required {
        let _ = writeln!(
            b,
            "            {{\n                let value = &params.{ident};\n                {push}\n            }}"
        );
    } else {
        let _ = writeln!(
            b,
            "            if let Some(value) = params.{ident}.as_ref() {{\n                {push}\n            }}"
        );
    }
}

/// One header parameter onto the request: a scalar as one field, an array
/// (`style: simple, explode: true`) as one field per item.
fn emit_header_param(b: &mut String, name: &str, ident: &str, ty: &str, required: bool) {
    let inner = ty
        .strip_prefix("Option<")
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(ty);
    let push = if inner.starts_with("Vec<") {
        format!(
            "for item in value {{\n                    request.header(\"{name}\", &item.to_string())?;\n                }}"
        )
    } else {
        format!("request.header(\"{name}\", &value.to_string())?;")
    };
    if required {
        let _ = writeln!(
            b,
            "            {{\n                let value = &params.{ident};\n                {push}\n            }}"
        );
    } else {
        let _ = writeln!(
            b,
            "            if let Some(value) = params.{ident}.as_ref() {{\n                {push}\n            }}"
        );
    }
}
