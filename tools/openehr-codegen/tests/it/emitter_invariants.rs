// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Emitter invariants over the real pipeline on the real vendored BMM inputs:
//! completeness, constructibility, determinism, source-package mirroring,
//! downstream closures and decision-map integrity.

use openehr_codegen::testsupport;

// ── completeness ────────────────────────────────────────────────────────────

/// Every class of every BMM generation is emitted or skipped for one of the
/// two sanctioned reasons, counted per generation.
#[test]
fn completeness_every_class_is_planned_or_sanctioned_skip() {
    for key in testsupport::crate_keys() {
        let rows = testsupport::completeness(key).unwrap();
        assert!(!rows.is_empty(), "{key}: no BMM generation resolved");
        for c in rows {
            let file = &c.file;
            assert!(
                c.silently_dropped.is_empty(),
                "{key} [{file}]: {} class(es) silently dropped from the emission plan: {:?}",
                c.silently_dropped.len(),
                c.silently_dropped,
            );
            assert_eq!(
                c.planned + c.skipped_mapped + c.skipped_abstract_unused,
                c.total,
                "{key} [{file}]: plan ({}) + mapped ({}) + abstract-unused ({}) != total ({})",
                c.planned,
                c.skipped_mapped,
                c.skipped_abstract_unused,
                c.total,
            );
            assert!(c.planned > 0, "{key} [{file}]: emitted zero types");
        }
    }
}

/// Every attribute every loaded BMM generation declares reaches an emitted Rust
/// field.
#[test]
fn completeness_every_declared_attribute_reaches_an_emitted_field() {
    for key in testsupport::crate_keys() {
        let (gaps, checked) = testsupport::attribute_gaps(key).unwrap();
        assert!(
            gaps.is_empty(),
            "{key}: {} BMM-declared attribute(s) reach no emitted field: {:?}",
            gaps.len(),
            gaps.iter()
                .map(|g| format!("[{}] {}.{} — {}", g.file, g.class, g.attribute, g.detail))
                .collect::<Vec<_>>(),
        );
        // Non-vacuity: LANG, with two generations, carries the substantial bar.
        assert!(
            checked > 0,
            "{key}: no (class, attribute) pair checked — the traversal is vacuous",
        );
        if key == "lang" {
            assert!(
                checked > 500,
                "lang: only {checked} (class, attribute) pair(s) checked across both BMM \
                 generations — the traversal no longer covers the model",
            );
        }
    }
}

/// No two BMM generations of one crate claim the same emitted file path or
/// crate-prelude identifier.
#[test]
fn generations_never_silently_pick_a_shape() {
    let conflicts = testsupport::generation_conflicts().unwrap();
    assert!(
        conflicts.is_empty(),
        "BMM generations of one crate collide: {:?}",
        conflicts
            .iter()
            .map(|c| format!("{}: {:?} claimed by {:?}", c.key, c.what, c.files))
            .collect::<Vec<_>>(),
    );
}

/// LANG emits both BMM generations completely at their own source-package
/// paths: v2.x under `bmm/`, `bmm_persistence/` and `beom/`
/// (`LANG/docs/bmm/master01-preface.adoc` §History), v3 under `bmm3/`.
#[test]
fn lang_emits_both_bmm_generations_at_their_own_paths() {
    let files = testsupport::rendered_files("lang").unwrap();
    let has = |p: &str| files.iter().any(|f| f == p);
    // Both generations of a colliding name (18 of them; these are the shapes the
    // ch.6–ch.8 chapter audits pinned as materially different).
    for (v2, bmm3) in [
        (
            "v1_1/bmm/core/bmm_class.rs",
            "v1_1/bmm3/core/entity/bmm_class.rs",
        ),
        (
            "v1_1/bmm/core/bmm_type.rs",
            "v1_1/bmm3/core/entity/bmm_type.rs",
        ),
        (
            "v1_1/bmm/core/bmm_container_type.rs",
            "v1_1/bmm3/core/entity/bmm_container_type.rs",
        ),
        (
            "v1_1/bmm/core/bmm_property.rs",
            "v1_1/bmm3/core/feature/bmm_property.rs",
        ),
        (
            "v1_1/bmm/core/bmm_model.rs",
            "v1_1/bmm3/core/model/bmm_model.rs",
        ),
        (
            "v1_1/bmm/core/bmm_model_element.rs",
            "v1_1/bmm3/core/bmm_model_element.rs",
        ),
    ] {
        assert!(has(v2), "LANG did not emit the BMM v2.x unit's {v2}");
        assert!(has(bmm3), "LANG did not emit the BMM3 unit's {bmm3}");
    }
    // The two classes the merge left descendant-less and therefore unemitted.
    for bmm3_only in [
        "v1_1/bmm3/core/entity/bmm_model_type.rs",
        "v1_1/bmm3/core/entity/bmm_module.rs",
    ] {
        assert!(has(bmm3_only), "LANG did not emit {bmm3_only}");
    }
    // The released 1.0.0 generation emits beside the 1.1.0 line (faithful
    // emission, #1942 — incl. the sanitised `obsolete_elom` package name).
    for v1_0 in [
        "v1_0/bmm/core/entity/bmm_class.rs",
        "v1_0/obsolete_elom/types/type_def_date.rs",
    ] {
        assert!(
            has(v1_0),
            "LANG did not emit the released 1.0.0 generation's {v1_0}"
        );
    }
}

// ── constructibility ────────────────────────────────────────────────────────

/// No emitted concrete class is a non-constructible infinite value.
#[test]
fn constructibility_no_non_constructible_type_is_emitted() {
    for key in testsupport::crate_keys() {
        let offenders = testsupport::constructibility_offenders(key).unwrap();
        assert!(
            offenders.is_empty(),
            "{key}: non-constructible type(s) would be emitted (unbroken construction cycle): \
             {offenders:?}. Break each cycle at a spec-cited back_reference edge.",
        );
    }
}

// ── determinism ─────────────────────────────────────────────────────────────

/// Planning the workspace twice yields identical plans (no nondeterministic
/// map/iteration order leaks into the decided shapes).
#[test]
fn determinism_plan_is_stable_across_runs() {
    for key in testsupport::crate_keys() {
        let a = testsupport::shapes::plan_shapes(key).unwrap();
        let b = testsupport::shapes::plan_shapes(key).unwrap();
        assert_eq!(a, b, "{key}: plan differs between two runs");
        assert!(!a.is_empty(), "{key}: empty plan");
    }
}

/// Rendering the workspace to an in-memory map twice yields byte-identical
/// output (the emitter is a deterministic function of the vendored inputs).
#[test]
fn determinism_render_is_byte_identical_across_runs() {
    let a = testsupport::emit_to_memory().unwrap();
    let b = testsupport::emit_to_memory().unwrap();
    assert!(!a.is_empty(), "rendered nothing");
    assert_eq!(
        a.len(),
        b.len(),
        "file count differs between two renders ({} vs {})",
        a.len(),
        b.len(),
    );
    for (path, body_a) in &a {
        assert!(b.contains_key(path), "{path} missing on 2nd render");
        let body_b = b.get(path).unwrap();
        assert_eq!(body_a, body_b, "{path}: body differs between two renders");
    }
}

// ── source-package mirroring + downstream closure ───────────────────────────

