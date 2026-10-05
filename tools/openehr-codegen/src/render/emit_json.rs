// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! JSON emitter: emits `impl serde::Serialize` + `impl serde::Deserialize` for
//! the generated spec types (BASE / RM / AM / TERM / LANG), into each spec
//! crate's own `src/json_serde.rs`, plus the `_type`-keyed structural dispatch
//! in `openehr-its`.
//!
//! Every fact comes from the BMM ([`Model::json_types`]); there is no XSD
//! input. The canonical-JSON wire shape is `_type` first, then BMM declaration
//! order, with absent rather than null or empty optional attributes
//! (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
//! §JSON Format).
//!
//! The impls are written out by hand rather than derived because the canonical
//! wire is none of serde's four enum representations
//! (<https://serde.rs/enum-representations.html>): the discriminator is a member
//! of the object whose presence is context-dependent, it routes deep descendants
//! onto an intermediate variant, and the closed key set must be enforced beside
//! it (`deny_unknown_fields` is incompatible with `flatten`). Each class gets
//! the long form from <https://serde.rs/deserialize-struct.html>.
//!
//! They land in the crate that DEFINES each type, because both serde and the
//! spec types are foreign to `openehr-its` (orphan rule, E0117); that also lets
//! them construct through `pub(crate)` fields and the validated constructors
//! ([`construction`]).
//!
//! The read side is STRICT over the generated RM model at our pin: an undeclared
//! key, a repeated key and an absent mandatory attribute are all refusals (see
//! [`emit_struct_deserialize`] for the released grounding). Out-of-order
//! members, a missing `_type` on a concrete slot, `Option`/`Vec` defaulting and
//! the `Interval` literal defaults are all accepted.

use crate::analyze::Model;
use crate::load::bmm::BmmSchema;
use crate::plan::construction;
use crate::plan::{JsonEnumDispatch, JsonField, JsonFieldKind, JsonType};
use std::fmt::Write as _;

/// One spec GENERATION paired with the crate+generation root its types are
/// named from.
#[derive(Clone, Copy)]
pub(crate) struct JsonSchema<'a> {
    pub model: &'a Model,
    pub schema: &'a BmmSchema,
    /// The generation root every type of this schema is named under — crate
    /// ident plus generation module, e.g. `openehr_rm::v1_2`. Types are named
    /// by full defining-module path (never a prelude), so every generation is
    /// codec-complete, twins of a class name two generations declare included:
    /// they are distinct Rust types, so their impls do not conflict.
    ///
    /// NOTE (adjudicated): such twins share a canonical `_type` string (the
    /// BMM class name is the same in both generations), and that is not an
    /// ambiguity: `Deserialize` is always invoked at a statically known Rust
    /// type, and `_type`-keyed dispatch only ever chooses among the variants of
    /// ONE enum — never across generations. No openEHR spec governs a BMM-model
    /// canonical-JSON wire at all (this codec is our own extension), so there is
    /// no cross-generation wire contract to break.
    pub root: &'a str,
    /// The generation's cross-crate index — resolves a `@SPEC` construction
    /// door parameter to the PAIRED dependency generation's defining module.
    pub external: &'a crate::analyze::External,
}

impl JsonSchema<'_> {
    /// The full defining-module path a spec class's type is named by, as seen
    /// from OUTSIDE the defining crate (the structural dispatch in
    /// `openehr-its`).
    fn path_of(&self, spec: &str) -> String {
        format!(
            "{}::{}",
            self.root,
            crate::render::emit::type_module_path(self.schema, spec)
        )
    }

    /// The same path as seen from INSIDE the defining crate: the leading crate
    /// segment becomes `crate` (`openehr_am::v1_4::…` → `crate::v1_4::…`),
    /// which is how the emitted impls — which live in the defining crate —
    /// name their own types.
    fn local_path_of(&self, spec: &str) -> String {
        let absolute = self.path_of(spec);
        absolute
            .split_once("::")
            .map_or_else(|| "crate".to_owned(), |(_, rest)| format!("crate::{rest}"))
    }

    /// Resolve one construction-door parameter type: a `@SPEC` marker becomes
    /// the generation-correct full type path (this generation's own module
    /// when it declares the class, else the paired dependency generation's);
    /// any other string is a literal Rust type and passes through.
    ///
    /// # Panics
    /// Panics when a marker names a class neither this generation nor its
    /// paired dependencies emit — a construction-table bug.
    fn door_param(&self, ty: &str) -> String {
        let Some(spec) = ty.strip_prefix('@') else {
            return ty.to_owned();
        };
        let ident = crate::render::naming::type_name(spec);
        if self.schema.classes.contains_key(spec) {
            return format!("{}::{ident}", self.local_path_of(spec));
        }
        let module = self.external.module_of(spec).unwrap_or_else(|| {
            panic!(
                "construction door parameter @{spec}: neither the declaring generation nor its                  paired dependency generations emit this class (a plan::construction table bug)"
            )
        });
        format!("{module}::{ident}")
    }
}

/// The environment one type's deserialize impl is emitted in: the type's own
/// module path, the shared runtime path, and the generation whose model and
/// pairings resolve construction-door parameters.
struct DeserializeEnv<'a> {
    /// The emitted type's defining-module path, crate-local form.
    path: &'a str,
    /// The shared `serde_support` runtime path.
    support: &'a str,
    /// The generation being emitted (resolves `@SPEC` door parameters).
    schema: &'a JsonSchema<'a>,
}

