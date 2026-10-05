// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The **crate → generation table**: declarative data recording, per emitted
//! spec crate, the BMM GENERATIONS it carries — vendored input file, emitted
//! generation-module name, per-generation spec version, which generation is
//! CURRENT, and the dependency GENERATIONS each one resolves against. Each
//! entry carries the `includes` citation that justifies its composition.
//!
//! This is the single source of truth for what the generator emits. [`compose`]
//! resolves an entry into per-generation loaded schemas + resolution models +
//! the [`External`] full-path index the render stage consumes, so `cli.rs`
//! never hand-merges schemas — the membership is data, not control flow.
//!
//! One generation is one COMPONENT VERSION, emitted completely at its own
//! version-named top module (`v1_2`, `v2_4`, …) mirroring its source package
//! structure. A component version can publish several specification units
//! (LANG 1.1.0 publishes BMM v2.x beside BMM3); each unit is emitted completely
//! inside the one generation module and units are never merged into one class
//! map, because a merge picks one shape per colliding name and discards the
//! other's attributes. The prelude carries the CURRENT generation's stable
//! units; older generations are reached by full module path.
//!
//! NOTE: five classes are declared by BOTH the RM 1.2.0 and BASE 1.3.0 BMM with
//! different shapes, and both twins are emitted, because RM
//! `docs/common/master08-resource_package.adoc` retains its Resource package
//! "only while needed by AOM 1.4 based archetypes and tools" and BASE
//! `docs/foundation_types/master00-amendment_record.adoc` records SPECAM-82
//! adding a legacy `CODE_PHRASE` for the same reason.
//!
//! The RM twin's `TRANSLATION_DETAILS.accreditaton` spelling is an upstream
//! defect the emitter reproduces: SPECPUB-6 corrected the BASE copy alone.

use crate::analyze::{External, Model, emittable_specs};
use crate::load::bmm::BmmSchema;
use std::collections::BTreeMap;
use std::path::Path;

/// The vendored BMM root. Paths below mirror the upstream ITS-BMM layout
/// (`components/<COMPONENT>/json/…`); the JSON forms are the codegen input for
/// our pinned versions (each schema's pin is recorded in its `PROVENANCE.md`).
const VENDOR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/bmm");

pub(crate) const BASE_BMM: &str = "components/BASE/json/openehr_base_1.3.0.bmm.json";
/// BASE's latest RELEASED generation (1.2.0, 09-Apr-2021) — the `stable`
/// profile's BASE pairing, RM 1.1.0 being modelled against it.
pub(crate) const BASE12_BMM: &str = "components/BASE/json/openehr_base_1.2.0.bmm.json";
pub(crate) const RM_BMM: &str = "components/RM/json/openehr_rm_1.2.0.bmm.json";
/// RM's latest RELEASED generation (1.1.0, 29-Sep-2020); its BMM `includes`
/// names `openehr_base_1.2.0` — the released pairing, first-hand.
pub(crate) const RM11_BMM: &str = "components/RM/json/openehr_rm_1.1.0.bmm.json";
pub(crate) const TERM_BMM: &str = "components/TERM/json/openehr_term_3.1.0.bmm.json";
pub(crate) const AM14_BMM: &str = "components/AM/json/openehr_am_1.4.0.bmm.json";
pub(crate) const AM24_BMM: &str = "components/AM/json/openehr_am_2.4.0.bmm.json";
/// LANG's **v2.x generation**: the stable BMM object model (`org.openehr.lang.bmm`),
/// its `P_BMM` persistence form (`…bmm_persistence`) and the beom expression model
/// (`…beom`, with `EXPR_*` and `STATEMENT_SET`/`ASSERTION`, which AM's rules/slots
/// reference). `LANG/docs/bmm/master01-preface.adoc` §History calls this "the
/// normative, tool-implemented version".
/// LANG's released 1.0.0 machine-readable BMM, emitted FAITHFULLY despite its
/// published defects: it declares no `includes`, so its BASE references stay
/// open slots, and BMM is TRIAL in that release.
pub(crate) const LANG10_BMM: &str = "components/LANG/json/openehr_lang_1.0.0.bmm.json";
pub(crate) const LANG_BMM: &str = "components/LANG/json/openehr_lang_1.1.0.bmm.json";
/// LANG's **v3 generation** (`org.openehr.lang.bmm3`): the evolved `BMM_*` object
/// model with the `EL_*` expression language and the `BMM_STATEMENT*` family,
/// specified separately per `LANG/docs/bmm3/master00-amendment_record.adoc`
/// (SPECLANG-14, "Formalise the BMM v2/v3 split"). It is a second GENERATION of
/// the same meta-model, not a second part of one model: 18 class names exist in
/// both files with materially different shapes, so the two are emitted side by
/// side (the AM precedent —
/// `BASE/docs/architecture_overview/master05-package_structure.adoc` §AM
/// Component, "Both versions are maintained side by side").
pub(crate) const LANG_BMM3: &str = "components/LANG/json/openehr_lang_1.1.0-bmm3.bmm.json";

