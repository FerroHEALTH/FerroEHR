// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 4, render: walks a merged BMM [`Model`] and produces byte-stable Rust
//! for the openEHR spec crates.
//!
//! A concrete class inlines its inherited fields, ancestor first; an abstract
//! class used as a property type is an enum over its subtypes; foundation
//! primitives and containers map to Rust types
//! ([`crate::plan::overrides::PRIMITIVES`],
//! [`crate::plan::overrides::MAPPED_CLASSES`]). Every public item carries the
//! BMM `documentation` or a synthesized line ([`synth_class_doc`],
//! [`synth_field_doc`]). Serde lives in `emit-json`.

use crate::analyze::{External, Model, class_paths};
use crate::load::bmm::{
    BmmClass, BmmConstant, BmmEnumValue, BmmEnumeration, BmmPropKind, BmmSchema, BmmType,
};
use crate::load::impls::SiblingImpls;
use crate::plan::composition::{
    CrateComposition, GenerationSpec, GenerationUnit, generation_variant,
};
use crate::plan::construction;
use crate::plan::overrides::{
    ContradictionKind, back_reference, class_binding, contradicted, type_override, untyped_field,
};
use crate::plan::{Emission, decide};
use crate::render::{doc_prose, naming};
use std::collections::{BTreeMap, BTreeSet};

/// Prepends a file-root `#![expect(clippy::disallowed_types)]` to a generated
/// body whose code names `serde_json::Value`.
///
/// A body with `#[cfg(feature = …)]` regions gets `#![allow(…)]` instead, since
/// the carrier may sit in a disabled region.
pub(crate) fn guard_value_carriers(body: &str) -> String {
    let code_carries = body.lines().any(|l| {
        let t = l.trim_start();
        !t.starts_with("//") && l.contains("serde_json::Value")
    });
    if !code_carries {
        return body.to_owned();
    }
    // After the header, the module docs and any `#![…]` blocks: a later blanket
    // `#![allow(clippy::all, …)]` would leave the expectation unfulfilled.
    let mut at = 0usize;
    let mut attr_depth = 0usize;
    for line in body.split_inclusive('\n') {
        let t = line.trim_start();
        let is_header = t.starts_with("//!") || (t.starts_with("//") && !t.starts_with("///"));
        let in_attr = attr_depth > 0 || t.starts_with("#![");
        if t.is_empty() || is_header || in_attr {
            if in_attr {
                attr_depth = (attr_depth + line.matches(['[', '(']).count())
                    .saturating_sub(line.matches([']', ')']).count());
            }
            at += line.len();
        } else {
            break;
        }
    }
    let guard = if body.contains("#[cfg(feature") {
        "#![allow(\n    clippy::disallowed_types,\n    reason = \"adjudicated free-form JSON \
         slots: serde_json::Value is workspace-banned (#1694); a generated carrier exists only \
         where the spec leaves the slot open — `allow`, not `expect`, because a carrier may sit \
         inside a feature-gated region and fire only under that feature\"\n)]\n"
    } else {
        "#![expect(\n    clippy::disallowed_types,\n    reason = \"adjudicated free-form JSON \
         slots: serde_json::Value is workspace-banned (#1694); a generated carrier exists only \
         where the spec leaves the slot open, and each adjudicated field's NOTE names its \
         citation\"\n)]\n"
    };
    // `at` accumulates whole-line lengths from `split_inclusive`, so it is a
    // char boundary by construction.
    let (head, tail) = body.split_at(at);
    format!("{head}{guard}{tail}")
}

/// A generated Rust source file (path relative to the crate `src/`, plus body).
pub(crate) struct GenFile {
    /// Relative path under the crate `src/`, e.g. `data_types/quantity/dv_quantity.rs`.
    pub path: String,
    /// The Rust source.
    pub body: String,
}

/// One emitted type and the module chain it lives in (for import + prelude).
struct Emitted {
    /// Module chain under the crate root, e.g. `["v1_3","base_types","identification","uid"]`.
    chain: Vec<String>,
    /// Rust type identifier, e.g. `Uid`.
    ident: String,
}

/// The generated files for one schema version plus its top-level module names.
pub(crate) struct Version {
    pub(crate) files: Vec<GenFile>,
    /// Top-level module names of this version (under its prefix, if any).
    top: BTreeSet<String>,
    /// The types this version emitted, for crate-prelude assembly (empty for a
    /// prefixed version, which emits its own prelude in-tree).
    emitted: Vec<Emitted>,
}

/// One BMM generation of a crate as the emitter consumes it.
///
/// Each generation emits completely under its own version-named top module, so
/// a class name two generations declare yields two Rust types.
pub(crate) struct CrateGeneration<'a> {
    /// The table row this generation emits from (module name, spec version,
    /// current marker).
    pub spec: &'static GenerationSpec,
    /// The generation's specification units, in table order.
    pub units: Vec<RenderUnit<'a>>,
    /// The full-path index resolving this generation's cross-crate references
    /// to its PAIRED dependency generations.
    pub external: &'a External,
}

/// One specification unit of a [`CrateGeneration`] as the render loop
/// consumes it.
pub(crate) struct RenderUnit<'a> {
    /// The table row this unit emits from (`in_prelude`).
    pub spec: &'static GenerationUnit,
    /// The resolution model this unit's classes resolve against.
    pub model: &'a Model,
    /// This unit's schema WITH its cross-schema re-emission closure grafted
    /// in (`augmented_schema` — a unit with an empty closure is unchanged).
    pub schema: &'a BmmSchema,
}