/// Every re-emitted cross-schema class (the AM 2.4 downstream closure) lands at
/// the SAME package path it occupies in its upstream source schema (BASE/LANG).
#[test]
fn mirroring_reemitted_classes_land_at_source_package_path() {
    let mirrors = testsupport::v2_4_reemit_mirrors().unwrap();
    assert!(!mirrors.is_empty(), "empty re-emission closure");
    for m in &mirrors {
        assert!(
            m.source_path.is_some(),
            "{}: re-emitted class has no upstream source package path",
            m.class,
        );
        assert!(
            m.augmented_path.is_some(),
            "{}: re-emitted class not grafted into the downstream schema",
            m.class,
        );
        assert_eq!(
            m.source_path, m.augmented_path,
            "{}: re-emitted at a different package path than its source",
            m.class,
        );
    }
}

/// The AM ⊃ LANG downstream closure is the expected re-emission set, and every
/// member is grafted into the AM 2.4 output.
#[test]
fn downstream_closure_contains_the_beom_bmm3_resource_set() {
    let closure = testsupport::reemit_closure("am", "v2_4").unwrap();
    // beom expression/statement subtree (LANG-origin, extended by AM's rules):
    for expected in [
        "EXPRESSION",
        "EXPR_LEAF",
        "EXPR_VALUE_REF",
        "STATEMENT",
        "STATEMENT_SET",
        "ASSERTION",
    ] {
        assert!(closure.contains(expected), "closure missing {expected}");
    }
    // the BMM-3 + EL object model (LANG-origin):
    assert!(
        closure.iter().any(|c| c.starts_with("BMM_")),
        "closure missing the BMM-3 object model",
    );
    assert!(
        closure.iter().any(|c| c.starts_with("EL_")),
        "closure missing the EL expression language",
    );
    // the BASE resource metatype (extended by AM's AUTHORED_ARCHETYPE etc.):
    for expected in ["AUTHORED_RESOURCE", "RESOURCE_DESCRIPTION"] {
        assert!(closure.contains(expected), "closure missing {expected}");
    }
}

/// AM analysis leaves LANG output untouched, and the AM-only rules leaves are
/// emitted into the AM crate only.
#[test]
fn downstream_closure_leaves_upstream_output_untouched() {
    let lang_files = testsupport::rendered_files("lang").unwrap();
    // LANG emits its own EXPR_LEAF (a shared closure member) independently:
    assert!(
        lang_files.iter().any(|p| p.ends_with("expr_leaf.rs")),
        "LANG did not emit its own EXPR_LEAF",
    );
    // The AM-only rules leaves must NOT appear in LANG's output:
    for leaf in ["expr_archetype_ref.rs", "expr_constraint.rs"] {
        assert!(
            !lang_files.iter().any(|p| p.ends_with(leaf)),
            "AM-only leaf {leaf} leaked into LANG output",
        );
    }
    // …and they ARE emitted into the AM crate, while the shared member is
    // re-emitted downstream too.
    let rendered = testsupport::emit_to_memory().unwrap();
    let am_paths: Vec<&String> = rendered
        .keys()
        .filter(|k| k.starts_with("openehr-am/"))
        .collect();
    for leaf in ["expr_archetype_ref.rs", "expr_constraint.rs"] {
        assert!(
            am_paths.iter().any(|p| p.ends_with(leaf)),
            "AM did not emit its own leaf {leaf}",
        );
    }
    assert!(
        am_paths
            .iter()
            .any(|p| p.contains("v2_4/") && p.ends_with("expr_leaf.rs")),
        "AM did not re-emit the shared EXPR_LEAF downstream",
    );
}

// ── decision-map integrity ──────────────────────────────────────────────────

/// Every decision entry carries a citation, and every binding-map entry names
/// a class or field that exists.
#[test]
fn decision_map_integrity_entries_are_cited_and_reference_real_classes() {
    for map in testsupport::decision_maps() {
        assert!(!map.entries.is_empty(), "{}: empty decision map", map.map);
        for e in &map.entries {
            assert!(
                !e.citation.trim().is_empty(),
                "{}: entry {:?} has an empty citation/flag",
                map.map,
                e.key,
            );
            assert!(
                !e.reason.trim().is_empty(),
                "{}: entry {:?} has an empty reason",
                map.map,
                e.key,
            );
            assert!(
                !e.decision.trim().is_empty(),
                "{}: entry {:?} has an empty decision",
                map.map,
                e.key,
            );
            if !map.check_existence {
                continue;
            }
            if let Some((class, field)) = e.key.split_once('.') {
                assert!(
                    testsupport::field_exists(class, field).unwrap(),
                    "{}: stale entry — {class}.{field} exists in no loaded schema",
                    map.map,
                );
            } else {
                assert!(
                    testsupport::class_exists(&e.key).unwrap(),
                    "{}: stale entry — class {:?} exists in no loaded schema",
                    map.map,
                    e.key,
                );
            }
        }
    }
}

/// Every existence-contradiction entry still names a mandatory `Hash` its owner
/// declares.
#[test]
fn existence_contradictions_name_a_still_mandatory_hash() {
    let stale = testsupport::decision_checks::stale_existence_contradictions().unwrap();
    assert!(
        stale.is_empty(),
        "stale existence-contradiction entries:\n{}",
        stale.join("\n")
    );
}

/// Every ITS-JSON `minItems` refusal still names an optional container the
/// ITS-JSON schema bounds, so an upstream schema change fails here.
#[test]
fn its_json_min_items_name_a_bounded_optional_container() {
    let stale = testsupport::decision_checks::stale_its_json_min_items().unwrap();
    assert!(
        stale.is_empty(),
        "stale ITS-JSON minItems entries:\n{}",
        stale.join("\n")
    );
}

/// Every ordered-hash entry still names a `Hash` its owner declares, so a
/// retyped attribute fails here instead of leaving a dead entry.
#[test]
fn ordered_hashes_name_a_declared_hash() {
    let stale = testsupport::decision_checks::stale_ordered_hashes().unwrap();
    assert!(
        stale.is_empty(),
        "stale ordered-hash entries:\n{}",
        stale.join("\n")
    );
}

/// Every adjudicated free-form field renders as `serde_json::Value` in some
/// generated file, under a NOTE carrying the entry's citation.
#[test]
fn untyped_field_adjudications_land_on_a_real_free_form_field() {
    let files = testsupport::emit_to_memory().unwrap();
    for (class, field, citation) in testsupport::untyped_fields() {
        let ident = if field == "type" {
            "r#type".to_string()
        } else {
            field.clone()
        };
        let hit = files.values().any(|body| {
            body.contains(&citation)
                && body.contains(&format!("pub {ident}: serde_json::Value"))
                && body.contains(&format!("(`{class}`)"))
        });
        assert!(
            hit,
            "{class}.{field}: no generated file carries the adjudication NOTE above a free-form \
             `{ident}: serde_json::Value` field — the entry is stale (the field is typed now) or \
             the NOTE is not being emitted",
        );
    }
}

/// Every declared additional subtype member widens its parent's polymorphic
/// slot in every composition defining both, and some composition does.
#[test]
fn subtype_extensions_widen_their_parents_variant_set() {
    for (parent, subtype) in testsupport::subtype_extensions() {
        let mut seen = 0_usize;
        for key in testsupport::crate_keys() {
            let Some(variants) = testsupport::enum_variants(key, &parent).unwrap() else {
                continue;
            };
            if testsupport::enum_variants(key, &subtype).unwrap().is_none() {
                continue;
            }
            seen += 1;
            assert!(
                variants.contains(&subtype),
                "{key}: {parent}'s variants {variants:?} do not include the declared \
                 extension subtype {subtype}",
            );
        }
        assert!(
            seen > 0,
            "no composition defines both {parent} and {subtype}: the subtype_extensions entry \
             is inert",
        );
    }
}

