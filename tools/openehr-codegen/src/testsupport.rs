// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The pipeline's facts for the emitter-invariant tests
//! (`tests/emitter_invariants.rs`), run on the real vendored inputs.

#![expect(
    clippy::disallowed_types,
    reason = "dev tooling over JSON artifacts (vendored BMM/OAS bundles, emitter reports) — not the \
              application (#1694)"
)]
use crate::analyze::invariants::{self, Bucket};
use crate::analyze::reemit::{augment_with_reemit, cross_schema_reemit};
use crate::analyze::{Model, class_paths};
use crate::cli;
use crate::load::bmm::{BmmPropKind, BmmSchema, BmmType};
use crate::load::impls::SiblingImpls;
use crate::load::oas;
use crate::plan::composition::{self, ComposedUnit, compose};
use crate::plan::overrides;
use crate::plan::{Emission, decide};
use crate::render::emit::{CrateGeneration, RenderUnit, emit_composed};
use crate::render::{emit_rest, model_query, naming};
use std::collections::{BTreeMap, BTreeSet};

pub mod decision_checks;
pub mod shapes;

type Error = Box<dyn std::error::Error>;

/// The composition keys, in emission order.
#[must_use]
pub fn crate_keys() -> Vec<&'static str> {
    composition::COMPOSITIONS.iter().map(|c| c.key).collect()
}

/// One generation row of a composition entry, flattened for the integrity
/// invariant.
#[derive(Debug, Clone)]
pub struct GenerationInfo {
    /// The emitted generation-module name (`v1_2`).
    pub module: String,
    /// The generation's implemented spec version.
    pub spec_version: String,
    /// The vendored spec files composing the generation, each with its
    /// prelude membership.
    pub units: Vec<(String, bool)>,
    /// Whether this is the crate's current generation.
    pub current: bool,
    /// Paired dependency generations merged into the model, as
    /// `(key, generation)` pairs in merge order.
    pub model_deps: Vec<(String, String)>,
    /// Paired dependency generations resolving cross-crate references, as
    /// `(key, generation)` pairs in lookup order.
    pub prelude_deps: Vec<(String, String)>,
}

/// One crate → generation composition entry, flattened for the integrity
/// invariant (the table is itself declarative decision data).
#[derive(Debug, Clone)]
pub struct CompositionInfo {
    /// The composition key.
    pub key: String,
    /// The emitted crate directory.
    pub crate_name: String,
    /// The crate's BMM generations, oldest first.
    pub generations: Vec<GenerationInfo>,
    /// The `includes` citation behind the composition.
    pub citation: String,
    /// The one-line reason.
    pub reason: String,
}

/// One specification unit's `includes`, as the vendored BMM file declares
/// them.
#[derive(Debug, Clone)]
pub struct UnitIncludes {
    /// The composition key.
    pub key: String,
    /// The generation-module name.
    pub module: String,
    /// The vendored BMM file.
    pub file: String,
    /// The `includes` schema ids (`openehr_base_1.3.0`), in file order.
    pub includes: Vec<String>,
}

/// The `includes` of every specification unit of every composition generation,
/// read from the vendored files.
///
/// # Errors
/// Returns an error if a vendored BMM file cannot be loaded.
pub fn unit_includes() -> Result<Vec<UnitIncludes>, Error> {
    let mut out = Vec::new();
    for c in composition::COMPOSITIONS {
        for g in c.generations {
            for u in g.units {
                out.push(UnitIncludes {
                    key: c.key.to_string(),
                    module: g.module.to_string(),
                    file: u.file.to_string(),
                    includes: composition::load_bmm(u.file)?.includes,
                });
            }
        }
    }
    Ok(out)
}

/// Every crate → generation composition entry.
#[must_use]
pub fn composition_infos() -> Vec<CompositionInfo> {
    let deps = |d: &[composition::DepGeneration]| {
        d.iter()
            .map(|d| (d.key.to_string(), d.generation.to_string()))
            .collect()
    };
    composition::COMPOSITIONS
        .iter()
        .map(|c| CompositionInfo {
            key: c.key.to_string(),
            crate_name: c.crate_name.to_string(),
            generations: c
                .generations
                .iter()
                .map(|g| GenerationInfo {
                    module: g.module.to_string(),
                    spec_version: g.spec_version.to_string(),
                    units: g
                        .units
                        .iter()
                        .map(|u| (u.file.to_string(), u.in_prelude))
                        .collect(),
                    current: g.current,
                    model_deps: deps(g.model_deps),
                    prelude_deps: deps(g.prelude_deps),
                })
                .collect(),
            citation: c.citation.to_string(),
            reason: c.reason.to_string(),
        })
        .collect()
}

// ── completeness ────────────────────────────────────────────────────────────

/// Per-generation class-count breakdown for the completeness invariant.
///
/// Counted per generation because a merged class map hides a cross-generation
/// name collision.
#[derive(Debug, Clone)]
pub struct Completeness {
    /// The composition key.
    pub key: String,
    /// The vendored BMM file this generation loads.
    pub file: String,
    /// Total classes this generation declares.
    pub total: usize,
    /// Classes planned (emitted, `decide` ≠ `Skip`).
    pub planned: usize,
    /// Classes skipped because they are mapped to Rust (primitive / container /
    /// marker / functional / service / constant-holder).
    pub skipped_mapped: usize,
    /// Classes skipped because they are abstract, have no concrete descendants
    /// and are never a field type.
    pub skipped_abstract_unused: usize,
    /// Classes skipped for neither sanctioned reason; the invariant requires none.
    pub silently_dropped: Vec<String>,
}

/// Computes the completeness breakdown of every BMM generation composing one
/// crate, in declaration order.
///
/// # Errors
/// Returns an error if the crate's BMM files cannot be loaded.
pub fn completeness(key: &str) -> Result<Vec<Completeness>, Error> {
    let c = compose(key)?;
    let mut out = Vec::new();
    for g in &c.generations {
        for u in &g.units {
            let mut breakdown = unit_completeness(&u.model, &u.schema);
            breakdown.key = key.to_string();
            breakdown.file = u.spec.file.to_string();
            out.push(breakdown);
        }
    }
    Ok(out)
}

/// The completeness breakdown of one BMM specification unit.
fn unit_completeness(model: &Model, schema: &BmmSchema) -> Completeness {
    let used = model.used_as_type();
    let mut planned = 0;
    let mut skipped_mapped = 0;
    let mut skipped_abstract_unused = 0;
    let mut silently_dropped = Vec::new();
    for (name, class) in &schema.classes {
        if !matches!(decide(model, class, &used), Emission::Skip) {
            planned += 1;
            continue;
        }
        if Model::is_mapped(name) {
            skipped_mapped += 1;
        } else if class.is_abstract && model.enum_variants(name).is_empty() && !used.contains(name)
        {
            skipped_abstract_unused += 1;
        } else {
            silently_dropped.push(name.clone());
        }
    }
    silently_dropped.sort();
    Completeness {
        key: String::new(),
        file: String::new(),
        total: schema.classes.len(),
        planned,
        skipped_mapped,
        skipped_abstract_unused,
        silently_dropped,
    }
}

/// One BMM-declared attribute that reaches no emitted Rust field.
#[derive(Debug, Clone)]
pub struct AttributeGap {
    /// The vendored BMM file declaring it.
    pub file: String,
    /// The declaring class.
    pub class: String,
    /// The declared attribute (BMM property name).
    pub attribute: String,
    /// Why it is a gap (which emitted type was expected to carry it).
    pub detail: String,
}

/// Every BMM-declared attribute of a crate's generations that reaches no
/// emitted struct field, with the number of `(class, attribute)` pairs checked.
///
/// A back-reference omitted by the `back_reference` map is not a gap.
///
/// # Errors
/// Returns an error if the crate's BMM files cannot be loaded.
pub fn attribute_gaps(key: &str) -> Result<(Vec<AttributeGap>, usize), Error> {
    let c = compose(key)?;
    let mut gaps = Vec::new();
    let mut checked = 0_usize;
    for g in &c.generations {
        for u in &g.units {
            checked += unit_attribute_gaps(u, &mut gaps);
        }
    }
    Ok((gaps, checked))
}

/// Collects one specification unit's attribute gaps into `gaps`, returning how
/// many `(class, attribute, carrier)` triples were checked.
fn unit_attribute_gaps(u: &ComposedUnit, gaps: &mut Vec<AttributeGap>) -> usize {
    let used = u.model.used_as_type();
    let mut checked = 0_usize;
    for (name, class) in &u.schema.classes {
        checked += class_attribute_gaps(u, name, class, &used, gaps);
    }
    checked
}