/// The emission shapes that produce a file: [`Emission`] minus `Skip`.
enum Shape<'a> {
    /// [`Emission::Struct`].
    Struct,
    /// [`Emission::Enum`].
    Enum(Vec<String>),
    /// [`Emission::PolyEnum`].
    PolyEnum(Vec<String>),
    /// [`Emission::EnumLiterals`].
    EnumLiterals(&'a BmmEnumeration),
    /// [`Emission::Newtype`].
    Newtype(&'a str),
}

impl<'a> Shape<'a> {
    /// The file-producing shape of an emission decision, or `None` for
    /// [`Emission::Skip`] (a mapped/primitive class emits no file).
    fn new(emission: Emission<'a>) -> Option<Self> {
        match emission {
            Emission::Struct => Some(Self::Struct),
            Emission::Enum(variants) => Some(Self::Enum(variants)),
            Emission::PolyEnum(variants) => Some(Self::PolyEnum(variants)),
            Emission::EnumLiterals(enumeration) => Some(Self::EnumLiterals(enumeration)),
            Emission::Newtype(prim) => Some(Self::Newtype(prim)),
            Emission::Skip => None,
        }
    }
}

/// Emits one specification unit under its generation's version-named top
/// module: the type files and the `mod.rs` tree.
pub(crate) fn emit_version(
    model: &Model,
    schema: &BmmSchema,
    prefix: &str,
    external: &External,
    impls: &SiblingImpls,
) -> Version {
    struct Planned<'a> {
        class: &'a BmmClass,
        shape: Shape<'a>,
        chain: Vec<String>,
    }

    // A mandatory construction cycle must be broken at a `back_reference` edge.
    model.assert_constructible(schema);

    let class_pkg = class_paths(schema);
    let used = model.used_as_type();

    // A spec referenced outside this version degrades to `serde_json::Value`; the
    // projection [`crate::analyze::emittable_specs`] computes.
    let mut local: BTreeSet<String> = BTreeSet::new();
    for (name, class) in &schema.classes {
        if !matches!(decide(model, class, &used), Emission::Skip) {
            local.insert(name.clone());
        }
    }

    let mut planned = Vec::new();
    let mut index: BTreeMap<String, Vec<String>> = BTreeMap::new(); // ident → chain
    let mut emitted: Vec<Emitted> = Vec::new();
    for (name, class) in &schema.classes {
        let Some(shape) = Shape::new(decide(model, class, &used)) else {
            continue;
        };
        let pkg = class_pkg.get(name).cloned().unwrap_or_default();
        let mut chain: Vec<String> = Vec::new();
        if !prefix.is_empty() {
            chain.extend(prefix.split("::").map(str::to_owned));
        }
        chain.extend(
            pkg.split('/')
                .filter(|s| !s.is_empty())
                .map(naming::module_ident),
        );
        chain.push(naming::field_ident(&to_snake(name)));
        index.insert(naming::type_name(name), chain.clone());
        emitted.push(Emitted {
            chain: chain.clone(),
            ident: naming::type_name(name),
        });
        // A polymorphic-concrete class's `{Name}Data` struct is exported too.
        if matches!(shape, Shape::PolyEnum(_)) {
            index.insert(format!("{}Data", naming::type_name(name)), chain.clone());
            emitted.push(Emitted {
                chain: chain.clone(),
                ident: format!("{}Data", naming::type_name(name)),
            });
        }
        planned.push(Planned {
            class,
            shape,
            chain,
        });
    }

    let mut files = Vec::new();
    for p in &planned {
        let has_sibling = impls.has_sibling(&p.chain);
        let body = match &p.shape {
            Shape::Struct => emit_struct(model, p.class, &index, &local, external, has_sibling),
            Shape::Enum(variants) => emit_enum(
                model,
                p.class,
                variants,
                false,
                &index,
                &local,
                external,
                has_sibling,
            ),
            Shape::PolyEnum(variants) => emit_enum(
                model,
                p.class,
                variants,
                true,
                &index,
                &local,
                external,
                has_sibling,
            ),
            Shape::EnumLiterals(enumeration) => {
                emit_enum_literals(p.class, enumeration, has_sibling)
            }
            Shape::Newtype(prim) => emit_newtype(p.class, prim, has_sibling),
        };
        files.push(GenFile {
            path: format!("{}.rs", p.chain.join("/")),
            body,
        });
    }

    let type_chains: Vec<Vec<String>> = planned.iter().map(|p| p.chain.clone()).collect();

    // The caller assembles the generation prelude file itself.
    let mut tree_chains = type_chains.clone();
    let prelude = format!("{prefix}::prelude");
    tree_chains.push(prelude.split("::").map(str::to_owned).collect());
    files.extend(crate::render::module_tree::emit_module_tree(&tree_chains));

    Version {
        files,
        top: prefix.split("::").take(1).map(str::to_owned).collect(),
        emitted,
    }
}

/// The module path a class's emitted type lives at within its generation
/// module, e.g. `core::bmm_class`.
#[must_use]
pub(crate) fn type_module_path(schema: &BmmSchema, class: &str) -> String {
    let pkg = class_paths(schema).get(class).cloned().unwrap_or_default();
    let mut chain: Vec<String> = pkg
        .split('/')
        .filter(|s| !s.is_empty())
        .map(naming::module_ident)
        .collect();
    chain.push(naming::field_ident(&to_snake(class)));
    chain.join("::")
}

