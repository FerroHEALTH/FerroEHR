// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 4 — RENDER. Walks a merged BMM [`Model`] and produces deterministic,
//! byte-stable Rust for the openEHR spec crates.
//!
//! Emission rules:
//! - A concrete class inlines all inherited fields, ancestor-first, so any
//!   field is one hop away.
//! - `Option<T>` for non-mandatory single properties, `Vec<T>` for containers.
//! - Untagged enums for abstract classes used as a property type — the closed
//!   polymorphic slots (`DATA_VALUE`, `ITEM`, …).
//! - Transparent newtypes for enumeration classes that are a bare primitive on
//!   the wire (`VALIDITY_KIND` → `String`).
//! - Generics only where the BMM declares them (`Interval<T>`); the type
//!   argument is emitted at each use site.
//! - Foundation primitives, containers and marker traits map to Rust types and
//!   are never emitted ([`crate::plan::overrides::PRIMITIVES`],
//!   [`crate::plan::overrides::MAPPED_CLASSES`]).
//! - Every emitted public item carries documentation: the BMM `documentation`
//!   where the vendored schema has it, else a deterministic synthesized line
//!   ([`synth_class_doc`], [`synth_field_doc`]), sanitized for rustdoc by
//!   [`sanitize_doc_prose`].
//!
//! These type files carry no serde attributes or derives: `emit-json` generates
//! the (de)serialization and the `_type` discriminator separately.

use crate::analyze::{External, Model, class_paths};
use crate::load::bmm::{
    BmmClass, BmmConstant, BmmEnumValue, BmmEnumeration, BmmPropKind, BmmSchema, BmmType,
};
use crate::load::impls::SiblingImpls;
use crate::plan::composition::{
    CrateComposition, GenerationSpec, GenerationUnit, generation_variant,
};
use crate::plan::construction;
use crate::plan::overrides::{back_reference, class_binding, type_override, untyped_field};
use crate::plan::{Emission, decide};
use crate::render::naming;
use std::collections::{BTreeMap, BTreeSet};

