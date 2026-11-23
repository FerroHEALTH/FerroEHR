// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The command-line interface: argument dispatch and the `cmd_*` handlers that
//! run each emit target and write its files.

use crate::analyze::emittable_specs;
use crate::analyze::reemit::{augment_with_reemit, cross_schema_reemit};
use crate::generated_tree::{declare_hand_written_modules, remove_generated_files_except};
use crate::load::bmm::BmmSchema;
use crate::load::impls::SiblingImpls;
use crate::load::{oas, xsd};
use crate::plan::composition::{self, ComposedGeneration, ComposedUnit, compose};
use crate::render::emit::{CrateGeneration, GenFile, RenderUnit, emit_composed, type_module_path};
use crate::render::emit_templates;
use crate::render::{
    emit, emit_json, emit_opt, emit_rest, emit_rm_model, emit_validate, emit_xml, model_query,
    naming, spdx,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod walk;

/// The `openehr-its` crate root.
const ITS_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/openehr-its");
/// The crate name behind [`ITS_ROOT`], for the SPDX header its emitted files
/// carry.
const ITS_CRATE: &str = "openehr-its";
/// The crate that receives the RM spec types, the static RM model and the
/// invariant cores.
const RM_CRATE: &str = "openehr-rm";
/// v1 (namespace `.../v1`) RM-instance XSD bundle dir — the Stage-1 parity target.
const XSD_V1_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/openehr-its/schemas/xml/its-xml-1.0.2-nsv1/ALL"
);
/// The AOM2 archetype-schema dir of the v1 bundle, the input to `emit-aom2`.
const XSD_AOM2_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/openehr-its/schemas/xml/its-xml-1.0.2-nsv1/AOM2"
);
/// The v2 XSD root; supplies the RM types the v1 `ALL/` bundle lacks or
/// carries stale.
const XSD_V2_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/openehr-its/schemas/xml/its-xml-2.0.0-nsv2"
);

/// The `emit-xml` output directory, relative to the workspace `crates/` dir.
const XML_GEN_DIR: &str = "openehr-its/src/xml/generated";
/// The `emit-rest` output directory, relative to the workspace `crates/` dir.
const REST_GEN_DIR: &str = "openehr-its/src/rest/generated";
/// The `emit-json` `_type`-dispatch output directory, relative to the workspace
/// `crates/` dir.
const JSON_GEN_DIR: &str = "openehr-its/src/json_codec/generated";

/// Exit code for an unrecognized subcommand (distinct from a pipeline failure,
/// which exits 1, so a wrapper script can tell a typo from a codegen error).
pub(crate) const EXIT_USAGE: u8 = 2;

/// One file an emit target produces, before it reaches the filesystem.
#[derive(Debug)]
pub(crate) struct EmittedFile {
    /// Where the file lands, relative to the root it is written under: the
    /// workspace `crates/` directory, or the workspace for a draft module.
    pub(crate) path: String,
    /// The crate that owns the file, whose SPDX header it carries.
    pub(crate) crate_name: &'static str,
    /// The rendered body, before the value-carrier guard, the SPDX header and
    /// `rustfmt`.
    pub(crate) body: String,
}

/// What an XSD-driven emit target produces: the files, plus the XSD elements
/// that matched no model field.
#[derive(Debug)]
pub(crate) struct XsdEmission {
    /// The rendered files.
    pub(crate) files: Vec<EmittedFile>,
    /// The XSD-only `(type, element)` pairs skipped for want of a model field.
    pub(crate) unmatched: Vec<(String, String)>,
}