const BASE_DOC: &str = "openEHR BASE (foundation + base types), generated from the BMM meta-model.";
const RM_DOC: &str = "openEHR RM (Reference Model), generated from the BMM meta-model.";
const LANG_DOC: &str = "openEHR LANG, generated from the BMM meta-model: one generation per \
    component version. `v1_1` (the 1.1.0 development line, the current generation) carries \
    the version's published specification units side by side — the STABLE, tool-implemented \
    BMM v2.x model (`bmm`, its `bmm_persistence` P_BMM form, the `beom` BEL expression model; \
    on the prelude) and the PAUSED BMM3 model (`bmm3`, full-path only) — plus the hand-written \
    ODIN/BEL/EL readers and the shared lexer for that version's notations. The generator's own \
    BMM reader lives in openehr-codegen (tooling, not spec).";

const AM_DOC: &str = "openEHR AM (Archetype Model): `v1_4` (AM 1.4.0, for ADL 1.4) and `v2_4` \
    (AM 2.4.0, for ADL 2) — both generated from BMM. Both ADL versions are in use.";
const TERM_DOC: &str = "openEHR TERM (Terminology) data model, generated from the BMM \
    meta-model. The vendored terminology XML content lives in `assets/` (data, not \
    generated); an XML→model loader is added when composition validation needs it.";

/// One dependency **generation** a generation resolves against: the dependency
/// crate's composition key plus the generation module inside it.
pub(crate) struct DepGeneration {
    /// Composition key of the dependency crate (`base`, `lang`, …).
    pub key: &'static str,
    /// The generation module inside that crate (`v1_3`, `v2`, …).
    pub generation: &'static str,
}

/// One vendored spec file composing a generation — a COMPONENT VERSION can
/// publish several machine-readable specifications side by side (LANG 1.1.0
/// publishes the BMM v2.x model AND the paused BMM3 model), each emitted
/// completely at its own package paths inside the one generation module.
pub(crate) struct GenerationUnit {
    /// The vendored BMM file (relative to the vendor root).
    pub file: &'static str,
    /// Whether the generation's prelude (and the crate prelude, when this
    /// generation is current) re-exports this unit's types. The stable
    /// specifications of a component version are on the prelude; a paused /
    /// trial sibling specification (LANG's BMM3) is reachable by full module
    /// path only, which also keeps the prelude collision-free where two
    /// units of one version declare the same class names.
    pub in_prelude: bool,
}

/// One openEHR COMPONENT-VERSION generation of an emitted crate, emitted
/// completely under its own version-named top module.
pub(crate) struct GenerationSpec {
    /// The emitted generation-module name (`v1_2`), derived from the
    /// component version the generation's files self-identify as.
    pub module: &'static str,
    /// The openEHR specification version this generation implements (the
    /// vendored files pin), emitted as the crate `Generation` enum variant's
    /// `spec_version()` value — the only place it appears.
    pub spec_version: &'static str,
    /// The vendored spec files composing this component version, in
    /// declaration order. Their emitted package paths must be disjoint
    /// (the emitter asserts it); for cross-crate reference resolution the
    /// LAST unit declaring a name wins (mirrors the retired merged-view
    /// semantics AM 2.4's LANG closure was built against).
    pub units: &'static [GenerationUnit],
    /// Whether this is the crate's CURRENT generation: the one the crate
    /// prelude re-exports and the emitted `Generation` enum's derived
    /// `Default` variant marks.
    /// Exactly one generation per crate is current ([`compose`] asserts it).
    pub current: bool,
    /// Dependency generations merged (in order, before this generation's own
    /// schema) into the resolution model — last-wins on name collision.
    pub model_deps: &'static [DepGeneration],
    /// Dependency generations whose exported names resolve this generation's
    /// cross-crate references, consulted in order (FIRST match wins — listing
    /// two generations of one dependency decides collisions by list order).
    pub prelude_deps: &'static [DepGeneration],
}