/// Prepend a file-root `#![expect(clippy::disallowed_types)]` to a generated
/// body whose CODE mentions the workspace-banned `serde_json::Value`.
///
/// Free-form JSON in generated output is admitted only where the model
/// adjudicates it (`plan::overrides::UNTYPED_FIELDS` writes the citation NOTE
/// at the field) or where the spec itself leaves the slot open (BMM `Any`,
/// `xs:anyType`, an out-of-closure reference). The `expect` is scoped to the
/// one file and stays honest: comment-only mentions insert nothing, and a
/// regeneration that stops carrying the type drops the attribute with it.
///
/// A body with feature-gated regions (`#[cfg(feature = …)]` — the ITS-REST
/// group modules, whose server trait and client are each behind a feature)
/// gets `#![allow(…)]` instead: the carrier may sit inside a gated region, so
/// under a selection without that feature an `expect` would be unfulfilled.
pub(crate) fn guard_value_carriers(body: &str) -> String {
    let code_carries = body.lines().any(|l| {
        let t = l.trim_start();
        !t.starts_with("//") && l.contains("serde_json::Value")
    });
    if !code_carries {
        return body.to_owned();
    }
    // Insert after the leading header (`//`) + module-doc (`//!`) block AND
    // after any existing inner-attribute (`#![…]`) blocks: a later same-level
    // blanket `#![allow(clippy::all, …)]` would mask the lint and leave the
    // expectation unfulfilled, so the guard must come last. An outer doc
    // comment (`///`) already belongs to the first item and stops the scan.
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
struct Version {
    files: Vec<GenFile>,
    /// Top-level module names of this version (under its prefix, if any).
    top: BTreeSet<String>,
    /// The types this version emitted, for crate-prelude assembly (empty for a
    /// prefixed version, which emits its own prelude in-tree).
    emitted: Vec<Emitted>,
}

/// One BMM generation of a crate as the emitter consumes it.
///
/// A crate composed of several generations (AM 1.4/2.4; LANG's stable v2.x
/// BMM beside the v3 development line — `LANG/docs/bmm3/master00-amendment_record.adoc`
/// SPECLANG-14) emits each one COMPLETELY from its own schema and its own
/// resolution model under its own version-named top module, so a class name
/// declared by two generations yields two Rust types at two module paths,
/// each with its own shape and its own intra-generation cross-references.
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

/// The emission shapes that produce a file: [`Emission`] minus its `Skip`
/// variant. Narrowing at the planning step makes "a skipped class reached the
/// render loop" unrepresentable instead of a runtime check.
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

/// Emit one specification unit under its generation's version-named top
/// module. Produces the type files and the `mod.rs` tree; the caller
/// assembles the generation prelude, the crate prelude and `lib.rs`. The
/// generation's spec version lives ONLY on the `Generation` enum (owner
/// ruling 2026-08-05: no version constants anywhere — a second copy of the
/// same fact can only drift, and `spec_version()` is a `const fn`, so even
/// const contexts need no constant).
fn emit_version(
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

    // Safeguard (owner ruling 2026-07-19): never emit a non-constructible type.
    // A mandatory single-valued construction cycle must be broken at a designated
    // owner/parent back-reference edge (`back_reference`); this fails loudly if a
    // cycle is left unbroken (e.g. a future BMM addition), pointing at the fix.
    model.assert_constructible(schema);

    let class_pkg = class_paths(schema);
    let used = model.used_as_type();

    // Spec class names emitted in this version; anything referenced outside it
    // degrades to `serde_json::Value` so the crate stays self-contained. This is
    // the same projection [`crate::analyze::emittable_specs`] computes — the one
    // the `External` prelude index and the `model-query` report resolve against.
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
            chain.push(prefix.to_string());
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
        // A polymorphic-concrete class emits a sibling `{Name}Data` struct in the
        // same file (the enum owns `{Name}`); export it from the prelude too so
        // downstream code (e.g. the generated XML impls) can name it.
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

    // Module tree, with the generation's own `<prefix>/prelude` registered so
    // the generation `mod.rs` declares it (the caller assembles the prelude
    // file itself, over the generation's prelude-carrying units).
    let mut tree_chains = type_chains.clone();
    tree_chains.push(vec![prefix.to_string(), "prelude".to_string()]);
    files.extend(emit_module_tree(&tree_chains));

    Version {
        files,
        top: std::iter::once(prefix.to_string()).collect(),
        emitted,
    }
}

/// The crate-relative module path a class's emitted type lives at WITHIN its
/// generation module, e.g. `core::bmm_class` — the same chain
/// [`emit_version`] builds under the generation prefix, so a downstream
/// emitter can name any type by
/// `<crate>::<generation>::<type_module_path>::<Ident>`.
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

/// Emit a fully-composed crate: every BMM generation rendered completely
/// under its version-named top module (with its own prelude + `SPEC_VERSION`),
/// the crate prelude re-exporting the CURRENT generation only, and `lib.rs`
/// carrying the crate `SPEC_VERSION` and the [`crate::plan::composition`]
/// table's emitted `Generation` enum.
///
/// `generations` pairs each composed generation with the (possibly augmented —
/// `cross_schema_reemit`) schema the render loop consumes.
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

/// Build every `mod.rs` from the set of emitted module chains.
fn emit_module_tree(chains: &[Vec<String>]) -> Vec<GenFile> {
    // dir path ("" = root is handled by lib.rs) → set of child module idents.
    let mut dirs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for chain in chains {
        // Walk the chain left to right, growing `dir` to the join of every
        // segment already passed — the index-free form of `chain[..i]`/`chain[i]`.
        let mut dir = String::new();
        for (i, segment) in chain.iter().enumerate() {
            if i > 0 {
                dirs.entry(dir.clone()).or_default().insert(segment.clone());
                dir.push('/');
            }
            dir.push_str(segment);
        }
    }
    dirs.into_iter()
        .map(|(dir, children)| {
            let mut b = String::from("// @generated by openehr-codegen — DO NOT EDIT.\n\n");
            // A module is a public item too, so it needs its own docs
            // (`missing_docs`, rustc lint listing).
            b.push_str(&format!(
                "//! The openEHR spec package `{dir}` — generated module tree.\n\n"
            ));
            for c in &children {
                b.push_str(&format!("pub mod {c};\n"));
            }
            GenFile {
                path: format!("{dir}/mod.rs"),
                body: b,
            }
        })
        .collect()
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
    // Lint exceptions inherent to faithful spec generation: the spec owns the
    // doc text, the subtype names and the attribute names, so satisfying the
    // doc, name-prefix and struct-field lints would fork the model.
    // `reason` is mandatory (`allow_attributes_without_reason` is deny); `expect`
    // is wrong because a given crate need not trigger every listed lint.
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
    // No crate-level SPEC_VERSION const (owner ruling 2026-08-05): a
    // multi-generation crate has no single implemented spec version — the
    // `Generation` enum is the authority (per-variant `spec_version()`, the
    // derived `Default` marking the current generation). A fixed crate-root
    // pin would contradict the selected generation the moment a consumer
    // configures a non-current one.
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
    // Variants sharing one spec version fold into a single or-pattern arm
    // (LANG's v2/v3 both carry the 1.1.0-line release) — identical match
    // arms are a clippy deny in the generated output.
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
    // The doc's example token is THIS crate's current generation, not a
    // hardcoded one: a fixed `"v1_2"` told openehr-term's readers its token
    // was `v1_2` when the only token it has is `v3_1`.
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
    // Both the crate prelude (assembled from the CURRENT generation's emitted
    // set — the chains carry the generation module) and each generation's own
    // in-tree prelude are emitted through this one function; only the entry
    // set differs.
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

/// The struct definition (doc, derive, fields) under the name `struct_ty`,
/// without the file header. `struct_ty` is normally `type_name(class)`, but a
/// polymorphic-concrete class emits its own instances as `{Name}Data` (the
/// enum owns `{Name}`). The canonical `_type` stays the class name either way.
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

    // A class with a validating construction door (`plan::construction`) emits
    // its fields `pub(crate)` instead of `pub`: outside this crate the only way
    // to obtain a value is the hand-written `*_impl.rs` constructor, which runs
    // the released grammar. `pub(crate)` rather than fully private because the
    // grammar itself is hand-written spec behaviour in a sibling module of the
    // same crate, and the generator never writes into `*_impl.rs`.
    let field_vis = if construction::is_validated(&class.name) {
        "pub(crate)"
    } else {
        "pub"
    };
    // `(ident, rust_type)` for each emitted field, in emission order — the read
    // accessors a validated class needs, and the parameter order its constructor
    // is called in by the generated codecs.
    let mut emitted: Vec<(String, String)> = Vec::new();

    let props = model.flattened_props(class);
    let mut prev_owner: Option<&str> = None;
    let mut first = true;
    for rp in &props {
        let p = rp.prop;
        // A designated owner/parent back-reference is a non-data navigational
        // association, never forward-owned data and never on the canonical wire;
        // emitting it as an owning field makes the type a non-constructible
        // infinite value (see `back_reference`). Omit it from the struct + serde;
        // behavioural access, if ever needed, belongs in a hand-written
        // `*_impl.rs`. This is the only sanctioned way to break a mandatory
        // construction cycle — a forward composition is never relaxed.
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

        // The wire name (rename) and literal default are consumed by the JSON
        // codec emitter (`emit-json`, which reads them from the BMM), not from a
        // struct attribute — so no serde/`openehr` field attribute is emitted here.
        let ident = naming::field_ident(&p.name);
        let rust_ty = field_type(model, class, p, generics, subst, local, external);
        // A field that degraded to free-form JSON *and* carries an adjudication
        // gets the decision written at the site: silence over an untyped slot in
        // a generated spec crate is indistinguishable from an oversight. The
        // NOTE is conditional on the degrade actually happening, so a
        // composition where the type IS resolvable (the AM24 re-emission of
        // `EL_CASE`, where `C_OBJECT` is local) emits the typed field with no note.
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

/// The imports every enum variant's payload embeds.
///
/// For a variant threaded over the enum's own params (`IntervalEvent<T>`) that
/// is just the variant type; for a bound-filled variant
/// (`DvInterval<DvOrdered>`) it also includes the auto-filled bound arguments.
/// This mirrors the payload decision, so a bound type the payload never names
/// is not imported.
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
///
/// The enum's own type is never imported into its own file, and a spec neither
/// the local ident index nor an external prelude places is skipped.
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
///
/// No serde/`_type` derive is emitted: canonical-JSON (de)serialization is
/// provided by the emitted `ToJson`/`FromJson` impls in `openehr-its`
/// (`emit-json`), so the type is a plain data record.
fn push_struct_prologue(b: &mut String, class: &BmmClass, struct_ty: &str, gen_decl: &str) {
    doc_block_or(
        b,
        class.documentation.as_deref(),
        "",
        &synth_class_doc(&class.name),
        &synth_class_summary(&class.name),
    );
    push_spec_alias(b, &class.name, struct_ty, "");
    // A class the construction map records as staying a plain record although a
    // reader might expect a validating door carries that adjudication here, at
    // the public fields it is about — silence over an unguarded identifier type
    // is indistinguishable from an oversight.
    if let Some(note) = construction::plain_record_note(&class.name) {
        b.push_str(&format!("// NOTE: {note}\n"));
    }
    b.push_str("#[derive(Debug, Clone, PartialEq)]\n");
    b.push_str(&format!("pub struct {struct_ty}{gen_decl} {{\n"));
}

/// The `// inherited: X` group header a property opens, if it starts a new
/// inheritance group.
///
/// The header carries a blank line before it — but not as the very first line
/// inside the braces, which rustfmt strips.
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
///
/// A designated owner/parent back-reference is a non-data navigational
/// association, never forward-owned data and never on the canonical wire;
/// emitting it as an owning field makes the type a non-constructible infinite
/// value. This is the only sanctioned way to break a mandatory construction
/// cycle — a forward composition is never relaxed.
fn push_back_reference_note(b: &mut String, prop: &str, citation: &str) {
    b.push_str(&format!(
        "    // NOTE: `{prop}` (BMM-mandatory back-reference) omitted — {citation}. \
         A back-reference is not forward-owned data and never appears on \
         the canonical wire; emitting it as an owning field would make \
         this type non-constructible.\n"
    ));
}

/// The NOTE emitted beside a field that degraded to free-form JSON *and*
/// carries an adjudication.
///
/// Silence over an untyped slot in a generated spec crate is
/// indistinguishable from an oversight. The note is conditional on the degrade
/// actually happening, so a composition where the type IS resolvable (the AM24
/// re-emission of `EL_CASE`, where `C_OBJECT` is local) emits the typed field
/// with no note.
fn push_untyped_field_note(b: &mut String, owner: &str, class: &str, prop: &str) {
    if let Some(adj) = untyped_field(owner, prop).or_else(|| untyped_field(class, prop)) {
        b.push_str(&format!(
            "    // NOTE: free-form JSON is adjudicated here, not accidental — {}. {}\n",
            adj.citation, adj.reason
        ));
    }
}

/// The read accessors + door NOTE for a class whose fields are `pub(crate)`
/// behind a validating constructor (`plan::construction`).
///
/// Emitting the accessors here rather than hand-writing one per class keeps the
/// scheme mechanical: a field added to a validated class by a spec bump gains
/// its reader automatically, and the accessor set can never drift from the field
/// set. `String` reads back as `&str` (the idiomatic borrow); every other type
/// reads back by reference.
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
        // `&Option<T>` is never the right accessor return (clippy::ref_option:
        // it forces the caller to reborrow and blocks passing a plain `Some(&x)`)
        // — an optional field reads back as `Option<&T>`, with `Option<String>`
        // borrowing all the way down to `Option<&str>`.
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

/// Emit the class's `BMM_CLASS.constants` as an `impl {ty} { pub const … }`
/// block (empty when the class has none). `ty` is the struct the constants hang
/// on — `type_name(class)` for a plain struct, `{Name}Data` for a polymorphic
/// slot. Each constant carries a doc line citing its verbatim BMM name (the BMM
/// is the authority); the literal is decoded from the raw BMM `value`.
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
        doc_block(&mut b, c.documentation.as_deref(), "    ");
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

/// Decode a BMM constant's raw `value` to a Rust `(type, literal)` pair. A JSON
/// number keys off the BMM `type` (`Real`/`Double` → `f64`, else `i64`); a JSON
/// string carries a quoted `"…"` (→ `&str`) or `'…'` (→ `char`) literal,
/// a bareword cross-reference to a sibling constant (→ `Self::OTHER`), or a
/// boolean keyword. Numeric character references (`&#42;`) and Eiffel octal
/// escapes (`\015`) inside literals are decoded.
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

/// Decode a STRING-valued BMM constant to a Rust `(type, literal)` pair.
///
/// A quoted `"…"` is a `&str`, a quoted `'…'` a `char`, a bareword naming a
/// sibling constant a `Self::OTHER` cross-reference, and a bareword under a
/// `Boolean` type a bool. Anything else is emitted verbatim as a string
/// literal — the safest total decoding.
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

/// Compute a field's Rust type (the JSON codec handles `None`/empty omission at
/// its field call sites, so no attribute is needed on the field).
///
/// This is the single field-shape decision: the struct renderer above calls it
/// for every flattened property, and [`crate::render::model_query`] calls it to
/// report the decision, so the report cannot drift from the emitted code.
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

/// The inner Rust type of a single-valued property, boxed where leaving it by
/// value would make the struct infinitely sized.
///
/// The cycle may be direct self-recursion, mutual recursion
/// (`RESOURCE_DESCRIPTION` ↔ `AUTHORED_RESOURCE`), or F-bounded recursion through
/// an auto-filled generic arg (`DV_QUANTITY` → `normal_range:
/// DvInterval<DvOrdered>`, and `DvOrdered`'s variants include `DV_QUANTITY`), so
/// every spec name the rendered type embeds by value is checked, not just its
/// head. A type already behind an indirection (`Vec`, `BTreeMap`, `BTreeSet`)
/// breaks the cycle on its own, and boxing it would be redundant.
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
        format!("Box<{inner}>")
    } else {
        inner
    }
}

/// The Rust type of a container property.
///
/// A byte buffer (`Array<Octet>` / `List<Octet>`, e.g. `DV_MULTIMEDIA.data`) is
/// inline base64 *text* on the canonical wire, not a JSON array, so it carries
/// the base64 verbatim as a `String` (decoding is a behaviour-layer concern),
/// like other broader-than-a-crate openEHR types; its optionality follows the
/// property.
///
/// Every other container's shape follows its BMM existence and cardinality:
///
/// - mandatory with cardinality `0..*` is `Vec<T>` (the model genuinely
///   admits zero members);
/// - mandatory `1..*` is `NonEmptyVec<T>` — the declared lower bound is a
///   structural statement about the model, so the type carries it and the
///   strict readers refuse `[]` at parse;
/// - optional with no non-empty rule is `Option<Vec<T>>`, because the RM
///   distinguishes Void from empty on a `0..1` container — `FOLDER.items` /
///   `FOLDER.folders` are both `0..1` with no non-empty invariant
///   (`org.openehr.rm.common.folder.adoc` §Attributes), so "absent" and
///   "present-but-empty" are two legitimate model states;
/// - optional under an invariant of the `x /= Void implies not x.is_empty`
///   family is `Option<NonEmptyVec<T>>` — no valid instance can carry
///   present-but-empty, so it stops being representable
///   (`LOCATABLE.Links_valid`, `org.openehr.rm.common.locatable.adoc`
///   §Invariants).
///
/// The `1..*` source is the BMM minus
/// [`crate::plan::overrides::cardinality_contradicted`] (a class whose own
/// invariants contradict its declared lower bound). The write direction is
/// unaffected either way: the canonical-JSON writer omits an empty list
/// whether it is `None` or `Some(vec![])` (ITS-REST overview `Resources.md`
/// §JSON Format — "attributes (even required ones) that are `Null` or an
/// empty list (array) SHOULD be absent when serialized"); the reader is the
/// direction that gains (`absent` → `None`, `[]` → `Some(vec![])`, or a
/// refusal where the type is `NonEmptyVec`).
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
        && !crate::plan::overrides::cardinality_contradicted(&class.name, &p.name);
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
    // The enum is generic over the abstract class's declared params that any
    // concrete variant uses (`VERSION<T>` exposes `T` only through
    // `ORIGINAL_VERSION.data: T`); `used_generic_params` resolves this uniformly
    // so a bare reference elsewhere renders the same arity.
    let enum_generics: Vec<String> = model
        .used_generic_params(&class.name)
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    let gen_decl = generic_decl(&enum_generics);
    let mut b = String::new();

    // Compute payloads first (so imports can be derived from what they touch).
    // Each entry is `(variant ident, payload type, doc line)` — a variant is a
    // public item `missing_docs` checks, and the BMM has no per-subtype text for
    // a closed slot, so the line is synthesized from the subtype's spec name.
    let payloads: Vec<(String, String, String)> = variants
        .iter()
        .map(|d| enum_variant_payload(model, class, d, &enum_generics, local, external))
        .collect();

    // A polymorphic *concrete* class also carries its own instances: append a
    // `{Name}({Name}Data)` variant last (least-rich, so richer subtypes match
    // first on the untagged wire), and emit the `{Name}Data` struct in-file.
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

    // No serde: the canonical-JSON `_type` dispatch (abstract slots require
    // `_type`; concrete polymorphic slots default a `_type`-less value to the
    // base type) is emitted as a native `FromJson` impl in `openehr-its`
    // (`emit-json`), and serialization as a native `ToJson` impl there. This
    // enum is a plain closed subtype set with no derive/serde attributes.
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
    // A transparent primitive newtype; canonical-JSON (de)serialization is the
    // emitted `ToJson`/`FromJson` impl in `openehr-its` (`emit-json`), which
    // delegates through the inner primitive.
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

/// Resolve an enumeration's constants to `(ident, wire)` pairs, applying the same
/// value rule as the RM-model emitter (`BMM_ENUMERATION`: explicit `item_values`
/// win; an INTEGER with none assumes 0,1,2,…; a STRING with none takes each
/// constant name as its own value).
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
/// A variant is a public item `missing_docs` checks, and the BMM has no
/// per-subtype text for a closed slot, so the doc line is synthesized from the
/// subtype's spec name.
///
/// The payload threads the enum's own generic params when the subtype belongs
/// to the same generic family (`Event<T>` → `PointEvent(PointEvent<T>)`), and
/// otherwise bound-fills the variant (`DvInterval(DvInterval<DvOrdered>)`). It
/// is boxed when leaving it by value would make the enum infinitely sized:
/// either the payload embeds the enum type through a bound-filled argument
/// (`EL_TERMINAL` ⊇ `EL_CASE_TABLE<EL_TERMINAL>`), or the variant's own fields
/// reach back to the enum (`BMM_TYPE` ⊇ `BMM_CONTAINER_TYPE` whose `base_type`
/// is a `BMM_TYPE`). A `Vec`/map payload already breaks the cycle.
fn enum_variant_payload(
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
    let payload = if cyclic {
        format!("Box<{payload}>")
    } else {
        payload
    };
    let doc = format!("The `{subtype}` subtype of `{}`.", class.name);
    (variant, payload, doc)
}

/// Emit a BMM enumeration class as a real Rust enum: one variant per named
/// constant, plus a tolerance-preserving `Other(String|i32)` catch-all.
///
/// The hand-written serde is provably byte-identical to the transparent
/// `String`/`i32` newtype it replaces: `serialize` writes `as_str`/`value` (the
/// constant token for a known variant, the verbatim payload for `Other`), and
/// `deserialize` reads the bare primitive then maps it through the total
/// `from_wire`/`from_value`. Because `as_str ∘ from_wire` (and `value ∘
/// from_value`) is the identity for every input, the round-trip preserves every
/// byte. A strict `TryFrom` seam alongside rejects out-of-set values with a
/// per-enum typed error and never yields `Other`.
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

    // Canonical-JSON (de)serialization is the emitted `ToJson`/`FromJson` impl in
    // `openehr-its` (`emit-json`): `ToJson` writes `as_str`/`value` (the constant
    // token or verbatim `Other` payload) and `FromJson` maps the bare primitive
    // through the total `from_wire`/`from_value`, byte-identical to the primitive
    // it replaces. No serde impl is emitted here.

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

/// Precise `use` lines for a struct's referenced spec types: `crate::…` for
/// types emitted in this crate, the dependency generation's full
/// defining-module path for dependency types.
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

/// Resolve a referenced spec type to a `use` line: local (`crate::…`) wins,
/// then the dependency generation's defining module; an unresolved type needs
/// no import (it rendered as `serde_json::Value`).
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

/// The generated file's banner + its module documentation. Every generated type
/// file IS a module, and `missing_docs` checks modules, so the file carries an
/// inner `//!` summary naming the spec class it realizes (an out-of-line
/// module's inner docs satisfy the lint at the `pub mod` declaration site).
/// `has_sibling` adds the sibling-`*_impl.rs` banner line — emitted for exactly
/// the classes that HAVE one on disk (`crate::load::impls::SiblingImpls`), so
/// the banner never points a reader at a file that does not exist.
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

/// A rustdoc search alias carrying the verbatim openEHR spelling, so
/// `EHR_STATUS` finds `EhrStatus`. Skipped when the Rust name already equals the
/// spec name — rustc rejects an alias identical to the item name.
fn push_spec_alias(b: &mut String, spec: &str, rust_ty: &str, indent: &str) {
    if spec != rust_ty {
        b.push_str(&format!("{indent}#[doc(alias = \"{spec}\")]\n"));
    }
}

/// The synthesized class-doc line for a BMM class the vendored schema carries no
/// `documentation` for (10 such classes across the pinned schemas as of the
/// current pins) — `missing_docs` admits no exceptions, and an honest
/// synthesized line beats a silent gap.
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

/// Emit a crate's `use` block as a single lexicographically-sorted list (so the
/// output matches `rustfmt`'s default import ordering — `crate::…` before
/// `openehr_base::…`), followed by a blank line. `fixed` holds always-present
/// uses (none now that the type files carry no derive/serde); `imports` holds
/// the per-file resolved spec imports.
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

/// Emit `doc` as `///` lines, falling back to `fallback` when the vendored BMM
/// carries no documentation for the item. Every public item needs docs
/// (`missing_docs`), so there is no "no docs" branch. `summary_hint` is the
/// synthesized summary line prepended when the prose opens with a sentence
/// too long to stand as the RFC 1574 summary (empty = leave verbatim).
fn doc_block_or(
    b: &mut String,
    doc: Option<&str>,
    indent: &str,
    fallback: &str,
    summary_hint: &str,
) {
    if doc.is_some_and(|d| !d.trim().is_empty()) {
        doc_block_summarized(b, doc, indent, summary_hint);
    } else {
        b.push_str(&format!("{indent}/// {fallback}\n"));
    }
}

/// Emit a one-sentence `summary` line, then the BMM prose as the detail
/// paragraph after a blank `///` — the documented rustdoc shape (the first
/// paragraph is what search results and module indexes show:
/// <https://doc.rust-lang.org/rustdoc/how-to-write-documentation.html>).
fn doc_summary_then(b: &mut String, summary: &str, doc: Option<&str>, indent: &str) {
    b.push_str(&format!("{indent}/// {summary}\n"));
    if doc.is_some_and(|d| !d.trim().is_empty()) {
        b.push_str(&format!("{indent}///\n"));
        doc_block(b, doc, indent);
    }
}

fn doc_block(b: &mut String, doc: Option<&str>, indent: &str) {
    doc_block_summarized(b, doc, indent, "");
}

fn doc_block_summarized(b: &mut String, doc: Option<&str>, indent: &str, summary_hint: &str) {
    let Some(doc) = doc else { return };
    let mut block = DocBlock::default();
    for line in doc.lines() {
        block.push_line(line.trim_end());
    }
    let mut out = block.finish();
    split_long_first_paragraph(&mut out, summary_hint);

    for line in &out {
        if line.is_empty() {
            b.push_str(&format!("{indent}///\n"));
        } else {
            b.push_str(&format!("{indent}/// {line}\n"));
        }
    }
}

/// The line-by-line rewriter behind [`doc_block_summarized`].
///
/// Spec prose carries example blocks (ODIN snippets, `YYYY-MM-DDTHH:MM:SS` date
/// formats) that rustdoc would compile as Rust doctests and choke on. Both forms
/// it recognizes are neutralized so the docs render as text, never run: a bare
/// triple-backtick fence has its opening tagged `text` (the closing stays bare),
/// and a run of 4-space-indented lines is wrapped in a `text` fence. Prose
/// OUTSIDE those blocks additionally goes through [`sanitize_doc_prose`] (bare
/// URLs, stray brackets/angle brackets — the rustdoc deny-lints).
#[derive(Default)]
struct DocBlock<'a> {
    /// The rewritten lines, in order.
    out: Vec<String>,
    /// Pending prose lines, sanitized as one segment so a code span may span
    /// lines (the BMM has such spans, e.g. `BMM_SCHEMA_DESCRIPTOR.schema_id`).
    prose: Vec<&'a str>,
    /// Inside an explicit triple-backtick fence.
    in_fence: bool,
    /// Inside an auto-wrapped indented block.
    in_indent: bool,
}