/// Writes an emit target's files under `root` and returns the written paths.
///
/// # Errors
/// Returns the underlying filesystem error.
pub(crate) fn write_emitted(root: &Path, files: &[EmittedFile]) -> std::io::Result<Vec<PathBuf>> {
    let mut written = Vec::with_capacity(files.len());
    for f in files {
        let full = root.join(&f.path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_generated(&full, f.crate_name, &f.body)?;
        written.push(full);
    }
    Ok(written)
}

pub(crate) fn cmd_check() -> Result<(), Box<dyn std::error::Error>> {
    // Loads every generation of every composition entry, resolves its
    // dependencies and proves the model constructible.
    for comp in composition::COMPOSITIONS {
        let c = compose(comp.key)?;
        for g in &c.generations {
            for u in &g.units {
                let abstract_n = u.schema.classes.values().filter(|c| c.is_abstract).count();
                let generic_n = u
                    .schema
                    .classes
                    .values()
                    .filter(|c| !c.generic_params.is_empty())
                    .count();
                u.model.assert_constructible(&u.schema);
                println!(
                    "✓ {}::{} {}: schema={} release={} classes={} (abstract={abstract_n}, \
                     generic={generic_n}, constructible)",
                    comp.key,
                    g.spec.module,
                    u.spec.file,
                    u.schema.schema_name,
                    u.schema.rm_release,
                    u.schema.classes.len(),
                );
            }
        }
    }
    Ok(())
}

/// Reports what the vendored BMM states about every class attribute beside
/// the Rust field shape the emitter writes ([`model_query`]).
///
/// Usage: `model-query [--class NAME] [--attribute NAME] [--component KEY]
/// [--flattened] [--format table|tsv|json]`. `--flattened` reports carried
/// attributes, inherited ones included.
pub(crate) fn cmd_model_query(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (query, format) = model_query::parse_args(args)?;
    print!("{}", model_query::render(&query, format)?);
    Ok(())
}

/// Emit the ITS-REST contract (DTOs, param structs, server trait, route table)
/// for each API group into `openehr-its/src/rest/generated/`.
pub(crate) fn cmd_emit_rest() -> Result<(), Box<dyn std::error::Error>> {
    let files = render_rest_files()?;
    let written = write_emitted(&crates_root(), &files)?;
    rustfmt(&written)?;
    println!(
        "emitted {} files into {}",
        written.len(),
        crates_root().join(REST_GEN_DIR).display()
    );
    Ok(())
}

/// Renders the ITS-REST contract for every API group, the `common` module and
/// the module declaration file, in memory.
///
/// # Errors
/// Returns an error if the RM/BASE compositions or a vendored OAS bundle cannot
/// be loaded.
pub(crate) fn render_rest_files() -> Result<Vec<EmittedFile>, Box<dyn std::error::Error>> {
    // `overview` is the release's index document and declares no `paths`.
    const GROUPS: &[&str] = &[
        "admin",
        "definition",
        "demographic",
        "ehr",
        "query",
        "system",
    ];
    // Against the current RM/BASE generations; OAS `$ref` names equal the Rust
    // type names.
    let base = compose("base")?;
    let rm = compose("rm")?;
    let names = emit_rest::RmNames {
        base: generation_type_paths(
            base.current(),
            &format!("openehr_base::{}", base.current().spec.module),
        ),
        rm: generation_type_paths(
            rm.current(),
            &format!("openehr_rm::{}", rm.current().spec.module),
        ),
    };

    let oas_dir = Path::new(ITS_ROOT).join("vendor/rest-oas");

    // The hoist analysis needs every bundle.
    let mut bundles: Vec<(&str, oas::Oas)> = Vec::new();
    for group in GROUPS {
        let oas = oas::Oas::parse_file(&oas_dir.join(format!("{group}-codegen.openapi.yaml")))?;
        bundles.push((group, oas));
    }
    let hoisted = emit_rest::hoist_set(&bundles, &names);

    let mut files = Vec::new();
    // `common` emits from the merged per-name view; the copies are identical.
    {
        let merged = oas::Oas::merged_schemas(&bundles, &hoisted);
        files.push(EmittedFile {
            path: format!("{REST_GEN_DIR}/common.rs"),
            crate_name: ITS_CRATE,
            body: emit_rest::emit_common(&merged, &names, &hoisted),
        });
    }
    for (group, oas) in &bundles {
        files.push(EmittedFile {
            path: format!("{REST_GEN_DIR}/{group}.rs"),
            crate_name: ITS_CRATE,
            body: emit_rest::emit_group(oas, group, &names, &hoisted),
        });
    }
    let mod_rs = {
        let mut s = String::from(
            "// @generated by openehr-codegen (emit-rest) — DO NOT EDIT.\n\
             //! ITS-REST contract, one module per API group, plus the shared\n\
             //! `common` module (cross-group hoisted component schemas).\n\n\
             pub mod common;\n",
        );
        for g in GROUPS {
            s.push_str(&format!("pub mod {g};\n"));
        }
        s
    };
    files.push(EmittedFile {
        path: format!("{REST_GEN_DIR}/mod.rs"),
        crate_name: ITS_CRATE,
        body: mod_rs,
    });
    Ok(files)
}

/// Emits canonical-XML `ToXml`/`FromXml` impls for the RM/BASE spec types into
/// `openehr-its/src/xml/generated/`, for both the v1 and v2 lineages.
pub(crate) fn cmd_emit_xml() -> Result<(), Box<dyn std::error::Error>> {
    let emission = render_xml_files()?;
    let written = write_emitted(&crates_root(), &emission.files)?;
    rustfmt(&written)?;
    println!(
        "emitted {} files into {} ({} XSD-only elements without a BMM field skipped)",
        written.len(),
        crates_root().join(XML_GEN_DIR).display(),
        emission.unmatched.len()
    );
    Ok(())
}

/// Renders the canonical-XML impls and their module declaration file in
/// memory, with the XSD-only elements that matched no BMM field.
///
/// # Errors
/// Returns an error if the RM/BASE compositions or the vendored XSD bundles
/// cannot be loaded, or if the XML emission itself fails.
pub(crate) fn render_xml_files() -> Result<XsdEmission, Box<dyn std::error::Error>> {
    // Every RM/BASE generation, including the closure twins re-emitted into
    // `openehr-rm`.
    let (base, rm) = (compose("base")?, compose("rm")?);
    let mut generations = Vec::new();
    for (krate, composed) in [("openehr_base", &base), ("openehr_rm", &rm)] {
        for g in &composed.generations {
            let unit = g.unit()?;
            let root = format!("{krate}::{}", g.spec.module);
            generations.push((root, unit, augmented_schema(g, unit), g));
        }
    }

    // The lineages differ only by the root `xmlns`. The v1 `ALL/` bundle lacks
    // `Ehr.xsd` and the demographic schema, so the v2 schemas follow it,
    // first wins.
    let v1 = xsd::XsdModel::parse_files(&xsd::xml_emit_files(
        Path::new(XSD_V1_DIR),
        Path::new(XSD_V2_DIR),
    ))?;

    let schemas: Vec<emit_xml::XmlSchema<'_>> = generations
        .iter()
        .map(|(root, unit, schema, g)| emit_xml::XmlSchema {
            model: &unit.model,
            schema,
            root,
            external: &g.external,
        })
        .collect();
    let mut unmatched = Vec::new();
    let body = emit_xml::emit_file(&schemas, &v1, &mut unmatched)?;

    let mod_rs = "// @generated by openehr-codegen (emit-xml) — DO NOT EDIT.\n\
        //! Canonical-XML `ToXml`/`FromXml` impls for the RM/BASE spec types.\n\n\
        mod impls;\n";

    Ok(XsdEmission {
        files: vec![
            EmittedFile {
                path: format!("{XML_GEN_DIR}/impls.rs"),
                crate_name: ITS_CRATE,
                body,
            },
            EmittedFile {
                path: format!("{XML_GEN_DIR}/mod.rs"),
                crate_name: ITS_CRATE,
                body: mod_rs.to_owned(),
            },
        ],
        unmatched,
    })
}

/// Emits the canonical-JSON serde impls for every generated spec type into
/// each crate's `src/json_serde.rs` (the orphan rule), and the `_type`
/// dispatch into `openehr-its/src/json_codec/generated/`.
pub(crate) fn cmd_emit_json() -> Result<(), Box<dyn std::error::Error>> {
    let files = render_json_files()?;
    let mut written = write_emitted(&crates_root(), &files)?;
    for f in &files {
        if f.path.ends_with(JSON_SERDE_FILE) {
            let lib = crates_root().join(&f.path).with_file_name("lib.rs");
            declare_json_serde_module(&lib, &mut written)?;
        }
    }
    rustfmt(&written)?;
    println!("emitted {} files", written.len());
    Ok(())
}

/// Where a spec crate's emitted canonical-JSON impls live, relative to that
/// crate's directory.
const JSON_SERDE_FILE: &str = "src/json_serde.rs";

/// Renders the canonical-JSON serde impls and the `_type` dispatch in memory.
///
/// # Errors
/// Returns an error if a composition's BMM files cannot be loaded, or if the
/// structural-dispatch priority list names an unknown composition key.
pub(crate) fn render_json_files() -> Result<Vec<EmittedFile>, Box<dyn std::error::Error>> {
    // One JsonSchema per (crate, generation), over the augmented schema.
    struct PreparedUnit {
        root: String,
        aug: BmmSchema,
    }
    let keys = ["base", "rm", "lang", "am", "term"];
    let mut composed = Vec::new();
    for k in keys {
        composed.push(compose(k)?);
    }
    // One prepared entry per (generation, unit), flattened in table order.
    let prepared: Vec<Vec<PreparedUnit>> = composed
        .iter()
        .map(|c| {
            let krate = c.comp.crate_name.replace('-', "_");
            c.generations
                .iter()
                .flat_map(|g| {
                    g.units.iter().map(|u| PreparedUnit {
                        root: format!("{krate}::{}", g.spec.module),
                        aug: augmented_schema(g, u),
                    })
                })
                .collect()
        })
        .collect();
    let schemas_by_crate: Vec<Vec<emit_json::JsonSchema<'_>>> = composed
        .iter()
        .zip(&prepared)
        .map(|(c, units)| {
            c.generations
                .iter()
                .flat_map(|g| g.units.iter().map(move |u| (g, u)))
                .zip(units)
                .map(|((g, u), p)| emit_json::JsonSchema {
                    model: &u.model,
                    schema: &p.aug,
                    root: &p.root,
                    external: &g.external,
                })
                .collect()
        })
        .collect();

    // One file per spec crate: the orphan rule, and access to `pub(crate)` fields.
    let mut files = Vec::new();
    for (c, schemas) in composed.iter().zip(&schemas_by_crate) {
        let krate = c.comp.crate_name.replace('-', "_");
        files.push(EmittedFile {
            path: format!("{}/{JSON_SERDE_FILE}", c.comp.crate_name),
            crate_name: c.comp.crate_name,
            body: emit_json::emit_file(schemas, &krate),
        });
    }

    // A `_type` several components declare resolves RM, then BASE, then the
    // archetype components. No openEHR spec governs a cross-component `_type`
    // namespace: our own design.
    let mut structural_schemas: Vec<emit_json::JsonSchema<'_>> = Vec::new();
    for key in ["rm", "base", "lang", "am", "term"] {
        let i = keys
            .iter()
            .position(|k| *k == key)
            .ok_or("structural priority names an unknown composition key")?;
        let schemas = schemas_by_crate
            .get(i)
            .ok_or("structural priority names an unknown composition key")?;
        // The current generation resolves first.
        let comp = composed
            .get(i)
            .ok_or("structural priority names an unknown composition key")?;
        let flags: Vec<bool> = comp
            .generations
            .iter()
            .flat_map(|g| g.units.iter().map(|_| g.spec.current))
            .collect();
        for (current, schema) in flags.iter().zip(schemas) {
            if *current {
                structural_schemas.push(*schema);
            }
        }
        for (current, schema) in flags.iter().zip(schemas) {
            if !*current {
                structural_schemas.push(*schema);
            }
        }
    }

    files.push(EmittedFile {
        path: format!("{JSON_GEN_DIR}/structural.rs"),
        crate_name: ITS_CRATE,
        body: emit_json::emit_structural_file(&structural_schemas),
    });

    let mod_rs = "// @generated by openehr-codegen (emit-json) — DO NOT EDIT.\n\
        //! The canonical-JSON `_type` dispatch (the per-type `serde` impls live in\n\
        //! each spec crate's own `json_serde` module).\n\n\
        pub mod structural;\n";
    files.push(EmittedFile {
        path: format!("{JSON_GEN_DIR}/mod.rs"),
        crate_name: ITS_CRATE,
        body: mod_rs.to_owned(),
    });

    Ok(files)
}

/// Declares the emitted `json_serde` module in a spec crate's generated
/// `lib.rs`.
fn declare_json_serde_module(
    lib: &Path,
    written: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    const DECL: &str = "mod json_serde;";
    let mut body = std::fs::read_to_string(lib)?;
    if body.contains(DECL) {
        return Ok(());
    }
    body.push_str(
        "\n// canonical-JSON `serde` impls (openehr-codegen -- emit-json), auto-declared:\n",
    );
    body.push_str(DECL);
    body.push('\n');
    std::fs::write(lib, &body)?;
    written.push(lib.to_path_buf());
    Ok(())
}

/// Renders one XSD-driven constraint-model module (`types.rs`, `impls.rs`,
/// `mod.rs`) for `crates/openehr-its/src/<dir>`; shared types resolve to the
/// generated `openehr-base`/`openehr-rm` impls.
///
/// # Errors
/// Returns an error if the XSD closure cannot be parsed.
fn render_xsd_model(
    paths: &SpecPaths,
    files: &[PathBuf],
    dir: &str,
    target: &'static emit_opt::ModelTarget,
    module: &emit_opt::ModuleSpec,
) -> Result<XsdEmission, Box<dyn std::error::Error>> {
    let xsd = xsd::XsdModel::parse_files(files)?;
    let model = emit_opt::OptModel::new(&xsd, &paths.base, &paths.rm, target);
    let mut unmatched = Vec::new();
    let bodies = [
        ("types.rs", model.emit_types()),
        ("impls.rs", model.emit_impls(&mut unmatched)),
        ("mod.rs", emit_opt::emit_module(module)),
    ];
    Ok(XsdEmission {
        files: bodies
            .into_iter()
            .map(|(name, body)| EmittedFile {
                path: format!("{ITS_CRATE}/src/{dir}/{name}"),
                crate_name: ITS_CRATE,
                body,
            })
            .collect(),
        unmatched,
    })
}

/// Emits the OPT 1.4 model (`opt14`) from the AM/OPT XSD closure.
pub(crate) fn cmd_emit_opt() -> Result<(), Box<dyn std::error::Error>> {
    write_xsd_emission("emit-opt", &render_opt_files()?)
}

/// Renders the OPT 1.4 model in memory.
///
/// # Errors
/// Returns an error if the RM/BASE compositions or the AM/OPT XSD closure
/// cannot be loaded.
pub(crate) fn render_opt_files() -> Result<XsdEmission, Box<dyn std::error::Error>> {
    render_xsd_model(
        &current_spec_paths()?,
        &xsd::am_files_v1(Path::new(XSD_V1_DIR)),
        "opt14",
        &emit_opt::OPT_TARGET,
        &emit_opt::OPT_MODULE,
    )
}

/// Emits both AOM2 archetype XML serializations: `aom2`, the persistent form
/// (`P_Archetype.xsd`), and `aom2_model`, the model form (`Archetype.xsd`).
///
/// Two closures, since both schemas declare a top-level `archetype` element
/// with different types ([`xsd::AOM2_FILES`], [`xsd::AOM2_MODEL_FILES`]).
pub(crate) fn cmd_emit_aom2() -> Result<(), Box<dyn std::error::Error>> {
    write_xsd_emission("emit-aom2", &render_aom2_files()?)
}

/// Renders both AOM2 archetype serializations in memory, as one emission.
///
/// # Errors
/// Returns an error if the RM/BASE compositions or either AOM2 XSD closure
/// cannot be loaded.
pub(crate) fn render_aom2_files() -> Result<XsdEmission, Box<dyn std::error::Error>> {
    let paths = current_spec_paths()?;
    let aom2_dir = Path::new(XSD_AOM2_DIR);
    let mut persistent = render_xsd_model(
        &paths,
        &xsd::aom2_files(aom2_dir),
        "aom2",
        &emit_opt::AOM2_TARGET,
        &emit_opt::AOM2_MODULE,
    )?;
    let mut model = render_xsd_model(
        &paths,
        &xsd::aom2_model_files(aom2_dir),
        "aom2_model",
        &emit_opt::AOM2_MODEL_TARGET,
        &emit_opt::AOM2_MODEL_MODULE,
    )?;
    persistent.files.append(&mut model.files);
    persistent.unmatched.append(&mut model.unmatched);
    Ok(persistent)
}

/// Writes an XSD-driven target's files and reports what it skipped.
///
/// # Errors
/// Returns the underlying filesystem error, or a `rustfmt` failure.
fn write_xsd_emission(
    target: &str,
    emission: &XsdEmission,
) -> Result<(), Box<dyn std::error::Error>> {
    let written = write_emitted(&crates_root(), &emission.files)?;
    rustfmt(&written)?;
    println!(
        "{target}: emitted {} files into {} ({} XSD-only elements without a matching field \
         skipped)",
        written.len(),
        crates_root().display(),
        emission.unmatched.len()
    );
    Ok(())
}

/// Emits the static RM attribute/type model (`openehr-rm/src/model/`), the AQL
/// planner's oracle, and declares `pub mod model;` if absent.
pub(crate) fn cmd_emit_rm_model() -> Result<(), Box<dyn std::error::Error>> {
    let files = render_rm_model_files()?;
    let mut written = write_emitted(&crates_root(), &files)?;
    let src = crates_root().join(RM_CRATE).join("src");
    for g in &compose("rm")?.generations {
        // The same declaration `inject_rm_model` writes under `emit`.
        let gen_mod = src.join(g.spec.module).join("mod.rs");
        if !gen_mod.exists() {
            continue;
        }
        let mut body = std::fs::read_to_string(&gen_mod)?;
        if !body.contains(RM_MODEL_DECL) {
            body.push_str(RM_MODEL_DECL);
            body.push('\n');
            std::fs::write(&gen_mod, &body)?;
            written.push(gen_mod);
        }
    }
    rustfmt(&written)?;
    println!("emitted {} files into {}", files.len(), src.display());
    Ok(())
}

/// Renders the static RM model for every RM generation in memory.
///
/// # Errors
/// Returns an error if the RM composition's BMM files cannot be loaded.
pub(crate) fn render_rm_model_files() -> Result<Vec<EmittedFile>, Box<dyn std::error::Error>> {
    let rm = compose("rm")?;
    let mut out = Vec::new();
    for g in &rm.generations {
        let unit = g.unit()?;
        out.extend(rm_crate_files(prefix_gen_files(
            emit_rm_model::emit_files(&unit.model, &rm_model_schemas(g, unit))?,
            g.spec.module,
        )));
    }
    Ok(out)
}

/// The verbatim schemas an RM generation's attribute model reads packages
/// from: the unit's own, then its paired dependency generations'.
fn rm_model_schemas<'a>(g: &'a ComposedGeneration, unit: &'a ComposedUnit) -> Vec<&'a BmmSchema> {
    std::iter::once(&unit.schema)
        .chain(&g.dep_schemas)
        .collect()
}

/// Turn `openehr-rm`-destined generated files (paths relative to the crate's
/// `src/`) into [`EmittedFile`]s addressed from the workspace `crates/` dir.
fn rm_crate_files(files: Vec<GenFile>) -> Vec<EmittedFile> {
    files
        .into_iter()
        .map(|f| EmittedFile {
            path: format!("{RM_CRATE}/src/{}", f.path),
            crate_name: RM_CRATE,
            body: f.body,
        })
        .collect()
}

/// Emits the RM class-invariant cores (`openehr-rm/src/validate/generated.rs`);
/// the hand-written `validate.rs` declares the module.
pub(crate) fn cmd_emit_validate() -> Result<(), Box<dyn std::error::Error>> {
    let files = render_validate_files()?;
    let written = write_emitted(&crates_root(), &files)?;
    rustfmt(&written)?;
    println!(
        "emitted {} file(s) into {}",
        files.len(),
        crates_root().join(RM_CRATE).join("src").display()
    );
    Ok(())
}

/// Renders the RM class-invariant cores for every RM generation in memory.
///
/// # Errors
/// Returns an error if the RM composition's BMM files cannot be loaded.
pub(crate) fn render_validate_files() -> Result<Vec<EmittedFile>, Box<dyn std::error::Error>> {
    let rm = compose("rm")?;
    let mut out = Vec::new();
    for g in &rm.generations {
        let unit = g.unit()?;
        let aug = augmented_schema(g, unit);
        out.extend(rm_crate_files(prefix_gen_files(
            emit_validate::emit_files(&unit.model, &aug),
            g.spec.module,
        )));
        out.extend(rm_crate_files(prefix_gen_files(
            walk::walk_files(g, unit, &aug, walk::released_generation(&rm.generations)?),
            g.spec.module,
        )));
    }
    Ok(out)
}

/// Prefixes every generated file path with the generation module directory.
fn prefix_gen_files(files: Vec<GenFile>, module: &str) -> Vec<GenFile> {
    files
        .into_iter()
        .map(|f| GenFile {
            path: format!("{module}/{}", f.path),
            body: f.body,
        })
        .collect()
}

/// Appends the generated invariant-core file to `openehr-rm`'s file set.
fn inject_validate(files: &mut Vec<GenFile>, mut validate_files: Vec<GenFile>) {
    files.append(&mut validate_files);
}

/// The declaration an RM generation's `mod.rs` carries for the static RM
/// model.
const RM_MODEL_DECL: &str = "pub mod model;";

/// Appends the generation-prefixed RM-model files to `openehr-rm`'s file set
/// and declares the module in its generation `mod.rs`.
fn inject_rm_model(files: &mut Vec<GenFile>, mut model_files: Vec<GenFile>, module: &str) {
    let gen_mod = format!("{module}/mod.rs");
    for f in files.iter_mut() {
        if f.path == gen_mod && !f.body.contains(RM_MODEL_DECL) {
            f.body.push_str(RM_MODEL_DECL);
            f.body.push('\n');
        }
    }
    files.append(&mut model_files);
}

/// Diagnostic: parse the vendored v1 RM-instance XSDs and print a summary +
/// a couple of flattened views, to validate the XSD reader.
pub(crate) fn cmd_check_xsd() -> Result<(), Box<dyn std::error::Error>> {
    let files = xsd::v1_files(Path::new(XSD_V1_DIR));
    let model = xsd::XsdModel::parse_files(&files)?;
    println!(
        "✓ v1 XSDs: namespace={} types={}",
        model.namespace,
        model.types.len()
    );
    let abstract_n = model.types.values().filter(|t| t.is_abstract).count();
    println!("  abstract={abstract_n}");
    for probe in ["ELEMENT", "DV_CODED_TEXT", "COMPOSITION"] {
        if let Some(t) = model.types.get(probe) {
            let (attrs, elems) = model.flattened(probe);
            println!(
                "  {probe}: base={:?} attrs=[{}] elems=[{}]",
                t.base,
                attrs
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
                elems
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
    }
    let dv = model.descendants("DATA_VALUE");
    println!(
        "  DATA_VALUE concrete descendants ({}): {}",
        dv.len(),
        dv.join(",")
    );
    Ok(())
}

/// A generation's own schema with its cross-schema re-emission closure
/// grafted in: each upstream class whose Rust form widens here is re-emitted
/// locally at its source package path.
fn augmented_schema(g: &ComposedGeneration, u: &ComposedUnit) -> BmmSchema {
    let reemit = cross_schema_reemit(&u.model, &u.schema);
    let dep_refs: Vec<&BmmSchema> = g.dep_schemas.iter().collect();
    augment_with_reemit(&u.schema, &u.model, &reemit, &dep_refs)
}

/// The CURRENT BASE and RM generations' class → module-path maps, the pair
/// every XSD-driven target resolves already-generated RM instance types against.
#[derive(Debug)]
struct SpecPaths {
    /// The BASE generation's map.
    base: BTreeMap<String, String>,
    /// The RM generation's map.
    rm: BTreeMap<String, String>,
}

/// Build the CURRENT BASE and RM generations' spec-path maps.
///
/// # Errors
/// Returns an error if either composition's BMM files cannot be loaded.
fn current_spec_paths() -> Result<SpecPaths, Box<dyn std::error::Error>> {
    let base = compose("base")?;
    let rm = compose("rm")?;
    Ok(SpecPaths {
        base: generation_spec_paths(
            base.current(),
            &format!("openehr_base::{}", base.current().spec.module),
        ),
        rm: generation_spec_paths(
            rm.current(),
            &format!("openehr_rm::{}", rm.current().spec.module),
        ),
    })
}

/// Spec class name → the full generation-module path of its defining module.
fn generation_spec_paths(g: &ComposedGeneration, root: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for u in &g.units {
        for s in emittable_specs(&u.model, &u.schema) {
            let path = format!("{root}::{}", type_module_path(&u.schema, &s));
            out.insert(s, path);
        }
    }
    out
}

/// Rust type name → full generation-module type path, for `emit-rest`.
fn generation_type_paths(g: &ComposedGeneration, root: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for u in &g.units {
        for s in emittable_specs(&u.model, &u.schema) {
            let ident = naming::type_name(&s);
            let path = format!("{root}::{}::{ident}", type_module_path(&u.schema, &s));
            out.insert(ident, path);
        }
    }
    out
}

/// The hand-written `*_impl.rs` siblings a generated crate carries.
fn sibling_impls(crate_name: &str) -> SiblingImpls {
    SiblingImpls::scan(&crates_root().join(crate_name).join("src"))
}

fn crates_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("crates")
}

/// The generation-twin template sources (`render::emit_templates`).
fn templates_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("templates")
}

pub(crate) fn cmd_emit(_outdir: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let files = render_emit_files()?;
    for comp in composition::COMPOSITIONS {
        let crate_files: Vec<&EmittedFile> = files
            .iter()
            .filter(|f| f.crate_name == comp.crate_name)
            .collect();
        write_crate(comp.crate_name, &crate_files)?;
    }
    eprintln!("{INCOMPLETE_AFTER_EMIT}");
    Ok(())
}

/// Renders every generated spec crate's `src/` tree in memory, template stamps
/// included.
///
/// # Errors
/// Returns an error if a composition's BMM files cannot be loaded, or if a
/// template stamp collides with a generated file.
pub(crate) fn render_emit_files() -> Result<Vec<EmittedFile>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    for comp in composition::COMPOSITIONS {
        for f in render_crate_files(comp)? {
            out.push(EmittedFile {
                path: format!("{}/src/{}", comp.crate_name, f.path),
                crate_name: comp.crate_name,
                body: f.body,
            });
        }
    }
    Ok(out)
}