/// Every composition entry carries a citation, lists an own BMM file, loads,
/// and names only known dependency keys.
#[test]
fn composition_table_integrity() {
    let keys: Vec<String> = testsupport::composition_infos()
        .iter()
        .map(|c| c.key.clone())
        .collect();
    for info in testsupport::composition_infos() {
        assert!(
            !info.citation.trim().is_empty(),
            "composition {:?} has an empty citation",
            info.key,
        );
        assert!(
            !info.reason.trim().is_empty(),
            "composition {:?} has an empty reason",
            info.key,
        );
        assert!(
            !info.generations.is_empty(),
            "composition {:?} lists no generation",
            info.key,
        );
        assert!(
            !info.crate_name.is_empty(),
            "composition {:?} has no crate name",
            info.key,
        );
        // Exactly one CURRENT generation per crate, and generation-module
        // names are unique within the crate.
        assert_eq!(
            info.generations.iter().filter(|g| g.current).count(),
            1,
            "composition {:?} must declare exactly one current generation",
            info.key,
        );
        let mut modules: Vec<&str> = info.generations.iter().map(|g| g.module.as_str()).collect();
        modules.sort_unstable();
        modules.dedup();
        assert_eq!(
            modules.len(),
            info.generations.len(),
            "composition {:?} declares duplicate generation modules",
            info.key,
        );
        for g in &info.generations {
            assert!(
                !g.spec_version.trim().is_empty(),
                "composition {:?} generation {:?} has no spec version",
                info.key,
                g.module,
            );
            for (dep_key, dep_gen) in g.model_deps.iter().chain(g.prelude_deps.iter()) {
                assert!(
                    keys.iter().any(|k| k == dep_key),
                    "composition {:?} references unknown dependency key {dep_key:?}",
                    info.key,
                );
                assert!(
                    testsupport::composition_infos()
                        .iter()
                        .find(|c| &c.key == dep_key)
                        .is_some_and(|c| c.generations.iter().any(|g| &g.module == dep_gen)),
                    "composition {:?} references unknown generation {dep_gen:?} of {dep_key:?}",
                    info.key,
                );
            }
        }
        // Resolving the composition loads every member/dependency BMM file, one
        // completeness row per specification UNIT.
        let unit_n: usize = info.generations.iter().map(|g| g.units.len()).sum();
        let rows = testsupport::completeness(&info.key);
        assert!(
            rows.as_ref().is_ok_and(|r| r.len() == unit_n),
            "composition {:?} failed to resolve one row per specification unit",
            info.key,
        );
    }
}

/// Every `includes` a vendored unit declares is in its generation's
/// `model_deps` and named by the composition's citation.
#[test]
fn composition_follows_the_bmm_includes() {
    let infos = testsupport::composition_infos();
    for unit in testsupport::unit_includes().unwrap() {
        let info = infos.iter().find(|c| c.key == unit.key).unwrap();
        let generation = info
            .generations
            .iter()
            .find(|g| g.module == unit.module)
            .unwrap();
        for include in &unit.includes {
            let (component, version) = include
                .strip_prefix("openehr_")
                .and_then(|rest| rest.split_once('_'))
                .unwrap_or_else(|| panic!("{}: unrecognised include id {include:?}", unit.file));
            let honoured = generation.model_deps.iter().any(|(dep_key, dep_gen)| {
                dep_key == component
                    && infos
                        .iter()
                        .find(|c| &c.key == dep_key)
                        .and_then(|c| c.generations.iter().find(|g| &g.module == dep_gen))
                        .is_some_and(|g| g.spec_version == version)
            });
            assert!(
                honoured,
                "{} ({}::{}) includes {include}, but the generation's model_deps pair no \
                 {component} generation of spec version {version}",
                unit.file, unit.key, unit.module,
            );
            assert!(
                info.citation.contains(include.as_str()),
                "{} includes {include}, which the {:?} composition citation does not name",
                unit.file,
                unit.key,
            );
        }
    }
}

// ── invariant classification (assertion-dialect analyzer) ────────────────────

/// The assertion-dialect analyzer puts every RM class invariant in exactly one
/// bucket, with the per-bucket counts pinned.
#[test]
fn invariant_classification_is_a_total_tripartition() {
    let rows = testsupport::classify_invariants("rm").unwrap();
    // `ITEM_TAG.Inv_key_valid` is complex: `key.is_justified` is a BMM function
    // call.
    for (generation, p_total, p_emitted, p_hook, p_complex) in
        [("v1_1", 155, 90, 34, 31), ("v1_2", 155, 90, 34, 31)]
    {
        let rows: Vec<_> = rows.iter().filter(|r| r.generation == generation).collect();
        let total = rows.len();
        let emitted = rows.iter().filter(|r| r.bucket == "emitted").count();
        let hook = rows
            .iter()
            .filter(|r| r.bucket == "runtime-hook-missing")
            .count();
        let complex = rows.iter().filter(|r| r.bucket == "complex").count();

        // Every row lands in one of the three known buckets (no unexpected label).
        assert_eq!(
            emitted + hook + complex,
            total,
            "{generation}: some invariant carries an unknown bucket label",
        );
        assert_eq!(total, p_total, "{generation}: RM invariant count changed");
        assert_eq!(emitted, p_emitted, "{generation}: EMITTED count drifted");
        assert_eq!(
            hook, p_hook,
            "{generation}: RUNTIME-HOOK-MISSING count drifted"
        );
        assert_eq!(complex, p_complex, "{generation}: COMPLEX count drifted");
    }

    // A non-emitted row always names its reason; an emitted row never does.
    for r in &rows {
        if r.bucket == "emitted" {
            assert!(
                r.reason.is_empty(),
                "{}::{} is emitted but carries a reason",
                r.class,
                r.name,
            );
        } else {
            assert!(
                !r.reason.is_empty(),
                "{}::{} ({}) is missing its reason",
                r.class,
                r.name,
                r.bucket,
            );
        }
    }
}

/// Pins the classifier's bucket for representative invariants.
#[test]
fn invariant_classification_spot_checks() {
    let rows = testsupport::classify_invariants("rm").unwrap();
    let bucket = |class: &str, name: &str| -> &'static str {
        rows.iter()
            .find(|r| r.class == class && r.name == name)
            .map_or("MISSING", |r| r.bucket)
    };
    assert_eq!(bucket("DV_IDENTIFIER", "Id_valid"), "emitted");
    assert_eq!(bucket("DV_DATE", "Value_valid"), "emitted");
    assert_eq!(bucket("LOCATABLE", "Links_valid"), "emitted");
    assert_eq!(
        bucket("COMPOSITION", "Category_validity"),
        "runtime-hook-missing",
    );
    assert_eq!(
        bucket("COMPOSITION", "Territory_valid"),
        "runtime-hook-missing"
    );
    assert_eq!(bucket("EHR", "Compositions_valid"), "complex");
    assert_eq!(bucket("HISTORY", "Period_consistency"), "complex");
}

// ── invariant-core emission (emit-validate) ──────────────────────────────────