/// The shared hand-written runtime path, as named from inside the crate being
/// emitted (`openehr-base` cannot refer to itself by crate name).
fn support_path(krate: &str) -> &'static str {
    if krate == "openehr_base" {
        "crate::serde_support"
    } else {
        "::openehr_base::serde_support"
    }
}

/// Emit one spec crate's whole `json_serde.rs`: an `impl Serialize` + an
/// `impl Deserialize` per instantiable type, in schema then class order.
///
/// `krate` is the crate's Rust ident (`openehr_rm`), used only to decide how the
/// shared runtime is named.
pub(crate) fn emit_file(schemas: &[JsonSchema<'_>], krate: &str) -> String {
    let support = support_path(krate);
    let mut b = String::new();
    // `unused_qualifications`: the emitted impls name every item by its full
    // path so the emitter never has to reason about import scope — inherent to
    // text generation, not a defect in the output.
    // The whole set is ONE file-level `#![allow(…, reason = "…")]`: `reason` is
    // mandatory (`clippy::allow_attributes_without_reason` is deny workspace-
    // wide), an inner attribute is exempt from `clippy::allow_attributes`, and
    // `expect` is wrong for a blanket list because no single emission triggers
    // every listed lint.
    b.push_str(
        "// @generated by openehr-codegen (emit-json) — DO NOT EDIT.\n\
         //! Canonical-JSON `serde::Serialize`/`serde::Deserialize` impls for this\n\
         //! crate's generated spec types.\n\
         //!\n\
         //! Manual (never derived) long-form impls per\n\
         //! <https://serde.rs/deserialize-struct.html>: the canonical wire's\n\
         //! `_type` discriminator, its context-dependent presence, deep-descendant\n\
         //! dispatch and closed key set are not expressible with serde attributes.\n\
         //! The shared runtime is `openehr_base::serde_support`.\n\n\
         #![allow(\n    \
         clippy::all,\n    \
         clippy::pedantic,\n    \
         clippy::nursery,\n    \
         unused_variables,\n    \
         unused_mut,\n    \
         unused_qualifications,\n    \
         reason = \"mechanically generated codec text: every item is named by its \
         full path and every branch shape is emitted uniformly, so style and \
         unused-binding lints do not apply — the hand-written runtime carries the \
         lint bar\"\n\
         )]\n\n",
    );
    for s in schemas {
        for ty in s.model.json_types(s.schema) {
            let path = s.local_path_of(json_type_spec(&ty));
            emit_serialize(&mut b, &ty, &path);
            emit_deserialize(
                &mut b,
                &ty,
                &DeserializeEnv {
                    path: &path,
                    support,
                    schema: s,
                },
            );
        }
    }
    b
}

// ── Structural dispatch (`structural_check`) ─────────────────────────────────

/// The bound-fill type argument a generic class's structural decode is
/// monomorphized with.
///
/// A generic spec class (`DV_INTERVAL<T>`, `HISTORY<T>`, `ORIGINAL_VERSION<T>`,
/// `POINT_EVENT<T>`, …) has no single wire type argument: the canonical-JSON
/// `_type` names only the container class, and the element type is whatever the
/// declaring attribute said. The untyped fill decodes each element as an opaque
/// value, so the container's OWN shape (its mandatory attributes and their JSON
/// kinds) is checked while the elements are accepted verbatim — exactly the
/// monomorphization the hand-written typed dispatch uses for the same classes.
const GENERIC_FILL: &str = "::serde_json::Value";

/// Why a non-struct JSON type is unreachable as a `_type` dispatch key.
///
/// A newtype or literal-enum shape writes its BARE payload (no `_type` member
/// — see `emit_serialize`), and an untagged enum carries no `_type` of its own
/// either (the active variant's payload supplies it, and every such payload is
/// itself an emitted struct with its own arm). Each is recorded in the emitted
/// header, never silently dropped.
///
/// A struct yields `None`: it IS dispatchable, and the caller handles it.
fn undispatchable(ty: &JsonType) -> Option<(String, &'static str)> {
    match ty {
        JsonType::Enum { rust, .. } => Some((
            rust.clone(),
            "untagged enum: the wire `_type` is the active variant's, which has its own arm",
        )),
        JsonType::Newtype { rust, .. } => Some((
            rust.clone(),
            "transparent newtype: serializes as its bare primitive payload, never `_type`-tagged",
        )),
        JsonType::EnumLiterals { rust, .. } => Some((
            rust.clone(),
            "BMM enumeration: a literal token/integer on the wire, never `_type`-tagged",
        )),
        JsonType::Struct { .. } => None,
    }
}

/// Emit the generated `structural_check` dispatch file: `_type` → deserialize
/// the node into that class's generated Rust type and discard the value, so the
/// codec is the single structural-conformance authority for EVERY emitted class
/// rather than only the invariant-bearing ones.
///
/// Precedence when several schemas declare the same spec class name (110 names
/// in the current vendored set — `RESOURCE_DESCRIPTION`, `AUTHORED_RESOURCE`,
/// the whole `BMM_*` family, …): the FIRST schema in `schemas` wins, so the
/// caller passes them in dispatch priority. Every shadowed twin is listed in
/// the emitted header, never dropped silently.
pub(crate) fn emit_structural_file(schemas: &[JsonSchema<'_>]) -> String {
    let StructuralInventory {
        arms,
        declared,
        shadowed,
        skipped,
    } = structural_inventory(schemas);

    let mut b = String::new();
    b.push_str(
        "// @generated by openehr-codegen (emit-json) — DO NOT EDIT.\n\
         //! Structural conformance dispatch: `_type` → the emitted `Deserialize`\n\
         //! of that spec class.\n\
         //!\n\
         //! The canonical-JSON codec is the structural-conformance authority for a\n\
         //! wire node (mandatory attributes present, JSON kinds right, nested slot\n\
         //! `_type`s resolvable). This dispatch makes that authority reachable for\n\
         //! EVERY emitted class from a `_type` string alone, so a per-node validation\n\
         //! walk is not limited to the classes that happen to carry a class\n\
         //! invariant. The decoded value is discarded — only the `Result` matters.\n\
         //!\n\
         //! Coverage: one arm per class the generator emits as a STRUCT (every\n\
         //! concrete class, plus the few abstract classes emitted as structs because\n\
         //! their concrete descendants live in another schema). Deterministic\n\
         //! exclusions, by shape:\n\
         //!\n\
         //! - an untagged **enum** (abstract or polymorphic slot) carries no `_type`\n\
         //!   of its own — the active variant's payload does, and every payload is an\n\
         //!   emitted struct with its own arm;\n\
         //! - a transparent **newtype** and a BMM **enumeration** serialize as their\n\
         //!   bare primitive payload, so no `_type` member exists to dispatch on.\n\
         //!\n\
         //! Generic classes are monomorphized with an opaque element type, so the\n\
         //! container's own shape is checked and its elements are accepted verbatim\n\
         //! (each element is a `_type` node in its own right).\n\
         //!\n\
         //! NOTE: no openEHR spec governs a cross-component `_type` namespace (the\n\
         //! canonical-JSON `_type` is a class name, and several components' BMMs\n\
         //! declare the same name with different attributes) — our own design: the\n\
         //! emitter resolves a collision by schema priority, in the order the CLI\n\
         //! passes the schemas, and every shadowed twin is listed below.\n\n\
         #![allow(\n    \
         clippy::all,\n    \
         clippy::pedantic,\n    \
         clippy::nursery,\n    \
         unused_qualifications,\n    \
         reason = \"mechanically generated dispatch text: one uniform arm per emitted \
         class, every item named by its full path — length and style lints do not \
         apply, the hand-written runtime carries the lint bar\"\n\
         )]\n\n",
    );
    let _ = writeln!(b, "// Shadowed twins ({}):", shadowed.len());
    for (spec, winner, loser) in &shadowed {
        let _ = writeln!(b, "//   {spec}: {winner} wins over {loser}");
    }
    let _ = writeln!(b, "// Shapes with no `_type` key ({}):", skipped.len());
    for (rust, why) in &skipped {
        let _ = writeln!(b, "//   {rust}: {why}");
    }
    b.push('\n');
    b.push_str(
        "/// Deserialize `node` as the emitted spec class named by `ty` and discard the\n\
         /// value: `Some(Ok(()))` when the node conforms structurally,\n\
         /// `Some(Err(_))` when it does not, `None` when `ty` names no emitted\n\
         /// class.\n\
         ///\n\
         /// # Errors\n\
         /// The inner `Err` is the canonical reader's error for a node that does not\n\
         /// deserialize as `ty` (a missing mandatory attribute, an undeclared or\n\
         /// repeated key, a wrong JSON kind, an unresolvable nested slot `_type`),\n\
         /// carrying the JSON path to the offending node — the same door, and the\n\
         /// same message, as a direct `crate::json::from_canonical_value` read.\n\
         pub fn structural_check(\n    \
         ty: &str,\n    \
         node: &::serde_json::Value,\n\
         ) -> ::core::option::Option<::core::result::Result<(), crate::json::JsonParseError>> {\n    \
         match ty {\n",
    );
    for (spec, expr) in &arms {
        let _ = writeln!(
            b,
            "{spec:?} => ::core::option::Option::Some(\
             crate::json::from_canonical_value::<{expr}>(node).map(|_| ())),"
        );
    }
    b.push_str("_ => ::core::option::Option::None,\n}\n}\n");

    // The declared-key table.
    b.push_str(
        "\n/// The wire keys the spec class `ty` declares, sorted, or `None` when\n\
         /// `ty` names no emitted struct.\n\
         ///\n\
         /// Emitted from the SAME field view as the reader's undeclared-key refusal,\n\
         /// so a caller that must answer \"is this key declared?\" WITHOUT decoding —\n\
         /// the validation dispatcher's allocation-free fast path — can never\n\
         /// disagree with the reader that would decode it. `_type` is the canonical\n\
         /// discriminator, not a modelled attribute, and is never listed.\n\
         #[must_use]\n\
         pub fn declared_fields(ty: &str) -> ::core::option::Option<&'static [&'static str]> {\n    \
         match ty {\n",
    );
    for (spec, keys) in &declared {
        let list = keys
            .iter()
            .map(|k| format!("{k:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(b, "{spec:?} => ::core::option::Option::Some(&[{list}]),");
    }
    b.push_str("_ => ::core::option::Option::None,\n}\n}\n");
    b
}

/// The `_type` dispatch inventory the structural-check file is emitted from.
struct StructuralInventory {
    /// `spec class` → the expression that decodes a node as it.
    arms: std::collections::BTreeMap<String, String>,
    /// `spec class` → its sorted declared wire keys, the SAME closure the
    /// reader's undeclared-key refusal is emitted from — so the validation
    /// dispatcher can answer "is this key declared?" without a decode, and can
    /// never disagree with the reader.
    declared: std::collections::BTreeMap<String, Vec<String>>,
    /// `(spec class, the arm that won, the arm it shadowed)`.
    shadowed: Vec<(String, String, String)>,
    /// `(Rust shape, why it is unreachable as a `_type` dispatch key)`.
    skipped: Vec<(String, &'static str)>,
}

/// Collects the dispatch inventory across every schema, in dispatch priority.
///
/// The FIRST schema declaring a spec class name wins; the loser is recorded as
/// a shadowed twin, never dropped silently.
fn structural_inventory(schemas: &[JsonSchema<'_>]) -> StructuralInventory {
    let mut arms: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    let mut declared: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    let mut shadowed: Vec<(String, String, String)> = Vec::new();
    let mut skipped: Vec<(String, &'static str)> = Vec::new();
    for s in schemas {
        for ty in s.model.json_types(s.schema) {
            // A newtype / literal-enum shape writes its BARE payload (no `_type`
            // member — see `emit_serialize`), and an untagged enum carries no
            // `_type` of its own either (the active variant's payload supplies
            // it, and every such payload is itself an emitted struct with its
            // own arm). Neither is reachable as a `_type` dispatch key, so
            // neither can have one — each is recorded, never silently dropped.
            let JsonType::Struct {
                spec,
                rust,
                generics,
                fields,
            } = &ty
            else {
                skipped.extend(undispatchable(&ty));
                continue;
            };
            let mut keys: Vec<String> = fields.iter().map(|f| f.wire_name.clone()).collect();
            keys.sort();
            declared.entry(spec.clone()).or_insert(keys);

            let expr = format!("{}::{rust}{}", s.path_of(spec), generic_fill(generics));
            match arms.entry(spec.clone()) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(expr);
                }
                std::collections::btree_map::Entry::Occupied(held) => {
                    shadowed.push((spec.clone(), held.get().clone(), expr));
                }
            }
        }
    }
    StructuralInventory {
        arms,
        declared,
        shadowed,
        skipped,
    }
}

/// The `<…>` bound-fill for a generic class's dispatch expression, empty for a
/// non-generic one.
///
/// Generic classes are monomorphized with an opaque element type, so the
/// container's own shape is checked and its elements are accepted verbatim.
fn generic_fill(generics: &[String]) -> String {
    if generics.is_empty() {
        return String::new();
    }
    let args = vec![GENERIC_FILL; generics.len()].join(", ");
    format!("<{args}>")
}

/// The spec class a [`JsonType`] realizes — the key its Rust path is resolved by
/// ([`JsonSchema::path_of`]).
fn json_type_spec(ty: &JsonType) -> &str {
    match ty {
        JsonType::Struct { spec, .. }
        | JsonType::Enum { spec, .. }
        | JsonType::Newtype { spec, .. }
        | JsonType::EnumLiterals { spec, .. } => spec,
    }
}

/// `(impl<…: Bound> , <…>)` — the impl-generics header and the type-args suffix.
fn generic_header(generics: &[String], bound: &str) -> (String, String) {
    if generics.is_empty() {
        (String::new(), String::new())
    } else {
        let bounded = generics
            .iter()
            .map(|g| format!("{g}: {bound}"))
            .collect::<Vec<_>>()
            .join(", ");
        (format!("<{bounded}>"), format!("<{}>", generics.join(", ")))
    }
}

/// The `impl<'de, …>` header + type-args suffix for a `Deserialize` impl.
///
/// Generic parameters are bound by `DeserializeOwned`, not `Deserialize<'de>`:
/// every emitted spec type owns its data (no borrowed fields anywhere — see
/// <https://serde.rs/lifetimes.html>), and the untagged structural fallback has
/// to deserialize a parameter from a LOCAL buffered value whose lifetime is
/// shorter than `'de`, which only the `for<'a>` bound permits.
fn deserialize_header(generics: &[String]) -> (String, String) {
    if generics.is_empty() {
        ("<'de>".to_owned(), String::new())
    } else {
        let bounded = generics
            .iter()
            .map(|g| format!("{g}: ::serde::de::DeserializeOwned"))
            .collect::<Vec<_>>()
            .join(", ");
        (
            format!("<'de, {bounded}>"),
            format!("<{}>", generics.join(", ")),
        )
    }
}

/// `PhantomData<(T, U)>` for a visitor that must carry the impl's generic
/// parameters (an item declared inside a function body does not inherit them).
fn phantom(generics: &[String]) -> (String, String, String) {
    if generics.is_empty() {
        (String::new(), String::new(), String::new())
    } else {
        let params = generics.join(", ");
        (
            format!("<{params}>"),
            format!("(::core::marker::PhantomData<({params},)>)"),
            "(::core::marker::PhantomData)".to_owned(),
        )
    }
}

// ── Serialize side ───────────────────────────────────────────────────────────

/// Emit `impl serde::Serialize` for one [`JsonType`].
fn emit_serialize(b: &mut String, ty: &JsonType, path: &str) {
    match ty {
        JsonType::Struct {
            spec,
            rust,
            generics,
            fields,
        } => {
            let (hdr, args) = generic_header(generics, "::serde::Serialize");
            let _ = write!(
                b,
                "impl{hdr} ::serde::Serialize for {path}::{rust}{args} {{\n\
                 fn serialize<__S: ::serde::Serializer>(&self, __serializer: __S) \
                 -> ::core::result::Result<__S::Ok, __S::Error> {{\n"
            );
            // `_type` plus every unconditionally-written field.
            let fixed = 1 + fields
                .iter()
                .filter(|f| {
                    matches!(
                        f.kind,
                        JsonFieldKind::Plain | JsonFieldKind::NonEmptyContainer
                    )
                })
                .count();
            let _ = writeln!(b, "let mut __n = {fixed}usize;");
            for f in fields {
                emit_count_field(b, f);
            }
            let _ = write!(
                b,
                "let mut __st = ::serde::Serializer::serialize_struct(__serializer, \"{spec}\", __n)?;\n\
                 ::serde::ser::SerializeStruct::serialize_field(&mut __st, \"_type\", \"{spec}\")?;\n"
            );
            for f in fields {
                emit_write_field(b, f);
            }
            b.push_str("::serde::ser::SerializeStruct::end(__st)\n}\n}\n\n");
        }
        JsonType::Enum {
            rust,
            generics,
            variant_idents,
            ..
        } => {
            let (hdr, args) = generic_header(generics, "::serde::Serialize");
            let _ = write!(
                b,
                "impl{hdr} ::serde::Serialize for {path}::{rust}{args} {{\n\
                 fn serialize<__S: ::serde::Serializer>(&self, __serializer: __S) \
                 -> ::core::result::Result<__S::Ok, __S::Error> {{ match self {{\n"
            );
            for ident in variant_idents {
                let _ = writeln!(
                    b,
                    "{path}::{rust}::{ident}(__x) => ::serde::Serialize::serialize(__x, __serializer),"
                );
            }
            b.push_str("} }\n}\n\n");
        }
        JsonType::Newtype { rust, .. } => {
            let _ = write!(
                b,
                "impl ::serde::Serialize for {path}::{rust} {{\n\
                 fn serialize<__S: ::serde::Serializer>(&self, __serializer: __S) \
                 -> ::core::result::Result<__S::Ok, __S::Error> {{\n\
                 ::serde::Serialize::serialize(&self.0, __serializer)\n}}\n}}\n\n"
            );
        }
        JsonType::EnumLiterals {
            rust,
            string_backed,
            ..
        } => {
            // Byte-identical to the bare primitive it replaces: `as_str` = the
            // constant token (verbatim payload for `Other`), `value` = the
            // constant integer.
            let body = if *string_backed {
                "::serde::Serializer::serialize_str(__serializer, self.as_str())"
            } else {
                "::serde::Serializer::serialize_i32(__serializer, self.value())"
            };
            let _ = write!(
                b,
                "impl ::serde::Serialize for {path}::{rust} {{\n\
                 fn serialize<__S: ::serde::Serializer>(&self, __serializer: __S) \
                 -> ::core::result::Result<__S::Ok, __S::Error> {{ {body} }}\n}}\n\n"
            );
        }
    }
}

/// Emit the member-count contribution of one conditionally-written field.
///
/// `serialize_struct` takes the number of members that will actually be written,
/// so a field the omission rules drop must not be counted. The unconditional
/// fields are folded into the initial constant by the caller.
fn emit_count_field(b: &mut String, f: &JsonField) {
    let rust = &f.rust_name;
    match f.kind {
        JsonFieldKind::Plain | JsonFieldKind::NonEmptyContainer => {}
        JsonFieldKind::Optional => {
            let _ = writeln!(b, "if self.{rust}.is_some() {{ __n += 1; }}");
        }
        JsonFieldKind::Container => {
            let _ = writeln!(b, "if !self.{rust}.is_empty() {{ __n += 1; }}");
        }
        JsonFieldKind::OptionalContainer => {
            let _ = writeln!(
                b,
                "if self.{rust}.as_ref().is_some_and(|__v| !__v.is_empty()) {{ __n += 1; }}"
            );
        }
    }
}

/// Emit the write call for one struct field, per its omission kind (`None` /
/// empty-list attributes are absent, never `null` and never `[]`).
fn emit_write_field(b: &mut String, f: &JsonField) {
    let wire = &f.wire_name;
    let rust = &f.rust_name;
    match f.kind {
        // `Plain` and `NonEmptyContainer` share a body deliberately: a `1..*`
        // container is non-empty by construction, so like a mandatory scalar it
        // is always written — there is no omit-when-empty branch to emit.
        JsonFieldKind::Plain | JsonFieldKind::NonEmptyContainer => {
            let _ = writeln!(
                b,
                "::serde::ser::SerializeStruct::serialize_field(&mut __st, \"{wire}\", &self.{rust})?;"
            );
        }
        JsonFieldKind::Optional => {
            let _ = writeln!(
                b,
                "if let Some(__v) = &self.{rust} {{ ::serde::ser::SerializeStruct::serialize_field(&mut __st, \"{wire}\", __v)?; }}"
            );
        }
        JsonFieldKind::Container => {
            let _ = writeln!(
                b,
                "if !self.{rust}.is_empty() {{ ::serde::ser::SerializeStruct::serialize_field(&mut __st, \"{wire}\", &self.{rust})?; }}"
            );
        }
        // An optional container omits BOTH states an empty array could carry, the
        // released rule being about emptiness rather than optionality:
        // `…/overview/Resources.md` §JSON Format — attributes "that are `Null` or
        // an empty list (array) SHOULD be absent when serialized as JSON".
        // Present-but-empty therefore lives in the typed model, never on the wire.
        JsonFieldKind::OptionalContainer => {
            let _ = writeln!(
                b,
                "if let Some(__v) = &self.{rust} && !__v.is_empty() {{ ::serde::ser::SerializeStruct::serialize_field(&mut __st, \"{wire}\", __v)?; }}"
            );
        }
    }
}

// ── Deserialize side ─────────────────────────────────────────────────────────

/// Emit `impl serde::Deserialize` for one [`JsonType`].
fn emit_deserialize(b: &mut String, ty: &JsonType, env: &DeserializeEnv<'_>) {
    let path = env.path;
    let support = env.support;
    match ty {
        JsonType::Struct {
            spec,
            rust,
            generics,
            fields,
        } => emit_struct_deserialize(b, spec, rust, generics, fields, env),
        JsonType::Enum {
            spec: _,
            rust,
            generics,
            variant_idents,
            dispatch,
        } => emit_enum_deserialize(b, rust, generics, variant_idents, dispatch, path, support),
        JsonType::Newtype { rust, .. } => {
            let _ = write!(
                b,
                "impl<'de> ::serde::Deserialize<'de> for {path}::{rust} {{\n\
                 fn deserialize<__D: ::serde::Deserializer<'de>>(__deserializer: __D) \
                 -> ::core::result::Result<Self, __D::Error> {{\n\
                 ::core::result::Result::Ok({path}::{rust}(::serde::Deserialize::deserialize(__deserializer)?))\n\
                 }}\n}}\n\n"
            );
        }
        JsonType::EnumLiterals {
            rust,
            string_backed,
            ..
        } => {
            let body = if *string_backed {
                format!(
                    "struct __V;\n\
                     impl<'de> ::serde::de::Visitor<'de> for __V {{\n\
                     type Value = {path}::{rust};\n\
                     fn expecting(&self, __f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {{\n\
                     __f.write_str(\"a `{rust}` token\")\n}}\n\
                     fn visit_str<__E: ::serde::de::Error>(self, __v: &str) \
                     -> ::core::result::Result<Self::Value, __E> {{\n\
                     ::core::result::Result::Ok({path}::{rust}::from_wire(__v))\n}}\n}}\n\
                     ::serde::Deserializer::deserialize_str(__deserializer, __V)"
                )
            } else {
                format!(
                    "let __v: i32 = ::serde::Deserialize::deserialize(__deserializer)?;\n\
                     ::core::result::Result::Ok({path}::{rust}::from_value(__v))"
                )
            };
            let _ = write!(
                b,
                "impl<'de> ::serde::Deserialize<'de> for {path}::{rust} {{\n\
                 fn deserialize<__D: ::serde::Deserializer<'de>>(__deserializer: __D) \
                 -> ::core::result::Result<Self, __D::Error> {{\n{body}\n}}\n}}\n\n"
            );
        }
    }
}

/// The `Option<…>` a field's map slot is read into, and the expression that
/// turns the slot into the final field value.
///
/// Every slot is an `Option`, so a REPEATED key is detected uniformly
/// (`de::Error::duplicate_field`) — stricter than the retired reader, which let
/// the last occurrence win. RFC 8259 §4 leaves a repeated name undefined and no
/// canonical writer emits one, so refusing is the never-lax reading.
struct FieldSlot {
    /// The Rust type the map slot holds.
    slot_ty: String,
    /// How the slot's value is read from the map (`::serde::de::MapAccess::next_value`
    /// at the slot type).
    read: &'static str,
}

/// The slot type + finalizer for one field, per its omission kind.
///
/// A `null` member is read as absence wherever the retired reader did so (a
/// mandatory field's `null` is a missing field, an optional field's `null` is
/// `None`, a defaulted flag's `null` is the default), by reading the slot as
/// `Option<T>`; a container's `null` stays an error, because `null` is not an
/// array and no conformant producer writes one
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
/// §JSON Format).
fn field_slot(f: &JsonField, elem: &str) -> FieldSlot {
    match f.kind {
        JsonFieldKind::Plain | JsonFieldKind::NonEmptyContainer | JsonFieldKind::Optional => {
            FieldSlot {
                slot_ty: format!("::core::option::Option<::core::option::Option<{elem}>>"),
                read: "next_value",
            }
        }
        JsonFieldKind::Container | JsonFieldKind::OptionalContainer => FieldSlot {
            slot_ty: format!("::core::option::Option<{elem}>"),
            read: "next_value",
        },
    }
}

/// Emits `impl serde::Deserialize` for a struct: the long form from
/// <https://serde.rs/deserialize-struct.html> — a field-identifier enum whose
/// default arm refuses the key, plus a `visit_map` visitor.
///
/// The reader is STRICT — an undeclared key is a refusal — grounded on
/// `docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md` L75
/// (the wildcard-free ITS-XML schemas cannot validate an undeclared element) and
/// L87 ("SHOULD" for JSON), plus the published ITS-JSON schemas closing 128 of
/// their 134 object definitions with `additionalProperties: false`.
///
/// The closure is over the generated RM model at our pin, never over the
/// vendored ITS-JSON 1.1.0 schema, which is stale in both directions. NOTE:
/// refusing at a SHOULD anchor is our decision — it is the only reading under
/// which the JSON and XML encodings share one data model.
fn emit_struct_deserialize(
    b: &mut String,
    spec: &str,
    rust: &str,
    generics: &[String],
    fields: &[JsonField],
    env: &DeserializeEnv<'_>,
) {
    let path = env.path;
    let support = env.support;
    let schema = env.schema;
    let (hdr, args) = deserialize_header(generics);
    let (vis_args, vis_body, vis_ctor) = phantom(generics);
    let mut known: Vec<&str> = fields.iter().map(|f| f.wire_name.as_str()).collect();
    known.sort_unstable();
    let known_list = known
        .iter()
        .map(|k| format!("{k:?}"))
        .collect::<Vec<_>>()
        .join(", ");

    let _ = write!(
        b,
        "impl{hdr} ::serde::Deserialize<'de> for {path}::{rust}{args} {{\n\
         fn deserialize<__D: ::serde::Deserializer<'de>>(__deserializer: __D) \
         -> ::core::result::Result<Self, __D::Error> {{\n\
         const __FIELDS: &[&str] = &[{known_list}];\n"
    );

    // The field-identifier enum: the closed key set plus the discriminator.
    b.push_str("enum __Field { __Type, ");
    for (i, _) in fields.iter().enumerate() {
        let _ = write!(b, "__F{i}, ");
    }
    b.push_str("}\n");
    b.push_str(
        "impl<'de> ::serde::Deserialize<'de> for __Field {\n\
         fn deserialize<__D: ::serde::Deserializer<'de>>(__deserializer: __D) \
         -> ::core::result::Result<Self, __D::Error> {\n\
         struct __KeyVisitor;\n\
         impl<'de> ::serde::de::Visitor<'de> for __KeyVisitor {\n\
         type Value = __Field;\n\
         fn expecting(&self, __f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {\n\
         __f.write_str(\"an object member name\")\n}\n\
         fn visit_str<__E: ::serde::de::Error>(self, __v: &str) \
         -> ::core::result::Result<__Field, __E> {\n\
         match __v {\n\
         \"_type\" => ::core::result::Result::Ok(__Field::__Type),\n",
    );
    for (i, f) in fields.iter().enumerate() {
        let _ = writeln!(
            b,
            "{:?} => ::core::result::Result::Ok(__Field::__F{i}),",
            f.wire_name
        );
    }
    let _ = write!(
        b,
        "_ => ::core::result::Result::Err({support}::unknown_field(__v, {spec:?}, __FIELDS)),\n\
         }}\n}}\n}}\n\
         ::serde::Deserializer::deserialize_identifier(__deserializer, __KeyVisitor)\n\
         }}\n}}\n"
    );

    // The visitor.
    let _ = write!(
        b,
        "struct __Visitor{vis_args}{vis_body};\n\
         impl{hdr} ::serde::de::Visitor<'de> for __Visitor{args} {{\n\
         type Value = {path}::{rust}{args};\n\
         fn expecting(&self, __f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {{\n\
         __f.write_str(\"an openEHR `{spec}` object\")\n}}\n\
         fn visit_map<__A: ::serde::de::MapAccess<'de>>(self, mut __map: __A) \
         -> ::core::result::Result<Self::Value, __A::Error> {{\n\
         let mut __seen_type = false;\n"
    );
    let slots: Vec<FieldSlot> = fields.iter().map(|f| field_slot(f, "_")).collect();
    for (i, slot) in slots.iter().enumerate() {
        let _ = writeln!(b, "let mut __s{i}: {} = None;", slot.slot_ty);
    }
    b.push_str("while let Some(__key) = ::serde::de::MapAccess::next_key::<__Field>(&mut __map)? {\n match __key {\n");
    let _ = write!(
        b,
        "__Field::__Type => {{\n\
         if __seen_type {{ return ::core::result::Result::Err(::serde::de::Error::duplicate_field(\"_type\")); }}\n\
         __seen_type = true;\n\
         ::serde::de::MapAccess::next_value_seed(&mut __map, {support}::ExpectedType({spec:?}))?;\n\
         }}\n"
    );
    for (i, (f, slot)) in fields.iter().zip(&slots).enumerate() {
        let _ = write!(
            b,
            "__Field::__F{i} => {{\n\
             if __s{i}.is_some() {{ return ::core::result::Result::Err(::serde::de::Error::duplicate_field({:?})); }}\n\
             __s{i} = Some(::serde::de::MapAccess::{}(&mut __map)?);\n\
             }}\n",
            f.wire_name, slot.read
        );
    }
    b.push_str("}\n}\n");

    // Build the value — through the validating constructor when the class has
    // one (`plan::construction`), so a malformed identifier refuses at PARSE,
    // path-named, in every document position.
    let door = construction::validated_ctor(spec);
    let finals: Vec<String> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| field_final(f, i))
        .collect();
    if let Some((params, fallible)) = door {
        assert_eq!(
            params.len(),
            finals.len(),
            "construction map declares {} constructor parameter(s) for {spec}, but the \
             canonical-JSON field view has {} field(s)",
            params.len(),
            finals.len(),
        );
        // Bind BY FIELD NAME, call in the table's declared order: the door
        // signature is one canonical contract across generations, while each
        // generation's BMM may declare the fields in a different order.
        let mut locals = String::new();
        let mut names = Vec::new();
        for (i, (param, ty)) in params.iter().enumerate() {
            let value = fields
                .iter()
                .position(|f| f.rust_name == *param)
                .and_then(|j| finals.get(j))
                .unwrap_or_else(|| {
                    panic!(
                        "construction map parameter {param:?} of {spec} names no emitted field \
                         (fields: {:?})",
                        fields.iter().map(|f| &f.rust_name).collect::<Vec<_>>()
                    )
                });
            let ty = schema.door_param(ty);
            let _ = writeln!(locals, "let __a{i}: {ty} = {value};");
            names.push(format!("__a{i}"));
        }
        let tail = if fallible {
            format!(
                "__built.map_err(|__e| ::serde::de::Error::custom(::std::format!(\"{spec}: {{__e}}\")))"
            )
        } else {
            "::core::result::Result::Ok(__built)".to_owned()
        };
        let _ = write!(
            b,
            "{locals}let __built = {path}::{rust}::new({});\n{tail}\n",
            names.join(", ")
        );
    } else {
        let _ = writeln!(b, "::core::result::Result::Ok({path}::{rust} {{");
        for (f, value) in fields.iter().zip(&finals) {
            let _ = writeln!(b, "{}: {value},", f.rust_name);
        }
        b.push_str("})\n");
    }
    b.push_str("}\n}\n");
    let _ = write!(
        b,
        "::serde::Deserializer::deserialize_struct(__deserializer, {spec:?}, __FIELDS, __Visitor{vis_ctor})\n\
         }}\n}}\n\n"
    );
}

/// The expression that turns field `i`'s map slot into the final field value.
fn field_final(f: &JsonField, i: usize) -> String {
    let wire = &f.wire_name;
    match (&f.kind, &f.default) {
        (JsonFieldKind::Plain, Some(default)) => {
            format!("__s{i}.flatten().unwrap_or({default})")
        }
        (JsonFieldKind::Plain, None) | (JsonFieldKind::NonEmptyContainer, _) => {
            format!("__s{i}.flatten().ok_or_else(|| ::serde::de::Error::missing_field({wire:?}))?")
        }
        (JsonFieldKind::Optional, _) => format!("__s{i}.flatten()"),
        (JsonFieldKind::Container, _) => format!("__s{i}.unwrap_or_default()"),
        (JsonFieldKind::OptionalContainer, _) => format!("__s{i}"),
    }
}

/// Emit `impl serde::Deserialize` for a closed-subtype-set / polymorphic enum:
/// `_type`-keyed dispatch with the two-path reader (see
/// `openehr_base::serde_support`), or the structural untagged fallback.
fn emit_enum_deserialize(
    b: &mut String,
    rust: &str,
    generics: &[String],
    variant_idents: &[String],
    dispatch: &JsonEnumDispatch,
    path: &str,
    support: &str,
) {
    let (hdr, args) = deserialize_header(generics);
    let (vis_args, vis_body, vis_ctor) = phantom(generics);
    let _ = write!(
        b,
        "impl{hdr} ::serde::Deserialize<'de> for {path}::{rust}{args} {{\n\
         fn deserialize<__D: ::serde::Deserializer<'de>>(__deserializer: __D) \
         -> ::core::result::Result<Self, __D::Error> {{\n"
    );
    match dispatch {
        JsonEnumDispatch::ByType {
            arms,
            self_ident,
            spec_name,
            expected,
        } => {
            let tags = arms
                .iter()
                .map(|(s, _)| format!("{s:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = write!(
                b,
                "const __TAGS: &[&str] = &[{tags}];\n\
                 struct __Visitor{vis_args}{vis_body};\n\
                 impl{hdr} ::serde::de::Visitor<'de> for __Visitor{args} {{\n\
                 type Value = {path}::{rust}{args};\n\
                 fn expecting(&self, __f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {{\n\
                 __f.write_str(\"an openEHR `{spec_name}` object\")\n}}\n\
                 fn visit_map<__A: ::serde::de::MapAccess<'de>>(self, mut __map: __A) \
                 -> ::core::result::Result<Self::Value, __A::Error> {{\n\
                 let (__tag, __buffered) = {support}::read_slot_tag(&mut __map, __TAGS)?;\n\
                 match __tag {{\n\
                 Some({support}::TagMatch::Known(__t)) => {{\n\
                 let __rest = {support}::TaggedRest::new(Some(__t), __buffered, __map);\n\
                 match __t {{\n"
            );
            for (spec, ident) in arms {
                let _ = writeln!(
                    b,
                    "{spec:?} => ::core::result::Result::Ok({path}::{rust}::{ident}(::serde::Deserialize::deserialize(__rest)?)),"
                );
            }
            let _ = write!(
                b,
                "__other => ::core::result::Result::Err({support}::unexpected_type(\
                 {spec_name:?}, __other, {expected:?})),\n\
                 }}\n}}\n\
                 Some({support}::TagMatch::Unknown(__other)) => ::core::result::Result::Err(\
                 {support}::unexpected_type({spec_name:?}, &__other, {expected:?})),\n"
            );
            if let Some(ident) = self_ident {
                let _ = write!(
                    b,
                    "None => {{\n\
                     let __rest = {support}::TaggedRest::new(None, __buffered, __map);\n\
                     ::core::result::Result::Ok({path}::{rust}::{ident}(::serde::Deserialize::deserialize(__rest)?))\n\
                     }}\n"
                );
            } else {
                let _ = writeln!(
                    b,
                    "None => ::core::result::Result::Err({support}::missing_type({spec_name:?}, {expected:?})),"
                );
            }
            let _ = write!(
                b,
                "}}\n}}\n}}\n\
                 ::serde::Deserializer::deserialize_map(__deserializer, __Visitor{vis_ctor})\n"
            );
        }
        JsonEnumDispatch::Structural { variant_idents: sv } => {
            // Structural untagged fallback: the targets carry no `_type`, so the
            // value is buffered once and each variant is tried in declaration
            // order, first success wins — serde's own untagged representation
            // does exactly this (<https://serde.rs/enum-representations.html>),
            // and it is the only shape that can backtrack.
            let _ = variant_idents; // Serialize-order idents; `sv` is the same list
            b.push_str(
                "let __value = <::serde_json::Value as ::serde::Deserialize>::deserialize(__deserializer)?;\n",
            );
            for ident in sv {
                let _ = writeln!(
                    b,
                    "if let ::core::result::Result::Ok(__v) = ::serde::Deserialize::deserialize(&__value).map({path}::{rust}::{ident}) {{ return ::core::result::Result::Ok(__v); }}"
                );
            }
            let _ = writeln!(
                b,
                "::core::result::Result::Err(::serde::de::Error::custom(\"no variant of `{rust}` matched the JSON value\"))"
            );
        }
    }
    b.push_str("}\n}\n\n");
}
