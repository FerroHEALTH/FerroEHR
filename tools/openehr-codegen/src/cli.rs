// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The command-line interface: argument dispatch, input loading, and the
//! stage-orchestrating `cmd_*` handlers that wire LOAD → ANALYZE → PLAN →
//! RENDER together and write each emit target's files.

use crate::analyze::{augment_with_reemit, cross_schema_reemit, emittable_specs};
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
/// The `openehr-its` crate root (holds the vendored XSDs/OAS and receives the
/// generated XML/REST code). `../../crates/openehr-its` from this tool's
/// `tools/openehr-codegen` manifest dir.
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
/// The AOM2 archetype-schema dir of the v1 bundle — the input to `emit-aom2`.
/// The bundle's own `examples/` documents live beside it and are the corpus the
/// generated codec is gated against.
const XSD_AOM2_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/openehr-its/schemas/xml/its-xml-1.0.2-nsv1/AOM2"
);
/// v2 (namespace `.../v2`) XSD root (per-component release folders). Supplies the
/// RM-instance types the v1 `ALL/` bundle lacks (EHR + demographic) or carries
/// stale (extract) to the emit-xml input.
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
const EXIT_USAGE: u8 = 2;

/// One file an emit target produces, before it reaches the filesystem.
///
/// Separating the rendered text from the write lets a target's whole
/// orchestration run in memory (`testsupport`) and on disk (the `cmd_*`
/// handlers) through ONE code path, so the tested text and the emitted text
/// cannot drift.
#[derive(Debug)]
pub(crate) struct EmittedFile {
    /// Where the file lands, relative to the workspace `crates/` directory.
    pub(crate) path: String,
    /// The crate that owns the file, whose SPDX header it carries.
    pub(crate) crate_name: &'static str,
    /// The rendered body, before the value-carrier guard, the SPDX header and
    /// `rustfmt`.
    pub(crate) body: String,
}

/// What an XSD-driven emit target produces: the rendered files, plus the
/// XSD-declared elements that matched no field in the model behind them and are
/// therefore reported rather than emitted.
#[derive(Debug)]
pub(crate) struct XsdEmission {
    /// The rendered files.
    pub(crate) files: Vec<EmittedFile>,
    /// The XSD-only `(type, element)` pairs skipped for want of a model field.
    pub(crate) unmatched: Vec<(String, String)>,
}