/// The generated invariant-core file accounts for every classified RM
/// invariant: each emittable one in the realization register, each inert one
/// in the pending-adjudication register.
#[test]
fn invariant_core_file_accounts_for_emitted_and_inert_invariants() {
    let files = testsupport::emit_to_memory().unwrap();
    let gen_file = files
        .get("openehr-rm/src/v1_2/validate/generated.rs")
        .expect("emit-validate did not produce v1_2/validate/generated.rs");

    // Every emittable invariant is accounted for, and named in the register the
    // emitter renders into the generated file.
    let accounted = testsupport::accounted_emitted_invariants("rm").unwrap();
    assert!(!accounted.is_empty(), "no emittable RM invariants found");
    let unaccounted: Vec<_> = accounted
        .iter()
        .filter(|a| a.venue == "UNACCOUNTED")
        .map(|a| format!("{}.{}", a.class, a.name))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "assertion-dialect-emittable invariants with no realization-register row \
         (adjudicate each into `plan::overrides::INVARIANT_REALIZATIONS`): {unaccounted:?}",
    );
    for a in &accounted {
        assert!(
            gen_file.contains(&format!("`{}.{}`", a.class, a.name)),
            "emittable invariant {}.{} is not named in the realization register",
            a.class,
            a.name,
        );
        assert!(
            !a.reason.trim().is_empty() && !a.citation.trim().is_empty(),
            "register row {}.{} has no reason/citation",
            a.class,
            a.name,
        );
    }

    // Every runtime-hook-missing (inert) RM invariant is named in the pending
    // register — the same classifier verdict the emitter derives it from.
    let rows = testsupport::classify_invariants("rm").unwrap();
    let hook: Vec<_> = rows
        .iter()
        .filter(|r| r.bucket == "runtime-hook-missing")
        .collect();
    assert!(!hook.is_empty(), "no runtime-hook-missing invariants found");
    for r in hook {
        assert!(
            gen_file.contains(&r.name),
            "inert invariant {}::{} is not named in the pending register",
            r.class,
            r.name,
        );
    }
}

/// Each realization-register venue claim holds: a `Core` row's name is a
/// violation literal in the core file, and an `Impl`/`Wire`/`App` file exists
/// and names the invariant.
#[test]
fn realization_register_venues_are_real() {
    let files = testsupport::emit_to_memory().unwrap();
    let gen_file = files
        .get("openehr-rm/src/v1_2/validate/generated.rs")
        .expect("emit-validate did not produce v1_2/validate/generated.rs");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");

    for a in testsupport::accounted_emitted_invariants("rm").unwrap() {
        let where_ = format!("{}.{}", a.class, a.name);
        // The cited spec file is a real vendored spec document.
        assert!(
            repo.join(&a.citation).is_file(),
            "{where_}: citation {} is not a vendored spec file",
            a.citation,
        );
        match a.venue {
            "Core" => {
                assert!(
                    gen_file.contains(&format!("fn {}(", a.site)),
                    "{where_}: core {} is not defined in validate/generated.rs",
                    a.site,
                );
                assert!(
                    gen_file.contains(&format!("Invariant {} failed", a.name))
                        || gen_file.contains(&format!("invariant_failed(\"{}\"", a.name))
                        // A parameterised core names the invariant in its `(value, name)` row table.
                        || gen_file.contains(&format!("\"{}\"),", a.name)),
                    "{where_}: no violation message for it in validate/generated.rs",
                );
            }
            "Impl" | "Wire" | "App" => {
                let path = repo.join(&a.site);
                assert!(
                    path.is_file(),
                    "{where_}: realizing file {} is missing",
                    a.site
                );
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(
                    names_in_a_string_literal(&text, &a.name),
                    "{where_}: realizing file {} does not name the invariant inside a \
                     violation-producing string literal — a comment mention is narration, \
                     not enforcement (#2443)",
                    a.site,
                );
            }
            "Excluded" | "Unrealized" => assert!(
                a.site.is_empty(),
                "{where_}: a non-realizing venue must name no site, got {}",
                a.site,
            ),
            other => panic!("{where_}: unexpected venue {other}"),
        }
    }
}

/// Whether `name` occurs on a non-comment line of `text`, so a literal wrapped
/// across lines counts and a prose mention does not.
fn names_in_a_string_literal(text: &str, name: &str) -> bool {
    text.lines()
        .any(|line| !line.trim_start().starts_with("//") && line.contains(name))
}

/// A comment mention never satisfies the venue-claim predicate; a violation
/// literal does, on a quote-less continuation line included.
#[test]
fn a_prose_mention_does_not_satisfy_a_venue_claim() {
    let prose = "// The Details_valid rule is realized elsewhere.\nfn f() {}";
    assert!(!names_in_a_string_literal(prose, "Details_valid"));
    let doc = "/// Invariant Details_valid failed — see elsewhere.\nfn f() {}";
    assert!(!names_in_a_string_literal(doc, "Details_valid"));
    let quoted = "out.push(failed(\"Invariant Details_valid failed on type X\"));";
    assert!(names_in_a_string_literal(quoted, "Details_valid"));
    let wrapped = "return Err(precondition(\n    \"data excluded \\\n     Details_valid: revision-history-only\",\n));";
    assert!(names_in_a_string_literal(wrapped, "Details_valid"));
}

/// An emitted-bucket invariant with no register row accounts as `UNACCOUNTED`.
#[test]
fn a_seeded_unrealized_emit_is_unaccounted() {
    let seeded = testsupport::account_invariants(&[
        // Emittable (a plain emptiness check) but registered nowhere.
        ("SEEDED_CLASS", "Seeded_valid", "not seeded_field.is_empty"),
        // Complex / hook-missing verdicts are not part of this accounting.
        (
            "SEEDED_CLASS",
            "Seeded_complex",
            "items.for_all (i: ITEM | i.is_valid)",
        ),
        // A real, registered invariant stays accounted.
        (
            "LOCATABLE",
            "Archetype_node_id_valid",
            "not archetype_node_id.is_empty",
        ),
    ]);
    let venues: Vec<(String, &str)> = seeded
        .iter()
        .map(|a| (format!("{}.{}", a.class, a.name), a.venue))
        .collect();
    assert_eq!(
        venues,
        vec![
            ("LOCATABLE.Archetype_node_id_valid".to_owned(), "Core"),
            ("SEEDED_CLASS.Seeded_valid".to_owned(), "UNACCOUNTED"),
        ],
        "a seeded unrealized emit must account as UNACCOUNTED",
    );
}

/// `plan::overrides::DIALECT_PREDICATES` covers exactly the classifier's
/// `RUNTIME_PREDICATES`.
#[test]
fn dialect_predicates_match_the_classifier() {
    use std::collections::BTreeSet;
    let table: BTreeSet<String> = testsupport::dialect_predicates()
        .into_iter()
        .map(|(p, _)| p)
        .collect();
    let classifier: BTreeSet<String> = testsupport::runtime_predicates().into_iter().collect();
    assert_eq!(
        table, classifier,
        "DIALECT_PREDICATES and the classifier's RUNTIME_PREDICATES drifted apart",
    );
    // Every mapped runtime function is non-empty (the integrity test in
    // `decision_map_integrity_*` already checks citation/reason/decision).
    for (predicate, runtime_fn) in testsupport::dialect_predicates() {
        assert!(
            !runtime_fn.trim().is_empty(),
            "dialect predicate {predicate:?} has no runtime function",
        );
    }
}

// ── constants emission (BMM `BMM_CLASS.constants`) ───────────────────────────