/// Emits a fully composed crate: every generation under its version-named top
/// module, the crate prelude over the current generation, and `lib.rs` with
/// the `Generation` enum.
#[must_use]
pub(crate) fn emit_composed(
    comp: &CrateComposition,
    generations: &[CrateGeneration<'_>],
    impls: &SiblingImpls,
) -> Vec<GenFile> {
    let mut files: Vec<GenFile> = Vec::new();
    let mut top: BTreeSet<String> = BTreeSet::new();
    let mut current: Vec<Emitted> = Vec::new();
    for g in generations {
        let mut paths: BTreeSet<String> = BTreeSet::new();
        let mut gen_emitted: Vec<Emitted> = Vec::new();
        for u in &g.units {
            let v = emit_version(u.model, u.schema, g.spec.module, g.external, impls);
            for f in &v.files {
                assert!(
                    // The module tree and the generation mod.rs are shared
                    // between units; type files must be unit-unique.
                    !f.path.ends_with("mod.rs")
                        && !f.path.ends_with("prelude.rs")
                        && paths.insert(f.path.clone())
                        || f.path.ends_with("mod.rs")
                        || f.path.ends_with("prelude.rs"),
                    "openehr-codegen: two specification units of {:?} both emit {:?} — unit \
                     package paths must be disjoint so each unit lands at its own path \
                     (emitting one over the other silently picks a single shape for a \
                     colliding class).",
                    g.spec.module,
                    f.path,
                );
            }
            merge_files(&mut files, v.files);
            top.extend(v.top);
            if u.spec.in_prelude {
                gen_emitted.extend(v.emitted);
            }
        }
        files.push(emit_prelude(
            &gen_emitted,
            &format!("{}/prelude.rs", g.spec.module),
        ));
        if g.spec.current {
            current = gen_emitted;
        }
    }
    files.push(emit_prelude(&current, "prelude.rs"));
    files.push(emit_lib(&top, comp));
    files
}

/// Merge one unit's rendered files into the crate set, unioning the shared
/// `mod.rs` module declarations where both units contribute to one directory.
fn merge_files(files: &mut Vec<GenFile>, new: Vec<GenFile>) {
    for f in new {
        if let Some(existing) = files.iter_mut().find(|e| e.path == f.path) {
            // Only mod.rs trees legitimately collide (the disjoint-path
            // assert upstream guarantees it); union their `pub mod` lines.
            for line in f.body.lines() {
                if line.starts_with("pub mod ") && !existing.body.contains(line) {
                    existing.body.push_str(line);
                    existing.body.push('\n');
                }
            }
        } else {
            files.push(f);
        }
    }
}

fn emit_lib(top: &BTreeSet<String>, comp: &CrateComposition) -> GenFile {
    // The marker leads the file: every generated/hand-written scan in the
    // pipeline (and out-of-tree tooling) tests the FIRST line.
    let mut b = String::from("// @generated by openehr-codegen — DO NOT EDIT.\n");
    for line in comp.doc.lines() {
        b.push_str(&format!("//! {line}\n"));
    }
    b.push_str("//!\n//! The type files are generated; hand-written spec behaviour\n");
    b.push_str("//! lives in sibling `*_impl.rs` files.\n\n");
    // The spec owns the doc text and names, so these lints would fork the model;
    // `allow`, since a given crate need not trigger each one.
    b.push_str(
        "#![allow(\n    \
         clippy::doc_markdown,\n    \
         clippy::doc_link_with_quotes,\n    \
         clippy::tabs_in_doc_comments,\n    \
         clippy::doc_lazy_continuation,\n    \
         clippy::doc_overindented_list_items,\n    \
         clippy::struct_excessive_bools,\n    \
         clippy::struct_field_names,\n    \
         clippy::module_inception,\n    \
         clippy::large_enum_variant,\n    \
         clippy::enum_variant_names,\n    \
         reason = \"inherent to faithful openEHR spec generation: verbatim spec \
         prose in doc comments, and spec-owned class/variant/field names (a field \
         name IS the normative BMM attribute name)\"\n\
         )]\n\
         // The generation preludes ARE this crate's public surface: the crate \
         prelude re-exports the current generation, and each generation module \
         re-exports its own types. That is the design (codegen.md), and it is \
         the one place `clippy::pub_use` is satisfied by an exception rather \
         than by removal. `expect` rather than `allow`: if a crate ever stops \
         re-exporting, the exception self-reports instead of outliving its \
         reason.\n\
         #![expect(clippy::pub_use, reason = \"the generation preludes are this \
         crate's public surface\")]\n\
         // A vendored BMM model is a deep, mutually-recursive type graph (the LANG \
         // BMM-3 expression/statement families reach several hundred levels), so \
         // auto-trait inference — `Send`/`Sync`/`RefUnwindSafe`, which rustdoc \
         // evaluates for every item — overflows the default limit of 128. Raising \
         // the limit is exactly what rustc prescribes for that overflow \
         // (<https://doc.rust-lang.org/reference/attributes/limits.html>); it \
         // changes no emitted type.\n\
         #![recursion_limit = \"512\"]\n\n",
    );
    for m in top {
        b.push_str(&format!("pub mod {m};\n"));
    }
    b.push_str("pub mod prelude;\n");
    b.push_str(&crate::plan::drafts::lib_declaration(comp.crate_name));
    // No crate-level SPEC_VERSION: the `Generation` enum is the authority.
    b.push_str(&emit_generation_enum(comp));
    GenFile {
        path: "lib.rs".to_string(),
        body: b,
    }
}

/// The crate's `Generation` enum + parse-error type, generated from the
/// composition table so it can never drift from what is actually emitted.
fn emit_generation_enum(comp: &CrateComposition) -> String {
    let variants: Vec<(String, &GenerationSpec)> = comp
        .generations
        .iter()
        .map(|g| (generation_variant(g.module), g))
        .collect();
    let current_module = comp
        .generations
        .iter()
        .find(|g| g.current)
        .or_else(|| comp.generations.first())
        .map_or("", |g| g.module);
    let mut b = String::from(
        "\n/// The BMM generations this crate emits, one variant per version module,\n\
         /// oldest first.\n\
         ///\n\
         /// Generated from the openehr-codegen composition table — the single\n\
         /// authority for which generations exist. [`std::fmt::Display`] and\n\
         /// [`std::str::FromStr`] round-trip the generation-module name (`\"",
    );
    b.push_str(comp.generations.first().map_or("", |g| g.module));
    b.push_str(
        "\"`). `Generation::default()` is the crate's CURRENT generation — the\n\
         /// one `crate::prelude` re-exports (the composition table's `current`\n\
         /// marker, via the std `#[default]` variant attribute).\n\
         #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]\n\
         pub enum Generation {\n",
    );
    for (variant, g) in &variants {
        let default_attr = if g.current { "    #[default]\n" } else { "" };
        b.push_str(&format!(
            "    /// The `{}` generation — openEHR specification version {}.\n{default_attr}    {variant},\n",
            g.module, g.spec_version,
        ));
    }
    b.push_str("}\n\nimpl Generation {\n");
    b.push_str(
        "    /// Returns the openEHR specification version this generation implements.\n    \
         #[must_use]\n    pub const fn spec_version(self) -> &'static str {\n        match self {\n",
    );
    // Variants sharing one spec version fold into one or-pattern arm.
    let mut version_arms: Vec<(&str, Vec<String>)> = Vec::new();
    for (variant, g) in &variants {
        if let Some((_, vs)) = version_arms.iter_mut().find(|(v, _)| *v == g.spec_version) {
            vs.push(format!("Self::{variant}"));
        } else {
            version_arms.push((g.spec_version, vec![format!("Self::{variant}")]));
        }
    }
    for (version, patterns) in &version_arms {
        b.push_str(&format!(
            "            {} => \"{version}\",\n",
            patterns.join(" | ")
        ));
    }
    // The example token is this crate's current generation.
    b.push_str(&format!(
        "        }}\n    }}\n\n    /// Returns the generation token — the version-module name\n    \
         /// (`\"{current_module}\"`), which is also the [`std::fmt::Display`] and\n    \
         /// [`std::str::FromStr`] form.\n    \
         #[must_use]\n    pub const fn as_str(self) -> &'static str {{\n        match self {{\n",
    ));
    for (variant, g) in &variants {
        b.push_str(&format!(
            "            Self::{variant} => \"{}\",\n",
            g.module
        ));
    }
    let tokens = comp
        .generations
        .iter()
        .map(|g| g.module)
        .collect::<Vec<_>>()
        .join("`, `");
    b.push_str(&format!(
        "        }}\n    }}\n}}\n\n\
         impl std::fmt::Display for Generation {{\n    \
         fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        \
         f.write_str(self.as_str())\n    }}\n}}\n\n\
         /// Error returned when parsing a [`Generation`] from an unknown token.\n\
         ///\n\
         /// The valid tokens are the generation-module names (`{tokens}`).\n\
         #[derive(Debug, Clone, PartialEq, Eq)]\n\
         pub struct GenerationParseError {{\n    unrecognized: String,\n}}\n\n\
         impl std::fmt::Display for GenerationParseError {{\n    \
         fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n        \
         write!(\n            f,\n            \"unknown generation {{:?}} (valid: `{tokens}`)\",\n            \
         self.unrecognized\n        )\n    }}\n}}\n\n\
         impl std::error::Error for GenerationParseError {{}}\n\n\
         impl std::str::FromStr for Generation {{\n    type Err = GenerationParseError;\n\n    \
         fn from_str(s: &str) -> Result<Self, Self::Err> {{\n        match s {{\n",
    ));
    for (variant, g) in &variants {
        b.push_str(&format!(
            "            \"{}\" => Ok(Self::{variant}),\n",
            g.module
        ));
    }
    b.push_str(
        "            other => Err(GenerationParseError {\n                \
         unrecognized: other.to_owned(),\n            }),\n        }\n    }\n}\n",
    );
    b
}

fn emit_prelude(emitted: &[Emitted], path: &str) -> GenFile {
    // Emits the crate prelude and each generation's prelude; only the entries differ.
    let mut b = String::from(
        "// @generated by openehr-codegen — DO NOT EDIT.\n\
         //! Prelude: re-exports every generated spec type of ONE generation.\n\
         //!\n//! Per-file imports are precise;\n\
         //! downstream crates and hand-written code may `use <path>::*`.\n\
         //!\n\
         //! The crate-level prelude re-exports the CURRENT generation only (the\n\
         //! composition table's `current` marker); every generation module also\n\
         //! carries its own `prelude`. An older generation's types are reached\n\
         //! through its generation module (its prelude or full module paths) —\n\
         //! no cross-generation collision resolution exists anywhere.\n\n",
    );
    let mut lines: Vec<String> = emitted
        .iter()
        .map(|e| format!("pub use crate::{}::{};", e.chain.join("::"), e.ident))
        .collect();
    lines.sort();
    for l in lines {
        b.push_str(&l);
        b.push('\n');
    }
    GenFile {
        path: path.to_string(),
        body: b,
    }
}

fn emit_struct(
    model: &Model,
    class: &BmmClass,
    index: &BTreeMap<String, Vec<String>>,
    local: &BTreeSet<String>,
    external: &External,
    has_sibling: bool,
) -> String {
    let ty = naming::type_name(&class.name);
    let generics = struct_generics(model, class);
    let subst = class_binding(&class.name);

    let mut b = String::new();
    let imports = import_lines(model, class, &generics, &subst, &ty, index, external);
    struct_header(&mut b, &class.name, &imports, has_sibling);
    b.push_str(&render_struct_def(
        model, class, &ty, &generics, &subst, local, external,
    ));
    b.push_str(&render_constants(class, &ty));
    b
}

/// The params a struct is generic over (see `used_generic_params`).
pub(crate) fn struct_generics(model: &Model, class: &BmmClass) -> Vec<String> {
    model
        .used_generic_params(&class.name)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// The struct definition under the name `struct_ty`, without the file header;
/// a polymorphic-concrete class uses `{Name}Data`.
fn render_struct_def(
    model: &Model,
    class: &BmmClass,
    struct_ty: &str,
    generics: &[String],
    subst: &BTreeMap<String, String>,
    local: &BTreeSet<String>,
    external: &External,
) -> String {
    let gen_decl = generic_decl(generics);
    let mut b = String::new();
    push_struct_prologue(&mut b, class, struct_ty, &gen_decl);

    // A class behind a validating construction door (`plan::construction`) has
    // `pub(crate)` fields, read by its hand-written `*_impl.rs` constructor.
    let field_vis = if construction::is_validated(&class.name) {
        "pub(crate)"
    } else {
        "pub"
    };
    // `(ident, rust_type)` per field, in the constructor's parameter order.
    let mut emitted: Vec<(String, String)> = Vec::new();

    let props = model.flattened_props(class);
    let mut prev_owner: Option<&str> = None;
    let mut first = true;
    for rp in &props {
        let p = rp.prop;
        // A back-reference is omitted from the struct and serde.
        if let Some(citation) = back_reference(&rp.owner, &p.name) {
            push_back_reference_note(&mut b, &p.name, citation);
            continue;
        }
        if let Some(header) = inherited_group_header(rp, &class.name, prev_owner, first) {
            b.push_str(&header);
        }
        prev_owner = Some(rp.owner.as_str());
        first = false;
        doc_block_or(
            &mut b,
            p.documentation.as_deref(),
            "    ",
            &synth_field_doc(&rp.owner, &p.name),
            "",
        );

        // The wire name and default live in `emit-json`.
        let ident = naming::field_ident(&p.name);
        let rust_ty = field_type(model, class, p, generics, subst, local, external);
        // An adjudicated field that degraded to free-form JSON carries its NOTE.
        if rust_ty.contains("serde_json::Value") {
            push_untyped_field_note(&mut b, &rp.owner, &class.name, &p.name);
        }
        b.push_str(&format!("    {field_vis} {ident}: {rust_ty},\n"));
        emitted.push((ident, rust_ty));
    }

    b.push_str("}\n");
    if let Some(citation) = construction::validated_citation(&class.name) {
        b.push_str(&render_accessors(
            &class.name,
            struct_ty,
            &emitted,
            citation,
        ));
    }
    b
}

/// The `<A, B>` generic-parameter declaration for a parameter-name list, empty
/// when there are none.
fn generic_decl(params: &[String]) -> String {
    if params.is_empty() {
        String::new()
    } else {
        format!("<{}>", params.join(", "))
    }
}

/// The imports every enum variant's payload embeds, bound-filled arguments
/// included.
fn variant_imports(
    model: &Model,
    variants: &[String],
    enum_generics: &[String],
    ty: &str,
    index: &BTreeMap<String, Vec<String>>,
    external: &External,
) -> BTreeSet<String> {
    let mut imports: BTreeSet<String> = BTreeSet::new();
    for d in variants {
        let mut roots = BTreeSet::new();
        let d_generic = !model.used_generic_params(d).is_empty();
        if d_generic && !enum_generics.is_empty() {
            roots.insert(d.clone());
        } else {
            model.effective_roots(&BmmType::Simple(d.clone()), &mut roots);
        }
        for r in roots {
            add_import(&mut imports, &r, ty, index, external);
        }
    }
    imports
}

/// The imports the in-file `{Name}Data` struct needs for the class's own fields.
fn data_struct_imports(
    model: &Model,
    class: &BmmClass,
    data_generics: &[String],
    data_subst: &BTreeMap<String, String>,
    ty: &str,
    index: &BTreeMap<String, Vec<String>>,
    external: &External,
) -> Vec<String> {
    model
        .referenced_specs(class, data_generics, data_subst)
        .iter()
        .filter_map(|spec| {
            let ident = naming::type_name(spec);
            if ident == ty {
                return None;
            }
            if let Some(chain) = index.get(&ident) {
                Some(format!("use crate::{}::{};", chain.join("::"), ident))
            } else {
                external
                    .module_of(spec)
                    .map(|path| format!("use {path}::{ident};"))
            }
        })
        .collect()
}

/// Appends the struct's doc block, spec alias, construction note and the
/// `derive` + opening `pub struct` lines.
fn push_struct_prologue(b: &mut String, class: &BmmClass, struct_ty: &str, gen_decl: &str) {
    doc_block_or(
        b,
        class.documentation.as_deref(),
        "",
        &synth_class_doc(&class.name),
        &synth_class_summary(&class.name),
    );
    push_spec_alias(b, &class.name, struct_ty, "");
    // The plain-record adjudication, at the public fields it concerns.
    if let Some(note) = construction::plain_record_note(&class.name) {
        b.push_str(&format!("// NOTE: {note}\n"));
    }
    b.push_str("#[derive(Debug, Clone, PartialEq)]\n");
    b.push_str(&format!("pub struct {struct_ty}{gen_decl} {{\n"));
}

/// The `// inherited: X` group header a property opens, if it starts a new
/// inheritance group, with a blank line before it except first in the braces.
fn inherited_group_header(
    rp: &crate::analyze::ResolvedProp<'_>,
    class_name: &str,
    prev_owner: Option<&str>,
    first: bool,
) -> Option<String> {
    if rp.owner == class_name || prev_owner == Some(rp.owner.as_str()) {
        return None;
    }
    let sep = if first { "" } else { "\n" };
    Some(format!("{sep}    // inherited: {}\n", rp.owner))
}

/// The NOTE emitted in place of an omitted back-reference field.
fn push_back_reference_note(b: &mut String, prop: &str, citation: &str) {
    b.push_str(&format!(
        "    // NOTE: `{prop}` (BMM-mandatory back-reference) omitted — {citation}. \
         A back-reference is not forward-owned data and never appears on \
         the canonical wire; emitting it as an owning field would make \
         this type non-constructible.\n"
    ));
}

/// The NOTE emitted beside an adjudicated field that degraded to free-form JSON.
fn push_untyped_field_note(b: &mut String, owner: &str, class: &str, prop: &str) {
    if let Some(adj) = untyped_field(owner, prop).or_else(|| untyped_field(class, prop)) {
        b.push_str(&format!(
            "    // NOTE: free-form JSON is adjudicated here, not accidental — {}. {}\n",
            adj.citation, adj.reason
        ));
    }
}

/// The read accessors and door NOTE for a class whose fields are `pub(crate)`
/// behind a validating constructor; `String` reads back as `&str`.
fn render_accessors(
    spec_name: &str,
    struct_ty: &str,
    fields: &[(String, String)],
    citation: &str,
) -> String {
    let arity = construction::validated_ctor(spec_name).map_or(0, |(p, _)| p.len());
    assert_eq!(
        arity,
        fields.len(),
        "construction map declares arity {arity} for {spec_name}, but the BMM \
         emits {} field(s): the hand-written constructor and the generated \
         codec calls would disagree",
        fields.len()
    );
    let mut b = String::new();
    b.push_str(&format!(
        "\n/// Read access to the `pub(crate)` fields of [`{struct_ty}`].\n\
         ///\n\
         /// The fields are not `pub`: the release states a **constraint over this\n\
         /// class's own field values** (a lexical form, or a class invariant), so\n\
         /// construction checks it and is the only door \u{2014} {citation}\n\
         ///\n\
         /// The validating constructor lives in the hand-written `*_impl.rs` sibling\n\
         /// (the generator never writes into it); every generated codec builds this\n\
         /// type through that constructor.\n\
         impl {struct_ty} {{\n"
    ));
    for (i, (ident, rust_ty)) in fields.iter().enumerate() {
        if i > 0 {
            b.push('\n');
        }
        // An optional field reads back as `Option<&T>` (`clippy::ref_option`).
        let (ret, expr) = match rust_ty
            .strip_prefix("Option<")
            .and_then(|inner| inner.strip_suffix('>'))
        {
            Some("String") => (
                "Option<&str>".to_owned(),
                format!("self.{ident}.as_deref()"),
            ),
            Some(inner) => (
                format!("Option<&{inner}>"),
                format!("self.{ident}.as_ref()"),
            ),
            None if rust_ty == "String" => ("&str".to_owned(), format!("&self.{ident}")),
            None => (format!("&{rust_ty}"), format!("&self.{ident}")),
        };
        b.push_str(&format!(
            "    /// The `{ident}` this instance was constructed with, checked at the door.\n    \
             #[must_use]\n    \
             pub fn {ident}(&self) -> {ret} {{\n        {expr}\n    }}\n"
        ));
    }
    b.push_str("}\n");
    b
}

/// Emits the class's `BMM_CLASS.constants` as an `impl {ty}` block of `pub
/// const` items, each doc citing its BMM name.
fn render_constants(class: &BmmClass, ty: &str) -> String {
    if class.constants.is_empty() {
        return String::new();
    }
    let siblings: BTreeSet<&str> = class.constants.iter().map(|c| c.name.as_str()).collect();
    let mut b = String::new();
    b.push_str(&format!("\nimpl {ty} {{\n"));
    for (i, c) in class.constants.iter().enumerate() {
        if i > 0 {
            b.push('\n');
        }
        doc_prose::doc_block_summarized(&mut b, c.documentation.as_deref(), "    ", "");
        b.push_str(&format!("    /// BMM constant `{}`.\n", c.name));
        let (rust_ty, lit) = const_literal(c, &siblings);
        b.push_str(&format!(
            "    pub const {}: {rust_ty} = {lit};\n",
            naming::const_ident(&c.name)
        ));
    }
    b.push_str("}\n");
    b
}

/// Decodes a BMM constant's raw `value` to a Rust `(type, literal)` pair,
/// decoding numeric character references (`&#42;`) and Eiffel octal escapes
/// (`\015`).
fn const_literal(c: &BmmConstant, siblings: &BTreeSet<&str>) -> (String, String) {
    let is_real = matches!(c.type_name.as_str(), "Real" | "Double");
    match &c.value {
        serde_json::Value::Number(n) if is_real => (
            "f64".to_string(),
            format!("{:?}", n.as_f64().unwrap_or(0.0)),
        ),
        serde_json::Value::Number(n) => ("i64".to_string(), format!("{}", n.as_i64().unwrap_or(0))),
        serde_json::Value::Bool(b) => ("bool".to_string(), format!("{b}")),
        serde_json::Value::String(s) => {
            string_const_literal(s.trim(), &c.type_name, is_real, siblings)
        }
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            ("&str".to_string(), "\"\"".to_string())
        }
    }
}

/// Decodes a string-valued BMM constant: `"…"` is `&str`, `'…'` a `char`, a
/// sibling name `Self::OTHER`, a bareword under `Boolean` a bool, and anything
/// else a string literal.
fn string_const_literal(
    t: &str,
    type_name: &str,
    is_real: bool,
    siblings: &BTreeSet<&str>,
) -> (String, String) {
    if let Some(inner) = strip_delims(t, '"') {
        return ("&str".to_string(), format!("{:?}", decode_entities(inner)));
    }
    if let Some(inner) = strip_delims(t, '\'') {
        return ("char".to_string(), format!("{:?}", decode_char(inner)));
    }
    if siblings.contains(t) {
        let rust_ty = if is_real { "f64" } else { "i64" };
        return (
            rust_ty.to_string(),
            format!("Self::{}", naming::const_ident(t)),
        );
    }
    if type_name == "Boolean" {
        return (
            "bool".to_string(),
            format!("{}", t.eq_ignore_ascii_case("true")),
        );
    }
    ("&str".to_string(), format!("{t:?}"))
}

/// Strip a matching pair of delimiter characters (`"…"` or `'…'`) from `s`,
/// returning the inner text; `None` if `s` is not so delimited.
fn strip_delims(s: &str, delim: char) -> Option<&str> {
    s.strip_prefix(delim).and_then(|r| r.strip_suffix(delim))
}

/// Decode numeric character references (`&#42;` → `*`) in a BMM literal, without
/// byte slicing (clinical-text-safe). Non-references pass through verbatim.
fn decode_entities(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' || chars.peek() != Some(&'#') {
            out.push(c);
            continue;
        }
        chars.next();
        let num = read_reference_digits(&mut chars);
        if let Some(ch) = num.parse::<u32>().ok().and_then(char::from_u32) {
            out.push(ch);
        } else {
            out.push_str("&#");
            out.push_str(&num);
        }
    }
    out
}