impl<'a> DocBlock<'a> {
    /// Sanitizes and emits the pending prose segment, if any.
    fn flush_prose(&mut self) {
        if self.prose.is_empty() {
            return;
        }
        let sanitized = sanitize_doc_prose(&self.prose.join("\n"));
        self.out.extend(sanitized.split('\n').map(str::to_string));
        self.prose.clear();
    }

    /// Takes one already-right-trimmed source line.
    fn push_line(&mut self, line: &'a str) {
        let stripped = line.trim_start();
        if stripped.starts_with("```") && !self.in_indent {
            self.toggle_fence(line, stripped);
            return;
        }
        if self.in_fence {
            self.out.push(line.to_string());
            return;
        }
        let lead = line.len() - stripped.len();
        self.track_indent_block(lead >= 4 && !stripped.is_empty(), stripped.is_empty());
        if self.in_indent {
            self.out.push(line.to_string());
        } else {
            self.prose.push(line);
        }
    }

    /// Opens or closes an explicit fence, tagging a bare opening as `text`.
    fn toggle_fence(&mut self, line: &str, stripped: &str) {
        self.flush_prose();
        if self.in_fence {
            self.in_fence = false;
            self.out.push(line.to_string());
            return;
        }
        self.in_fence = true;
        self.out.push(if stripped == "```" {
            line.replacen("```", "```text", 1)
        } else {
            line.to_string()
        });
    }

