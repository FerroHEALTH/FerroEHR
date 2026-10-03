// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! REST emitter: OAS → the Rust *contract* for one API group, into
//! `openehr-its/src/rest/generated/`.
//!
//! Spec-first: the vendored `-codegen` OAS is the source of truth. For each API
//! group this emits the transport DTOs (the non-RM component schemas), a param
//! struct per operation, a headers struct per documented response that
//! declares headers, a route table `(method, path, operationId)` with the
//! index-aligned table of each operation's declared parameters (every param
//! struct's own `PARAMS`, emitted from the same parameter list as its fields),
//! and the two halves of the same contract. The SERVER half (`mod server`) carries an
//! `#[async_trait]` trait (one typed method per operation, answering a
//! success-answer enum), and an axum `router` that binds every route to its
//! trait method. The CLIENT half (`mod client`) carries one method per
//! operation over a `rest::client::Client`, answering an outcome enum with one
//! variant per status the OAS documents. RM payload schemas resolve to the
//! generated `openehr_rm`/`openehr_base` crates rather than being re-emitted.
//! Both halves come from one OAS read, so they cannot drift from each other.

#![expect(
    clippy::disallowed_types,
    reason = "dev tooling over JSON artifacts (vendored BMM/OAS bundles, emitter reports) — not the \
              application (#1694)"
)]
use crate::load::oas::{Oas, Operation, Param, Response};
use crate::plan::overrides::{
    oas_monomorphization, rest_default_member, rest_docs_text_header, rest_docs_text_headers,
};
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

/// A proper `snake_case` Rust field ident for an OAS property/param name, then
/// keyword-escaped. Handles camelCase (`minOp` → `min_op`), acronym runs
/// (`SNOMED-CT` → `snomed_ct`), and separators (`Content-Type` → `content_type`,
/// `view:pass_through` → `view_pass_through`). A leading `_` (metadata keys like
/// `_type`) is preserved. Pair with a `#[serde(rename)]` to keep the wire name.
fn field_id(raw: &str) -> String {
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

/// The Rust type name for an OAS DTO schema key. Keys are authored in
/// `PascalCase` (`ResultSet`, `OperationalTemplateV2`) and kept verbatim; an
/// all-uppercase acronym (`AQL`, `SNOMEDCT`) is title-cased (`Aql`, `Snomedct`)
/// so it is idiomatic Rust.
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
/// An OAS property that is not in the schema's `required` list and does not set
/// `nullable: true` admits its declared type and nothing else — `null` is not a
/// member of `type: string` (or of a `$ref` to a string alias) under OpenAPI
/// 3.0 (<https://spec.openapis.org/oas/v3.0.3#schema-object>: "nullable …
/// Default value is false", and the Schema Object is a JSON Schema subset). No
/// component schema in the vendored ITS-REST bundles
/// (`crates/openehr-its/vendor/rest-oas/`) sets `nullable`, so an absent
/// optional property is **omitted** on the wire, never serialized as `null`.
const SKIP_NONE_ATTR: &str = "    #[serde(skip_serializing_if = \"Option::is_none\")]\n";

/// The generated field name for an OAS `additionalProperties` extension map.
const ADDITIONAL_PROPERTIES_FIELD: &str = "additional_properties";

/// Emit the flattened extension map for a DTO whose OAS schema leaves the
/// object open, or nothing when it declares `additionalProperties: false`.
///
/// An absent keyword is an open object: OpenAPI 3.0.3 §Schema Object says
/// "Consistent with JSON Schema, `additionalProperties` defaults to `true`",
/// and every vendored bundle is `openapi: 3.0.3`. So an absent keyword and
/// `additionalProperties: true` both carry arbitrary JSON values, and an
/// `additionalProperties: <schema>` form carries that schema's Rust type. A
/// `BTreeMap` keeps the emitted order deterministic, and `#[serde(flatten)]`
/// puts the entries at the object's own level — which is what "additional
/// properties" means in JSON Schema — while collecting every undeclared key on
/// the way in. An empty map serializes to nothing, so a DTO with no extensions
/// is byte-identical to one emitted before this field existed.
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

/// Component schemas the OAS declares GENERIC through a discriminator-typed
/// field, per the SM's own class definition — the one live case is
/// `UPDATE_VERSION<T>` (SM `update_version.adoc`: "An object representing an
/// update to an existing `VERSION` … The back-end will construct a full
/// `VERSION<T>` object"), whose OAS rendering flattens `T` into the per-group
/// `data: Versionable` ref. The hoisted shared module emits the struct with
/// the real generic parameter; each group aliases it at its own `Versionable`.
const GENERIC_OVER: &[(&str, &str)] = &[("UpdateVersion", "data")];

/// The `$ref` names a schema reaches, skipping a genericized field's subtree.
fn ref_names_of(name: &str, schema: &Value, out: &mut BTreeSet<String>) {
    let skip = GENERIC_OVER
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, f)| *f);
    walk_refs(schema, skip, out);
}