/// Reads a numeric character reference's digits, consuming a closing `;`.
fn read_reference_digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut num = String::new();
    while let Some(&d) = chars.peek() {
        if d == ';' {
            chars.next();
            break;
        }
        if !d.is_ascii_digit() {
            break;
        }
        num.push(d);
        chars.next();
    }
    num
}

/// Decode a single-character BMM literal body: numeric references first, then an
/// Eiffel octal escape (`\015` → CR); falls back to the first decoded char.
fn decode_char(inner: &str) -> char {
    let decoded = decode_entities(inner);
    if let Some(rest) = decoded.strip_prefix('\\')
        && !rest.is_empty()
        && rest.bytes().all(|b| (b'0'..=b'7').contains(&b))
        && let Some(ch) = u32::from_str_radix(rest, 8).ok().and_then(char::from_u32)
    {
        return ch;
    }
    decoded.chars().next().unwrap_or('\u{0}')
}

/// Computes a field's Rust type; [`crate::render::model_query`] reports the
/// same decision.
pub(crate) fn field_type(
    model: &Model,
    class: &BmmClass,
    p: &crate::load::bmm::BmmProperty,
    generics: &[String],
    subst: &BTreeMap<String, String>,
    local: &BTreeSet<String>,
    external: &External,
) -> String {
    match &p.kind {
        BmmPropKind::Single(t) => {
            let inner = single_field_type(model, class, p, t, generics, subst, local, external);
            if p.is_mandatory {
                inner
            } else {
                format!("Option<{inner}>")
            }
        }
        BmmPropKind::Container {
            item, cardinality, ..
        } => container_field_type(
            model,
            class,
            p,
            ContainerShape { item, cardinality },
            generics,
            subst,
            local,
            external,
        ),
    }
}