/// Collects one class's attribute gaps into `gaps`, returning how many
/// `(attribute, carrier)` pairs were checked.
fn class_attribute_gaps(
    u: &ComposedUnit,
    name: &str,
    class: &crate::load::bmm::BmmClass,
    used: &BTreeSet<String>,
    gaps: &mut Vec<AttributeGap>,
) -> usize {
    let carriers = attribute_carriers(name, class, &u.model, used);
    let mut checked = 0_usize;
    for p in &class.properties {
        if overrides::back_reference(name, &p.name).is_some() {
            continue;
        }
        for carrier in &carriers {
            checked += 1;
            if !emitted_field_names(&u.model, carrier).contains(&p.name) {
                gaps.push(AttributeGap {
                    file: u.spec.file.to_string(),
                    class: name.to_owned(),
                    attribute: p.name.clone(),
                    detail: format!("missing from the emitted `{carrier}` fields"),
                });
            }
        }
    }
    checked
}

/// The emitted types that must carry a class's declared attributes: the struct
/// itself, or each concrete variant of a closed subtype set.
fn attribute_carriers(
    name: &str,
    class: &crate::load::bmm::BmmClass,
    model: &Model,
    used: &BTreeSet<String>,
) -> Vec<String> {
    match decide(model, class, used) {
        Emission::Struct | Emission::PolyEnum(_) => vec![name.to_owned()],
        Emission::Enum(variants) => variants,
        Emission::EnumLiterals(_) | Emission::Newtype(_) | Emission::Skip => Vec::new(),
    }
}

/// The emitted field names of a class, flattened, back-references omitted.
fn emitted_field_names(model: &Model, class_name: &str) -> BTreeSet<String> {
    model.get(class_name).map_or_else(BTreeSet::new, |cls| {
        model
            .flattened_props(cls)
            .iter()
            .filter(|rp| overrides::back_reference(&rp.owner, &rp.prop.name).is_none())
            .map(|rp| rp.prop.name.clone())
            .collect()
    })
}

/// A file path or prelude identifier claimed by more than one BMM generation of
/// the same crate.
#[derive(Debug, Clone)]
pub struct GenerationConflict {
    /// The composition key.
    pub key: String,
    /// The conflicting artifact (an emitted `src/` path, or a prelude ident).
    pub what: String,
    /// The vendored BMM files that both claim it.
    pub files: Vec<String>,
}

/// Emitted-path conflicts between the BMM generations composing each crate;
/// the invariant requires none.
///
/// # Errors
/// Returns an error if any composition's BMM files cannot be loaded.
pub fn generation_conflicts() -> Result<Vec<GenerationConflict>, Error> {
    let mut out = Vec::new();
    for comp in composition::COMPOSITIONS {
        let c = compose(comp.key)?;
        let mut paths: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let gens: Vec<CrateGeneration<'_>> = c
            .generations
            .iter()
            .map(|g| CrateGeneration {
                spec: g.spec,
                units: g
                    .units
                    .iter()
                    .map(|u| RenderUnit {
                        spec: u.spec,
                        model: &u.model,
                        schema: &u.schema,
                    })
                    .collect(),
                external: &g.external,
            })
            .collect();
        // Path conflicts do not depend on the banner input, so an empty sibling set
        // suffices; `lib.rs` and `prelude.rs` are assembled once per crate.
        for f in emit_composed(comp, &gens, &SiblingImpls::default()) {
            let file = c
                .generations
                .iter()
                .find(|g| f.path.starts_with(&format!("{}/", g.spec.module)))
                .map_or("(crate-level)", |g| g.spec.module);
            paths.entry(f.path).or_default().push(file.to_string());
        }
        for (what, files) in paths {
            if files.len() > 1 {
                out.push(GenerationConflict {
                    key: comp.key.to_string(),
                    what,
                    files,
                });
            }
        }
    }
    Ok(out)
}

// ── invariant classification (assertion-dialect analyzer) ────────────────────

/// One classified BMM invariant with the bucket the assertion-dialect analyzer
/// assigns it.
#[derive(Debug, Clone)]
pub struct ClassifiedInvariant {
    /// The generation module (`v1_2`) whose schema declares it.
    pub generation: String,
    /// The owning BMM class name.
    pub class: String,
    /// The BMM invariant name.
    pub name: String,
    /// The verbatim assertion expression.
    pub expr: String,
    /// The bucket: `"emitted"`, `"runtime-hook-missing"`, or `"complex"`.
    pub bucket: &'static str,
    /// The hook/complex reason (empty for `"emitted"`).
    pub reason: String,
}