/// Walks a schema value collecting every `$ref` target name.
///
/// `skip_field` is dropped only at the `properties` level, so the marker is
/// passed exactly one level below `properties` and nowhere else.
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

/// The cross-group HOIST set: component schemas that appear in more than one
/// group bundle with byte-identical definitions (key-order-independent), are
/// not RM/BASE-resolved, and whose transitive `$ref` closure (minus a
/// genericized field) stays inside {RM, BASE, the hoist set} — so the shared
/// module is self-contained. Everything else keeps per-group emission (a
/// schema like `Versionable` is textually shared but semantically per-group:
/// its discriminator mapping differs).
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

/// Emit the shared `common` module: the cross-group hoisted DTOs (byte-identical
/// component schemas whose ref closure is self-contained — [`hoist_set`]), each
/// emitted exactly once. The one genericized schema ([`GENERIC_OVER`]) emits
/// with its real SM generic parameter; the groups alias it at their own type
/// argument.
#[must_use]
pub(crate) fn emit_common(oas: &Oas, names: &RmNames, hoisted: &BTreeSet<String>) -> String {
    let ctx = Ctx {
        oas,
        names,
        dtos: hoisted,
        hoisted,
        in_common: true,
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
    let ctx = Ctx {
        oas,
        names,
        dtos: &dtos,
        hoisted,
        in_common: false,
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
            // Hoisted into `common`; a GENERIC-over schema gets a group-local
            // alias binding the group's own type argument (the flattened
            // `data` ref — e.g. this group's `Versionable`), so group refs
            // keep using the bare name.
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

    // ── per-operation param structs ──
    let mut ops = oas.operations();
    for op in &mut ops {
        append_docs_text_headers(op);
    }
    for op in &ops {
        emit_params_struct(&mut b, op, &ctx);
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
        let media: Vec<String> = op.request_media.iter().map(|m| format!("{m:?}")).collect();
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
    /// Whether we are emitting the shared `common` module itself (hoisted
    /// names render bare) or a group module (hoisted names render
    /// `super::common::…`, except a [`GENERIC_OVER`] name, which the group
    /// aliases locally).
    in_common: bool,
}

impl Ctx<'_> {
    /// Map a `$ref` schema to its Rust type.
    ///
    /// A schema whose KEY does not match its class — the released bundles
    /// rename `CLUSTER` to `Clstr` and give every generic INSTANTIATION its own
    /// flat key — is resolved through the monomorphization map read from each
    /// schema's own `title` (see `plan::overrides::OAS_MONOMORPHIZATIONS`);
    /// without it these emit as `allOf`-truncated DTOs that drop their
    /// inherited members and the spec type's strict reader.
    ///
    /// RM/BASE spec types resolve to the TYPED spec structs: since the
    /// foundation rewrite every spec type carries emitted manual
    /// `serde::Serialize`/`Deserialize` impls (its crate's `json_serde.rs`),
    /// and those impls ARE the strict canonical-JSON reader — so a typed field
    /// is strict by construction where an untyped `Value` silently accepted
    /// anything. A GENERIC-over hoisted schema has a group-local alias
    /// (bare name); every other hoisted schema lives in `super::common`. A ref
    /// to something emitted nowhere is resolved and mapped structurally.
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

    /// Map an OAS schema to a Rust type. RM `$ref`s resolve to the spec crate
    /// preludes; local DTO refs to the bare name; unknown/complex shapes degrade
    /// to `serde_json::Value` (the same honest fallback the BMM emitter uses).
    fn rust_type(&self, schema: &Value) -> String {
        if let Some(name) = Oas::ref_name(schema) {
            return self.ref_type(&name, schema);
        }
        // `allOf` COMPOSITION. A schema whose only structural content is a
        // single-`$ref` `allOf` is a pure alias for its referent — OAS 3.0
        // composes independently-validated definitions
        // (<https://spec.openapis.org/oas/v3.0.3#composition-and-inheritance-polymorphism>),
        // so an empty own-contribution leaves exactly the referent's shape. The
        // released OAS uses it to give one `ITEM_TAG` schema a per-resource name.
        // A schema that ALSO declares `properties` is a genuine extension and
        // reaches the object branch instead.
        if schema.get("properties").is_none()
            && let Some(members) = schema.get("allOf").and_then(Value::as_array)
        {
            match members.as_slice() {
                [only] => return self.rust_type(only),
                // A multi-member `allOf` composes several schemas into one
                // object, which needs a MERGED struct this emitter does not
                // build. The released OAS contains none (every `allOf` in the
                // vendored bundles has exactly one member), so this arm is
                // unreachable today and carries the payload untyped rather
                // than picking one member and silently losing the others.
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
                            // The item schema goes through `rust_type` VERBATIM
                            // (never pre-resolved): a `$ref` item must keep its
                            // name so `items: {$ref: ResultSetColumn}` emits
                            // `Vec<ResultSetColumn>`. Resolving first discards
                            // the name and the item degrades to an untyped
                            // `serde_json::Value`. The `$ref` arm above still
                            // resolves structurally when the name is not one
                            // this emitter binds.
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

    /// The `allOf`-flattened object shape of a component schema: the members it
    /// accepts (ancestors first, then its own, in document order) and the union
    /// of every `required` list on the chain.
    ///
    /// OAS 3.0 models inheritance as `allOf` composition — a derived schema is
    /// "validated against all the schemas" it composes plus its own definition,
    /// and `discriminator` "can be used to aid in serialization,
    /// deserialization, and validation" of exactly that construct
    /// (<https://spec.openapis.org/oas/v3.0.3#composition-and-inheritance-polymorphism>).
    /// The released ITS-REST bundles use it for the whole RM subtype chain and
    /// for `UpdateAttestation` extending `UpdateAudit`, so a DTO emitted from
    /// its OWN `properties` alone silently loses every inherited member (an
    /// `UPDATE_ATTESTATION` without `change_type`/`committer`).
    ///
    /// A member redeclared by a descendant (the `_type` enum narrowing) keeps
    /// the ancestor's position and takes the descendant's schema, so the
    /// emitted field order stays deterministic.
    fn merged_object(&self, schema: &Value) -> (Vec<(String, Value)>, BTreeSet<String>) {
        let mut props: Vec<(String, Value)> = Vec::new();
        let mut required: BTreeSet<String> = BTreeSet::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        self.collect_object(schema, &mut props, &mut required, &mut seen);
        (props, required)
    }

    /// Accumulate one link of the [`Ctx::merged_object`] chain: ancestors first
    /// (depth-first through `allOf`), then this schema's own contribution.
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

    /// Map an OAS **parameter** schema to a Rust type.
    ///
    /// A parameter is transported as TEXT, not as JSON: its wire form is fixed
    /// by the parameter's `style`/`explode` serialization rules
    /// (<https://spec.openapis.org/oas/v3.0.3#style-values>) — e.g. the
    /// `openehr-item-tag` header is `style: simple, explode: true`, whose
    /// values read `key="flag",value="follow-up"`, not JSON objects. So an
    /// array parameter whose items are a structured schema carries the raw
    /// parameter values (one `String` per occurrence) and the handler decodes
    /// the style-encoded content; only primitive items keep their mapped type,
    /// which is exactly what a text value can be coerced to.
    fn param_rust_type(&self, schema: &Value) -> String {
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
    // A `discriminator.mapping` schema is OAS polymorphism over the canonical
    // `_type` (every released mapping uses `propertyName: _type`): it emits a
    // real enum over the mapping's targets, dispatched by the same strict
    // tag-anywhere machinery the spec crates' own emitted impls use — never an
    // untyped alias (`Versionable` was `pub type … =
    // serde_json::Value`).
    if let Some(mapping) = schema
        .get("discriminator")
        .and_then(|d| d.get("mapping"))
        .and_then(Value::as_object)
    {
        // A base carrying `x-discriminator-value` is INSTANTIABLE in its own
        // right — the extension names the `_type` an instance of the base itself
        // sends — so its members survive: it emits as a `…Data` struct and joins
        // the enum as one more variant. A base WITHOUT it is abstract and stays
        // a pure union over its mapping.
        let base_tag = schema
            .get("x-discriminator-value")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let (props, required) = ctx.merged_object(schema);
        let base = match (base_tag, props.is_empty()) {
            (Some(tag), false) => {
                let data_ty = format!("{}Data", dto_type(name));
                emit_struct(b, name, &data_ty, None, &props, &required, schema, ctx);
                Some((tag, data_ty))
            }
            _ => None,
        };
        emit_discriminator_enum(b, name, mapping, base.as_ref(), ctx);
        return;
    }
    // object with named properties → struct; everything else → an alias.
    if schema.get("type").and_then(Value::as_str) == Some("object")
        && schema.get("properties").is_some()
    {
        let ty_name = dto_type(name);
        let (props, required) = ctx.merged_object(schema);
        // The SM-generic hoisted schema ([`GENERIC_OVER`]) emits with its real
        // type parameter in the shared module; the flattened field types as
        // `T` and each group binds it via a local alias.
        let generic_field = if ctx.in_common {
            GENERIC_OVER
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, f)| *f)
        } else {
            None
        };
        emit_struct(
            b,
            name,
            &ty_name,
            generic_field,
            &props,
            &required,
            schema,
            ctx,
        );
    } else {
        // string/array/map/ref alias.
        let _ = writeln!(
            b,
            "/// The `{name}` ITS-REST OAS component schema (a non-object shape, so\n\
             /// it is an alias rather than a struct).\n\
             pub type {} = {};\n",
            dto_type(name),
            ctx.rust_type(schema)
        );
    }
}

/// Emit one transport-DTO struct: `props`/`required` are the `allOf`-flattened
/// shape ([`Ctx::merged_object`]), `generic_field` names the property carried as
/// the type parameter `T` (the [`GENERIC_OVER`] envelope), and `schema` is the
/// declaring schema, read for its `additionalProperties` policy.
#[expect(
    clippy::too_many_arguments,
    reason = "one emission site each for the schema's identity, its Rust name, the generic binding, the flattened shape and the declaring schema — bundling them into a struct would only rename the same arguments"
)]
fn emit_struct(
    b: &mut String,
    name: &str,
    ty_name: &str,
    generic_field: Option<&str>,
    props: &[(String, Value)],
    all_required: &BTreeSet<String>,
    schema: &Value,
    ctx: &Ctx,
) {
    // The vendored OAS `required` list, minus the docs-text-wins
    // corrections (`plan::overrides::REST_OPTIONAL_OVERRIDES` — the
    // ITS-REST docs text wins every conflict with the released OAS; where
    // it contradicts the OAS shape, the field is emitted optional).
    let required: BTreeSet<&str> = all_required
        .iter()
        .map(String::as_str)
        .filter(|f| crate::plan::overrides::rest_optional_override(ty_name, f).is_none())
        .collect();
    // A schema declaring `additionalProperties: false` is CLOSED by the released
    // OAS, so the DTO must refuse an undeclared member rather than accept what
    // the specification's own computable artifact rejects. serde's
    // `deny_unknown_fields` is the exact realization, and it is mutually
    // exclusive with the flatten extension map by construction.
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
    let generics = if generic_field.is_some() { "<T>" } else { "" };
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
            StructField {
                owner: name,
                ty_name,
                pname,
                pschema,
                is_generic: generic_field == Some(pname.as_str()),
                is_required: required.contains(pname.as_str()),
            },
            ctx,
        );
    }
    emit_additional_properties(b, name, schema, ctx);
    b.push_str("}\n\n");
}