/// The item type and declared cardinality of a container property.
#[derive(Clone, Copy)]
struct ContainerShape<'a> {
    item: &'a BmmType,
    cardinality: &'a Option<crate::load::bmm::BmmCardinality>,
}

/// The inner Rust type of a single-valued property, boxed where by-value would
/// make the struct infinitely sized.
///
/// Every spec name the type embeds by value is checked, so mutual and
/// F-bounded recursion are caught (`DV_QUANTITY.normal_range:
/// DvInterval<DvOrdered>`).
#[expect(
    clippy::too_many_arguments,
    reason = "the field-shape decision threads the same resolution tables `field_type` takes; bundling them would hide which each renderer reads"
)]
fn single_field_type(
    model: &Model,
    class: &BmmClass,
    p: &crate::load::bmm::BmmProperty,
    t: &BmmType,
    generics: &[String],
    subst: &BTreeMap<String, String>,
    local: &BTreeSet<String>,
    external: &External,
) -> String {
    let overridden = type_override(&class.name, &p.name);
    let inner = match overridden {
        Some(rust) => rust.to_string(),
        None => model.render_type(t, generics, subst, local, external),
    };
    let already_indirect = inner.starts_with("Vec<") || inner.starts_with("std::collections::");
    let cyclic = overridden.is_none() && !already_indirect && {
        let mut roots = BTreeSet::new();
        model.effective_roots(t, &mut roots);
        roots.iter().any(|r| {
            !Model::is_mapped(r)
                && (r == &class.name || model.reaches(r, &class.name, &mut BTreeSet::new()))
        })
    };
    if cyclic {
        return format!("Box<{inner}>");
    }
    let inner = crate::plan::ordered_hashes::retype(model, class, &p.name, inner);
    if overridden.is_none()
        && (crate::plan::layout::boxes_field(model, class, p.is_mandatory, &inner, generics)
            || crate::plan::drafts::boxes_draft_field(&class.name, &p.name))
    {
        format!("Box<{inner}>")
    } else {
        inner
    }
}