    /// Opens or closes the auto-wrapped fence around an indented run.
    fn track_indent_block(&mut self, is_indent_line: bool, is_blank: bool) {
        if is_indent_line && !self.in_indent {
            self.flush_prose();
            self.out.push("```text".to_string());
            self.in_indent = true;
        } else if self.in_indent && !is_indent_line && !is_blank {
            self.out.push("```".to_string());
            self.in_indent = false;
        }
    }

    /// Flushes the tail state and yields the rewritten lines.
    fn finish(mut self) -> Vec<String> {
        self.flush_prose();
        if self.in_indent {
            self.out.push("```".to_string());
        }
        if self.in_fence {
            self.out.push("```".to_string());
        }
        self.out
    }
}

/// Reshapes an over-long first doc paragraph into the RFC 1574 summary form.
///
/// When the first paragraph exceeds `clippy::too_long_first_doc_paragraph`'s
/// 200-character threshold, the first sentence stays as the summary and the
/// remainder moves below a blank line. BMM prose the split cannot apply to
/// (no sentence boundary inside the budget) is left verbatim — the lint then
/// cannot split within the budget gains the caller's synthesized
/// `summary_hint` as its summary paragraph instead (empty hint = verbatim).
fn split_long_first_paragraph(out: &mut Vec<String>, summary_hint: &str) {
    let para_end = out.iter().position(String::is_empty).unwrap_or(out.len());
    let text_len: usize = out
        .iter()
        .take(para_end)
        .map(|l| l.chars().count() + 1)
        .sum::<usize>()
        .saturating_sub(1);
    if text_len <= 200 {
        return;
    }
    let mut budget: usize = 200;
    for li in 0..para_end {
        let Some(line) = out.get(li).cloned() else {
            return;
        };
        if let Some(cut) = sentence_end_within(&line, budget) {
            let (head, tail) = line.split_at_checked(cut).unwrap_or((line.as_str(), ""));
            let head = head.trim_end().to_string();
            let tail = tail.trim_start().to_string();
            let replacement = if tail.is_empty() {
                vec![head, String::new()]
            } else {
                vec![head, String::new(), tail]
            };
            out.splice(li..=li, replacement);
            return;
        }
        // A line that ends the sentence at its very end splits between lines.
        let trimmed = line.trim_end();
        if trimmed.ends_with(['.', '!', '?']) && trimmed.chars().count() <= budget {
            out.insert(li + 1, String::new());
            return;
        }
        budget = budget.saturating_sub(line.chars().count() + 1);
        if budget == 0 {
            break;
        }
    }
    // No sentence boundary inside the budget: prepend the synthesized summary
    // when the caller supplied one; otherwise leave the prose verbatim.
    if !summary_hint.is_empty() {
        out.insert(0, String::new());
        out.insert(0, summary_hint.to_string());
    }
}