/// One DTO field's emission inputs.
#[derive(Clone, Copy)]
struct StructField<'a> {
    /// The OAS component schema name the field belongs to.
    owner: &'a str,
    /// The Rust type name of the emitted DTO.
    ty_name: &'a str,
    /// The OAS property name.
    pname: &'a str,
    /// The OAS property schema.
    pschema: &'a Value,
    /// Whether the field carries the DTO's generic parameter.
    is_generic: bool,
    /// Whether the field is required after the docs-text-wins corrections.
    is_required: bool,
}

/// Emits one DTO field: its doc line, serde attributes and typed declaration.
fn emit_struct_field(b: &mut String, f: StructField<'_>, ctx: &Ctx) {
    let ident = field_id(f.pname);
    let mut ty = if f.is_generic {
        "T".to_string()
    } else {
        ctx.rust_type(f.pschema)
    };
    if !f.is_required {
        ty = format!("Option<{ty}>");
    }
    // A struct field is a public item `missing_docs` checks; the OAS property
    // name is the honest, deterministic summary.
    let _ = writeln!(b, "    /// The `{}` property of `{}`.", f.pname, f.owner);
    // A docs-text-wins correction carries its citation into the generated code
    // (the OAS lists the field as required; the ITS-REST docs text wins).
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
    if let Some(rename) = naming::serde_rename(f.pname, &ident) {
        let _ = writeln!(b, "    #[serde(rename = \"{rename}\")]");
    }
    if !f.is_required {
        b.push_str(SKIP_NONE_ATTR);
    }
    if default_member.is_some() {
        b.push_str("    #[serde(default)]\n");
    }
    let _ = writeln!(b, "    pub {ident}: {ty},");
}

