// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The **draft-module table**: FerroEHR-local BMM schemas that are no openEHR
//! release, each emitted as one generated module of an application crate.
//!
//! A draft change proposal is still a BMM model, so the codegen rule forbids
//! hand-transcribing it, but it is no openEHR specification, so it does not
//! belong in the published `openehr-*` crates either. Each entry names its
//! FerroEHR-local input under `draft/bmm/`, the crate and module it lands in,
//! and the published generations it resolves against. No openEHR spec governs
//! where a draft package is generated: our own design (#5124).
//!
//! A draft module carries no prelude (the application crates re-export
//! nothing) and no `Generation` enum (a draft has no released generations);
//! its types are named by full module path.

use crate::analyze::{External, Model, emittable_specs};
use crate::load::bmm::BmmSchema;
use crate::plan::composition::{DepGeneration, external_index, generation_dep_schemas};
use std::collections::BTreeSet;
use std::path::Path;

/// The FerroEHR-local draft BMM root. Nothing under it is vendored: each file
/// is FerroEHR's own text under BUSL-1.1.
const DRAFT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/draft/bmm");

/// One draft BMM schema and the application module it is emitted into.
pub(crate) struct DraftModule {
    /// The draft BMM file, relative to the draft root.
    pub file: &'static str,
    /// The receiving crate's name, which decides the SPDX header.
    pub crate_name: &'static str,
    /// The receiving crate's directory, relative to the workspace root.
    pub crate_dir: &'static str,
    /// The top module the draft is emitted under, inside the crate's `src/`.
    pub module: &'static str,
    /// The schema ids the draft must declare in its `includes`, exactly.
    pub includes: &'static [&'static str],
    /// Dependency generations merged (in order, before the draft) into the
    /// resolution model; last-wins on a name collision.
    pub model_deps: &'static [DepGeneration],
    /// Dependency generations whose exported names resolve the draft's
    /// references, first match wins.
    pub prelude_deps: &'static [DepGeneration],
    /// The module's doc comment, emitted into its `mod.rs`.
    pub doc: &'static str,
}

/// The draft access log of `docs/plans/access-log.md`, emitted into
/// `app/ferroehr` until openEHR adopts the package.
pub(crate) const DRAFTS: &[DraftModule] = &[DraftModule {
    file: "openehr_access_log_0.1.0.bmm.json",
    crate_name: "ferroehr",
    crate_dir: "app/ferroehr",
    module: "access_log_draft",
    includes: &["openehr_rm_1.2.0"],
    model_deps: &[
        DepGeneration {
            key: "base",
            generation: "v1_3",
        },
        DepGeneration {
            key: "rm",
            generation: "v1_2",
        },
    ],
    prelude_deps: &[
        DepGeneration {
            key: "rm",
            generation: "v1_2",
        },
        DepGeneration {
            key: "base",
            generation: "v1_3",
        },
    ],
    doc: "The draft openEHR access log (`org.openehr.rm.access_log` 0.1.0), generated from its \
          FerroEHR-local BMM `tools/openehr-codegen/draft/bmm/openehr_access_log_0.1.0.bmm.json`.\n\
          \n\
          The package is a draft change proposal and no openEHR release, so it is emitted here \
          rather than in `openehr-rm`; if openEHR adopts it, it moves to `openehr-rm` as a \
          generation of the RM. Its classes resolve against RM 1.2.0 and BASE 1.3.0, the \
          schemas the draft includes, and read and write the canonical JSON form with `_type` \
          through the same strict runtime as the spec crates.",
}];

/// A draft resolved into its schema, resolution model and reference index.
pub(crate) struct ComposedDraft {
    /// The draft's own schema.
    pub schema: BmmSchema,
    /// The merged resolution model: the dependency generations, then the draft.
    pub model: Model,
    /// The dependency generations' schemas (`model_deps`, in order).
    pub dep_schemas: Vec<BmmSchema>,
    /// The full-path index resolving the draft's references to the published
    /// generations.
    pub external: External,
}

/// Load one draft BMM file by its path under the draft root.
///
/// # Errors
/// Returns an error if the file cannot be read or parsed.
fn load_draft(file: &str) -> Result<BmmSchema, Box<dyn std::error::Error>> {
    let path = Path::new(DRAFT).join(file);
    let src =
        std::fs::read_to_string(&path).map_err(|e| format!("draft BMM {}: {e}", path.display()))?;
    Ok(BmmSchema::parse_json(&src)?)
}

/// The schema id a loaded schema answers to in another schema's `includes`
/// (`openehr_rm_1.2.0`).
fn schema_id(schema: &BmmSchema) -> String {
    format!("openehr_{}_{}", schema.schema_name, schema.rm_release)
}