/// Returns the byte index just past the first sentence terminator that ends a
/// sentence within the first `budget` characters of `line`, if any.
///
/// A terminator is `.`/`!`/`?` followed by a space, excluding the common
/// abbreviations spec prose uses (`e.g.`, `i.e.`, `etc.`, `vs.`, `cf.`).
fn sentence_end_within(line: &str, budget: usize) -> Option<usize> {
    let mut chars_seen: usize = 0;
    let mut prev_word_end: Option<usize> = None;
    let mut iter = line.char_indices().peekable();
    while let Some((i, c)) = iter.next() {
        chars_seen += 1;
        if chars_seen > budget {
            return None;
        }
        if matches!(c, '.' | '!' | '?') && iter.peek().is_some_and(|&(_, n)| n == ' ') {
            let word_start = prev_word_end.map_or(0, |w| w + 1);
            let word = line.get(word_start..i).unwrap_or_default();
            let abbrev = matches!(
                word.trim_start_matches('(').to_ascii_lowercase().as_str(),
                "e.g" | "i.e" | "etc" | "vs" | "cf" | "viz" | "resp" | "incl"
            );
            if !abbrev {
                return Some(i + c.len_utf8());
            }
        }
        if c == ' ' {
            prev_word_end = Some(i);
        }
    }
    None
}