/// The Rust type of a container property.
///
/// A byte buffer (`Array<Octet>`) is base64 text on the wire, so a `String`.
/// Otherwise:
///
/// - mandatory `0..*` is `Vec<T>`;
/// - mandatory `1..*` is `NonEmptyVec<T>`, less
///   [`crate::plan::overrides::contradicted`] entries;
/// - optional is `Option<Vec<T>>`, since the RM tells Void from empty
///   (`org.openehr.rm.common.folder.adoc` §Attributes);
/// - optional under an `x /= Void implies not x.is_empty` invariant is
///   `Option<NonEmptyVec<T>>` (`org.openehr.rm.common.locatable.adoc`
///   §Invariants).
#[expect(
    clippy::too_many_arguments,
    reason = "the field-shape decision threads the same resolution tables `field_type` takes; bundling them would hide which each renderer reads"
)]
fn container_field_type(
    model: &Model,
    class: &BmmClass,
    p: &crate::load::bmm::BmmProperty,
    shape: ContainerShape<'_>,
    generics: &[String],
    subst: &BTreeMap<String, String>,
    local: &BTreeSet<String>,
    external: &External,
) -> String {
    if shape.item.root_name() == "Octet" {
        return if p.is_mandatory {
            "String".to_string()
        } else {
            "Option<String>".to_string()
        };
    }
    let item_ty = model.render_type(shape.item, generics, subst, local, external);
    let lower_bound_one = shape.cardinality.as_ref().is_some_and(|c| c.lower >= 1)
        && !contradicted(&class.name, &p.name, ContradictionKind::LowerBound);
    let nonempty_when_present = crate::analyze::nonempty_optional_lists_cached(model)
        .iter()
        .any(|(decl, attr)| {
            attr == &p.name && (decl == &class.name || model.inherits(&class.name, decl))
        });
    match (p.is_mandatory, lower_bound_one, nonempty_when_present) {
        (true, true, _) => {
            format!("{}::NonEmptyVec<{item_ty}>", external.containers_path())
        }
        (true, false, _) => format!("Vec<{item_ty}>"),
        (false, _, true) => {
            format!(
                "Option<{}::NonEmptyVec<{item_ty}>>",
                external.containers_path()
            )
        }
        (false, _, false) => format!("Option<Vec<{item_ty}>>"),
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the enum renderer needs the class, its variant set, both shape \
              flags (own-instances struct, hand-written sibling) and all three \
              per-version resolution tables (ident index, local class set, \
              external preludes); bundling the tables would hide which of them \
              each renderer actually reads"
)]
fn emit_enum(
    model: &Model,
    class: &BmmClass,
    variants: &[String],
    self_data: bool,
    index: &BTreeMap<String, Vec<String>>,
    local: &BTreeSet<String>,
    external: &External,
    has_sibling: bool,
) -> String {
    let ty = naming::type_name(&class.name);
    // Generic over the abstract class's params any concrete variant uses.
    let enum_generics: Vec<String> = model
        .used_generic_params(&class.name)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let gen_decl = generic_decl(&enum_generics);
    let mut b = String::new();

    // `(variant ident, payload type, doc line)`, so imports derive from payloads.
    let payloads: Vec<(String, String, String)> = variants
        .iter()
        .map(|d| enum_variant_payload(model, class, d, &enum_generics, local, external))
        .collect();

    // A polymorphic concrete class adds `{Name}({Name}Data)` last, so richer
    // subtypes match first.
    let data_ty = format!("{ty}Data");
    let data_generics = struct_generics(model, class);
    let data_subst = class_binding(&class.name);
    let mut payloads = payloads;
    if self_data {
        let data_payload = if data_generics.is_empty() {
            data_ty.clone()
        } else {
            format!("{data_ty}<{}>", data_generics.join(", "))
        };
        payloads.push((
            ty.clone(),
            data_payload,
            format!(
                "An instance of `{}` itself (its own, least-rich form).",
                class.name
            ),
        ));
    }

    let mut imports = variant_imports(model, variants, &enum_generics, &ty, index, external);
    // The in-file `{Name}Data` struct pulls in imports for the class's own fields.
    if self_data {
        imports.extend(data_struct_imports(
            model,
            class,
            &data_generics,
            &data_subst,
            &ty,
            index,
            external,
        ));
    }

    // No serde: `emit-json` writes the `_type` dispatch.
    file_header(&mut b, &class.name, has_sibling);
    write_uses(&mut b, &[], &imports);

    // The `{Name}Data` struct (the class's own instances) precedes the enum.
    if self_data {
        b.push_str(&render_struct_def(
            model,
            class,
            &data_ty,
            &data_generics,
            &data_subst,
            local,
            external,
        ));
        b.push_str(&render_constants(class, &data_ty));
        b.push('\n');
    }

    let slot = if self_data {
        "Polymorphic slot"
    } else {
        "Closed subtype set"
    };
    doc_summary_then(
        &mut b,
        &format!(
            "{slot} of `{}`, dispatched on each payload's `_type`.",
            class.name
        ),
        class.documentation.as_deref(),
        "",
    );
    push_spec_alias(&mut b, &class.name, &ty, "");
    b.push_str("#[derive(Debug, Clone, PartialEq)]\n");
    b.push_str(&format!("pub enum {ty}{gen_decl} {{\n"));
    for (variant, payload, doc) in &payloads {
        b.push_str(&format!("    /// {doc}\n"));
        b.push_str(&format!("    {variant}({payload}),\n"));
    }
    b.push_str("}\n");
    if model.in_schema(&class.name, crate::plan::layout::RM_SCHEMA) {
        b.push_str(&render_variant_froms(&ty, &gen_decl, &payloads));
    }
    b
}

/// One `From<payload> for Enum` per variant of an RM enum; a payload type two
/// variants share gets none.
fn render_variant_froms(ty: &str, gen_decl: &str, payloads: &[(String, String, String)]) -> String {
    let inner = |payload: &str| -> String {
        payload
            .strip_prefix("Box<")
            .and_then(|p| p.strip_suffix('>'))
            .unwrap_or(payload)
            .to_string()
    };
    let mut b = String::new();
    for (variant, payload, _) in payloads {
        let source = inner(payload);
        if payloads
            .iter()
            .filter(|(_, p, _)| inner(p) == source)
            .count()
            > 1
        {
            continue;
        }
        let value = if payload.starts_with("Box<") {
            "Box::new(value)"
        } else {
            "value"
        };
        b.push_str(&format!(
            "\nimpl{gen_decl} From<{source}> for {ty}{gen_decl} {{\n    \
             fn from(value: {source}) -> Self {{\n        Self::{variant}({value})\n    }}\n}}\n"
        ));
    }
    b
}

fn emit_newtype(class: &BmmClass, prim: &str, has_sibling: bool) -> String {
    let ty = naming::type_name(&class.name);
    let mut b = String::new();
    file_header(&mut b, &class.name, has_sibling);
    doc_block_or(
        &mut b,
        class.documentation.as_deref(),
        "",
        &synth_class_doc(&class.name),
        &synth_class_summary(&class.name),
    );
    push_spec_alias(&mut b, &class.name, &ty, "");
    // A transparent primitive newtype; `emit-json` writes its codec.
    b.push_str("#[derive(Debug, Clone, PartialEq, Eq)]\n");
    b.push_str(&format!("pub struct {ty}(pub {prim});\n"));
    b
}

/// One enumeration constant resolved for emission: the BMM constant name, its
/// Rust variant identifier, and its wire value (the string token or the integer).
struct EnumLit {
    /// The verbatim BMM constant name (`release_candidate`) — for doc lines.
    name: String,
    /// The Rust variant identifier (`ReleaseCandidate`).
    ident: String,
    wire: EnumLitWire,
}

enum EnumLitWire {
    Str(String),
    Int(i32),
}