/// Classifies every `BMM_CLASS.invariants` expression in a crate's own schema,
/// sorted by (class, name).
///
/// # Errors
/// Returns an error if the crate's BMM files cannot be loaded.
pub fn classify_invariants(key: &str) -> Result<Vec<ClassifiedInvariant>, Error> {
    let c = compose(key)?;
    let mut out = Vec::new();
    for g in &c.generations {
        for u in &g.units {
            for (class, def) in &u.schema.classes {
                for (name, expr) in &def.invariants {
                    let (bucket, reason) = match invariants::classify(expr) {
                        Bucket::Emitted => ("emitted", String::new()),
                        Bucket::RuntimeHookMissing(r) => ("runtime-hook-missing", r.to_string()),
                        Bucket::Complex(r) => ("complex", r.to_string()),
                    };
                    out.push(ClassifiedInvariant {
                        generation: g.spec.module.to_string(),
                        class: class.clone(),
                        name: name.clone(),
                        expr: expr.clone(),
                        bucket,
                        reason,
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| (&a.generation, &a.class, &a.name).cmp(&(&b.generation, &b.class, &b.name)));
    Ok(out)
}

// ── constructibility ────────────────────────────────────────────────────────

/// The concrete classes of a crate that an unbroken mandatory construction
/// cycle makes non-constructible; the invariant requires none.
///
/// # Errors
/// Returns an error if the crate's BMM files cannot be loaded.
pub fn constructibility_offenders(key: &str) -> Result<Vec<String>, Error> {
    let c = compose(key)?;
    let mut out: Vec<String> = c
        .generations
        .iter()
        .flat_map(|g| &g.units)
        .flat_map(|u| u.model.constructibility_violations(&u.schema))
        .collect();
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

// ── determinism ─────────────────────────────────────────────────────────────

/// Renders every generated spec crate's `src/` tree (`emit`) in memory, keyed
/// by path relative to `crates/`, byte-equal to the on-disk file before
/// `rustfmt`.
///
/// # Errors
/// Returns an error if a composition's BMM files cannot be loaded, or if a
/// template stamp collides with a generated file.
pub fn emit_to_memory() -> Result<BTreeMap<String, String>, Error> {
    Ok(to_memory(cli::render_emit_files()?))
}

/// Renders the `emit-json` output in memory, keyed by path relative to
/// `crates/`.
///
/// # Errors
/// Returns an error if a composition's BMM files cannot be loaded.
pub fn emit_json_to_memory() -> Result<BTreeMap<String, String>, Error> {
    Ok(to_memory(cli::render_json_files()?))
}

/// Renders the `emit-xml` output in memory, keyed by path relative to
/// `crates/`.
///
/// # Errors
/// Returns an error if the RM/BASE compositions or the vendored XSD bundles
/// cannot be loaded, or if the XML emission itself fails.
pub fn emit_xml_to_memory() -> Result<BTreeMap<String, String>, Error> {
    Ok(to_memory(cli::render_xml_files()?.files))
}

/// Renders the `emit-rest` output in memory, keyed by path relative to
/// `crates/`.
///
/// # Errors
/// Returns an error if the RM/BASE compositions or a vendored OAS bundle cannot
/// be loaded.
pub fn emit_rest_to_memory() -> Result<BTreeMap<String, String>, Error> {
    Ok(to_memory(cli::render_rest_files()?))
}

/// Renders the `emit-opt` output in memory, keyed by path relative to
/// `crates/`.
///
/// # Errors
/// Returns an error if the RM/BASE compositions or the AM/OPT XSD closure cannot
/// be loaded.
pub fn emit_opt_to_memory() -> Result<BTreeMap<String, String>, Error> {
    Ok(to_memory(cli::render_opt_files()?.files))
}

/// Renders the `emit-aom2` output in memory, keyed by path relative to
/// `crates/`.
///
/// # Errors
/// Returns an error if the RM/BASE compositions or either AOM2 XSD closure
/// cannot be loaded.
pub fn emit_aom2_to_memory() -> Result<BTreeMap<String, String>, Error> {
    Ok(to_memory(cli::render_aom2_files()?.files))
}

/// Renders the `emit-rm-model` output in memory, keyed by path relative to
/// `crates/`.
///
/// # Errors
/// Returns an error if the RM composition's BMM files cannot be loaded.
pub fn emit_rm_model_to_memory() -> Result<BTreeMap<String, String>, Error> {
    Ok(to_memory(cli::render_rm_model_files()?))
}

/// Renders the `emit-validate` output in memory, keyed by path relative to
/// `crates/`.
///
/// # Errors
/// Returns an error if the RM composition's BMM files cannot be loaded.
pub fn emit_validate_to_memory() -> Result<BTreeMap<String, String>, Error> {
    Ok(to_memory(cli::render_validate_files()?))
}

/// Maps rendered files to path and bytes through [`cli::generated_bytes`].
fn to_memory(files: Vec<cli::EmittedFile>) -> BTreeMap<String, String> {
    files
        .into_iter()
        .map(|f| {
            let bytes = cli::generated_bytes(f.crate_name, &f.body);
            (f.path, bytes)
        })
        .collect()
}

// ── cross-schema re-emission (source-package mirroring + downstream closure) ──

/// One re-emitted cross-schema class with its upstream and downstream package
/// paths.
#[derive(Debug, Clone)]
pub struct Mirror {
    /// The re-emitted class name.
    pub class: String,
    /// Its package path in the upstream source schema (BASE/LANG).
    pub source_path: Option<String>,
    /// Its package path in the augmented downstream (AM 2.4) schema.
    pub augmented_path: Option<String>,
}

/// One generation's declared model as `CLASS` → attribute name →
/// `attribute_signature` text.
pub type GenerationAttributeMap = BTreeMap<String, BTreeMap<String, String>>;

/// The model delta between two generations of one crate, in BOTH directions.
#[derive(Debug, Clone)]
pub struct GenerationDelta {
    /// Class names the newer generation declares and the older does not.
    pub classes_added: Vec<String>,
    /// `CLASS.attribute` pairs the newer generation declares on classes both
    /// generations share, absent from the older.
    pub attributes_added: Vec<String>,
    /// Class names the older generation declares and the newer does not.
    pub classes_removed: Vec<String>,
    /// `CLASS.attribute` pairs the older generation declares on classes both
    /// generations share, absent from the newer.
    pub attributes_removed: Vec<String>,
    /// Attributes both generations declare on a shared class whose signature
    /// differs, as `CLASS.attribute: <older> -> <newer>`.
    pub attributes_changed: Vec<String>,
}

/// The canonical signature of one declared BMM property.
///
/// `[?]<type>` for a single property, `[?]<container><item> [lower..upper]`
/// for a container; `?` marks `0..1`. Examples: `ITEM_TREE`, `?DV_TEXT`,
/// `List<LINK> [0..*]`.
#[must_use]
fn attribute_signature(property: &crate::load::bmm::BmmProperty) -> String {
    let existence = if property.is_mandatory { "" } else { "?" };
    match &property.kind {
        BmmPropKind::Single(t) => format!("{existence}{}", type_text(t)),
        BmmPropKind::Container {
            container_type,
            item,
            cardinality,
        } => {
            let bounds = cardinality.as_ref().map_or_else(
                || " [unstated]".to_owned(),
                |c| {
                    let upper = c.upper.map_or_else(|| "*".to_owned(), |u| u.to_string());
                    format!(" [{}..{upper}]", c.lower)
                },
            );
            format!("{existence}{container_type}<{}>{bounds}", type_text(item))
        }
    }
}

/// A BMM type reference as text (`DV_INTERVAL<DV_QUANTITY>`).
fn type_text(t: &BmmType) -> String {
    match t {
        BmmType::Simple(name) => name.clone(),
        BmmType::Generic { root, params } => {
            let args: Vec<String> = params.iter().map(type_text).collect();
            format!("{root}<{}>", args.join(","))
        }
    }
}

/// The declared attribute signatures of one generation of crate `key`, units
/// folded last-wins.
///
/// # Errors
/// Returns an error if the composition or the generation fails to load.
pub fn generation_attribute_map(key: &str, module: &str) -> Result<GenerationAttributeMap, Error> {
    let c = compose(key)?;
    let g = find_generation(&c, module)?;
    let mut out = GenerationAttributeMap::new();
    for u in &g.units {
        for (name, class) in &u.schema.classes {
            out.insert(
                name.clone(),
                class
                    .properties
                    .iter()
                    .map(|p| (p.name.clone(), attribute_signature(p)))
                    .collect(),
            );
        }
    }
    Ok(out)
}

/// Computes the model delta between two loaded generation maps.
#[must_use]
pub fn attribute_delta(
    older: &GenerationAttributeMap,
    newer: &GenerationAttributeMap,
) -> GenerationDelta {
    let classes_added: Vec<String> = newer
        .keys()
        .filter(|k| !older.contains_key(*k))
        .cloned()
        .collect();
    let classes_removed: Vec<String> = older
        .keys()
        .filter(|k| !newer.contains_key(*k))
        .cloned()
        .collect();
    let mut attributes_added = Vec::new();
    let mut attributes_removed = Vec::new();
    let mut attributes_changed = Vec::new();
    for (class, new_attrs) in newer {
        let Some(old_attrs) = older.get(class) else {
            continue;
        };
        for (name, new_signature) in new_attrs {
            match old_attrs.get(name) {
                None => attributes_added.push(format!("{class}.{name}")),
                Some(old_signature) if old_signature != new_signature => attributes_changed.push(
                    format!("{class}.{name}: {old_signature} -> {new_signature}"),
                ),
                Some(_) => {}
            }
        }
        for name in old_attrs.keys() {
            if !new_attrs.contains_key(name) {
                attributes_removed.push(format!("{class}.{name}"));
            }
        }
    }
    GenerationDelta {
        classes_added,
        attributes_added,
        classes_removed,
        attributes_removed,
        attributes_changed,
    }
}

/// Computes the attribute-signature delta from generation `older` to `newer`
/// of crate `key`.
///
/// # Errors
/// Returns an error if the composition or either generation fails to load.
pub fn generation_attribute_delta(
    key: &str,
    older: &str,
    newer: &str,
) -> Result<GenerationDelta, Error> {
    Ok(attribute_delta(
        &generation_attribute_map(key, older)?,
        &generation_attribute_map(key, newer)?,
    ))
}

/// Find one generation of a composed crate by its module name.
fn find_generation<'a>(
    c: &'a composition::Composed,
    module: &str,
) -> Result<&'a composition::ComposedGeneration, Error> {
    c.generations
        .iter()
        .find(|g| g.spec.module == module)
        .ok_or_else(|| format!("composition {:?} has no generation {module:?}", c.comp.key).into())
}

/// The AM 2.4 re-emission closure with each class's source and augmented
/// package paths.
///
/// # Errors
/// Returns an error if the AM/BASE/LANG BMM files cannot be loaded.
pub fn v2_4_reemit_mirrors() -> Result<Vec<Mirror>, Error> {
    let am = compose("am")?;
    let v2_4 = find_generation(&am, "v2_4")?;
    let unit = v2_4.unit()?;
    let reemit = cross_schema_reemit(&unit.model, &unit.schema);
    let dep_refs: Vec<&BmmSchema> = v2_4.dep_schemas.iter().collect();
    let aug = augment_with_reemit(&unit.schema, &unit.model, &reemit, &dep_refs);
    let aug_paths = class_paths(&aug);

    // First wins, in the order `augment_with_reemit` grafts.
    let mut source_paths: BTreeMap<String, String> = BTreeMap::new();
    for dep in &v2_4.dep_schemas {
        for (cls, path) in class_paths(dep) {
            source_paths.entry(cls).or_insert(path);
        }
    }

    Ok(reemit
        .iter()
        .map(|class| Mirror {
            class: class.clone(),
            source_path: source_paths.get(class).cloned(),
            augmented_path: aug_paths.get(class).cloned(),
        })
        .collect())
}

/// The raw re-emission closure `analyze::cross_schema_reemit` reports for
/// generation `module` of crate `key`.
///
/// # Errors
/// Returns an error if the composition's BMM files cannot be loaded or
/// `module` names no generation of `key`.
pub fn reemit_closure(key: &str, module: &str) -> Result<BTreeSet<String>, Error> {
    let c = compose(key)?;
    let g = find_generation(&c, module)?;
    Ok(g.units
        .iter()
        .flat_map(|u| cross_schema_reemit(&u.model, &u.schema))
        .collect())
}

/// The type files (`"<crate>/<path>"`) rendered for crate `key` alone.
///
/// # Errors
/// Returns an error if the crate's BMM files cannot be loaded.
pub fn rendered_files(key: &str) -> Result<Vec<String>, Error> {
    let c = compose(key)?;
    let gens: Vec<CrateGeneration<'_>> = c
        .generations
        .iter()
        .map(|g| CrateGeneration {
            spec: g.spec,
            units: g
                .units
                .iter()
                .map(|u| RenderUnit {
                    spec: u.spec,
                    model: &u.model,
                    schema: &u.schema,
                })
                .collect(),
            external: &g.external,
        })
        .collect();
    let files = emit_composed(c.comp, &gens, &SiblingImpls::default());
    Ok(files.into_iter().map(|f| f.path).collect())
}

// ── decision-map integrity ──────────────────────────────────────────────────

/// One declarative decision entry, flattened for integrity checks.
#[derive(Debug, Clone)]
pub struct DeclEntry {
    /// The lookup key (`CLASS`, `CLASS.field`, or a primitive/class name).
    pub key: String,
    /// The decision the entry encodes.
    pub decision: String,
    /// The spec citation, or the explicit our-own-design flag.
    pub citation: String,
    /// The one-line reason.
    pub reason: String,
}

/// Every declarative decision entry, tagged by its source map.
#[derive(Debug, Clone)]
pub struct DeclMap {
    /// The map's name (`back_reference`, `class_binding`, …).
    pub map: &'static str,
    /// Whether the entry's `(class[, field])` must exist in a loaded schema.
    pub check_existence: bool,
    /// The entries.
    pub entries: Vec<DeclEntry>,
}

/// Every declarative decision map, flattened for the integrity invariant.
#[must_use]
pub fn decision_maps() -> Vec<DeclMap> {
    vec![
        DeclMap {
            map: "back_reference",
            check_existence: true,
            entries: overrides::BACK_REFERENCES
                .iter()
                .map(|e| DeclEntry {
                    key: format!("{}.{}", e.class, e.field),
                    decision: "owner/parent back-reference (omitted from struct)".to_string(),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "ref_target",
            check_existence: true,
            entries: overrides::REF_TARGETS
                .iter()
                .map(|e| DeclEntry {
                    key: format!("{}.{}", e.class, e.attr),
                    decision: e.target.map_or_else(
                        || "the RM text names no target class".to_string(),
                        |t| format!("references {t}"),
                    ),
                    citation: e.citation.to_string(),
                    reason: "The BMM types the attribute by the reference class alone; the \
                             target is stated only in the class text."
                        .to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "rest_path_identifier",
            check_existence: false,
            entries: overrides::REST_PATH_IDENTIFIERS
                .iter()
                .map(|e| DeclEntry {
                    key: e.component.to_string(),
                    decision: e.class.map_or_else(
                        || "no openEHR identifier".to_string(),
                        |c| format!("carries a {c}"),
                    ),
                    citation: e.citation.to_string(),
                    reason: "The OAS states the identifier class only in the parameter's \
                             description."
                        .to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "rest_discriminator_alias",
            check_existence: false,
            entries: overrides::REST_DISCRIMINATOR_ALIASES
                .iter()
                .map(|e| DeclEntry {
                    key: format!("{}#{}", e.schema, e.tag),
                    decision: "read as the instantiable base".to_string(),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        decision_checks::class_binding_map(),
        decision_checks::elided_argument_map(),
        DeclMap {
            map: "type_override",
            check_existence: true,
            entries: overrides::TYPE_OVERRIDES
                .iter()
                .map(|e| DeclEntry {
                    key: format!("{}.{}", e.class, e.field),
                    decision: e.rust_type.to_string(),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "untyped_field",
            check_existence: true,
            entries: overrides::UNTYPED_FIELDS
                .iter()
                .map(|e| DeclEntry {
                    key: format!("{}.{}", e.class, e.field),
                    decision: "adjudicated free-form JSON (serde_json::Value)".to_string(),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "field_default",
            check_existence: true,
            entries: overrides::FIELD_DEFAULTS
                .iter()
                .map(|e| DeclEntry {
                    key: format!("{}.{}", e.owner, e.field),
                    decision: format!("default = {}", e.default),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        DeclMap {
            // The keys are OAS schema names; `oas_monomorphizations` checks them.
            map: "oas_monomorphization",
            check_existence: false,
            entries: overrides::OAS_MONOMORPHIZATIONS
                .iter()
                .map(|e| DeclEntry {
                    key: e.schema.to_string(),
                    decision: format!("{} → {}", e.title, e.rust_type),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        decision_checks::ordered_hash_map(),
        DeclMap {
            map: "spec_contradiction",
            check_existence: true,
            entries: overrides::SPEC_CONTRADICTIONS
                .iter()
                .map(|e| DeclEntry {
                    key: format!("{}.{}", e.owner, e.field),
                    decision: format!("{:?} not realized", e.kind),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "unrenderable_default",
            check_existence: true,
            entries: overrides::UNRENDERABLE_DEFAULTS
                .iter()
                .map(|e| DeclEntry {
                    key: format!("{}.{}", e.owner, e.field),
                    decision: "vendored `default` facet deliberately not realized".to_string(),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "xml_bmm_only_allowlist",
            check_existence: true,
            entries: crate::plan::xml_overrides::XML_BMM_ONLY_ALLOWLIST
                .iter()
                .map(|e| DeclEntry {
                    key: format!("{}.{}", e.spec, e.wire_name),
                    decision: "append as trailing canonical-XML element".to_string(),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "mapped_classes",
            check_existence: false,
            entries: overrides::MAPPED_CLASSES
                .iter()
                .map(|e| DeclEntry {
                    key: e.name.to_string(),
                    decision: "mapped to Rust / never emitted".to_string(),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "primitives",
            check_existence: false,
            entries: overrides::PRIMITIVES
                .iter()
                .map(|e| DeclEntry {
                    key: e.spec.to_string(),
                    decision: e.rust.to_string(),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "subtype_extensions",
            check_existence: true,
            entries: overrides::SUBTYPE_EXTENSIONS
                .iter()
                .map(|e| DeclEntry {
                    key: e.subtype.to_string(),
                    decision: format!("additional variant of {}", e.parent),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
        DeclMap {
            map: "dialect_predicates",
            // The keys are predicate spellings; the classifier test checks them.
            check_existence: false,
            entries: overrides::DIALECT_PREDICATES
                .iter()
                .map(|e| DeclEntry {
                    key: e.predicate.to_string(),
                    decision: format!("→ {}", e.runtime_fn),
                    citation: e.citation.to_string(),
                    reason: e.reason.to_string(),
                })
                .collect(),
        },
    ]
}

/// The assertion-dialect predicate → runtime-function map (predicate spelling,
/// runtime function), for the lockstep test against the classifier.
#[must_use]
pub fn dialect_predicates() -> Vec<(String, String)> {
    overrides::DIALECT_PREDICATES
        .iter()
        .map(|e| (e.predicate.to_string(), e.runtime_fn.to_string()))
        .collect()
}

/// One emittable class invariant with the venue the realization register
/// (`plan::overrides::INVARIANT_REALIZATIONS`) assigns it.
#[derive(Debug, Clone)]
pub struct AccountedInvariant {
    /// The owning BMM class name.
    pub class: String,
    /// The BMM invariant name.
    pub name: String,
    /// The venue name, or `"UNACCOUNTED"` when the register has no row.
    pub venue: &'static str,
    /// The realizing site; empty for non-realizing venues.
    pub site: String,
    /// The class's vendored spec file, repo-relative; empty when unaccounted.
    pub citation: String,
    /// The one-line reason; empty when unaccounted.
    pub reason: String,
}

/// Accounts a crate's emittable class invariants against the realization
/// register, sorted by `(class, name)`.
///
/// # Errors
/// Returns an error if the crate's BMM files cannot be loaded.
pub fn accounted_emitted_invariants(key: &str) -> Result<Vec<AccountedInvariant>, Error> {
    let c = compose(key)?;
    let triples: Vec<(String, String, String)> = c
        .generations
        .iter()
        .flat_map(|g| &g.units)
        .flat_map(|u| &u.schema.classes)
        .flat_map(|(class, def)| {
            def.invariants
                .iter()
                .map(move |(name, expr)| (class.clone(), name.clone(), expr.clone()))
        })
        .collect();
    Ok(account(
        triples
            .iter()
            .map(|(c, n, e)| (c.as_str(), n.as_str(), e.as_str())),
    ))
}

/// Accounts an explicit `(class, invariant, assertion-expression)` set against
/// the realization register.
#[must_use]
pub fn account_invariants(triples: &[(&str, &str, &str)]) -> Vec<AccountedInvariant> {
    account(triples.iter().copied())
}

/// Accounts the invariants of `key` the classifier judges not mechanically
/// evaluable against the realization register.
///
/// # Errors
///
/// Returns an error for an unknown `key` or unloadable BMM inputs.
pub fn accounted_complex_invariants(key: &str) -> Result<Vec<AccountedInvariant>, Error> {
    let c = compose(key)?;
    let triples: Vec<(String, String, String)> = c
        .generations
        .iter()
        .flat_map(|g| &g.units)
        .flat_map(|u| &u.schema.classes)
        .flat_map(|(class, def)| {
            def.invariants
                .iter()
                .map(move |(name, expr)| (class.clone(), name.clone(), expr.clone()))
        })
        .collect();
    Ok(present(overrides::account_complex(
        triples
            .iter()
            .map(|(c, n, e)| (c.as_str(), n.as_str(), e.as_str())),
    )))
}

/// Shared body of [`accounted_emitted_invariants`] and [`account_invariants`].
fn account<'a>(
    triples: impl Iterator<Item = (&'a str, &'a str, &'a str)>,
) -> Vec<AccountedInvariant> {
    present(overrides::account_emitted(triples))
}

/// Renders accounted rows into the test-facing [`AccountedInvariant`] shape.
fn present(rows: Vec<overrides::AccountedInvariant>) -> Vec<AccountedInvariant> {
    rows.into_iter()
        .map(|a| {
            let venue = a.realization.map_or("UNACCOUNTED", |r| match r.venue {
                overrides::InvariantVenue::Core => "Core",
                overrides::InvariantVenue::Impl => "Impl",
                overrides::InvariantVenue::Wire => "Wire",
                overrides::InvariantVenue::App => "App",
                overrides::InvariantVenue::Excluded => "Excluded",
                overrides::InvariantVenue::Unrealized => "Unrealized",
            });
            AccountedInvariant {
                class: a.class,
                name: a.name,
                venue,
                site: a.realization.map(|r| r.site.to_owned()).unwrap_or_default(),
                citation: a.realization.map_or_else(String::new, |r| {
                    format!("{}/{}", overrides::class_doc_dir(r.spec_file), r.spec_file)
                }),
                reason: a
                    .realization
                    .map(|r| r.reason.to_owned())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

/// The declared additional polymorphic subtype members, as `(parent, subtype)`
/// pairs.
#[must_use]
pub fn subtype_extensions() -> Vec<(String, String)> {
    overrides::SUBTYPE_EXTENSIONS
        .iter()
        .map(|e| (e.parent.to_string(), e.subtype.to_string()))
        .collect()
}

/// The adjudicated free-form (`serde_json::Value`) fields, as
/// `(class, field, citation)` triples.
#[must_use]
pub fn untyped_fields() -> Vec<(String, String, String)> {
    overrides::UNTYPED_FIELDS
        .iter()
        .map(|e| {
            (
                e.class.to_string(),
                e.field.to_string(),
                e.citation.to_string(),
            )
        })
        .collect()
}

/// The immediate concrete variants the emitter gives `class`'s polymorphic slot
/// in the composition `key`, or `None` when that composition's model does not
/// define `class`.
///
/// # Errors
/// Returns an error if the composition's BMM files cannot be loaded.
pub fn enum_variants(key: &str, class: &str) -> Result<Option<Vec<String>>, Error> {
    let c = compose(key)?;
    // Newest generation first.
    Ok(c.generations
        .iter()
        .rev()
        .flat_map(|g| g.units.iter().rev())
        .find(|u| u.model.get(class).is_some())
        .map(|u| {
            let mut variants = u.model.enum_variants(class);
            variants.sort();
            variants
        }))
}

/// The classifier's recognised runtime-backed leaf predicates
/// (`analyze::invariants::RUNTIME_PREDICATES`).
#[must_use]
pub fn runtime_predicates() -> Vec<String> {
    invariants::runtime_predicates()
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Whether `class` exists in any loaded composition model.
///
/// # Errors
/// Returns an error if any composition's BMM files cannot be loaded.
pub fn class_exists(class: &str) -> Result<bool, Error> {
    for key in crate_keys() {
        if compose(key)?
            .generations
            .iter()
            .flat_map(|g| &g.units)
            .any(|u| u.model.get(class).is_some())
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Whether `class` carries a flattened field `field` in any loaded composition
/// model.
///
/// # Errors
/// Returns an error if any composition's BMM files cannot be loaded.
pub fn field_exists(class: &str, field: &str) -> Result<bool, Error> {
    for key in crate_keys() {
        for u in compose(key)?.generations.iter().flat_map(|g| &g.units) {
            if let Some(cls) = u.model.get(class)
                && u.model
                    .flattened_props(cls)
                    .iter()
                    .any(|rp| rp.prop.name == field)
            {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The attribute names composition `key`'s own schema declares on `class`, or
/// `None` if it declares no such class.
///
/// # Errors
/// Returns an error if the composition's BMM files cannot be loaded.
pub fn declared_attributes(key: &str, class: &str) -> Result<Option<BTreeSet<String>>, Error> {
    let c = compose(key)?;
    // Newest generation first (the retired merged-view semantics).
    Ok(c.generations
        .iter()
        .rev()
        .flat_map(|g| g.units.iter().rev())
        .find_map(|u| u.schema.classes.get(class))
        .map(|cls| cls.properties.iter().map(|p| p.name.clone()).collect()))
}

/// One vendored `default` facet, with the emitter's disposition of it.
#[derive(Debug, Clone)]
pub struct VendoredDefault {
    /// Composition key the facet was read from.
    pub key: String,
    /// The DECLARING class.
    pub owner: String,
    /// The property carrying the facet.
    pub field: String,
    /// The facet text, verbatim from the schema.
    pub facet: String,
    /// The Rust expression the emitter derives, or `None` when the facet is not
    /// renderable in the property's type.
    pub rendered: Option<String>,
}

/// Every `default` facet of the vendored schemas with what the emitter makes
/// of it; the hand-written `field_default` table must not overlap it.
///
/// # Errors
/// Returns an error if a vendored BMM file cannot be loaded.
pub fn vendored_defaults() -> Result<Vec<VendoredDefault>, Error> {
    let mut out = Vec::new();
    for key in crate_keys() {
        for u in compose(key)?.generations.iter().flat_map(|g| &g.units) {
            for (owner, class) in &u.schema.classes {
                for prop in &class.properties {
                    if let Some(facet) = &prop.default {
                        out.push(VendoredDefault {
                            key: key.to_string(),
                            owner: owner.clone(),
                            field: prop.name.clone(),
                            facet: facet.clone(),
                            rendered: overrides::vendored_default(prop, |name| u.model.get(name)),
                        });
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Whether `(owner, field)` is an adjudicated un-renderable `default` facet.
#[must_use]
pub fn default_unrenderable(owner: &str, field: &str) -> bool {
    overrides::default_unrenderable(owner, field)
}

/// The adjudicated un-renderable `default` facets as `(owner, field, facets)`,
/// `facets` being the vendored values each entry's citation describes.
#[must_use]
pub fn unrenderable_defaults() -> Vec<(String, String, Vec<String>)> {
    overrides::UNRENDERABLE_DEFAULTS
        .iter()
        .map(|u| {
            (
                u.owner.to_string(),
                u.field.to_string(),
                u.facets.iter().map(ToString::to_string).collect(),
            )
        })
        .collect()
}

/// Resolves a rendered default as `plan::overrides::resolve_default` does.
#[must_use]
pub fn resolve_default(default: &str, resolve: impl Fn(&str) -> String) -> String {
    overrides::resolve_default(default, resolve)
}

/// The hand-written `field_default` residue as `(owner, field)` pairs.
#[must_use]
pub fn hand_written_defaults() -> Vec<(String, String)> {
    overrides::FIELD_DEFAULTS
        .iter()
        .map(|d| (d.owner.to_string(), d.field.to_string()))
        .collect()
}

/// One OAS monomorphization beside the titles the vendored bundles declare.
#[derive(Debug, Clone)]
pub struct OasMonomorphizationCheck {
    /// The OAS `components/schemas` key.
    pub schema: String,
    /// The `title` the decision map claims the schema declares.
    pub declared_title: String,
    /// The generated Rust type the map resolves it to.
    pub rust_type: String,
    /// The vendored `title` values for that key, empty when none declares it.
    pub vendored_titles: BTreeSet<String>,
}

/// Every `OAS_MONOMORPHIZATIONS` entry checked against the vendored bundles'
/// `title` text.
///
/// # Errors
/// Returns an error if a vendored OAS bundle cannot be read or parsed.
pub fn oas_monomorphizations() -> Result<Vec<OasMonomorphizationCheck>, Error> {
    let oas_dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/openehr-its/vendor/rest-oas"
    ))
    .to_path_buf();
    let mut bundles = Vec::new();
    for entry in std::fs::read_dir(&oas_dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|x| x == "yaml") {
            bundles.push(oas::Oas::parse_file(&path)?);
        }
    }
    Ok(overrides::OAS_MONOMORPHIZATIONS
        .iter()
        .map(|m| OasMonomorphizationCheck {
            schema: m.schema.to_string(),
            declared_title: m.title.to_string(),
            rust_type: m.rust_type.to_string(),
            vendored_titles: bundles
                .iter()
                .flat_map(oas::Oas::schemas)
                .filter(|(name, _)| name == m.schema)
                .filter_map(|(_, s)| {
                    s.get("title")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                })
                .collect(),
        })
        .collect())
}

/// The schema names of the shared-module fallback document, and the `allOf`
/// bases the hoisted schemas reach; the second must be a subset of the first.
///
/// # Errors
/// Returns an error if a vendored OAS bundle or a BMM file cannot be loaded.
pub fn merged_fallback_schema_names() -> Result<(BTreeSet<String>, BTreeSet<String>), Error> {
    let oas_dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/openehr-its/vendor/rest-oas"
    ))
    .to_path_buf();
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&oas_dir)
        .map_err(|e| e.to_string())?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    paths.retain(|p| p.extension().is_some_and(|x| x == "yaml"));
    paths.sort();
    let mut bundles: Vec<(&str, oas::Oas)> = Vec::new();
    for path in &paths {
        bundles.push(("", oas::Oas::parse_file(path)?));
    }

    let base = compose("base")?;
    let rm = compose("rm")?;
    let type_paths = |c: &composition::Composed| -> Result<BTreeMap<String, String>, Error> {
        let krate = c.comp.crate_name.replace('-', "_");
        let g = c
            .generations
            .iter()
            .find(|g| g.spec.current)
            .ok_or("composition has no current generation")?;
        let mut out = BTreeMap::new();
        for u in &g.units {
            for spec in crate::analyze::emittable_specs(&u.model, &u.schema) {
                let ident = naming::type_name(&spec);
                let path = format!(
                    "{krate}::{}::{}::{ident}",
                    g.spec.module,
                    crate::render::emit::type_module_path(&u.schema, &spec)
                );
                out.insert(ident, path);
            }
        }
        Ok(out)
    };
    let names = emit_rest::RmNames {
        base: type_paths(&base)?,
        rm: type_paths(&rm)?,
    };
    let hoisted = emit_rest::hoist_set(&bundles, &names);
    let merged = oas::Oas::merged_schemas(&bundles, &hoisted);
    let carried: BTreeSet<String> = merged.schemas().iter().map(|(n, _)| n.clone()).collect();

    // The `allOf` bases the hoisted schemas reach, from the source bundles.
    let mut bases = BTreeSet::new();
    for (_, o) in &bundles {
        for (name, schema) in o.schemas() {
            if !hoisted.contains(&name) {
                continue;
            }
            if let Some(members) = schema
                .get("allOf")
                .and_then(serde_json::Value::as_array)
                .map(|m| m.iter().filter_map(oas::Oas::ref_name).collect::<Vec<_>>())
            {
                bases.extend(members);
            }
        }
    }
    Ok((carried, bases))
}

// ── model-query report ──────────────────────────────────────────────────────

/// Renders the `model-query` report over the vendored BMM inputs, as the CLI
/// prints it.
///
/// `format` is `table`, `tsv` or `json`; `flattened` gives one row per carried
/// attribute, inherited ones included.
///
/// # Errors
/// Returns an error if a vendored BMM file cannot be loaded, if `format` is not
/// a valid format, or if a filter names a component/class/attribute the loaded
/// model does not have.
pub fn model_query(
    component: Option<&str>,
    class: Option<&str>,
    attribute: Option<&str>,
    format: &str,
    flattened: bool,
) -> Result<String, Error> {
    model_query::render(
        &model_query::Query {
            component,
            class,
            attribute,
            flattened,
        },
        model_query::Format::parse(format)?,
    )
}

/// Hand-written generation-twin files of `key`'s crate that are identical
/// modulo generation tokens, and so must be templates.
///
/// # Errors
/// Returns an error if the composition fails to load or a crate tree cannot
/// be read.
pub fn identical_hand_written_twins(key: &str) -> Result<Vec<String>, Error> {
    let comp = composition::COMPOSITIONS
        .iter()
        .find(|c| c.key == key)
        .ok_or_else(|| Error::from(format!("unknown composition key {key}")))?;
    let Some(current) = crate::render::emit_templates::current_generation(comp) else {
        return Err(Error::from(format!("no current generation for {key}")));
    };
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates")
        .join(comp.crate_name)
        .join("src");
    let current_files = hand_written_files(&src.join(current.module))?;
    let mut identical = Vec::new();
    for generation in comp.generations {
        if generation.module == current.module {
            continue;
        }
        for (rel, body) in hand_written_files(&src.join(generation.module))? {
            let Some(current_body) = current_files.get(&rel) else {
                continue;
            };
            let normalized = crate::render::emit_templates::substitute(&body, generation, current);
            if &normalized == current_body {
                identical.push(format!("{}/{rel}", generation.module));
            }
        }
    }
    identical.sort();
    Ok(identical)
}

/// Hand-written `.rs` files under `dir` (relative slash path → body), the
/// module anchors excluded.
fn hand_written_files(dir: &std::path::Path) -> Result<BTreeMap<String, String>, Error> {
    let mut out = BTreeMap::new();
    if !dir.exists() {
        return Ok(out);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(|e| Error::from(e.to_string()))? {
            let path = entry.map_err(|e| Error::from(e.to_string()))?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let name = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs")
                || matches!(name, "mod.rs" | "prelude.rs")
            {
                continue;
            }
            let body = std::fs::read_to_string(&path).map_err(|e| Error::from(e.to_string()))?;
            if body
                .lines()
                .next()
                .is_some_and(|l| l.contains("@generated"))
            {
                continue;
            }
            let rel = path
                .strip_prefix(dir)
                .map_err(|e| Error::from(e.to_string()))?
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            out.insert(rel, body);
        }
    }
    Ok(out)
}

/// BMM functions both generations of one crate declare but only one realizes,
/// as `<CLASS>.<function>: realized in <gen>, missing in <gen>`.
///
/// An older generation realizing a function only the newer declares is
/// permitted (`docs/VERSIONS.md` §Spec version policy).
///
/// # Errors
/// Returns an error if the composition fails to load or a crate tree cannot be
/// read.
pub fn generation_function_divergence(key: &str) -> Result<Vec<String>, Error> {
    let comp = compose(key)?;
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates")
        .join(comp.comp.crate_name)
        .join("src");
    let mut realized: Vec<(&str, ClassFunctions)> = Vec::new();
    for generation in &comp.generations {
        realized.push((
            generation.spec.module,
            generation_class_functions(generation, &src)?,
        ));
    }
    let mut divergent = Vec::new();
    for (i, (module, classes)) in realized.iter().enumerate() {
        for (other_module, other_classes) in realized.iter().skip(i + 1) {
            push_pair_divergences(module, classes, other_module, other_classes, &mut divergent);
        }
    }
    divergent.sort();
    divergent.dedup();
    Ok(divergent)
}

/// Per class: the functions a generation DECLARES, and those it realizes.
type ClassFunctions = BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>;

/// The declared and realized function tables of one generation's classes.
///
/// # Errors
/// Returns an error if the generation's module tree cannot be read.
fn generation_class_functions(
    generation: &composition::ComposedGeneration,
    src: &std::path::Path,
) -> Result<ClassFunctions, Error> {
    let bodies = rust_bodies_by_stem(&src.join(generation.spec.module))?;
    let mut per_class: ClassFunctions = BTreeMap::new();
    for unit in &generation.units {
        for (name, class) in &unit.schema.classes {
            if let Some(entry) = class_function_status(name, class, &bodies) {
                per_class.insert(name.clone(), entry);
            }
        }
    }
    Ok(per_class)
}

/// Appends every function both generations declare that one realizes and the
/// other does not.
fn push_pair_divergences(
    module: &str,
    classes: &ClassFunctions,
    other_module: &str,
    other_classes: &ClassFunctions,
    out: &mut Vec<String>,
) {
    for (class, (_, functions)) in classes {
        let Some((other_declared, other_found)) = other_classes.get(class) else {
            continue;
        };
        for f in functions.intersection(other_declared) {
            if !other_found.contains(f) {
                out.push(format!(
                    "{class}.{f}: realized in {module}, missing in {other_module}"
                ));
            }
        }
    }
}

/// The functions one class declares and those its `*_impl.rs` sibling
/// realizes (a keyword name as `r#name`), or `None` without a sibling.
fn class_function_status(
    name: &str,
    class: &crate::load::bmm::BmmClass,
    bodies: &BTreeMap<String, String>,
) -> Option<(BTreeSet<String>, BTreeSet<String>)> {
    let stem = name.to_lowercase();
    let sibling = bodies.get(&format!("{stem}_impl"))?;
    let own = bodies.get(&stem).map(String::as_str).unwrap_or_default();
    let declared: BTreeSet<String> = class.functions.iter().map(|f| f.name.clone()).collect();
    let found = declared
        .iter()
        .filter(|f| {
            let item = format!("fn {f}(");
            let raw = format!("fn r#{f}(");
            sibling.contains(&item)
                || own.contains(&item)
                || sibling.contains(&raw)
                || own.contains(&raw)
        })
        .cloned()
        .collect();
    Some((declared, found))
}

/// Whether `body` applies items to `rust_type`: an `impl` block on it, or the
/// type passed to a macro that writes one, matched as a delimited identifier.
fn targets_type(body: &str, rust_type: &str) -> bool {
    body.lines().any(|line| {
        let line = line.trim();
        // `ordered_limit!(\n    DvCount,` — the type as a bare macro argument.
        if line.strip_suffix(',').is_some_and(|arg| arg == rust_type) {
            return true;
        }
        // `impl DvCount {`, `impl<T> ImportedVersion<T> {`, `impl Validate for DvCount {`
        line.starts_with("impl") && delimited_mention(line, rust_type)
    })
}

/// Whether `haystack` contains `needle` bounded by non-identifier characters,
/// so `Version` does not match inside `ImportedVersion`.
fn delimited_mention(haystack: &str, needle: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    // Split on the needle: a byte index can land inside a multi-byte character.
    let mut rest = haystack;
    while let Some(at) = rest.find(needle) {
        let (before, from_match) = rest.split_at(at);
        let Some(after) = from_match.strip_prefix(needle) else {
            return false;
        };
        if before.chars().next_back().is_none_or(|c| !is_ident(c))
            && after.chars().next().is_none_or(|c| !is_ident(c))
        {
            return true;
        }
        rest = after;
    }
    false
}

/// BMM-declared functions of `key`'s crate that no Rust method realizes, as
/// `<generation>/<CLASS>.<function>`, whether or not the class has an
/// `*_impl.rs` sibling.
///
/// # Errors
/// Returns an error if the composition fails to load or a crate tree cannot be
/// read.
pub fn unrealized_bmm_functions(key: &str) -> Result<Vec<String>, Error> {
    let comp = compose(key)?;
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates")
        .join(comp.comp.crate_name)
        .join("src");
    let mut missing = Vec::new();
    for generation in &comp.generations {
        let bodies = rust_bodies_by_stem(&src.join(generation.spec.module))?;
        for unit in &generation.units {
            for (name, class) in &unit.schema.classes {
                push_unrealized_functions(
                    name,
                    class,
                    generation.spec.module,
                    &bodies,
                    &mut missing,
                );
            }
        }
    }
    missing.sort();
    missing.dedup();
    Ok(missing)
}

/// Appends every BMM function of one class that no Rust method in `generation`
/// realizes; a class the emitter's maps give no Rust type is skipped.
fn push_unrealized_functions(
    name: &str,
    class: &crate::load::bmm::BmmClass,
    generation: &str,
    bodies: &BTreeMap<String, String>,
    out: &mut Vec<String>,
) {
    if class.functions.is_empty()
        || overrides::primitive(name).is_some()
        || overrides::is_mapped_class(name)
    {
        return;
    }
    for function in class.functions.iter().map(|f| &f.name) {
        if !function_is_realized(name, function, bodies_of(name, bodies), bodies) {
            out.push(format!("{generation}/{name}.{function}"));
        }
    }
}

/// The class's own two candidate bodies: its generated type file and its
/// hand-written `*_impl.rs` sibling.
fn bodies_of<'a>(name: &str, bodies: &'a BTreeMap<String, String>) -> [&'a str; 2] {
    let stem = name.to_lowercase();
    [
        bodies
            .get(&format!("{stem}_impl"))
            .map(String::as_str)
            .unwrap_or_default(),
        bodies.get(&stem).map(String::as_str).unwrap_or_default(),
    ]
}

/// Whether one BMM function is realized anywhere in its generation, by name or
/// raw identifier, in a body that names the type as an impl target.
fn function_is_realized(
    name: &str,
    function: &str,
    own: [&str; 2],
    bodies: &BTreeMap<String, String>,
) -> bool {
    let item = format!("fn {function}(");
    let raw_item = format!("fn r#{function}(");
    let realized = |body: &str| body.contains(&item) || body.contains(&raw_item);
    if own.iter().any(|body| realized(body)) {
        return true;
    }
    let rust_type = naming::type_name(name);
    bodies
        .values()
        .any(|body| realized(body) && targets_type(body, &rust_type))
}

/// Every `.rs` file under `dir` as file-stem → the concatenated bodies of every
/// file with that stem, generated and hand-written alike.
///
/// A stem is not unique within a generation (`bmm_class_impl` appears twice in
/// `openehr-lang` `v1_1`), so bodies are concatenated.
fn rust_bodies_by_stem(dir: &std::path::Path) -> Result<BTreeMap<String, String>, Error> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    if !dir.exists() {
        return Ok(out);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in std::fs::read_dir(&d).map_err(|e| Error::from(e.to_string()))? {
            let path = entry.map_err(|e| Error::from(e.to_string()))?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(std::ffi::OsStr::to_str) else {
                continue;
            };
            let body = std::fs::read_to_string(&path).map_err(|e| Error::from(e.to_string()))?;
            let slot = out.entry(stem.to_owned()).or_default();
            slot.push('\n');
            slot.push_str(&body);
        }
    }
    Ok(out)
}

/// The XSD-driven closures `emit-opt`/`emit-aom2` generate, each as
/// `(module name, schema files)`.
fn xsd_closures() -> [(&'static str, Vec<std::path::PathBuf>); 3] {
    let its = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/openehr-its");
    let v1_all = its.join("schemas/xml/its-xml-1.0.2-nsv1/ALL");
    let aom2 = its.join("schemas/xml/its-xml-1.0.2-nsv1/AOM2");
    [
        ("opt14", crate::load::xsd::am_files_v1(&v1_all)),
        ("aom2", crate::load::xsd::aom2_files(&aom2)),
        ("aom2_model", crate::load::xsd::aom2_model_files(&aom2)),
    ]
}

/// An `xs:enumeration`-faceted simple type an XSD-driven closure carries as
/// free text rather than as the closed value space the facet declares.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UntypedFacet {
    /// The emission closure (`opt14`, `aom2`, `aom2_model`).
    pub closure: String,
    /// The named `xs:simpleType` at fault.
    pub simple_type: String,
    /// Which property broke.
    pub problem: &'static str,
    /// The slot or Rust name the problem was observed at.
    pub detail: String,
}

/// Every `xs:enumeration`-faceted simple type of an XSD-driven closure that the
/// emitter does not carry as a typed enum, including slots that fall back to
/// `String`.
///
/// # Errors
/// When a closure's schema files cannot be read or parsed.
pub fn untyped_enumeration_facets() -> Result<Vec<UntypedFacet>, Error> {
    let targets = [
        &crate::render::emit_opt::OPT_TARGET,
        &crate::render::emit_opt::AOM2_TARGET,
        &crate::render::emit_opt::AOM2_MODEL_TARGET,
    ];
    let empty = BTreeMap::new();
    let mut out = Vec::new();
    for ((name, files), target) in xsd_closures().into_iter().zip(targets) {
        let xsd = crate::load::xsd::XsdModel::parse_files(&files).map_err(Error::from)?;
        let model = crate::render::emit_opt::OptModel::new(&xsd, &empty, &empty, target);
        let emitted = model.emit_types();
        let faceted: BTreeMap<&str, String> = xsd
            .simple_types
            .values()
            .filter(|t| !t.enumerations.is_empty())
            .map(|t| (t.name.as_str(), naming::type_name(&t.name)))
            .collect();

        for (spec, rust) in &faceted {
            if !emitted.contains(&format!("pub enum {rust} {{")) {
                out.push(UntypedFacet {
                    closure: name.to_owned(),
                    simple_type: (*spec).to_owned(),
                    problem: "the faceted simple type emits no typed enum",
                    detail: rust.clone(),
                });
            }
        }
        for (spec, fields) in &model.declared_field_types() {
            push_untyped_facet_slots(name, spec, fields, &faceted, &xsd, &mut out);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Reports every element slot of one emitted type whose faceted simple type
/// did not reach the field's declared Rust type.
fn push_untyped_facet_slots(
    closure: &str,
    spec: &str,
    fields: &[(String, String)],
    faceted: &BTreeMap<&str, String>,
    xsd: &crate::load::xsd::XsdModel,
    out: &mut Vec<UntypedFacet>,
) {
    let by_wire: BTreeMap<&str, &str> = fields
        .iter()
        .map(|(w, d)| (w.as_str(), d.as_str()))
        .collect();
    for elem in xsd.flattened(spec).1 {
        let Some(rust) = faceted.get(elem.type_name.as_str()) else {
            continue;
        };
        if !by_wire
            .get(elem.name.as_str())
            .is_some_and(|d| d.contains(rust.as_str()))
        {
            out.push(UntypedFacet {
                closure: closure.to_owned(),
                simple_type: elem.type_name.clone(),
                problem: "an element slot of a faceted simple type is not typed to its enum",
                detail: format!("{spec}.{}", elem.name),
            });
        }
    }
}

/// A concrete type a slot must carry that is missing from the slot's emitted
/// variant set.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LostVariant {
    /// The emission closure (`opt14`, `aom2`, `aom2_model`).
    pub closure: String,
    /// The slot's declared type — the dispatch enum's base.
    pub declared: String,
    /// The abstract type sitting between `declared` and the lost variant.
    pub via_abstract: String,
    /// The type at fault.
    pub lost: String,
    /// Which of the two properties broke.
    pub problem: &'static str,
}

/// Every concrete type an XSD-driven closure's dispatch enums fail to carry,
/// reading `xsi:type` variants as concrete descendants only.
///
/// An abstract type is not a legal `xsi:type` value, but each of its concrete
/// descendants is.
///
/// # Errors
/// When a closure's schema files cannot be read or parsed.
pub fn lost_dispatch_variants() -> Result<Vec<LostVariant>, Error> {
    let mut out = Vec::new();
    for (name, files) in xsd_closures() {
        let model = crate::load::xsd::XsdModel::parse_files(&files).map_err(Error::from)?;
        let slot_types: BTreeSet<String> = model
            .types
            .values()
            .flat_map(|owner| {
                let (attrs, elems) = model.flattened(&owner.name);
                attrs
                    .into_iter()
                    .map(|a| a.type_name)
                    .chain(elems.into_iter().map(|e| e.type_name))
                    .collect::<Vec<_>>()
            })
            .collect();

        for declared in &slot_types {
            let variants: BTreeSet<String> = model.descendants(declared).into_iter().collect();
            for abstract_ty in model.types.values().filter(|t| t.is_abstract) {
                if model.is_a(&abstract_ty.name, declared) {
                    push_lost_variants(
                        name,
                        declared,
                        &abstract_ty.name,
                        &variants,
                        &model,
                        &mut out,
                    );
                }
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Reports the concrete types below one abstract intermediate missing at one
/// slot, and the abstract type itself if it appears as a variant.
fn push_lost_variants(
    closure: &str,
    declared: &str,
    abstract_ty: &str,
    variants: &BTreeSet<String>,
    model: &crate::load::xsd::XsdModel,
    out: &mut Vec<LostVariant>,
) {
    for concrete in model.descendants(abstract_ty) {
        if !variants.contains(&concrete) {
            out.push(LostVariant {
                closure: closure.to_owned(),
                declared: declared.to_owned(),
                via_abstract: abstract_ty.to_owned(),
                lost: concrete,
                problem: "a concrete descendant is missing from the variant set",
            });
        }
    }
    if variants.contains(abstract_ty) {
        out.push(LostVariant {
            closure: closure.to_owned(),
            declared: declared.to_owned(),
            via_abstract: abstract_ty.to_owned(),
            lost: abstract_ty.to_owned(),
            problem: "an abstract type appears as an xsi:type variant",
        });
    }
}

/// One ITS-REST path-parameter component as the vendored codegen bundles
/// declare it, beside its `REST_PATH_IDENTIFIERS` entry.
#[derive(Debug)]
pub struct RestPathIdentifierCheck {
    /// The `#/components/parameters/` name.
    pub component: String,
    /// The component's description in each bundle that declares it as a path
    /// parameter (whitespace-normalised).
    pub descriptions: Vec<String>,
    /// The entry's identifier class, `None` when the component has no entry.
    pub entry: Option<(Option<String>, String)>,
}

/// Every path-parameter component of the vendored `*-codegen` OAS bundles and
/// every `REST_PATH_IDENTIFIERS` entry, joined by component name.
///
/// # Errors
/// Returns an error if a vendored OAS bundle cannot be read or parsed.
pub fn rest_path_identifiers() -> Result<Vec<RestPathIdentifierCheck>, Error> {
    let oas_dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/openehr-its/vendor/rest-oas"
    ))
    .to_path_buf();
    let mut descriptions: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(&oas_dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|n| n.ends_with("-codegen.openapi.yaml"))
        })
        .collect();
    paths.sort();
    for path in paths {
        for (component, location, description) in
            oas::Oas::parse_file(&path)?.parameter_components()
        {
            if location == "path" {
                descriptions
                    .entry(component)
                    .or_default()
                    .push(description.split_whitespace().collect::<Vec<_>>().join(" "));
            }
        }
    }
    for entry in overrides::REST_PATH_IDENTIFIERS {
        descriptions.entry(entry.component.to_string()).or_default();
    }
    Ok(descriptions
        .into_iter()
        .map(|(component, descriptions)| RestPathIdentifierCheck {
            entry: overrides::rest_path_identifier(&component)
                .map(|e| (e.class.map(str::to_string), e.citation.to_string())),
            component,
            descriptions,
        })
        .collect())
}

/// One operation of the vendored `*-codegen` OAS bundles: whether it declares
/// a request body, and the media types the loader read for it.
#[derive(Debug)]
pub struct RequestMediaCheck {
    /// The `operationId`.
    pub operation_id: String,
    /// Whether the operation declares a `requestBody`.
    pub has_body: bool,
    /// The `requestBody.content` keys the loader read.
    pub request_media: Vec<String>,
}

/// Every operation of the vendored `*-codegen` OAS bundles with its request
/// media types.
///
/// # Errors
/// Returns an error if a vendored OAS bundle cannot be read or parsed.
pub fn request_media() -> Result<Vec<RequestMediaCheck>, Error> {
    let oas_dir = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/openehr-its/vendor/rest-oas"
    ))
    .to_path_buf();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&oas_dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|n| n.ends_with("-codegen.openapi.yaml"))
        {
            for op in oas::Oas::parse_file(&path)?.operations() {
                out.push(RequestMediaCheck {
                    operation_id: op.operation_id,
                    has_body: op.request_body.is_some(),
                    request_media: op.request_media,
                });
            }
        }
    }
    Ok(out)
}