/// Every constant the BMM declares on an emitted class is a `pub const` in its
/// crate, checked over the two RM terminology identifier classes.
#[test]
fn emitted_classes_render_their_bmm_constants() {
    let files = testsupport::emit_to_memory().unwrap();
    let group = files
        .get("openehr-rm/src/v1_2/support/terminology/openehr_terminology_group_identifiers.rs")
        .unwrap();
    let code_set = files
        .get("openehr-rm/src/v1_2/support/terminology/openehr_code_set_identifiers.rs")
        .unwrap();

    // The 15 terminology-group identifier constants + the openEHR terminology id.
    for c in [
        "TERMINOLOGY_ID_OPENEHR",
        "GROUP_ID_AUDIT_CHANGE_TYPE",
        "GROUP_ID_VERSION_LIFE_CYCLE_STATE",
    ] {
        assert!(
            group.contains(&format!("pub const {c}:")),
            "group-identifiers output is missing constant {c}",
        );
    }
    assert!(
        group.contains(r#"= "openehr";"#),
        "constant value not emitted"
    );

    // The 7 code-set identifier constants (incl. the `_id`-less integrity one).
    for c in [
        "CODE_SET_ID_LANGUAGES",
        "CODE_SET_INTEGRITY_CHECK_ALGORITHMS",
        "CODE_SET_ID_NORMAL_STATUSES",
    ] {
        assert!(
            code_set.contains(&format!("pub const {c}:")),
            "code-set output is missing constant {c}",
        );
    }
}

/// The terminology and code-set invariants are enforced in
/// `validate::terminology`; only the four `VERSIONED_OBJECT` aggregates stay
/// `runtime-hook-missing`.
#[test]
fn register_records_terminology_invariants_as_enforced() {
    let files = testsupport::emit_to_memory().unwrap();
    let rows = testsupport::classify_invariants("rm").unwrap();
    // Every RM generation carries its own cores file with the same split.
    for generation in ["v1_1", "v1_2"] {
        let gen_path = format!("openehr-rm/src/{generation}/validate/generated.rs");
        let gen_file = files
            .get(&gen_path)
            .unwrap_or_else(|| panic!("emit-validate did not produce {gen_path}"));

        assert!(
            gen_file
                .contains("# Terminology-backed invariants (enforced in `validate::terminology`"),
            "{generation}: the terminology-enforcement register heading is missing",
        );

        // Each runtime-hook-missing invariant is terminology or a versioned-object
        // aggregate.
        let hook: Vec<_> = rows
            .iter()
            .filter(|r| r.generation == generation && r.bucket == "runtime-hook-missing")
            .collect();
        let mut enforced = 0usize;
        let mut aggregate = 0usize;
        let mut unadjudicated: Vec<String> = Vec::new();
        for r in &hook {
            let enforced_line = format!(
                "`{}.{}` — enforced in `validate::terminology`",
                r.class, r.name
            );
            let aggregate_line = format!(
                "`{}.{}` — versioned-object aggregate model",
                r.class, r.name
            );
            if gen_file.contains(&enforced_line) {
                enforced += 1;
            } else if gen_file.contains(&aggregate_line) {
                aggregate += 1;
            } else {
                unadjudicated.push(format!("{}::{}", r.class, r.name));
            }
        }
        assert!(
            unadjudicated.is_empty(),
            "{generation}: invariant(s) carry no adjudication verdict in the register: \
             {unadjudicated:?}",
        );
        assert_eq!(
            enforced, 30,
            "{generation}: terminology-enforced count drifted"
        );
        assert_eq!(
            aggregate, 4,
            "{generation}: versioned-aggregate pending count drifted"
        );
        assert_eq!(
            enforced + aggregate,
            hook.len(),
            "{generation}: register split is not total"
        );
    }
}

// ── ITS-REST OAS monomorphizations ──────────────────────────────────────────

/// Every `OAS_MONOMORPHIZATIONS` key exists in the vendored bundles with the
/// `title` the entry cites.
#[test]
fn oas_monomorphizations_match_the_vendored_titles() {
    let checks = testsupport::oas_monomorphizations().unwrap();
    assert!(!checks.is_empty(), "empty monomorphization map");
    for c in &checks {
        assert!(
            !c.vendored_titles.is_empty(),
            "{}: no vendored ITS-REST bundle declares this schema (stale entry)",
            c.schema,
        );
        assert!(
            c.vendored_titles.len() == 1 && c.vendored_titles.contains(&c.declared_title),
            "{}: the decision map cites title {:?} but the vendored bundles declare {:?}",
            c.schema,
            c.declared_title,
            c.vendored_titles,
        );
        assert!(
            c.rust_type.starts_with("openehr_rm::") || c.rust_type.starts_with("openehr_base::"),
            "{}: resolves to {:?}, which is not a generated spec type",
            c.schema,
            c.rust_type,
        );
    }
}

/// The generated ITS-REST contract carries no DTO struct for a monomorphized
/// schema; its references name the spec type.
#[test]
fn oas_monomorphizations_emit_as_spec_types_not_dtos() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/openehr-its/src/rest/generated");
    let mut bodies = String::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|x| x == "rs") {
            bodies.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    for c in testsupport::oas_monomorphizations().unwrap() {
        assert!(
            !bodies.contains(&format!("pub struct {} {{", c.schema)),
            "{} still emits as a transport DTO instead of resolving to {}",
            c.schema,
            c.rust_type,
        );
    }
    assert!(
        bodies.contains("pub contribution: openehr_base::prelude::ObjectRef,"),
        "the VERSION `contribution` field no longer resolves to the spec OBJECT_REF",
    );
}

/// Every generated transport DTO is closed (`deny_unknown_fields`) or keeps
/// undeclared members in `additional_properties` (OpenAPI 3.0.3 §Schema
/// Object).
#[test]
fn transport_dtos_are_closed_or_carry_the_extension_map() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/openehr-its/src/rest/generated");
    let mut dtos = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let body = std::fs::read_to_string(&path).unwrap();
        for block in body.split("transport DTO of this API group").skip(1) {
            let block = block.split("\n}\n").next().unwrap();
            let name = block
                .split("pub struct ")
                .nth(1)
                .and_then(|rest| rest.split([' ', '<']).next())
                .unwrap();
            let closed = block.contains("#[serde(deny_unknown_fields)]");
            let open = block.contains("pub additional_properties:");
            assert!(
                closed != open,
                "{}: {name} is {} — a DTO is closed (additionalProperties: false) or carries \
                 the extension map, exactly one of the two",
                path.display(),
                if closed {
                    "both closed and open"
                } else {
                    "neither closed nor open"
                },
            );
            dtos += 1;
        }
    }
    assert!(
        dtos > 40,
        "only {dtos} transport DTOs found — the scan lost its subject"
    );
}

/// The shared-module fallback document carries the `allOf` base closure of the
/// hoisted schemas, so `emit_common` can resolve each base.
#[test]
fn merged_schema_fallback_carries_the_allof_base_closure() {
    let (names, bases) = testsupport::merged_fallback_schema_names().unwrap();
    assert!(
        !bases.is_empty(),
        "no hoisted schema composes an `allOf` base — this test has lost its subject",
    );
    for base in &bases {
        assert!(
            names.contains(base),
            "the merged fallback document omits the `allOf` base {base:?}, so every schema \
             composing it emits allOf-truncated",
        );
    }
}

// ── vendored `default` facet ↔ the hand-written residue ─────────────────────