/// Renders one composition entry's whole file set: the spec types, the RM
/// model and invariant cores, and the template stamps.
///
/// # Errors
/// Returns an error if the composition's BMM files cannot be loaded, or if a
/// template stamp collides with a generated file.
fn render_crate_files(
    comp: &'static composition::CrateComposition,
) -> Result<Vec<GenFile>, Box<dyn std::error::Error>> {
    let c = compose(comp.key)?;
    let impls = sibling_impls(comp.crate_name);
    // One augmented schema per (generation, unit), flattened in table order —
    // the same shape the render loop consumes.
    let augmented: Vec<Vec<BmmSchema>> = c
        .generations
        .iter()
        .map(|g| g.units.iter().map(|u| augmented_schema(g, u)).collect())
        .collect();
    let gens: Vec<CrateGeneration<'_>> = c
        .generations
        .iter()
        .zip(&augmented)
        .map(|(g, augs)| CrateGeneration {
            spec: g.spec,
            units: g
                .units
                .iter()
                .zip(augs)
                .map(|(u, aug)| RenderUnit {
                    spec: u.spec,
                    model: &u.model,
                    schema: aug,
                })
                .collect(),
            external: &g.external,
        })
        .collect();
    let mut files = emit_composed(comp, &gens, &impls);
    if comp.key == "rm" {
        // Every RM generation carries its own attribute model and invariant cores.
        for (g, augs) in c.generations.iter().zip(&augmented) {
            let module = g.spec.module;
            let unit = g.unit()?;
            let aug = augs
                .first()
                .ok_or("an RM generation carries no specification unit")?;
            inject_rm_model(
                &mut files,
                prefix_gen_files(
                    emit_rm_model::emit_files(&unit.model, &rm_model_schemas(g, unit))?,
                    module,
                ),
                module,
            );
            inject_validate(
                &mut files,
                prefix_gen_files(emit_validate::emit_files(&unit.model, aug), module),
            );
            inject_validate(
                &mut files,
                prefix_gen_files(
                    walk::walk_files(g, unit, aug, walk::released_generation(&c.generations)?),
                    module,
                ),
            );
        }
    }
    // A template stamp colliding with a generated file is an error.
    for stamped in emit_templates::stamp_templates(&templates_root(), comp)? {
        if files.iter().any(|f| f.path == stamped.path) {
            return Err(format!(
                "template stamp collides with a generated file: {}",
                stamped.path
            )
            .into());
        }
        files.push(stamped);
    }
    Ok(files)
}