/// Make verbatim openEHR spec prose safe for the workspace's deny-level rustdoc
/// lints, leaving code spans untouched (rustdoc never lints inside them):
///
/// - a bare URL becomes an autolink `<url>`, and the asciidoc `url[label]` form
///   becomes a Markdown link (`rustdoc::bare_urls`);
/// - a literal `[…]` is escaped `\[…\]`, so prose is not read as a broken
///   intra-doc link (`rustdoc::broken_intra_doc_links`);
/// - a literal `<…>` is escaped `\<…\>`, so `"name <email>"` / `VERSION<T>` in
///   prose is not read as an HTML tag (`rustdoc::invalid_html_tags`);
/// - an unpaired backtick run is escaped, so it opens no dangling code span
///   (`rustdoc::unescaped_backticks`).
fn sanitize_doc_prose(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        match c {
            '`' => rest = copy_code_span(rest, &mut out),
            'h' if rest.starts_with("http://") || rest.starts_with("https://") => {
                rest = copy_url(rest, &mut out);
            }
            '[' | ']' | '<' | '>' => {
                out.push('\\');
                out.push(c);
                rest = after(rest, c.len_utf8());
            }
            c => {
                out.push(c);
                rest = after(rest, c.len_utf8());
            }
        }
    }
    out
}