/// Every vendored `default` facet is rendered or a named un-renderable entry.
#[test]
fn vendored_default_facets_are_totally_partitioned() {
    let facets = testsupport::vendored_defaults().unwrap();
    assert!(
        !facets.is_empty(),
        "the vendored schemas carry `default` facets — the loader dropped them again",
    );
    for f in &facets {
        assert!(
            f.rendered.is_some() || testsupport::default_unrenderable(&f.owner, &f.field),
            "{}: {}.{} carries the vendored default {:?}, which the emitter neither renders nor \
             adjudicates as un-renderable — add the derivation or an UNRENDERABLE_DEFAULTS entry",
            f.key,
            f.owner,
            f.field,
            f.facet,
        );
    }
    // No exclusion is stale: each names exactly the vendored values it cites.
    for (owner, field, recorded) in testsupport::unrenderable_defaults() {
        let mut vendored: Vec<&str> = facets
            .iter()
            .filter(|f| f.owner == owner && f.field == field && f.rendered.is_none())
            .map(|f| f.facet.as_str())
            .collect();
        vendored.sort_unstable();
        vendored.dedup();
        let mut recorded: Vec<&str> = recorded.iter().map(String::as_str).collect();
        recorded.sort_unstable();
        recorded.dedup();
        assert_eq!(
            vendored, recorded,
            "{owner}.{field}: the UNRENDERABLE_DEFAULTS entry adjudicates the facet(s) \
             {recorded:?}, but the vendored schemas carry un-rendered {vendored:?} — the entry \
             (and its citation) is stale",
        );
    }
}

/// TERM 3.1.0's four `status` defaults (`{default = active}`,
/// `org.openehr.term.terminology.code.adoc`) render as the
/// `TERMINOLOGY_STATUS` constant.
#[test]
fn term_status_defaults_render_as_the_enumeration_constant() {
    let facets = testsupport::vendored_defaults().unwrap();
    for owner in [
        "CODE_SET",
        "TERMINOLOGY_GROUP",
        "CODE",
        "TERMINOLOGY_CONCEPT",
    ] {
        let f = facets
            .iter()
            .find(|f| f.owner == owner && f.field == "status")
            .unwrap_or_else(|| panic!("{owner}.status carries no vendored default"));
        assert_eq!(f.facet, "active", "{owner}.status facet");
        assert_eq!(
            f.rendered.as_deref(),
            Some("@TERMINOLOGY_STATUS::Active"),
            "{owner}.status",
        );
        assert!(
            !testsupport::default_unrenderable(owner, "status"),
            "{owner}.status renders, so it must not stay adjudicated un-renderable",
        );
    }
    let resolved = testsupport::resolve_default("@TERMINOLOGY_STATUS::Active", |marker| {
        assert_eq!(marker, "@TERMINOLOGY_STATUS");
        "crate::v3_1::terminology::terminology_status::TerminologyStatus".to_owned()
    });
    assert_eq!(
        resolved,
        "crate::v3_1::terminology::terminology_status::TerminologyStatus::Active"
    );
    assert_eq!(
        testsupport::resolve_default("false", |marker| format!(
            "unexpected resolution of {marker}"
        )),
        "false"
    );
}

/// No `field_default` entry names a property with a renderable vendored
/// `default`.
#[test]
fn hand_written_field_defaults_never_duplicate_a_vendored_facet() {
    let facets = testsupport::vendored_defaults().unwrap();
    for (owner, field) in testsupport::hand_written_defaults() {
        assert!(
            !facets
                .iter()
                .any(|f| f.owner == owner && f.field == field && f.rendered.is_some()),
            "field_default entry {owner}.{field} duplicates a renderable vendored `default` \
             facet — delete it and let the derivation win",
        );
    }
}

/// The four `Point_interval` flags carry the vendored default in the emitted
/// reader, as the `Proper_interval` sites do.
#[test]
fn point_interval_flags_carry_their_vendored_defaults() {
    let facets = testsupport::vendored_defaults().unwrap();
    for (field, expected) in [
        ("lower_unbounded", "false"),
        ("upper_unbounded", "false"),
        ("lower_included", "true"),
        ("upper_included", "true"),
    ] {
        let f = facets
            .iter()
            .find(|f| f.owner == "Point_interval" && f.field == field)
            .unwrap_or_else(|| panic!("Point_interval.{field} carries no vendored default"));
        assert_eq!(
            f.rendered.as_deref(),
            Some(expected),
            "Point_interval.{field}",
        );
    }
}

// ── RM/BASE twin classes (two spec generations, both emitted) ───────────────

/// Pins the five class names both RM 1.2.0 and BASE 1.3.0 declare, and their
/// member divergences.
///
/// RM `docs/common/master08-resource_package.adoc` retains the ADL 1.4
/// resource package "only while needed by AOM 1.4 based archetypes and tools".
#[test]
fn rm_base_twin_classes_keep_both_generations() {
    let twins = [
        "AUTHORED_RESOURCE",
        "CODE_PHRASE",
        "RESOURCE_DESCRIPTION",
        "RESOURCE_DESCRIPTION_ITEM",
        "TRANSLATION_DETAILS",
    ];
    for class in twins {
        for key in ["rm", "base"] {
            assert!(
                testsupport::declared_attributes(key, class)
                    .unwrap()
                    .is_some(),
                "{key} no longer declares the twin class {class}",
            );
        }
    }

    // The RM twin spells `accreditation` as BASE does
    // (`org.openehr.rm.common.translation_details.adoc`) and keeps `copyright` on
    // the item.
    let rm_translation = testsupport::declared_attributes("rm", "TRANSLATION_DETAILS")
        .unwrap()
        .unwrap();
    let base_translation = testsupport::declared_attributes("base", "TRANSLATION_DETAILS")
        .unwrap()
        .unwrap();
    assert!(
        rm_translation.contains("accreditation") && !rm_translation.contains("accreditaton"),
        "the RM (ADL-1.4) TRANSLATION_DETAILS no longer carries the corrected `accreditation` \
         spelling — re-adjudicate against RM \
         docs/UML/classes/org.openehr.rm.common.translation_details.adoc",
    );
    assert!(
        base_translation.contains("accreditation") && !base_translation.contains("accreditaton"),
        "the BASE TRANSLATION_DETAILS no longer carries the SPECPUB-6-corrected `accreditation` \
         spelling",
    );

    let rm_item = testsupport::declared_attributes("rm", "RESOURCE_DESCRIPTION_ITEM")
        .unwrap()
        .unwrap();
    let base_description = testsupport::declared_attributes("base", "RESOURCE_DESCRIPTION")
        .unwrap()
        .unwrap();
    assert!(
        rm_item.contains("copyright"),
        "the RM (ADL-1.4) RESOURCE_DESCRIPTION_ITEM lost `copyright`, which the vendored \
         AM/Release-1.4 Resource.xsd also declares there",
    );
    assert!(
        base_description.contains("copyright"),
        "the BASE RESOURCE_DESCRIPTION lost `copyright`",
    );
}