/// One emitted crate and the BMM generations that compose it.
pub(crate) struct CrateComposition {
    /// Unique key (`base`, `rm`, `lang`, `am`, `term`).
    pub key: &'static str,
    /// Emitted crate directory.
    pub crate_name: &'static str,
    /// The crate's BMM generations, oldest first. Exactly one is `current`.
    pub generations: &'static [GenerationSpec],
    /// Crate doc comment (emitted into `lib.rs`).
    pub doc: &'static str,
    /// The `includes` citation that justifies the composition.
    pub citation: &'static str,
    /// One-line reason.
    pub reason: &'static str,
}

/// The declarative crate → generation table.
pub(crate) const COMPOSITIONS: &[CrateComposition] = &[
    CrateComposition {
        key: "base",
        crate_name: "openehr-base",
        generations: &[
            GenerationSpec {
                module: "v1_2",
                spec_version: "1.2.0",
                units: &[GenerationUnit {
                    file: BASE12_BMM,

                    in_prelude: true,
                }],
                current: false,
                model_deps: &[],
                prelude_deps: &[],
            },
            GenerationSpec {
                module: "v1_3",
                spec_version: "1.3.0",
                units: &[GenerationUnit {
                    file: BASE_BMM,

                    in_prelude: true,
                }],
                current: true,
                model_deps: &[],
                prelude_deps: &[],
            },
        ],
        doc: BASE_DOC,
        citation: "BASE BMMs (openehr_base_1.2.0 released + openehr_base_1.3.0 development) — no \
                   includes; the foundation crate. Both generations emitted side by side \
                   (#1936: the released generation stays selectable).",
        reason: "Foundation types; nothing below it.",
    },
    CrateComposition {
        key: "rm",
        crate_name: "openehr-rm",
        generations: &[
            GenerationSpec {
                module: "v1_1",
                spec_version: "1.1.0",
                units: &[GenerationUnit {
                    file: RM11_BMM,

                    in_prelude: true,
                }],
                current: false,
                model_deps: &[DepGeneration {
                    key: "base",
                    generation: "v1_2",
                }],
                prelude_deps: &[DepGeneration {
                    key: "base",
                    generation: "v1_2",
                }],
            },
            GenerationSpec {
                module: "v1_2",
                spec_version: "1.2.0",
                units: &[GenerationUnit {
                    file: RM_BMM,

                    in_prelude: true,
                }],
                current: true,
                model_deps: &[DepGeneration {
                    key: "base",
                    generation: "v1_3",
                }],
                prelude_deps: &[DepGeneration {
                    key: "base",
                    generation: "v1_3",
                }],
            },
        ],
        doc: RM_DOC,
        citation: "RM 1.2.0 BMM includes openehr_base_1.3.0; RM 1.1.0 BMM includes \
                   openehr_base_1.2.0 — each generation resolves against its OWN released \
                   pairing, first-hand from the files' `includes`. Five class names are \
                   declared by both an RM and its paired BASE file and the RM declaration \
                   wins the merge, which is correct in every case — see the module NOTE on \
                   the RM/BASE twin classes.",
        reason: "The domain model; RM 1.2.0 pairs with BASE 1.3.0, RM 1.1.0 with BASE 1.2.0.",
    },
    CrateComposition {
        key: "lang",
        crate_name: "openehr-lang",
        generations: &[
            GenerationSpec {
                module: "v1_0",
                spec_version: "1.0.0",
                units: &[GenerationUnit {
                    file: LANG10_BMM,
                    in_prelude: true,
                }],
                current: false,
                // The released file declares NO includes, so its BASE
                // references stay open slots; emitted verbatim.
                model_deps: &[],
                prelude_deps: &[],
            },
            GenerationSpec {
                module: "v1_1",
                spec_version: "1.1.0",
                units: &[
                    GenerationUnit {
                        file: LANG_BMM,
                        in_prelude: true,
                    },
                    GenerationUnit {
                        file: LANG_BMM3,
                        in_prelude: false,
                    },
                ],
                current: true,
                model_deps: &[DepGeneration {
                    key: "base",
                    generation: "v1_3",
                }],
                prelude_deps: &[DepGeneration {
                    key: "base",
                    generation: "v1_3",
                }],
            },
        ],
        doc: LANG_DOC,
        citation: "LANG 1.1.0 BMM includes openehr_base_1.3.0. ONE component-version \
                   generation (`v1_1`) composed of TWO published specification units, per the \
                   component's own index (LANG development 1.1.0 lists BMM — STABLE, \"the \
                   v2.x form in use by current tooling\" — and BMM3 — PAUSED — as sibling \
                   specifications of one version; SPECLANG-14, \
                   LANG/docs/bmm3/master00-amendment_record.adoc, formalised the split): the \
                   v2.x model file (bmm + bmm_persistence + beom packages, on the prelude) \
                   and the BMM3 file (the bmm3 package, full-path only — paused upstream, \
                   in-repo hold record #1920). 18 class names occur in both units with \
                   materially different shapes; the units' package paths are disjoint so \
                   both are emitted completely, and cross-crate resolution takes the LAST \
                   unit's declaration (the retired merged-view semantics AM 2.4's LANG \
                   closure was built against). The released LANG 1.0.0 machine-readable BMM \
                   emits the `v1_0` generation FAITHFULLY, its published defects carried \
                   verbatim (no `includes`, so BASE references stay open slots; unnamed \
                   BMM_CLASS/BMM_PACKAGE; an obsolete-elom package; BMM is TRIAL in that \
                   release) — the defect class is reported upstream, never worked around \
                   here.",
        reason: "The BMM/P_BMM object model, both extant generations; depends on BASE.",
    },
    CrateComposition {
        key: "am",
        crate_name: "openehr-am",
        generations: &[
            GenerationSpec {
                module: "v1_4",
                spec_version: "1.4.0",
                units: &[GenerationUnit {
                    file: AM14_BMM,

                    in_prelude: true,
                }],
                current: false,
                model_deps: &[DepGeneration {
                    key: "base",
                    generation: "v1_3",
                }],
                prelude_deps: &[
                    DepGeneration {
                        key: "base",
                        generation: "v1_3",
                    },
                    DepGeneration {
                        key: "lang",
                        generation: "v1_1",
                    },
                ],
            },
            GenerationSpec {
                module: "v2_4",
                spec_version: "2.4.0",
                units: &[GenerationUnit {
                    file: AM24_BMM,

                    in_prelude: true,
                }],
                current: true,
                model_deps: &[
                    DepGeneration {
                        key: "base",
                        generation: "v1_3",
                    },
                    DepGeneration {
                        key: "lang",
                        generation: "v1_1",
                    },
                ],
                prelude_deps: &[
                    DepGeneration {
                        key: "base",
                        generation: "v1_3",
                    },
                    DepGeneration {
                        key: "lang",
                        generation: "v1_1",
                    },
                ],
            },
        ],
        doc: AM_DOC,
        citation: "AM 1.4.0 BMM includes openehr_base_1.3.0; ARCHETYPE extends BASE's \
                   AUTHORED_RESOURCE, whose Rust form widens downstream, so the \
                   AUTHORED_RESOURCE + RESOURCE_DESCRIPTION closure re-emits crate-locally \
                   (#1699; `augmented_schema`). AM 2.4.0 BMM includes openehr_lang_1.1.0 + \
                   openehr_base_1.3.0; its rules package declares subtypes of LANG's beom \
                   expression classes (EXPR_ARCHETYPE_REF ⊂ EXPR_VALUE_REF, EXPR_CONSTRAINT ⊂ \
                   EXPR_LEAF), so the downstream re-emission closure is non-empty. It reaches \
                   classes of BOTH LANG generations (v2 beom + v3 BMM_*/EL_*), so its deps \
                   list both — model merge last-wins (v2 then v3), reference lookup first-wins \
                   (v3 then v2), preserving the pre-table one-name-per-type resolution.",
        reason: "Both extant ADL generations, side by side (BASE \
                 architecture_overview master05 §AM Component).",
    },
    CrateComposition {
        key: "term",
        crate_name: "openehr-term",
        generations: &[GenerationSpec {
            module: "v3_1",
            spec_version: "3.1.0",
            units: &[GenerationUnit {
                file: TERM_BMM,

                in_prelude: true,
            }],
            current: true,
            model_deps: &[DepGeneration {
                key: "base",
                generation: "v1_3",
            }],
            prelude_deps: &[DepGeneration {
                key: "base",
                generation: "v1_3",
            }],
        }],
        doc: TERM_DOC,
        citation: "TERM 3.1.0 BMM includes openehr_base_1.3.0 (TERMINOLOGY.date : Iso8601_date).",
        reason: "Terminology data model; depends on BASE.",
    },
];