/// Copies a backtick run to `out`, returning the unconsumed remainder.
///
/// A matched pair delimits a code span: it is copied verbatim, closing run
/// included. An unmatched run is escaped instead.
fn copy_code_span<'a>(rest: &'a str, out: &mut String) -> &'a str {
    let open = backtick_run(rest);
    if let Some(offset) = find_backtick_run(after(rest, open), open) {
        let end = open + offset + open;
        out.push_str(upto(rest, end));
        return after(rest, end);
    }
    for _ in 0..open {
        out.push_str("\\`");
    }
    after(rest, open)
}

/// Copies a URL to `out` as Markdown, returning the unconsumed remainder.
///
/// The asciidoc link form `https://host/path[label]` becomes a Markdown link. A
/// parenthesis in the URL would end the Markdown destination early, so such a
/// link stays an autolink with escaped brackets.
fn copy_url<'a>(rest: &'a str, out: &mut String) -> &'a str {
    let url = read_url(rest);
    let tail = after(rest, url.len());
    let plain_dest = !url.contains(['(', ')']);
    if let Some((consumed, label)) = read_link_label(tail).filter(|_| plain_dest) {
        out.push_str(&format!("[{label}]({url})"));
        return after(tail, consumed);
    }
    out.push_str(&format!("<{url}>"));
    tail
}