/// Pins the exact RM and BASE model delta between the stable profile's
/// released generations and the development pins: every attribute added,
/// removed or retyped, so a re-vendor fails until the profile boundary covers
/// it.
///
/// LANG is outside the pin: the application binds only its `Generation`
/// token, and `v1_1` declares 18 class names twice (`docs/VERSIONS.md` §Spec
/// version policy).
#[test]
fn profile_generation_delta_is_pinned() {
    // RM 1.2.0 adds `EHR.tags` and the spelling
    // `TRANSLATION_DETAILS.accreditation`; 1.1.0 keeps `accreditaton`.
    let rm =
        testsupport::generation_attribute_delta("rm", "v1_1", "v1_2").expect("rm generations load");
    assert_eq!(rm.classes_added, Vec::<String>::new());
    assert_eq!(
        rm.attributes_added,
        vec![
            "EHR.tags".to_owned(),
            "TRANSLATION_DETAILS.accreditation".to_owned()
        ]
    );

    // RM removes the nine 1.1.0-BMM-only classes and
    // `PARTY.reverse_relationships` (SPECRM-124,
    // `demographic/master00-amendment_record.adoc`).
    assert_eq!(
        rm.classes_removed,
        vec![
            "CITATION".to_owned(),
            "CONSUMABLE_USE".to_owned(),
            "RESOURCE_USAGE".to_owned(),
            "RESOURCE_USE".to_owned(),
            "SERVICE_USE".to_owned(),
            "VIEW_ENTRY".to_owned(),
            "VIEW_ITEM".to_owned(),
            "VIEW_SECTION".to_owned(),
            "VIEW_STATUS".to_owned(),
        ]
    );
    assert_eq!(
        rm.attributes_removed,
        vec![
            "PARTY.reverse_relationships".to_owned(),
            "TRANSLATION_DETAILS.accreditaton".to_owned()
        ]
    );

    // RM retypes `GENERIC_ENTRY.data` from `ITEM_TREE` to `ITEM` (SPECRM-18,
    // `integration/master00-amendment_record.adoc`).
    assert_eq!(
        rm.attributes_changed,
        vec!["GENERIC_ENTRY.data: ITEM_TREE -> ITEM".to_owned()]
    );

    // BASE 1.3.0 adds CODE_PHRASE (SPECAM-82), `Hrid` (SPECBASE-49) with
    // ARCHETYPE_ID's `value: Hrid` (SPECBASE-35), and RESOURCE_DESCRIPTION.title.
    let base = testsupport::generation_attribute_delta("base", "v1_2", "v1_3")
        .expect("base generations load");
    assert_eq!(
        base.classes_added,
        vec!["CODE_PHRASE".to_owned(), "Hrid".to_owned()]
    );
    assert_eq!(
        base.attributes_added,
        vec![
            "ARCHETYPE_ID.value".to_owned(),
            "RESOURCE_DESCRIPTION.title".to_owned()
        ]
    );
    assert_eq!(base.classes_removed, Vec::<String>::new());
    assert_eq!(base.attributes_removed, Vec::<String>::new());

    // BASE retypes `RESOURCE_DESCRIPTION.lifecycle_state` to `String`
    // (SPECBASE-39, `resource/master00-amendment_record.adoc`).
    assert_eq!(
        base.attributes_changed,
        vec!["RESOURCE_DESCRIPTION.lifecycle_state: Terminology_code -> String".to_owned()]
    );
}

/// The delta comparison sees a retype injected into a scratch copy of a
/// generation model, where the existence sets stay silent.
#[test]
fn a_retype_in_a_scratch_generation_fails_the_delta_pin() {
    let older = testsupport::generation_attribute_map("rm", "v1_1").expect("rm v1_1 loads");
    let mut newer = testsupport::generation_attribute_map("rm", "v1_2").expect("rm v1_2 loads");

    let clean = testsupport::attribute_delta(&older, &newer);
    assert!(
        !clean
            .attributes_changed
            .iter()
            .any(|entry| entry.starts_with("COMPOSITION.content:")),
        "the vendored RM generations already disagree about COMPOSITION.content — pick another \
         witness for the mutation",
    );

    // The synthetic retype: a container element type nothing in the RM models.
    newer
        .get_mut("COMPOSITION")
        .expect("RM v1_2 declares COMPOSITION")
        .insert(
            "content".to_owned(),
            "?List<SYNTHETIC_RETYPE> [0..*]".to_owned(),
        );
    let mutated = testsupport::attribute_delta(&older, &newer);

    assert!(
        mutated
            .attributes_changed
            .iter()
            .any(|entry| entry.starts_with("COMPOSITION.content: ")),
        "a retyped attribute did not reach attributes_changed — the delta is comparing names \
         again, and every retype is invisible to the pin",
    );
    assert_eq!(
        mutated.attributes_added, clean.attributes_added,
        "a retype must not read as an added attribute",
    );
    assert_eq!(
        mutated.attributes_removed, clean.attributes_removed,
        "a retype must not read as a removed attribute — which is precisely why the existence \
         sets alone cannot pin the model delta",
    );
}

/// The generations of one crate realize the same BMM-declared functions.
#[test]
fn generation_function_realization_agrees() {
    for key in ["base", "rm", "am", "term", "lang"] {
        let divergent =
            testsupport::generation_function_divergence(key).expect("crate tree readable");
        assert_eq!(
            divergent,
            Vec::<String>::new(),
            "{key}: a BMM-declared function is realized in one generation and not another — \
             the template (or its per-generation override) drifted",
        );
    }
}

/// The unrealized-function projection reports a class that has no behaviour
/// sibling but declares an unrealized function.
#[test]
fn the_unrealized_projection_measures_classes_without_a_behaviour_sibling() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/openehr-rm/src");
    let reported = testsupport::unrealized_bmm_functions("rm").expect("crate tree readable");

    // A reported entry whose class owns no `*_impl.rs` in its generation.
    let sibling_less = reported.iter().find(|entry| {
        let Some((generation, rest)) = entry.split_once('/') else {
            return false;
        };
        let Some((class, _)) = rest.split_once('.') else {
            return false;
        };
        let stem = format!("{}_impl.rs", class.to_lowercase());
        !walk_contains(&src.join(generation), &stem)
    });

    assert!(
        sibling_less.is_some(),
        "no reported gap belongs to a class without a behaviour sibling, so this test no \
         longer proves the projection reaches those classes. Either every such class is now \
         implemented — in which case delete this test with the #2247 record — or a sibling \
         gate has been reintroduced and the projection went silent again.",
    );
}

/// Whether any file named `name` exists under `dir`, at any depth.
fn walk_contains(dir: &std::path::Path, name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if walk_contains(&path, name) {
                return true;
            }
        } else if path.file_name().is_some_and(|f| f == name) {
            return true;
        }
    }
    false
}