/// Look up a composition entry by key.
///
/// # Errors
/// Returns an error if `key` names no composition entry (a codegen bug).
pub(crate) fn lookup(key: &str) -> Result<&'static CrateComposition, Box<dyn std::error::Error>> {
    COMPOSITIONS
        .iter()
        .find(|c| c.key == key)
        .ok_or_else(|| format!("unknown composition key {key:?}").into())
}

/// Load and parse one vendored BMM file (relative to [`VENDOR`]).
///
/// # Errors
/// Returns an error if the file cannot be read or parsed.
pub(crate) fn load_bmm(file: &str) -> Result<BmmSchema, Box<dyn std::error::Error>> {
    let src = std::fs::read_to_string(Path::new(VENDOR).join(file))?;
    Ok(BmmSchema::parse_json(&src)?)
}

/// One resolved specification unit of a composed generation: its loaded
/// schema and its resolution model (paired dependency generations below,
/// this unit's own schema on top).
pub(crate) struct ComposedUnit {
    /// The table row this unit was resolved from.
    pub spec: &'static GenerationUnit,
    /// This unit's own schema — one vendored file, verbatim.
    pub schema: BmmSchema,
    /// The merged resolution model (the generation's dependency schemas,
    /// then this unit alone) — a class of one unit never resolves an
    /// ancestor, field type or subtype against a sibling unit's definitions
    /// (LANG's BMM and BMM3 units declare 18 colliding names with different
    /// shapes).
    pub model: Model,
}