/// Resolves an enumeration's constants to `(ident, wire)` pairs: explicit
/// `item_values` win, an INTEGER with none counts from 0, a STRING with none
/// takes each name (`BMM_ENUMERATION`).
fn enum_literals(enumeration: &BmmEnumeration) -> Vec<EnumLit> {
    let is_int = enumeration.underlying_type == "INTEGER";
    enumeration
        .item_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let ident = naming::type_name(name);
            let wire = match enumeration.item_values.as_ref().and_then(|v| v.get(i)) {
                Some(BmmEnumValue::Int(v)) => {
                    EnumLitWire::Int(i32::try_from(*v).unwrap_or_default())
                }
                Some(BmmEnumValue::Str(s)) => EnumLitWire::Str(s.clone()),
                None if is_int => EnumLitWire::Int(i32::try_from(i).unwrap_or_default()),
                None => EnumLitWire::Str(name.clone()),
            };
            EnumLit {
                name: name.clone(),
                ident,
                wire,
            }
        })
        .collect()
}

/// One untagged-enum variant as `(ident, payload type, doc line)`.
///
/// The payload threads the enum's generic params within one generic family and
/// bound-fills otherwise; it is boxed when it reaches back to the enum.
pub(crate) fn enum_variant_payload(
    model: &Model,
    class: &BmmClass,
    subtype: &str,
    enum_generics: &[String],
    local: &BTreeSet<String>,
    external: &External,
) -> (String, String, String) {
    let variant = naming::type_name(subtype);
    let subtype_generic = !model.used_generic_params(subtype).is_empty();
    let payload = if subtype_generic && !enum_generics.is_empty() {
        format!("{variant}<{}>", enum_generics.join(", "))
    } else {
        model.render_type(
            &BmmType::Simple(subtype.to_owned()),
            enum_generics,
            &BTreeMap::new(),
            local,
            external,
        )
    };
    let already_indirect = payload.starts_with("Vec<") || payload.starts_with("std::collections::");
    let cyclic = !already_indirect && {
        let mut roots = BTreeSet::new();
        model.effective_roots(&BmmType::Simple(subtype.to_owned()), &mut roots);
        roots.contains(&class.name) || model.reaches(subtype, &class.name, &mut BTreeSet::new())
    };
    let payload =
        if cyclic || crate::plan::layout::boxes_variant(model, class, &payload, enum_generics) {
            format!("Box<{payload}>")
        } else {
            payload
        };
    let doc = format!("The `{subtype}` subtype of `{}`.", class.name);
    (variant, payload, doc)
}

/// Emits a BMM enumeration class as a Rust enum: one variant per constant,
/// plus an `Other` catch-all, with a strict `TryFrom` that never yields
/// `Other`.
fn emit_enum_literals(class: &BmmClass, enumeration: &BmmEnumeration, has_sibling: bool) -> String {
    let ty = naming::type_name(&class.name);
    let spec = &class.name;
    let is_int = enumeration.underlying_type == "INTEGER";
    let lits = enum_literals(enumeration);
    let err_ty = format!("Unknown{ty}");
    let (payload, err_inner): (&str, &str) = if is_int {
        ("i32", "i64")
    } else {
        ("String", "String")
    };

    let mut b = String::new();
    file_header(&mut b, spec, has_sibling);
    doc_block_or(
        &mut b,
        class.documentation.as_deref(),
        "",
        &synth_class_doc(spec),
        &synth_class_summary(spec),
    );
    push_spec_alias(&mut b, spec, &ty, "");
    emit_enum_declaration(&mut b, &ty, spec, payload, is_int, &lits);

    emit_enum_conversions(&mut b, &ty, is_int, &lits);
    emit_enum_try_from(&mut b, &ty, spec, &err_ty, is_int, &lits);

    // `emit-json` writes the codec through `from_wire`/`from_value`.

    emit_enum_error_type(&mut b, &ty, spec, &err_ty, err_inner, is_int);
    b
}

/// The enum declaration: its derive, one variant per constant, and the
/// tolerant `Other` payload variant.
fn emit_enum_declaration(
    b: &mut String,
    ty: &str,
    spec: &str,
    payload: &str,
    is_int: bool,
    lits: &[EnumLit],
) {
    b.push_str(if is_int {
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n"
    } else {
        "#[derive(Debug, Clone, PartialEq, Eq, Hash)]\n"
    });
    b.push_str(&format!("pub enum {ty} {{\n"));
    for lit in lits {
        match &lit.wire {
            EnumLitWire::Str(s) => b.push_str(&format!("    /// `{s}`\n")),
            EnumLitWire::Int(v) => b.push_str(&format!("    /// `{}` = {v}\n", lit.name)),
        }
        b.push_str(&format!("    {},\n", lit.ident));
    }
    b.push_str(&format!(
        "    /// A value outside the `{spec}` constant set.\n    ///\n    \
         /// NOTE: no openEHR spec governs an out-of-set value — our own\n    \
         /// tolerance-preserving design (`BMM_ENUMERATION` defines only the listed\n    \
         /// constants), retained so this enum's wire form stays byte-identical to\n    \
         /// the bare `{payload}` it replaces.\n    \
         Other({payload}),\n}}\n\n"
    ));
}

/// The enum's inherent wire conversions: the total `as_str`/`value` writer and
/// its tolerant `from_wire`/`from_value` reader.
fn emit_enum_conversions(b: &mut String, ty: &str, is_int: bool, lits: &[EnumLit]) {
    b.push_str(&format!("impl {ty} {{\n"));
    if is_int {
        emit_int_wire_conversions(b, lits);
    } else {
        emit_str_wire_conversions(b, lits);
    }
}

/// The `i32`-valued enum's `value`/`from_value` pair, closing the `impl` block.
fn emit_int_wire_conversions(b: &mut String, lits: &[EnumLit]) {
    b.push_str(
        "    /// The `i32` wire value of this constant (the verbatim payload for\n    \
         /// [`Self::Other`]).\n    #[must_use]\n    pub fn value(self) -> i32 {\n        match self {\n",
    );
    for lit in lits {
        if let EnumLitWire::Int(v) = &lit.wire {
            b.push_str(&format!("            Self::{} => {v},\n", lit.ident));
        }
    }
    b.push_str("            Self::Other(__v) => __v,\n        }\n    }\n\n");
    b.push_str(
        "    /// This constant for an `i32` wire value, tolerating an unknown\n    \
         /// value as [`Self::Other`] (total — never fails).\n    #[must_use]\n    \
         pub fn from_value(__v: i32) -> Self {\n        match __v {\n",
    );
    for lit in lits {
        if let EnumLitWire::Int(v) = &lit.wire {
            b.push_str(&format!("            {v} => Self::{},\n", lit.ident));
        }
    }
    b.push_str("            _ => Self::Other(__v),\n        }\n    }\n}\n\n");
}

/// The string-valued enum's `as_str`/`from_wire` pair, closing the `impl` block.
fn emit_str_wire_conversions(b: &mut String, lits: &[EnumLit]) {
    b.push_str(
        "    /// The wire string of this constant (the verbatim token for\n    \
         /// [`Self::Other`]).\n    #[must_use]\n    pub fn as_str(&self) -> &str {\n        match self {\n",
    );
    for lit in lits {
        if let EnumLitWire::Str(s) = &lit.wire {
            b.push_str(&format!("            Self::{} => {s:?},\n", lit.ident));
        }
    }
    b.push_str("            Self::Other(__s) => __s.as_str(),\n        }\n    }\n\n");
    b.push_str(
        "    /// This constant for a wire string, tolerating an unknown token\n    \
         /// as [`Self::Other`] (total — never fails).\n    #[must_use]\n    \
         pub fn from_wire(__s: &str) -> Self {\n        match __s {\n",
    );
    for lit in lits {
        if let EnumLitWire::Str(s) = &lit.wire {
            b.push_str(&format!("            {s:?} => Self::{},\n", lit.ident));
        }
    }
    b.push_str("            _ => Self::Other(__s.to_owned()),\n        }\n    }\n}\n\n");
}