/// `s` after its first `n` bytes — total (empty when `n` is out of range or not
/// on a char boundary), so no slicing can panic.
fn after(s: &str, n: usize) -> &str {
    s.get(n..).unwrap_or_default()
}

/// The first `n` bytes of `s` — total, like [`after`].
fn upto(s: &str, n: usize) -> &str {
    s.get(..n).unwrap_or_default()
}

/// The length of the backtick run at the start of `s`.
fn backtick_run(s: &str) -> usize {
    s.chars().take_while(|&c| c == '`').count()
}

/// The byte offset in `s` of the next backtick run of exactly `n`, if any.
fn find_backtick_run(s: &str, n: usize) -> Option<usize> {
    let mut rest = s;
    let mut base = 0;
    while let Some(idx) = rest.find('`') {
        let run = backtick_run(after(rest, idx));
        if run == n {
            return Some(base + idx);
        }
        base += idx + run;
        rest = after(rest, idx + run);
    }
    None
}

/// The URL at the start of `s`: up to whitespace or a delimiter, with trailing
/// sentence punctuation left outside the link.
fn read_url(s: &str) -> &str {
    let len = s
        .chars()
        .take_while(|&c| {
            !c.is_whitespace() && !matches!(c, '`' | '[' | ']' | '<' | '>' | '"' | '\'')
        })
        .map(char::len_utf8)
        .sum();
    let mut url = upto(s, len);
    while let Some(trimmed) = url.strip_suffix(['.', ',', ';', ':', ')']) {
        url = trimmed;
    }
    url
}

/// An asciidoc link label `[label]` at the start of `s`, if it is usable as
/// Markdown link text (no nested bracket or code span). Returns
/// `(bytes consumed incl. both brackets, label)`.
fn read_link_label(s: &str) -> Option<(usize, &str)> {
    let body = s.strip_prefix('[')?;
    let close = body.find(']')?;
    let label = upto(body, close);
    if label.is_empty() || label.contains(['[', '`']) {
        return None;
    }
    Some((close + 2, label))
}

/// `DV_QUANTITY` → `dv_quantity`, `Iso8601_date` → `iso8601_date`.
fn to_snake(spec: &str) -> String {
    spec.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{doc_block, sanitize_doc_prose};

    #[test]
    fn bare_url_becomes_an_autolink() {
        assert_eq!(
            sanitize_doc_prose("specified by https://www.rfc-editor.org/rfc/rfc1034."),
            "specified by <https://www.rfc-editor.org/rfc/rfc1034>.",
        );
    }

    #[test]
    fn asciidoc_link_becomes_a_markdown_link() {
        assert_eq!(
            sanitize_doc_prose("Coded using https://example.org/t.xml[openEHR vocabulary]."),
            "Coded using [openEHR vocabulary](https://example.org/t.xml).",
        );
    }

    #[test]
    fn prose_brackets_and_angle_brackets_are_escaped() {
        assert_eq!(
            sanitize_doc_prose(r#"in "name <email>" form, table [ [ String ] ]"#),
            r#"in "name \<email\>" form, table \[ \[ String \] \]"#,
        );
    }

    #[test]
    fn code_spans_are_copied_verbatim() {
        // rustdoc lints nothing inside a code span, so `Interval<T>` and a URL in
        // backticks must survive untouched — including a span crossing lines.
        assert_eq!(
            sanitize_doc_prose("substitutable for `Interval<T>` where needed"),
            "substitutable for `Interval<T>` where needed",
        );
        assert_eq!(
            sanitize_doc_prose("formed by\n\n`create_schema_id(\n  publisher)`\n\ne.g. x"),
            "formed by\n\n`create_schema_id(\n  publisher)`\n\ne.g. x",
        );
    }

    #[test]
    fn an_unpaired_backtick_is_escaped() {
        assert_eq!(
            sanitize_doc_prose("the property `tuple_constraint', and comes"),
            "the property \\`tuple_constraint', and comes",
        );
    }

    #[test]
    fn fenced_and_indented_blocks_are_left_alone() {
        let mut out = String::new();
        doc_block(
            &mut out,
            Some("Values:\n\n```\n\"YYYY-MM-DD\" -- [full]\n```"),
            "",
        );
        assert_eq!(
            out,
            "/// Values:\n///\n/// ```text\n/// \"YYYY-MM-DD\" -- [full]\n/// ```\n",
        );
    }
}