/// One resolved COMPONENT-VERSION generation of a composed crate.
pub(crate) struct ComposedGeneration {
    /// The table row this generation was resolved from.
    pub spec: &'static GenerationSpec,
    /// The generation's specification units, in table order.
    pub units: Vec<ComposedUnit>,
    /// The paired dependency generations' schemas (`model_deps`, in order).
    pub dep_schemas: Vec<BmmSchema>,
    /// The full-path index resolving `prelude_deps` cross-crate references.
    pub external: External,
}

/// A composition entry resolved into its per-generation schemas + models.
pub(crate) struct Composed {
    /// The composition entry this was resolved from.
    pub comp: &'static CrateComposition,
    /// The crate's generations, in table (oldest-first) order.
    pub generations: Vec<ComposedGeneration>,
}

impl Composed {
    /// The crate's CURRENT generation — the prelude/default surface.
    ///
    /// # Panics
    /// Panics if the table declares no current generation for the crate
    /// ([`compose`] validates exactly-one-current first, so this is
    /// unreachable on a composed value).
    #[must_use]
    #[expect(
        clippy::expect_used,
        reason = "compose() rejects a table row without exactly one current generation before \
                  constructing Composed, so a composed value always has one"
    )]
    pub(crate) fn current(&self) -> &ComposedGeneration {
        self.generations
            .iter()
            .find(|g| g.spec.current)
            .expect("a composed crate should carry exactly one current generation")
    }
}

impl ComposedGeneration {
    /// Returns the generation's single specification unit.
    ///
    /// Most component versions publish exactly one machine-readable unit;
    /// a caller that can only consume one (the XML/REST/OPT emits over the
    /// current RM/BASE) uses this and fails loudly on a multi-unit
    /// generation instead of silently picking a unit.
    ///
    /// # Errors
    /// Returns an error when the generation carries several units (LANG).
    pub(crate) fn unit(&self) -> Result<&ComposedUnit, Box<dyn std::error::Error>> {
        match self.units.as_slice() {
            [unit] => Ok(unit),
            units => Err(format!(
                "generation {:?} carries {} specification units — iterate `units` instead of \
                 assuming one",
                self.spec.module,
                units.len()
            )
            .into()),
        }
    }
}

/// The full Rust path of the module a dependency generation emits `spec` at
/// (`openehr_base::v1_3::base_types::identification::uid`) — the path the
/// [`External`] index hands the render stage.
fn generation_module_path(
    comp: &CrateComposition,
    generation: &str,
    schema: &BmmSchema,
    spec: &str,
) -> String {
    format!(
        "{}::{}::{}",
        comp.crate_name.replace('-', "_"),
        generation,
        crate::render::emit::type_module_path(schema, spec)
    )
}