/// The strict `TryFrom` seam, which never yields `Other`.
fn emit_enum_try_from(
    b: &mut String,
    ty: &str,
    spec: &str,
    err_ty: &str,
    is_int: bool,
    lits: &[EnumLit],
) {
    if is_int {
        b.push_str(&format!(
            "impl ::core::convert::TryFrom<i64> for {ty} {{\n    type Error = {err_ty};\n\n    \
             /// # Errors\n    /// Returns [`{err_ty}`] when `__v` is not a `{spec}` value\n    \
             /// (unlike [`Self::from_value`], which is total).\n    \
             fn try_from(__v: i64) -> ::core::result::Result<Self, Self::Error> {{\n        match __v {{\n"
        ));
        for lit in lits {
            if let EnumLitWire::Int(v) = &lit.wire {
                b.push_str(&format!(
                    "            {v} => ::core::result::Result::Ok(Self::{}),\n",
                    lit.ident
                ));
            }
        }
        b.push_str(&format!(
            "            _ => ::core::result::Result::Err({err_ty}(__v)),\n        }}\n    }}\n}}\n\n"
        ));
        return;
    }
    b.push_str(&format!(
        "impl ::core::convert::TryFrom<&str> for {ty} {{\n    type Error = {err_ty};\n\n    \
         /// # Errors\n    /// Returns [`{err_ty}`] when `__s` is not a `{spec}` value\n    \
         /// (unlike [`Self::from_wire`], which is total).\n    \
         fn try_from(__s: &str) -> ::core::result::Result<Self, Self::Error> {{\n        match __s {{\n"
    ));
    for lit in lits {
        if let EnumLitWire::Str(s) = &lit.wire {
            b.push_str(&format!(
                "            {s:?} => ::core::result::Result::Ok(Self::{}),\n",
                lit.ident
            ));
        }
    }
    b.push_str(&format!(
        "            _ => ::core::result::Result::Err({err_ty}(__s.to_owned())),\n        }}\n    }}\n}}\n\n"
    ));
}

/// The strict seam's error type (hand-rolled `Display` + `Error`, no
/// `thiserror` in the generated crates).
fn emit_enum_error_type(
    b: &mut String,
    ty: &str,
    spec: &str,
    err_ty: &str,
    err_inner: &str,
    is_int: bool,
) {
    b.push_str(&format!(
        "/// The error returned by [`{ty}::try_from`] for a value outside the `{spec}`\n\
         /// constant set.\n#[derive(Debug, Clone, PartialEq, Eq)]\npub struct {err_ty}(pub {err_inner});\n\n"
    ));
    let fmt = if is_int {
        format!("unknown {spec} value: {{}}")
    } else {
        format!("unknown {spec} value: {{:?}}")
    };
    b.push_str(&format!(
        "impl ::core::fmt::Display for {err_ty} {{\n    \
         fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {{\n        \
         ::core::write!(f, {fmt:?}, self.0)\n    }}\n}}\n\n"
    ));
    b.push_str(&format!("impl ::std::error::Error for {err_ty} {{}}\n"));
}

// ── import + header helpers ──────────────────────────────────────────────────

/// Precise `use` lines for a struct's referenced spec types.
fn import_lines(
    model: &Model,
    class: &BmmClass,
    generics: &[String],
    subst: &BTreeMap<String, String>,
    self_ident: &str,
    index: &BTreeMap<String, Vec<String>>,
    external: &External,
) -> BTreeSet<String> {
    let mut imports = BTreeSet::new();
    for spec in model.referenced_specs(class, generics, subst) {
        add_import(&mut imports, &spec, self_ident, index, external);
    }
    imports
}

/// Resolves a referenced spec type to a `use` line: local first, then the
/// dependency generation; an unresolved type needs none.
fn add_import(
    imports: &mut BTreeSet<String>,
    spec: &str,
    self_ident: &str,
    index: &BTreeMap<String, Vec<String>>,
    external: &External,
) {
    let ident = naming::type_name(spec);
    if ident == self_ident {
        return;
    }
    if let Some(chain) = index.get(&ident) {
        imports.insert(format!("use crate::{}::{};", chain.join("::"), ident));
    } else if let Some(path) = external.module_of(spec) {
        imports.insert(format!("use {path}::{ident};"));
    }
}

fn struct_header(b: &mut String, class: &str, imports: &BTreeSet<String>, has_sibling: bool) {
    file_header(b, class, has_sibling);
    write_uses(b, &[], imports);
}

/// The generated file's banner and `//!` summary naming its spec class, with
/// the sibling-`*_impl.rs` line when one exists.
fn file_header(b: &mut String, class: &str, has_sibling: bool) {
    b.push_str(&format!(
        "// @generated by openehr-codegen from BMM (`{class}`) — DO NOT EDIT.\n"
    ));
    if has_sibling {
        b.push_str("// Hand-written spec functions/invariants live in the sibling `*_impl.rs`.\n");
    }
    b.push_str(&format!(
        "\n//! The openEHR `{class}` spec class, generated from the vendored BMM\n\
         //! meta-model.\n\n"
    ));
}

/// A rustdoc search alias carrying the verbatim openEHR spelling; skipped when
/// it equals the Rust name.
fn push_spec_alias(b: &mut String, spec: &str, rust_ty: &str, indent: &str) {
    if spec != rust_ty {
        b.push_str(&format!("{indent}#[doc(alias = \"{spec}\")]\n"));
    }
}

/// The class-doc line for a BMM class the vendored schema leaves undocumented.
fn synth_class_summary(spec: &str) -> String {
    format!("The openEHR `{spec}` class.")
}

fn synth_class_doc(spec: &str) -> String {
    format!("The openEHR `{spec}` class (the vendored BMM carries no documentation for it).")
}

/// The synthesized attribute-doc line for a BMM property with no
/// `documentation`, naming the attribute and the class that declares it.
fn synth_field_doc(owner: &str, prop: &str) -> String {
    format!(
        "The `{prop}` attribute of openEHR `{owner}` (the vendored BMM carries no \
         documentation for it)."
    )
}

/// Emits a crate's `use` block sorted as `rustfmt` sorts it, then a blank line.
fn write_uses(b: &mut String, fixed: &[&str], imports: &BTreeSet<String>) {
    let mut all: BTreeSet<String> = imports.clone();
    for f in fixed {
        all.insert((*f).to_string());
    }
    for u in &all {
        b.push_str(u);
        b.push('\n');
    }
    b.push('\n');
}

/// Emits `doc` as `///` lines, or `fallback` when the BMM has none;
/// `summary_hint` is prepended when the prose's first sentence is too long for
/// a summary.
fn doc_block_or(
    b: &mut String,
    doc: Option<&str>,
    indent: &str,
    fallback: &str,
    summary_hint: &str,
) {
    if doc.is_some_and(|d| !d.trim().is_empty()) {
        doc_prose::doc_block_summarized(b, doc, indent, summary_hint);
    } else {
        b.push_str(&format!("{indent}/// {fallback}\n"));
    }
}

/// Emits a one-sentence `summary`, then the BMM prose as the detail paragraph.
fn doc_summary_then(b: &mut String, summary: &str, doc: Option<&str>, indent: &str) {
    b.push_str(&format!("{indent}/// {summary}\n"));
    if doc.is_some_and(|d| !d.trim().is_empty()) {
        b.push_str(&format!("{indent}///\n"));
        doc_prose::doc_block_summarized(b, doc, indent, "");
    }
}

/// `DV_QUANTITY` → `dv_quantity`, `Iso8601_date` → `iso8601_date`.
fn to_snake(spec: &str) -> String {
    spec.to_lowercase()
}