/// Returns the bytes an emitted file carries before `rustfmt`: the
/// value-carrier guard, the SPDX header its crate requires, and the body.
pub(crate) fn generated_bytes(crate_name: &str, body: &str) -> String {
    // A template-stamped body is hand-written text that already carries its own
    // scoped suppressions — the generated-file guard would duplicate them.
    let guarded = if body.starts_with(emit_templates::TEMPLATE_MARKER) {
        body.to_owned()
    } else {
        emit::guard_value_carriers(body)
    };
    spdx::stamp(crate_name, &guarded)
}

/// Writes one emitted file through [`generated_bytes`].
///
/// # Errors
/// Returns the underlying filesystem error.
pub(crate) fn write_generated(path: &Path, crate_name: &str, body: &str) -> std::io::Result<()> {
    std::fs::write(path, generated_bytes(crate_name, body))
}

/// Writes a generated crate's `src/` in place, first deleting its previously
/// generated files.
fn write_crate(crate_name: &str, files: &[&EmittedFile]) -> Result<(), Box<dyn std::error::Error>> {
    let src = crates_root().join(crate_name).join("src");
    // Delete only `@generated` files; the `draft` subtree belongs to `emit-drafts`.
    let drafts = src.join(crate::plan::drafts::MODULE);
    if src.exists() {
        remove_generated_files_except(&src, &drafts)?;
    }
    std::fs::create_dir_all(&src)?;
    let mut written = Vec::with_capacity(files.len());
    for f in files {
        let full = crates_root().join(&f.path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_generated(&full, crate_name, &f.body)?;
        written.push(full);
    }
    // Declares each hand-written `.rs` beside a generated `mod.rs` or `lib.rs`.
    declare_hand_written_modules(&src, "lib.rs", Some(&drafts), &mut written)?;
    rustfmt(&written)?;
    println!("emitted {} files into {}", files.len(), src.display());
    Ok(())
}

/// What `emit` alone leaves behind, printed at the end of every run: it purges
/// `json_serde.rs`, which only `emit-json` writes back.
const INCOMPLETE_AFTER_EMIT: &str = "\
note: `emit` alone leaves the workspace UNBUILDABLE — it removes each crate's
      json_serde.rs, which only `emit-json` writes back. Run the full set:
        emit-json emit-xml emit-rest emit-opt emit-aom2 emit-rm-model emit-validate
      or use the /regen-codegen skill, which runs them all plus the drift check.";

/// Runs `rustfmt` over the generated files.
pub(crate) fn rustfmt(files: &[PathBuf]) -> Result<(), Box<dyn std::error::Error>> {
    if files.is_empty() {
        return Ok(());
    }
    let status = std::process::Command::new("rustfmt")
        .args(["--edition", "2024", "--quiet"])
        .args(files)
        .status()?;
    if !status.success() {
        return Err(format!("rustfmt failed with status {status}").into());
    }
    Ok(())
}

#[cfg(test)]
mod emit_warning_tests {
    use super::INCOMPLETE_AFTER_EMIT;

    /// The note names `emit-json`, the one target that restores what `emit`
    /// removed.
    #[test]
    fn the_note_names_the_target_that_restores_the_tree() {
        assert!(INCOMPLETE_AFTER_EMIT.contains("emit-json"));
        assert!(INCOMPLETE_AFTER_EMIT.contains("json_serde.rs"));
        assert!(INCOMPLETE_AFTER_EMIT.contains("UNBUILDABLE"));
    }
}