/// Emit an OAS `discriminator.mapping` schema as a `_type`-dispatched enum.
///
/// One variant per mapping entry, in document order; each carries the mapped
/// target's Rust type (an RM/BASE spec type or a local DTO, through
/// [`Ctx::rust_type`]). Serialization delegates to the inner value (whose
/// emitted impl writes its own `_type`); deserialization dispatches on the
/// mapping keys with the shared strict tag-anywhere runtime
/// (`openehr_base::serde_support`) — an unknown `_type` is refused naming the
/// legal set, exactly like the spec crates' own closed-set enums.
///
/// `base` is `Some((tag, data_type))` when the schema is INSTANTIABLE ITSELF
/// (it carries `x-discriminator-value`): the base joins the union as a final
/// variant over its own `…Data` struct, and — because the OAS leaves `_type`
/// out of such a base's `required` list and gives it a `default` — an object
/// with NO discriminator reads as that base variant instead of being refused.
/// For a purely abstract base (no `x-discriminator-value`) a missing `_type` is
/// still an error: nothing could be constructed from it.
fn emit_discriminator_enum(
    b: &mut String,
    name: &str,
    mapping: &serde_json::Map<String, Value>,
    base: Option<&(String, String)>,
    ctx: &Ctx,
) {
    let ty_name = dto_type(name);
    let mut variants: Vec<(String, String, String)> = mapping
        .iter()
        .filter_map(|(tag, target)| {
            let target = target.as_str()?;
            let ref_name = target.rsplit('/').next()?.to_string();
            let rust_ty = ctx.rust_type(&serde_json::json!({ "$ref": target }));
            Some((tag.clone(), ref_name, rust_ty))
        })
        .collect();
    if let Some((tag, data_ty)) = base {
        variants.push((tag.clone(), ty_name.clone(), data_ty.clone()));
    }
    let variants = variants;
    let tag_list = variants
        .iter()
        .map(|(t, _, _)| t.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let _ = write!(
        b,
        "/// The `{name}` ITS-REST OAS component schema: `_type`-discriminated\n\
         /// polymorphism over its OAS `discriminator.mapping` targets.\n\
         #[derive(Debug, Clone)]\npub enum {ty_name} {{\n"
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
            .map(|(t, _, _)| format!("\"{t}\""))
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
    // An object with no `_type` at all: the concrete base's own form when the
    // schema declares one, otherwise a refusal (an abstract slot cannot pick a
    // variant without its discriminator).
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
        let mut ty = ctx.param_rust_type(&p.schema);
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

/// The `from_request` constructor of `op`'s param struct, the public form of
/// the decoding the generated handler runs, and `from_parts`, the decoding
/// itself, which both share.
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

/// The `Option<crate::rest::routes::IdentifierClass>` expression of `p`: the
/// class [`crate::plan::overrides::REST_PATH_IDENTIFIERS`] records for a path
/// parameter's component, `None` for any other parameter.
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

/// Whether `p` explodes: its declared `explode`, else the OAS 3.0.3 default
/// (§Parameter Object, `explode`) — `true` under the `form` style, which is the
/// default style of a query or cookie parameter.
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

/// Appends to `op` the request headers the ITS-REST docs text defines for it
/// beyond the OAS parameters (`plan::overrides::REST_DOCS_TEXT_HEADERS`), each
/// as an optional header parameter after the declared ones.
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
        emit_response_headers_impl(b, &headers_name(op, &variant), &resp.headers);
    }
}

/// The `ResponseHeaders` impl of one headers struct: a `Some` value is one
/// field, every item of a list header one field line.
fn emit_response_headers_impl(b: &mut String, name: &str, headers: &[String]) {
    let _ = write!(
        b,
        "impl crate::rest::runtime::ResponseHeaders for {name} {{\n    \
         fn into_header_map(\n        self,\n    \
         ) -> Result<http::HeaderMap, crate::rest::runtime::HeaderError> {{\n        \
         let mut map = http::HeaderMap::new();\n"
    );
    for h in headers {
        let call = if LIST_HEADERS.contains(&h.as_str()) {
            "append_headers"
        } else {
            "set_header"
        };
        let _ = writeln!(
            b,
            "        crate::rest::runtime::{call}(&mut map, \"{h}\", self.{})?;",
            field_id(h)
        );
    }
    b.push_str("        Ok(map)\n    }\n}\n\n");
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
/// An RFC 6570 query expansion (`{?name*}`) is dropped, and every `{name}`
/// segment becomes `{p<segment index>}`: axum's matcher refuses two routes
/// that name different captures at one position (`/ehr/{ehr_id}/ehr_status/
/// {version_uid}` beside `…/{uid_based_id}/tags`), and a positional name is
/// the same for every route of the group.
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

/// The server half of one API group: an inline `server` module carrying, per
/// operation, the success-answer enum, the trait method, and the axum handler
/// that decodes a request into the method's arguments and encodes its answer;
/// plus the `router` binding every route to its handler.
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

/// The success-answer enum of one operation: one variant per documented `2xx`
/// or `3xx` status, carrying the body as the client types it and the headers
/// struct when the OAS declares headers.
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

/// The trait method of one operation: the params struct, the body (canonical
/// JSON decoded into its type, any other media as text), and the
/// success-answer enum. The default body answers `NotImplemented`, so an
/// implementor overrides only the operations it supports.
fn emit_trait_method(b: &mut String, op: &Operation, ctx: &Ctx) {
    let method = field_id(&op.operation_id);
    let mut args = String::from("&self");
    if !op.parameters.is_empty() {
        let _ = write!(args, ", params: {}", param_struct_name(op));
    }
    if let Some((schema, required)) = &op.request_body {
        let ty = if json_request(op) {
            ctx.rust_type(schema)
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
    let shape = param_shape(&ctx.param_rust_type(&p.schema));
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

/// The axum handler of one operation: decode the path captures, the query
/// pairs, the headers and the body into the trait method's arguments, call
/// it, and encode its answer.
fn emit_server_handler(b: &mut String, op: &Operation, trait_name: &str, ctx: &Ctx) {
    let method = field_id(&op.operation_id);
    let response = server_response_name(op);
    let (_, captures) = axum_path(op);
    let has = |location: &str| op.parameters.iter().any(|p| p.location == location);
    let json = json_request(op);
    let mut extractors = String::from(
        "        axum::extract::State(api): axum::extract::State<std::sync::Arc<S>>,\n",
    );
    if has("header") || (op.request_body.is_some() && json) {
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
    if let Some((_, required)) = &op.request_body {
        let decode = match (json, *required) {
            (true, true) => "crate::rest::server::json_body(&headers, &body)?",
            (true, false) => "crate::rest::server::json_body_optional(&headers, &body)?",
            (false, true) => "crate::rest::server::text_body(&body)?",
            (false, false) => "crate::rest::server::text_body_optional(&body)?",
        };
        let _ = writeln!(b, "            let body = {decode};");
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

/// The client half of one API group: an inline `client` module carrying, per
/// operation, the outcome enum (one variant per documented status), a headers
/// struct per response that declares headers, and one method on the group's
/// client type over the hand-written `crate::rest::client::Client`.
///
/// Every request is built from the operation's OAS declaration: path
/// parameters substitute their `{name}` template (percent-encoded), query
/// parameters go on the query string (`style: form`, one pair per array
/// item, one pair per member of an object), header parameters become request
/// headers (one field per array item), and the body is canonical JSON when
/// the request content declares `application/json`, raw text otherwise. The
/// response is matched on its status against the documented set; a status the
/// OAS does not document is a typed error, never an outcome.
fn emit_client_module(out: &mut String, group: &str, ops: &[Operation], ctx: &Ctx) {
    let client_ty = format!("{}Client", naming::type_name(group));
    // Rendered into its own buffer: the module sits one level below the group
    // module, so a hoisted `super::common::…` reference the shared type mapper
    // produces needs one more `super` here.
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

/// The `PascalCase` variant name and the `http::StatusCode` constant path for
/// one documented status, both derived from the `http` crate's registry
/// (`canonical_reason`). A status the registry does not name emits a name
/// and a constant that do not exist, so the generated crate fails to compile
/// naming the status rather than the emitter skipping a documented outcome.
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

/// The responses the client matches for `op`: the OAS set, plus the docs-text
/// correction the ITS-REST overview makes to every create operation.
///
/// NOTE: `ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §Prefer header: under `return=minimal` "If no response body is returned,
/// the service SHOULD use `204 No Content`" — the docs text wins over the OAS,
/// which lists `201` alone for the create operations.
fn client_responses(op: &Operation) -> Vec<Response> {
    let has_prefer = op.parameters.iter().any(|p| p.name == "Prefer");
    let mut out: Vec<Response> = op
        .responses
        .iter()
        .map(|r| Response {
            status: r.status,
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
            media: Vec::new(),
            headers,
        });
        out.sort_by_key(|r| r.status);
    }
    out
}

/// The response headers the docs text defines as LISTS, so a repeated field
/// carries every value rather than the first.
///
/// NOTE: `ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §openehr-item-tag and openehr-version-item-tag: "The list of all `ITEM_TAG`"
/// (the OAS types the header `string`, one field line per tag).
const LIST_HEADERS: &[&str] = &["openehr-item-tag", "openehr-version-item-tag"];

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
    /// Several media types, or a non-JSON one: the body as received — the
    /// request `Accept` selected its form and the response `Content-Type`
    /// header names it.
    Raw,
}

/// How the client types the body of `resp` for `op`.
///
/// A `2xx` JSON body is present by the OAS unless the operation takes a
/// `Prefer` parameter and answers `201` (ITS-REST: "If the `Prefer` header is
/// missing or set to `return=minimal`, the body is empty" — every `201_*`
/// response component), so that one is `Option`. Every `4xx` answer carries
/// the error body as received, whether or not the OAS attaches a schema to
/// that status: "The response body MAY contain error details" (the `400`
/// response component), and the only schema the OAS ever attaches to a `4xx`
/// is `Error`.
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

/// The request-path expression for `op`: its OAS path with every `{name}`
/// substituted by the percent-encoded path parameter, and an RFC 6570 query
/// expansion (`{?name*}`) dropped — the query parameters carry it.
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
        let ty = ctx.param_rust_type(&p.schema);
        match p.location.as_str() {
            "query" => emit_query_param(b, &p.name, &ident, &ty, p.required),
            "header" => emit_header_param(b, &p.name, &ident, &ty, p.required),
            _ => {}
        }
    }
    let content_type_expr = if op.parameters.iter().any(|p| p.name == "Content-Type") {
        "params.content_type.as_deref()".to_string()
    } else {
        "None".to_string()
    };
    if let Some(required) = body_ty {
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
    let _ = write!(
        b,
        "            request.apply_options(&self.options);\n            \
         let answer = self.client.execute(request).await?;\n            \
         match answer.status() {{\n"
    );
    for resp in &client_responses(op) {
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
    b.push_str(
        "                _ => Err(answer.into_undocumented()),\n            \
         }\n        \
         }\n\n",
    );
}

/// One query parameter onto the request: a scalar as one pair, an array as
/// one pair per item, an object (`style: form, explode: true`) as one pair per
/// member.
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