/// The unrealized BMM functions equal `unrealized_bmm_functions.txt` exactly;
/// the list only shrinks (#2030).
#[test]
fn unrealized_bmm_functions_match_the_ratchet() {
    let recorded: Vec<String> = include_str!("unrealized_bmm_functions.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(str::to_owned)
        .collect();
    let mut actual = Vec::new();
    for key in ["base", "rm", "am", "term", "lang"] {
        for entry in testsupport::unrealized_bmm_functions(key).expect("crate tree readable") {
            actual.push(format!("{key}/{entry}"));
        }
    }
    actual.sort();
    assert_eq!(
        actual, recorded,
        "the unrealized-function set moved: implement the accessor (and delete its line from \
         tools/openehr-codegen/tests/it/unrealized_bmm_functions.txt), or — for a NEW entry — \
         realize it rather than recording it; the list only shrinks (#2030)",
    );
}

/// A hand-written file identical across a crate's generations modulo
/// generation tokens must be a template.
///
/// The `openehr-lang` `v1_0`/`v1_1` lexer and parser pairs differ because
/// `v1_0` is the ODIN-only Release 1.0.0 generation (#2984).
#[test]
fn hand_written_twins_are_templates() {
    for key in ["base", "rm", "am", "term", "lang"] {
        let identical =
            testsupport::identical_hand_written_twins(key).expect("crate tree readable");
        assert_eq!(
            identical,
            Vec::<String>::new(),
            "{key}: hand-written generation twins identical modulo generation tokens — \
             convert each to a template (or a per-generation override) under \
             tools/openehr-codegen/templates/",
        );
    }
}

/// An `xs:enumeration` facet set reaches the generated model as a typed enum,
/// and every slot declared with one is typed to it.
#[test]
fn every_enumeration_facet_set_emits_a_typed_enum() {
    let untyped = testsupport::untyped_enumeration_facets().expect("closures readable");
    assert_eq!(
        untyped,
        Vec::new(),
        "an xs:enumeration-faceted simple type is carried as free text instead of the \
         closed value space the facet declares"
    );
}

/// Reading `xsi:type` variants as concrete descendants only keeps every concrete
/// type below an abstract slot type as a variant.
#[test]
fn the_concrete_only_variant_reading_loses_no_document_shape() {
    let lost = testsupport::lost_dispatch_variants().expect("closures readable");
    assert_eq!(
        lost,
        Vec::new(),
        "the emitted xsi:type variant sets no longer cover every concrete shape a \
         document can present (or admit an abstract type as a variant)"
    );
}

/// The complex-invariant register's venue claims hold, and the five RM
/// resource-package adjudications are pinned by name and venue.
#[test]
fn complex_register_venues_are_real() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rows = testsupport::accounted_complex_invariants("rm").unwrap();
    // Every complex invariant carries a register row, across the merged closure.
    assert!(
        rows.len() >= 43,
        "the complex bucket accounts only {} rows — closure drift?",
        rows.len()
    );
    let unaccounted: Vec<String> = rows
        .iter()
        .filter(|a| a.venue == "UNACCOUNTED")
        .map(|a| format!("{}.{}", a.class, a.name))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "UNACCOUNTED-COMPLEX must stay empty (#2454) — adjudicate: {unaccounted:?}"
    );
    for a in rows.iter().filter(|a| a.venue != "UNACCOUNTED") {
        let where_ = format!("{}.{}", a.class, a.name);
        assert!(
            repo.join(&a.citation).is_file(),
            "{where_}: citation {} is not a vendored spec file",
            a.citation,
        );
        match a.venue {
            "Impl" | "Wire" | "App" => {
                let path = repo.join(&a.site);
                assert!(
                    path.is_file(),
                    "{where_}: realizing file {} is missing",
                    a.site
                );
                let text = std::fs::read_to_string(&path).unwrap();
                assert!(
                    names_in_a_string_literal(&text, &a.name),
                    "{where_}: realizing file {} does not name the invariant",
                    a.site,
                );
            }
            "Excluded" | "Unrealized" => assert!(
                a.site.is_empty(),
                "{where_}: a non-realizing venue must name no site, got {}",
                a.site,
            ),
            other => panic!("{where_}: unexpected complex venue {other}"),
        }
    }
    let venue_of = |class: &str, name: &str| {
        rows.iter()
            .find(|a| a.class == class && a.name == name)
            .map_or("MISSING", |a| a.venue)
    };
    assert_eq!(
        venue_of("AUTHORED_RESOURCE", "Translations_valid"),
        "App",
        "the ch.8 ingest realization row is pinned"
    );
    assert_eq!(venue_of("AUTHORED_RESOURCE", "Description_valid"), "App");
    assert_eq!(
        venue_of("AUTHORED_RESOURCE", "Languages_available_valid"),
        "Excluded"
    );
    assert_eq!(venue_of("RESOURCE_DESCRIPTION", "Language_valid"), "App");
    assert_eq!(
        venue_of("RESOURCE_DESCRIPTION", "Parent_resource_valid"),
        "Excluded"
    );
}

/// `REST_PATH_IDENTIFIERS` has exactly one entry per path parameter of the
/// codegen bundles, each quoting the bundles' description.
#[test]
fn rest_path_identifiers_match_the_oas_components() {
    let checks = testsupport::rest_path_identifiers().unwrap();
    assert!(!checks.is_empty(), "no path-parameter component found");
    for c in &checks {
        let Some((class, citation)) = &c.entry else {
            panic!(
                "{}: a path parameter with no REST_PATH_IDENTIFIERS entry",
                c.component
            );
        };
        assert!(
            !c.descriptions.is_empty(),
            "{}: no vendored bundle declares this path parameter (stale entry)",
            c.component
        );
        // The first quoted run of the citation is the OAS description it reads.
        let quoted = citation
            .split('"')
            .nth(1)
            .unwrap_or_else(|| panic!("{}: the citation quotes no OAS text", c.component));
        for d in &c.descriptions {
            assert!(
                d.contains(quoted),
                "{}: the citation quotes {quoted:?}, the bundle says {d:?}",
                c.component
            );
        }
        // A class is only claimed where the description names its source.
        if let Some(class) = class {
            assert!(
                ["HIER_OBJECT_ID", "OBJECT_VERSION_ID", "UID_BASED_ID"].contains(&class.as_str()),
                "{}: {class} is not a BASE identifier class",
                c.component
            );
            assert!(
                c.descriptions
                    .iter()
                    .all(|d| d.contains("uid") || d.contains("_id")),
                "{}: the description names no identifier",
                c.component
            );
        }
    }
}

/// Every OAS operation with a request body has its media types in
/// `ROUTE_REQUEST_MEDIA`, and none without one does.
#[test]
fn every_request_body_has_its_media_types() {
    let ops = testsupport::request_media().unwrap();
    assert!(
        ops.iter().any(|o| o.has_body),
        "no operation with a body found"
    );
    for o in &ops {
        assert_eq!(
            o.has_body,
            !o.request_media.is_empty(),
            "{}: body declared {}, media {:?}",
            o.operation_id,
            o.has_body,
            o.request_media
        );
    }
}

/// Pins every generated response body still typed `serde_json::Value`, with
/// its reason.
#[test]
fn untyped_response_bodies_are_pinned_with_their_reason() {
    const PINNED: &[&str] = &[];
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/openehr-its/src/rest/generated");
    let mut untyped = std::collections::BTreeSet::new();
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        let mut current = "";
        for line in text.lines() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("pub enum ") {
                current = rest.trim_end_matches(" {").trim();
            }
            if trimmed == "body: serde_json::Value,"
                || trimmed == "body: Option<serde_json::Value>,"
            {
                untyped.insert(current.to_owned());
            }
        }
    }
    let pinned: std::collections::BTreeSet<String> =
        PINNED.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(untyped, pinned, "untyped response bodies changed");
}

/// No mandatory `0..*` container is reachable from a client-committed
/// resource, since the reader reads its absence as empty and the typed passes
/// do not check presence.
#[test]
fn no_mandatory_plain_container_is_reachable_from_a_committed_resource() {
    let roots = [
        "COMPOSITION",
        "EHR_STATUS",
        "EHR_ACCESS",
        "FOLDER",
        "PERSON",
        "ORGANISATION",
        "GROUP",
        "AGENT",
        "ROLE",
        "CONTRIBUTION",
        "ORIGINAL_VERSION",
    ];
    let found = testsupport::shapes::reachable_mandatory_containers(&roots)
        .expect("the RM composition loads");
    assert!(
        found.is_empty(),
        "reachable mandatory plain containers: {found:?}"
    );
}