/// Resolve a draft row into its schema, model and reference index, refusing
/// a draft whose `includes` differ from the row or that names a type neither
/// it nor its dependency generations define.
///
/// # Errors
/// Returns an error if a BMM file cannot be loaded, the draft's `includes`
/// differ from the declared ones or name a schema the dependencies do not
/// load, or a class refers to a type that resolves nowhere.
pub(crate) fn compose_draft(
    draft: &'static DraftModule,
) -> Result<ComposedDraft, Box<dyn std::error::Error>> {
    let schema = load_draft(draft.file)?;
    let declared: BTreeSet<&str> = draft.includes.iter().copied().collect();
    let found: BTreeSet<&str> = schema.includes.iter().map(String::as_str).collect();
    if declared != found {
        return Err(format!(
            "draft {}: includes {found:?}, the draft table declares {declared:?}",
            draft.file
        )
        .into());
    }
    let dep_schemas = generation_dep_schemas(draft.model_deps)?;
    let loaded: BTreeSet<String> = dep_schemas.iter().map(schema_id).collect();
    if let Some(missing) = declared.iter().find(|id| !loaded.contains(**id)) {
        return Err(format!(
            "draft {}: includes {missing}, which no dependency generation loads",
            draft.file
        )
        .into());
    }
    let mut refs: Vec<&BmmSchema> = dep_schemas.iter().collect();
    refs.push(&schema);
    let model = Model::merged(&refs);
    let external = external_index(draft.prelude_deps, draft.crate_name)?;
    let unresolved = unresolved_types(&model, &schema, &external);
    if !unresolved.is_empty() {
        return Err(format!(
            "draft {}: types resolved nowhere (neither the draft nor its dependency \
             generations define them): {}",
            draft.file,
            unresolved.into_iter().collect::<Vec<_>>().join(", ")
        )
        .into());
    }
    Ok(ComposedDraft {
        schema,
        model,
        dep_schemas,
        external,
    })
}

/// Every `Class.attribute -> TYPE` a draft class refers to that neither the
/// draft emits nor a dependency generation exports.
///
/// The spec-crate emitter degrades such a reference to free-form JSON; a draft
/// refuses it, because a draft that names a type openEHR does not define is a
/// defect in the draft.
fn unresolved_types(model: &Model, schema: &BmmSchema, external: &External) -> BTreeSet<String> {
    let local = emittable_specs(model, schema);
    let mut out = BTreeSet::new();
    for (name, class) in &schema.classes {
        for spec in model.referenced_specs(class, &[], &std::collections::BTreeMap::default()) {
            if !local.contains(&spec) && !external.contains(&spec) {
                out.insert(format!("{name} -> {spec}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{DRAFTS, compose_draft, unresolved_types};
    use crate::load::bmm::BmmSchema;

    #[test]
    fn every_draft_composes_against_its_declared_includes() {
        for d in DRAFTS {
            let composed = compose_draft(d).unwrap_or_else(|e| panic!("{}: {e}", d.file));
            assert!(!composed.schema.classes.is_empty(), "{}", d.file);
        }
    }

    #[test]
    fn every_draft_module_is_named_as_a_draft() {
        // The drift check, `.gitattributes` and the Sonar scope find the draft
        // modules by the glob `app/*/src/*_draft`.
        for d in DRAFTS {
            assert!(
                d.module.ends_with("_draft") && d.crate_dir.starts_with("app/"),
                "{}: module {} in {} must match app/*/src/*_draft",
                d.file,
                d.module,
                d.crate_dir
            );
        }
    }

    #[test]
    fn a_type_no_schema_defines_is_refused() {
        let d = &DRAFTS[0];
        let composed = compose_draft(d).expect("the access log draft composes");
        let src = std::fs::read_to_string(std::path::Path::new(super::DRAFT).join(d.file))
            .expect("the draft BMM reads");
        // ACCESS_CHANNEL.interface, the one property after its name typed DV_CODED_TEXT.
        let name = "\"name\": \"interface\"";
        let (head, tail) = src.split_once(name).expect("ACCESS_CHANNEL.interface");
        let tail = tail.replacen("\"DV_CODED_TEXT\"", "\"NO_SUCH_CLASS\"", 1);
        let edited = format!("{head}{name}{tail}");
        let broken = BmmSchema::parse_json(&edited).expect("the edited draft parses");
        let deps = crate::plan::composition::generation_dep_schemas(d.model_deps)
            .expect("the dependency generations load");
        let mut all: Vec<&BmmSchema> = deps.iter().collect();
        all.push(&broken);
        let model = crate::analyze::Model::merged(&all);
        let unresolved = unresolved_types(&model, &broken, &composed.external);
        assert_eq!(
            unresolved.into_iter().collect::<Vec<_>>(),
            vec!["ACCESS_CHANNEL -> NO_SUCH_CLASS".to_owned()]
        );
    }
}