/// Find a generation row inside a composition entry.
///
/// # Errors
/// Returns an error if `generation` names no row of `comp` (a table bug).
fn generation_spec(
    comp: &'static CrateComposition,
    generation: &str,
) -> Result<&'static GenerationSpec, Box<dyn std::error::Error>> {
    comp.generations
        .iter()
        .find(|g| g.module == generation)
        .ok_or_else(|| {
            format!(
                "composition {:?} has no generation {generation:?} (table bug: a DepGeneration \
                 names a generation module its dependency crate does not declare)",
                comp.key
            )
            .into()
        })
}

/// Load one generation's units plus each unit's merged resolution model
/// (paired dependency generations below, the unit's own schema on top).
///
/// # Errors
/// Returns an error if any involved BMM file cannot be loaded, a dependency
/// reference names a missing key/generation, or the generation lists no unit.
fn generation_units(
    spec: &'static GenerationSpec,
) -> Result<(Vec<ComposedUnit>, Vec<BmmSchema>), Box<dyn std::error::Error>> {
    if spec.units.is_empty() {
        return Err(format!("generation {:?} lists no specification unit", spec.module).into());
    }
    let mut dep_schemas = Vec::with_capacity(spec.model_deps.len());
    for dep in spec.model_deps {
        let dep_comp = lookup(dep.key)?;
        let dep_spec = generation_spec(dep_comp, dep.generation)?;
        for unit in dep_spec.units {
            dep_schemas.push(load_bmm(unit.file)?);
        }
    }
    let mut units = Vec::with_capacity(spec.units.len());
    for unit in spec.units {
        let schema = load_bmm(unit.file)?;
        let mut refs: Vec<&BmmSchema> = dep_schemas.iter().collect();
        refs.push(&schema);
        let model = Model::merged(&refs);
        units.push(ComposedUnit {
            spec: unit,
            schema,
            model,
        });
    }
    Ok((units, dep_schemas))
}

/// Resolve a composition entry into its per-generation schemas, models, and
/// full-path [`External`] indexes.
///
/// # Errors
/// Returns an error if the table row is malformed (zero or several `current`
/// generations, duplicate module names), any BMM file cannot be loaded, or a
/// dependency reference names a missing key/generation.
pub(crate) fn compose(key: &str) -> Result<Composed, Box<dyn std::error::Error>> {
    let comp = lookup(key)?;
    if comp.generations.is_empty() {
        return Err(format!("composition {key:?} lists no generation").into());
    }
    let current_n = comp.generations.iter().filter(|g| g.current).count();
    if current_n != 1 {
        return Err(format!(
            "composition {key:?} declares {current_n} current generations (exactly one required)"
        )
        .into());
    }
    let mut modules: Vec<&str> = comp.generations.iter().map(|g| g.module).collect();
    modules.sort_unstable();
    modules.dedup();
    if modules.len() != comp.generations.len() {
        return Err(format!("composition {key:?} declares duplicate generation modules").into());
    }

    let mut generations = Vec::with_capacity(comp.generations.len());
    for spec in comp.generations {
        let (units, dep_schemas) = generation_units(spec)?;

        // The External index over prelude_deps: each dependency generation's
        // emittable specs mapped to full generation-module paths, consulted
        // first-wins across dependencies in table order. Within one
        // dependency generation the units fold LAST-wins (a later unit's
        // twin shadows an earlier one's — the retired merged-view semantics
        // AM 2.4's LANG closure was built against), realized here by one
        // folded map per dependency generation.
        let mut external = External::default().in_crate(comp.crate_name);
        for dep in spec.prelude_deps {
            let dep_comp = lookup(dep.key)?;
            let dep_spec = generation_spec(dep_comp, dep.generation)?;
            let (dep_units, _) = generation_units(dep_spec)?;
            let mut modules = BTreeMap::new();
            for unit in &dep_units {
                for s in emittable_specs(&unit.model, &unit.schema) {
                    let path = generation_module_path(dep_comp, dep.generation, &unit.schema, &s);
                    modules.insert(s, path);
                }
            }
            external = external.with(modules);
        }

        generations.push(ComposedGeneration {
            spec,
            units,
            dep_schemas,
            external,
        });
    }

    Ok(Composed { comp, generations })
}

/// The Rust identifier of a generation's `Generation`-enum variant
/// (`v1_2` → `V1_2`) — the module token upper-cased with its underscores
/// KEPT: collapsing them (`V12`) would make `v1_2` and a future `v12`
/// indistinguishable.
pub(crate) fn generation_variant(module: &str) -> String {
    module.to_uppercase()
}