/// Write an emit target's rendered files into the workspace `crates/` tree,
/// creating each file's parent directories, and return the written paths for
/// the caller's `rustfmt` pass.
///
/// # Errors
/// Returns the underlying filesystem error.
fn write_emitted(files: &[EmittedFile]) -> std::io::Result<Vec<PathBuf>> {
    let root = crates_root();
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

pub(crate) fn run() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map_or("check", String::as_str);
    let result = match cmd {
        "check" => cmd_check(),
        "emit" => cmd_emit(args.get(1).map(PathBuf::from)),
        "check-xsd" => cmd_check_xsd(),
        "emit-xml" => cmd_emit_xml(),
        "emit-json" => cmd_emit_json(),
        "emit-rest" => cmd_emit_rest(),
        "emit-opt" => cmd_emit_opt(),
        "emit-aom2" => cmd_emit_aom2(),
        "emit-rm-model" => cmd_emit_rm_model(),
        "emit-validate" => cmd_emit_validate(),
        "model-query" => cmd_model_query(args.get(1..).unwrap_or_default()),
        other => {
            eprintln!(
                "unknown command {other:?}; use `check`, `emit [OUTDIR]`, `check-xsd`, `emit-xml`, `emit-json`, `emit-rest`, `emit-opt`, `emit-aom2`, `emit-rm-model`, `emit-validate`, or `model-query`"
            );
            return std::process::ExitCode::from(EXIT_USAGE);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

fn cmd_check() -> Result<(), Box<dyn std::error::Error>> {
    // Every generation of every composition entry: load + parse each vendored
    // file, resolve its paired dependency generations, and prove the model
    // constructible — the full input-validation pass, not a summary of three
    // files. A candidate file for a NEW generation is checked by adding its
    // table row first (the table is the single authority; there is no
    // side-channel file list).
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

/// Report what the vendored BMM states about every class attribute of the
/// loaded components, beside the Rust field shape the emitter currently emits
/// for it — a read-only query over the same LOAD → ANALYZE → PLAN → RENDER
/// decisions `emit` drives (see [`model_query`] for the BMM column definitions
/// and their `LANG` citations).
///
/// Usage: `model-query [--class NAME] [--attribute NAME] [--component KEY]
/// [--flattened] [--format table|tsv|json]`; no filter reports the whole loaded
/// model.
///
/// `--flattened` switches from one row per class × DECLARED attribute to one
/// row per class × **carried** attribute (inherited ones included), adding the
/// declaring class in the `declared_on` column — the inheritance dimension a
/// declared-only view cannot express.
fn cmd_model_query(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let (query, format) = parse_model_query_args(args)?;
    print!("{}", model_query::render(&query, format)?);
    Ok(())
}

/// Parse `model-query`'s options (`--flag value` and `--flag=value` both work).
///
/// # Errors
/// Returns an error naming the valid options if an option is unknown, and an
/// error naming the valid values if `--format` is not one of them.
fn parse_model_query_args(
    args: &[String],
) -> Result<(model_query::Query<'_>, model_query::Format), Box<dyn std::error::Error>> {
    let mut query = model_query::Query::default();
    let mut format = model_query::Format::Table;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) => (f, Some(v)),
            None => (arg.as_str(), None),
        };
        // A valueless switch must not swallow the next argument.
        if flag == "--flattened" {
            query.flattened = true;
            i += 1;
            continue;
        }
        let value = if let Some(v) = inline {
            i += 1;
            Some(v)
        } else {
            let v = args.get(i + 1).map(String::as_str);
            i += 2;
            v
        };
        let required = || -> Result<&str, Box<dyn std::error::Error>> {
            value.ok_or_else(|| format!("option {flag} needs a value").into())
        };
        match flag {
            "--class" => query.class = Some(required()?),
            "--attribute" => query.attribute = Some(required()?),
            "--component" => query.component = Some(required()?),
            "--format" => format = model_query::Format::parse(required()?)?,
            other => {
                return Err(format!(
                    "unknown model-query option {other:?}; valid options: --class NAME, \
                     --attribute NAME, --component KEY, --flattened, --format {}",
                    model_query::Format::VALID
                )
                .into());
            }
        }
    }
    Ok((query, format))
}

/// Emit the ITS-REST contract (DTOs, param structs, server trait, route table)
/// for each API group into `openehr-its/src/rest/generated/`.
fn cmd_emit_rest() -> Result<(), Box<dyn std::error::Error>> {
    let files = render_rest_files()?;
    let written = write_emitted(&files)?;
    rustfmt(&written)?;
    println!(
        "emitted {} files into {}",
        written.len(),
        crates_root().join(REST_GEN_DIR).display()
    );
    Ok(())
}

/// Render the ITS-REST contract for every API group, plus the shared `common`
/// module and the module declaration file, without touching the filesystem.
///
/// The text half of [`cmd_emit_rest`], shared with the emitter-invariant tests
/// (`testsupport::emit_rest_to_memory`), so the CLI and the tests drive one
/// orchestration.
///
/// # Errors
/// Returns an error if the RM/BASE compositions or a vendored OAS bundle cannot
/// be loaded.
pub(crate) fn render_rest_files() -> Result<Vec<EmittedFile>, Box<dyn std::error::Error>> {
    // Every API group whose OAS declares operations. `overview` is excluded
    // because it is the release's index document: it declares no `paths`.
    // `system` DOES declare one — `system-codegen.openapi.yaml` `paths` `/`
    // `options` (operationId `options`, the STABLE Options-and-Conformance
    // operation) — so it is emitted like every other group. The completeness
    // rule admits no "nothing consumes it yet" exclusion.
    const GROUPS: &[&str] = &[
        "admin",
        "definition",
        "demographic",
        "ehr",
        "query",
        "system",
    ];
    // The REST contract is emitted against the CURRENT RM/BASE generations
    // (the wire the server serves); OAS $ref names are PascalCase
    // (`EhrStatus`) — the same as the emitted Rust type names — so map each
    // current generation's emittable specs to full generation-module type
    // paths.
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

    // Load every bundle up front: the cross-group hoist analysis needs the
    // whole set (schemas identical in every declaring group hoist into the
    // shared `common` module — `emit_rest::hoist_set`).
    let mut bundles: Vec<(&str, oas::Oas)> = Vec::new();
    for group in GROUPS {
        let oas = oas::Oas::parse_file(&oas_dir.join(format!("{group}-codegen.openapi.yaml")))?;
        bundles.push((group, oas));
    }
    let hoisted = emit_rest::hoist_set(&bundles, &names);

    let mut files = Vec::new();
    // `common` always emits from the merged per-name view (first declarer
    // wins; the copies are identical by the hoist analysis) — ONE emission
    // path, so no representative-vs-merged equivalence needs testing.
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

/// Emit canonical-XML `ToXml`/`FromXml` impls for the RM/BASE spec types into
/// `openehr-its/src/xml/generated/`. Generates both wire lineages: v1
/// (`.../v1`, parity target) and v2 (`.../v2`, latest).
fn cmd_emit_xml() -> Result<(), Box<dyn std::error::Error>> {
    let emission = render_xml_files()?;
    let written = write_emitted(&emission.files)?;
    rustfmt(&written)?;
    println!(
        "emitted {} files into {} ({} XSD-only elements without a BMM field skipped)",
        written.len(),
        crates_root().join(XML_GEN_DIR).display(),
        emission.unmatched.len()
    );
    Ok(())
}

/// Render the canonical-XML `ToXml`/`FromXml` impls and their module
/// declaration file without touching the filesystem, reporting the XSD-only
/// elements that matched no BMM field.
///
/// The text half of [`cmd_emit_xml`], shared with the emitter-invariant tests
/// (`testsupport::emit_xml_to_memory`), so the CLI and the tests drive one
/// orchestration.
///
/// # Errors
/// Returns an error if the RM/BASE compositions or the vendored XSD bundles
/// cannot be loaded, or if the XML emission itself fails.
pub(crate) fn render_xml_files() -> Result<XsdEmission, Box<dyn std::error::Error>> {
    // The XML codec covers the CURRENT RM/BASE generations (the wire the
    // server serves).
    let base = compose("base")?;
    let rm = compose("rm")?;
    let base_root = format!("openehr_base::{}", base.current().spec.module);
    let rm_root = format!("openehr_rm::{}", rm.current().spec.module);
    let base_unit = base.current().unit()?;
    let rm_unit = rm.current().unit()?;
    let rm_aug = augmented_schema(rm.current(), rm_unit);

    // The two ITS-XML lineages differ only by the root `xmlns`, so ONE `ToXml`
    // impl set serves both. The v1 `ALL/` bundle is not a complete RM closure
    // (no `Ehr.xsd`, no demographic schema, a stale `Extract.xsd`), so the input
    // is the v1 served core — first-wins for shared types — followed by the v2
    // RM-1.1.0 EHR/demographic/extract schemas that supply the LOCATABLE
    // subtypes it lacks.
    let v1 = xsd::XsdModel::parse_files(&xsd::xml_emit_files(
        Path::new(XSD_V1_DIR),
        Path::new(XSD_V2_DIR),
    ))?;

    let schemas = [
        emit_xml::XmlSchema {
            model: &base_unit.model,
            schema: &base_unit.schema,
            root: &base_root,
            external: &base.current().external,
        },
        emit_xml::XmlSchema {
            model: &rm_unit.model,
            // The XML codec covers the crate's re-emitted closure twins too
            // (the BASE Interval/Iso8601 family re-emitted into openehr-rm):
            // the impls must exist for every type a field of the augmented
            // schema names.
            schema: &rm_aug,
            root: &rm_root,
            external: &rm.current().external,
        },
    ];
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

/// Emit the canonical-JSON `serde::Serialize`/`serde::Deserialize` impls for
/// every generated spec type (BASE / RM / LANG / AM 1.4 + 2.4 / TERM).
///
/// The impls land in each spec crate's own `src/json_serde.rs` — both traits and
/// the spec types are foreign to `openehr-its`, so an impl there would violate
/// the orphan rule — and the `_type`-keyed structural dispatch stays in
/// `openehr-its/src/json_codec/generated/`, where it can name every crate.
/// Covers the same crate composition `emit` consumes, including AM 2.4's
/// cross-schema re-emission closure.
fn cmd_emit_json() -> Result<(), Box<dyn std::error::Error>> {
    let files = render_json_files()?;
    let mut written = write_emitted(&files)?;
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

/// Render the canonical-JSON `serde` impls and the `_type` dispatch without
/// touching the filesystem.
///
/// The text half of [`cmd_emit_json`], shared with the emitter-invariant tests
/// (`testsupport::emit_json_to_memory`), so the CLI and the tests drive one
/// orchestration. The module declaration [`cmd_emit_json`] appends to each spec
/// crate's `lib.rs` is a read-modify-write of a file this target does not
/// render, so it stays on the CLI path.
///
/// # Errors
/// Returns an error if a composition's BMM files cannot be loaded, or if the
/// structural-dispatch priority list names an unknown composition key.
pub(crate) fn render_json_files() -> Result<Vec<EmittedFile>, Box<dyn std::error::Error>> {
    // One JsonSchema per (crate, generation), each named from its generation
    // root — every generation is codec-complete, colliding twins included
    // (see `JsonSchema::root` for the adjudication).
    // Every generation's emission schema is the augmented one (`emit` renders
    // the re-emission closure, so the codec must cover it too).
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

    // One emitted file per SPEC CRATE: the impls must live where the types are
    // defined (orphan rule), and being in-crate is also what lets them read the
    // `pub(crate)` fields of a validated class and construct through its
    // hand-written door (`plan::construction`).
    let mut files = Vec::new();
    for (c, schemas) in composed.iter().zip(&schemas_by_crate) {
        let krate = c.comp.crate_name.replace('-', "_");
        files.push(EmittedFile {
            path: format!("{}/{JSON_SERDE_FILE}", c.comp.crate_name),
            crate_name: c.comp.crate_name,
            body: emit_json::emit_file(schemas, &krate),
        });
    }

    // The structural dispatch is keyed by the bare `_type`, so a name several
    // components declare resolves by SCHEMA PRIORITY: RM, then BASE, then the
    // archetype/meta components. The caller is the RM wire-boundary validator
    // and same-named twins differ materially, so decoding an RM node with
    // another component's shape would be wrong. No openEHR spec governs a
    // cross-component `_type` namespace — our own design.
    let mut structural_schemas: Vec<emit_json::JsonSchema<'_>> = Vec::new();
    for key in ["rm", "base", "lang", "am", "term"] {
        let i = keys
            .iter()
            .position(|k| *k == key)
            .ok_or("structural priority names an unknown composition key")?;
        let schemas = schemas_by_crate
            .get(i)
            .ok_or("structural priority names an unknown composition key")?;
        // Within a crate the CURRENT generation resolves first: the dispatch's
        // caller is the wire validator for the SERVED wire, so an older
        // generation's twin shape must never shadow the current one.
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

/// Declare the emitted `json_serde` module in a spec crate's generated
/// `lib.rs`.
///
/// `emit` regenerates `lib.rs` (and deletes every `@generated` file, this one
/// included), so the declaration is (re)appended by the target that owns the
/// file. Both intermediate states are self-consistent: after `emit` neither the
/// module nor its declaration exists, after `emit-json` both do.
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

/// Render one XSD-driven constraint-model module (`types.rs` + `impls.rs` +
/// `mod.rs`) destined for `crates/openehr-its/src/<dir>`.
///
/// Shared by every `emit_opt` target so the three modules (`opt14`, `aom2`,
/// `aom2_model`) stay structurally identical: only the XSD closure, the emission
/// target (module path + banners) and the module surface differ. The generate/
/// resolve partition is the same in all three — the closures share
/// `Resource.xsd`+`BaseTypes.xsd` with the RM-instance set, and those shared types
/// resolve to the already-generated `openehr-base`/`openehr-rm` XML impls while the
/// archetype constraint model is generated fresh.
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

/// Emit the OPT 1.4 model (`opt14`): typed Rust types + canonical-XML
/// `ToXml`/`FromXml` for `OPERATIONAL_TEMPLATE`, generated from the AM/OPT
/// constraint XSD closure (`Template.xsd` + includes). RM instance types
/// resolve to the already-generated `openehr-base`/`openehr-rm` impls.
fn cmd_emit_opt() -> Result<(), Box<dyn std::error::Error>> {
    write_xsd_emission("emit-opt", &render_opt_files()?)
}

/// Render the OPT 1.4 model without touching the filesystem.
///
/// The text half of [`cmd_emit_opt`], shared with the emit-target tests
/// (`testsupport::emit_opt_to_memory`), so the CLI and the tests drive one
/// orchestration.
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

/// Emit BOTH AOM2 archetype XML serializations the vendored bundle publishes,
/// each from its own closure into its own module:
///
/// - `aom2` — the **persistent** form (`P_Archetype.xsd` → `P_AUTHORED_ARCHETYPE`),
///   the shape the bundle's 8 example documents carry;
/// - `aom2_model` — the AOM **model** form (`Archetype.xsd` → `AUTHORED_ARCHETYPE`).
///
/// They are two closures rather than one merged model because both schemas
/// declare the same top-level element `archetype` with different root types and
/// define same-named supporting types; see [`xsd::AOM2_FILES`] /
/// [`xsd::AOM2_MODEL_FILES`] for the full adjudication, including why the model
/// form's entry points are typed to `AUTHORED_ARCHETYPE` and not to the
/// `abstract` `ARCHETYPE` the schema's global element names.
fn cmd_emit_aom2() -> Result<(), Box<dyn std::error::Error>> {
    write_xsd_emission("emit-aom2", &render_aom2_files()?)
}

/// Render both AOM2 archetype serializations without touching the filesystem.
///
/// The text half of [`cmd_emit_aom2`], shared with the emit-target tests
/// (`testsupport::emit_aom2_to_memory`), so the CLI and the tests drive one
/// orchestration. Both closures' files (and both skipped-element reports) come
/// back as one emission — the subcommand writes them in one pass.
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

/// Write an XSD-driven target's rendered files and report what it skipped.
///
/// # Errors
/// Returns the underlying filesystem error, or a `rustfmt` failure.
fn write_xsd_emission(
    target: &str,
    emission: &XsdEmission,
) -> Result<(), Box<dyn std::error::Error>> {
    let written = write_emitted(&emission.files)?;
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

/// Emit the static RM attribute/type model (`openehr-rm/src/model/`) — the AQL
/// planner's spec-pinned oracle. Generated from the same
/// BASE + RM BMM `emit` consumes. Writes the `model/` subtree in place (does not
/// touch the generated spec files) and declares `pub mod model;` in `lib.rs` if
/// absent, so it is correct run standalone; `emit` produces the identical output.
fn cmd_emit_rm_model() -> Result<(), Box<dyn std::error::Error>> {
    let files = render_rm_model_files()?;
    let mut written = write_emitted(&files)?;
    let src = crates_root().join(RM_CRATE).join("src");
    for g in &compose("rm")?.generations {
        // `emit` is the usual authority for the generation `mod.rs`, where it
        // declares the module in the RENDERED body (`inject_rm_model`). This
        // target does not render that file, so the standalone run declares the
        // module by read-modify-write — the same text, so both runs leave the
        // tree byte-identical.
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

/// Render the static RM attribute/type model for every RM generation, without
/// touching the filesystem.
///
/// The text half of [`cmd_emit_rm_model`], shared with `emit`
/// ([`render_emit_files`]) and the emit-target tests
/// (`testsupport::emit_rm_model_to_memory`), so all three drive one
/// orchestration. The `pub mod model;` declaration [`cmd_emit_rm_model`] adds to
/// each generation's `mod.rs` is a read-modify-write of a file this target does
/// not render, so it stays on the CLI path — over the same [`RM_MODEL_DECL`]
/// text [`inject_rm_model`] appends to `emit`'s rendered body.
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

/// Emit the RM class-invariant cores (`openehr-rm/src/validate/generated.rs`) —
/// the single source both the typed `Validate` impls and the fast path call.
/// Writes the one generated file in place; the module declaration
/// (`pub(crate) mod generated;`) is a permanent hand edit in the hand-written
/// `validate.rs`, so this target never touches a hand-written file. `emit`
/// produces the identical output (via `inject_validate`).
fn cmd_emit_validate() -> Result<(), Box<dyn std::error::Error>> {
    let files = render_validate_files()?;
    let written = write_emitted(&files)?;
    rustfmt(&written)?;
    println!(
        "emitted {} file(s) into {}",
        files.len(),
        crates_root().join(RM_CRATE).join("src").display()
    );
    Ok(())
}

/// Render the RM class-invariant cores for every RM generation, without
/// touching the filesystem.
///
/// The text half of [`cmd_emit_validate`], shared with `emit`
/// ([`render_emit_files`]) and the emit-target tests
/// (`testsupport::emit_validate_to_memory`), so all three drive one
/// orchestration.
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
    }
    Ok(out)
}

/// Prefix every generated file path with the generation module directory
/// (`model/data.rs` → `v1_2/model/data.rs`) — the RM model/validate subtrees
/// are generation-scoped like the type files they describe.
fn prefix_gen_files(files: Vec<GenFile>, module: &str) -> Vec<GenFile> {
    files
        .into_iter()
        .map(|f| GenFile {
            path: format!("{module}/{}", f.path),
            body: f.body,
        })
        .collect()
}

/// Append the generated invariant-core file to `openehr-rm`'s file set. The
/// module is declared by the permanent `pub(crate) mod generated;` hand edit in
/// the hand-written `validate.rs` (inside the generation module), so nothing
/// else is touched here.
fn inject_validate(files: &mut Vec<GenFile>, mut validate_files: Vec<GenFile>) {
    files.append(&mut validate_files);
}

/// The declaration an RM generation's `mod.rs` carries for the emitted static
/// RM model.
///
/// One text, two writers — [`inject_rm_model`] appends it to `emit`'s rendered
/// body, [`cmd_emit_rm_model`] to the file on disk — so the standalone target
/// and `emit` cannot disagree about the declaration.
const RM_MODEL_DECL: &str = "pub mod model;";

/// Append the generated RM-model files to `openehr-rm`'s file set and declare
/// the module in its generation `mod.rs` (the authority for the generation's
/// layout). `model_files` are already generation-prefixed
/// ([`prefix_gen_files`]).
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
fn cmd_check_xsd() -> Result<(), Box<dyn std::error::Error>> {
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
/// grafted in (`cross_schema_reemit` → `augment_with_reemit`): every upstream
/// class whose Rust form WIDENS in this generation (a closed-subtype-set enum
/// the generation's own classes extend) is re-emitted crate-locally at its
/// source package path, so downstream references resolve against the widened
/// local twin — the completeness hard rule (owner 2026-07-19), applied
/// uniformly to EVERY generation (rm re-emits BASE's `Interval`/`Iso8601_type`
/// family; AM 1.4 re-emits `AUTHORED_RESOURCE` + `RESOURCE_DESCRIPTION`). A
/// generation with an empty closure comes back unchanged, so applying this
/// unconditionally is safe by construction.
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

/// Spec class name → the full generation-module path of its defining module
/// (`openehr_rm::v1_2::…`), over one generation's emittable specs — the map
/// shape the XSD/OAS emitters resolve shared types against.
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

/// `PascalCase` Rust type name → full generation-module TYPE path
/// (`openehr_rm::v1_2::…::EhrStatus`), over one generation's emittable specs —
/// the map shape `emit-rest` resolves OAS `$ref` names against.
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

/// The hand-written `*_impl.rs` siblings a generated crate already carries — an
/// emitter INPUT (`crate::load::impls`), read before rendering so a type file's
/// banner names a sibling only when one exists.
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

fn cmd_emit(_outdir: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
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

/// Render every generated spec crate's `src/` tree without touching the
/// filesystem.
///
/// The text half of [`cmd_emit`], shared with the emit-target tests
/// (`testsupport::emit_to_memory`), so the CLI and the tests drive one
/// orchestration — the whole file set included, generation-twin template stamps
/// and all. The purge, the hand-written-module weave and the `rustfmt` pass are
/// filesystem operations over files this target does not render, so they stay on
/// the CLI path.
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

/// Render one composition entry's whole file set: the spec types, the RM
/// crate's model + invariant cores, and the generation-twin template stamps.
///
/// ONE uniform path for every crate, driven by the declarative
/// `plan::composition::COMPOSITIONS` table (see it for the `includes`
/// citations). `cross_schema_reemit` grafts the COMPLETE set of upstream classes
/// whose Rust form widens in a generation — a full, non-minimal re-emission
/// (owner ruling 2026-07-19). `openehr-rm` additionally carries the
/// static RM model and the invariant cores, emitted here so a plain `emit` keeps
/// the crate self-consistent.
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
        // EVERY RM generation carries its own attribute model + invariant cores
        // (the same uniform rule as the per-generation codecs): a selectable
        // generation is a complete peer, not a types-only shell.
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
        }
    }
    // Generation-twin templates: one hand-written source per family, stamped
    // per generation (render::emit_templates). A path collision with a
    // generated file is a defect, never a silent overwrite.
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

/// Returns the exact bytes an emitted file carries before `rustfmt`: the
/// value-carrier guard where it applies, then the SPDX header its destination
/// crate's licensing requires, then the rendered body.
///
/// Every emitted file goes through here — [`write_generated`] on the CLI path,
/// `testsupport` on the in-memory one — so a new emission site cannot ship a
/// file whose licensing does not travel with it, and the tested bytes are the
/// written bytes.
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

/// Write one emitted file, with the guard + SPDX header [`generated_bytes`]
/// applies.
///
/// # Errors
/// Returns the underlying filesystem error.
fn write_generated(path: &Path, crate_name: &str, body: &str) -> std::io::Result<()> {
    std::fs::write(path, generated_bytes(crate_name, body))
}

/// Write a generated crate's `src/` in place. Wipes the crate's previously
/// generated files first, preserving the hand-written siblings beside them.
///
/// Each file's `path` is relative to the workspace `crates/` directory, exactly
/// as [`render_emit_files`] addresses it.
fn write_crate(crate_name: &str, files: &[&EmittedFile]) -> Result<(), Box<dyn std::error::Error>> {
    let src = crates_root().join(crate_name).join("src");
    // Preserve hand-written code: delete only previously-`@generated`
    // files, never the hand-written `*_impl.rs` / spec-behaviour modules beside
    // them. (A stale generated file no longer emitted this run is `@generated` →
    // removed; a hand-written sibling is kept.)
    if src.exists() {
        remove_generated_files(&src)?;
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
    // Weave hand-written modules into the generated tree: any hand-written `.rs`
    // beside a generated `mod.rs` (or at the crate root beside `lib.rs`) is
    // declared `pub mod <name>;` so the hand-written `*_impl.rs` files compile without the
    // generator owning them. Deterministic (sorted scan) → drift-check-stable.
    declare_hand_written_modules(&src, &mut written)?;
    rustfmt(&written)?;
    println!("emitted {} files into {}", files.len(), src.display());
    Ok(())
}

/// What `emit` alone leaves behind, printed at the end of every run.
///
/// `emit` purges each crate's `json_serde.rs` and its declaration, and only
/// `emit-json` writes them back, so the workspace does not compile in between.
/// The failure then surfaces far from its cause as unsatisfied
/// `serde::Serialize` bounds in whichever crate builds first, and the recovery
/// costs a full rebuild. Saying so at the end of the run is cheaper than
/// discovering it.
const INCOMPLETE_AFTER_EMIT: &str = "\
note: `emit` alone leaves the workspace UNBUILDABLE — it removes each crate's
      json_serde.rs, which only `emit-json` writes back. Run the full set:
        emit-json emit-xml emit-rest emit-opt emit-aom2 emit-rm-model emit-validate
      or use the /regen-codegen skill, which runs them all plus the drift check.";

/// Whether a file's first line marks it as generated (`// @generated …`).
fn is_generated_file(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .is_ok_and(|s| s.lines().next().is_some_and(|l| l.contains("@generated")))
}

/// Recursively delete every `@generated` file under `dir` (leaving hand-written
/// files + their directories in place), then prune directories left empty.
fn remove_generated_files(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            remove_generated_files(&path)?;
            if std::fs::read_dir(&path)?.next().is_none() {
                std::fs::remove_dir(&path)?;
            }
        } else if is_generated_file(&path) {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// For each directory whose module anchor (`lib.rs` at the crate root, else
/// `mod.rs`) is **generated**, append `pub mod <name>;` for every hand-written
/// child module not already declared — both hand-written `.rs` files
/// (`foo_impl.rs`) and hand-written module directories (a subdir whose own
/// `mod.rs` is hand-written, e.g. `odin/`). A hand-written anchor is left
/// untouched: it manages its own submodules (so we never duplicate an `odin/`
/// mod.rs's private `mod lexer;`). Modified anchors are added to `written` for
/// rustfmt.
fn declare_hand_written_modules(
    src: &Path,
    written: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    // (dir, module-anchor file) pairs: the root anchors on lib.rs, each subdir on
    // its mod.rs. Collect all dirs first (sorted) for deterministic output.
    let mut dirs = vec![src.to_path_buf()];
    collect_dirs(src, &mut dirs)?;
    dirs.sort();
    for dir in dirs {
        let anchor = if dir == src {
            dir.join("lib.rs")
        } else {
            dir.join("mod.rs")
        };
        // Only extend a GENERATED anchor. A hand-written `mod.rs` (a hand-written
        // module directory) owns its own `mod`/`pub mod` declarations, so
        // appending here would duplicate them.
        let anchor_generated = is_generated_file(&anchor);
        if !anchor.exists() || !anchor_generated {
            continue;
        }
        let hand_written = hand_written_module_stems(&dir)?;
        if hand_written.is_empty() {
            continue;
        }
        if append_module_declarations(&anchor, &hand_written)? {
            written.push(anchor);
        }
    }
    Ok(())
}

/// The module stems in `dir` the generated module tree does not declare,
/// sorted.
///
/// A hand-written module DIRECTORY is one whose own `mod.rs` is not generated;
/// a fully-generated subpackage's `mod.rs` is already declared by the parent's
/// generated module tree and is skipped. Template-stamped copies ARE generated
/// files, but the generated module tree does not know them — they are woven
/// exactly like the hand-written siblings they replace.
///
/// # Errors
/// A directory that cannot be read.
fn hand_written_module_stems(dir: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut stems: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        let Some(stem) = p.file_stem().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        if p.is_dir() {
            let child_mod = p.join("mod.rs");
            if child_mod.exists() && !is_generated_file(&child_mod) {
                stems.push(stem.to_owned());
            }
        } else if p.extension().and_then(std::ffi::OsStr::to_str) == Some("rs")
            && !matches!(stem, "mod" | "lib" | "prelude")
            && (!is_generated_file(&p) || emit_templates::is_template_stamped(&p))
        {
            stems.push(stem.to_owned());
        }
    }
    stems.sort();
    Ok(stems)
}

/// Appends the missing `pub mod` declarations to a generated anchor,
/// reporting whether it was modified.
///
/// # Errors
/// A read or write failure on the anchor.
fn append_module_declarations(
    anchor: &Path,
    stems: &[String],
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut body = std::fs::read_to_string(anchor)?;
    let mut appended = false;
    for m in stems {
        let decl = format!("pub mod {m};");
        if body.contains(&decl) {
            continue;
        }
        if !appended {
            body.push_str("\n// hand-written modules (spec behaviour), auto-declared:\n");
            appended = true;
        }
        body.push_str(&decl);
        body.push('\n');
    }
    if appended {
        std::fs::write(anchor, &body)?;
    }
    Ok(appended)
}

/// Recursively collect subdirectories of `dir` into `out`.
fn collect_dirs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            out.push(path.clone());
            collect_dirs(&path, out)?;
        }
    }
    Ok(())
}

/// Run `rustfmt` over the generated files so the emitted output is exactly what
/// `cargo fmt --all --check` expects (line wrapping, empty `{}`, import order,
/// …), instead of the emitter having to reproduce every rustfmt rule by hand.
fn rustfmt(files: &[PathBuf]) -> Result<(), Box<dyn std::error::Error>> {
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

    /// The note must name `emit-json` — the one target that restores what
    /// `emit` removed. A note that only says "run the others" sends the reader
    /// back to the documentation the foot-gun already survived.
    #[test]
    fn the_note_names_the_target_that_restores_the_tree() {
        assert!(INCOMPLETE_AFTER_EMIT.contains("emit-json"));
        assert!(INCOMPLETE_AFTER_EMIT.contains("json_serde.rs"));
        assert!(INCOMPLETE_AFTER_EMIT.contains("UNBUILDABLE"));
    }
}
