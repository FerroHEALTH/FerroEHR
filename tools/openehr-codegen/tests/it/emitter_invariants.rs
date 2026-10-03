// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Emitter invariants, tested as properties over the **real** pipeline on the
//! **real** vendored BMM inputs (design doc §1). Each test drives
//! `openehr_codegen::testsupport`, which runs the same LOAD → ANALYZE → PLAN →
//! RENDER stages `cli.rs` drives. These are the machine-checked replacements for
//! the emitter's guarantees that no other check enforces:
//! completeness, constructibility, byte-determinism, source-package mirroring,
//! downstream-closure correctness, and decision-map integrity.

use openehr_codegen::testsupport;

// ── completeness ────────────────────────────────────────────────────────────

/// Every class of **every BMM generation** is accounted for in the emission
/// plan: nothing is silently dropped, and the only skips are the two sanctioned
/// reasons (mapped-to-Rust, or abstract-with-no-descendants-and-unused). The
/// counts add up to the generation's full class list.
///
/// Counting per generation rather than over a merged class map is the point: a
/// merge keeps every NAME, so a name-level count over it passes while one
/// generation's classes have been replaced wholesale by the other's.
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
/// field — the attribute-level half of completeness, which the class-NAME count
/// above cannot see.
///
/// This is the gate the LANG two-schema merge defeated: merging the stable v2.x
/// BMM and the v3 development line into one class map left every class NAME
/// present while discarding one generation's attribute set for each of the 18
/// names both declare (`LANG/docs/bmm3/master00-amendment_record.adoc`
/// SPECLANG-14 formalises the v2/v3 split).
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
        // Non-vacuity: a broken traversal that checked nothing would also report
        // zero gaps, so pin that real work happened. LANG is the composition this
        // gate exists for (two generations, 205 declared classes between them), so
        // it carries the substantial bar.
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

/// No two BMM generations of one crate claim the same emitted file path or the
/// same crate-prelude identifier.
///
/// A shared path means one generation's output overwrites the other's — a
/// silently picked shape, which is exactly the defect the per-generation
/// emission exists to prevent. A shared prelude identifier means the crate's
/// one-type-per-Rust-name contract is broken.
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

/// LANG emits BOTH extant BMM generations completely, each at its own
/// source-package path: the stable v2.x model under `bmm/`, `bmm_persistence/`
/// and `beom/` (`LANG/docs/bmm/master01-preface.adoc` §History — "the normative,
/// tool-implemented version"), and the v3 development line under `bmm3/`
/// (`LANG/docs/bmm3/master01-preface.adoc` §Previous Versions). A name declared
/// by both — `BMM_CLASS`, `BMM_TYPE`, … — yields two Rust types.
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

/// No emitted concrete class is a non-constructible infinite value (an unbroken
/// mandatory single-valued construction cycle). Formerly a `panic!` mid-emit;
/// now a test that FAILS the suite listing every offender.
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
        let a = testsupport::plan_shapes(key).unwrap();
        let b = testsupport::plan_shapes(key).unwrap();
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

/// The AM ⊃ LANG downstream closure contains the expected cross-`includes`
/// re-emission set — the beom expression/statement subtree, the BMM-3 + EL
/// object model, and the BASE resource metatype — and every member is grafted
/// into the AM 2.4 output.
#[test]
fn downstream_closure_contains_the_beom_bmm3_resource_set() {
    let closure = testsupport::v2_4_reemit_closure().unwrap();
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

/// Upstream (LANG) output is untouched by AM analysis: LANG emits its own
/// expression classes independently, the AM-only rules leaves never leak into
/// LANG, and those leaves ARE emitted into the AM crate.
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

/// Every declarative decision entry carries a non-empty citation/flag, and —
/// for the binding maps — refers to a class/field that EXISTS in the loaded
/// schemas (no stale entries). The mapped-class denylist and primitive type map
/// are exempt from existence (extra entries there are inert), but still must be
/// cited.
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

/// Every adjudicated free-form field is a REAL degrade whose adjudication is
/// written at the site: some generated file renders the field as
/// `serde_json::Value` and carries the `// NOTE:` with the entry's citation.
///
/// This is the non-vacuity half of the decision-map integrity test — an entry
/// for a field that turned out to be typeable would otherwise sit unnoticed,
/// which is exactly the "silence over an untyped slot" this map exists to end.
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

/// Every declared additional subtype member actually widens its parent's
/// polymorphic slot: in every composition whose model defines both classes, the
/// parent's variant set contains the subtype (and at least one composition does
/// define both, so no entry is inert).
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

/// The crate → schema-merge table is itself declarative decision data: every
/// composition entry carries a non-empty citation, lists at least one own BMM
/// file, resolves (its member/dependency BMM files load), and references only
/// known dependency keys.
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

// ── invariant classification (assertion-dialect analyzer) ────────────────────

/// The assertion-dialect analyzer accounts for **every** RM 1.2.0 class
/// invariant in exactly one bucket — nothing is silently dropped. `emitted +
/// runtime-hook-missing + complex == total`, and the per-bucket counts are
/// pinned as the current classifier's verdict (a deliberate classifier or
/// BMM-pin change updates them under review), so an accidental
/// mis-classification or a dropped invariant fails the suite.
#[test]
fn invariant_classification_is_a_total_tripartition() {
    let rows = testsupport::classify_invariants("rm").unwrap();
    // Pinned per-generation tripartitions (design doc §4 R5): a change here is
    // deliberate. ITEM_TAG.Inv_key_valid is COMPLEX, not EMITTED:
    // `key.is_justified` is a boolean-returning BMM function (a method call),
    // which the assertion-dialect emitter cannot project from a field — so
    // 90 emitted / 31 complex for RM 1.2.0 (R5b). RM 1.1.0 (the released
    // generation, #1942) carries its own pinned split.
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

/// Spot-checks pinning the classifier's verdict on representative invariants so
/// the buckets cannot silently swap: a terminology/code-set predicate needs a
/// runtime hook, a quantifier is complex, and a plain emptiness/`Void` check is
/// emittable.
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

/// The generated invariant-core file (`emit-validate`) honestly accounts for
/// every RM class invariant the classifier bucketed, in both directions:
/// every **emittable** invariant resolves to a realization venue and is named
/// in the realization register, and every **inert** (runtime-hook-missing)
/// invariant is named in the pending-adjudication register — so nothing the
/// classifier flagged is silently dropped.
///
/// The accounted set is DERIVED from the classifier (no hardcoded invariant
/// list): an invariant added by a spec bump, or one that changes bucket, enters
/// this accounting automatically and fails the test until it is adjudicated
/// into the register.
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

/// Each realization-register venue claim is checked against reality: a `Core`
/// row's invariant name is a violation-message literal in the generated core
/// file and its core function exists there; an `Impl` / `Wire` / `App` row's
/// cited realizing file exists and names the invariant. A venue claim that stops
/// being true therefore fails here rather than reading as enforcement that
/// silently is not.
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
                        // A parameterised core drives its message from an
                        // inline (value, invariant-name) row table — the name
                        // appears as that quoted row, same file, same
                        // violation producer.
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

/// Whether `name` occurs on a NON-COMMENT line of `text` — the venue-claim
/// predicate.
///
/// A realizing site names the invariant in its violation literal (spec
/// spellings never occur as code identifiers), and a literal may wrap across
/// continuation lines whose own line carries no quote — so the honest,
/// wrap-robust rule is "named outside a comment". A doc-comment or prose
/// mention — narration, the #2443 defect — never satisfies it.
fn names_in_a_string_literal(text: &str, name: &str) -> bool {
    text.lines()
        .any(|line| !line.trim_start().starts_with("//") && line.contains(name))
}

/// The venue-claim predicate itself, mutation-proven in both directions: a
/// prose/comment mention never satisfies it, a violation literal does — the
/// name on a quote-less continuation line included.
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

/// The negative case of the accounting invariant: an invariant the classifier
/// buckets **emitted** with no realization-register row accounts as
/// `UNACCOUNTED` — the failure mode this accounting exists to catch (an
/// emittable invariant that no venue realizes used to be indistinguishable
/// from one a core enforces).
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

/// The assertion-dialect predicate → runtime-function table
/// (`plan::overrides::DIALECT_PREDICATES`) covers **exactly** the classifier's
/// recognised runtime-backed leaf predicates (`RUNTIME_PREDICATES`) — no
/// recognised predicate lacks a runtime hook, and no stale table entry names a
/// predicate the classifier no longer recognises.
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

/// Every constant the BMM declares on an **emitted** class is rendered as a
/// `pub const` in that crate's output — the R5 constants-emission completeness
/// guard. Checked over the two RM terminology identifier classes (22 constants)
/// whose hand-written `*_impl.rs` was deleted in favour of emission.
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

/// The register records the runtime-hook adjudication state: the terminology /
/// code-set family is enforced in `validate::terminology` and only the four
/// `VERSIONED_OBJECT` aggregate invariants stay `runtime-hook-missing`. Pins the
/// split so a regression (an invariant silently sliding back to "pending", or the
/// enforced count drifting) fails the suite.
#[test]
fn register_records_terminology_invariants_as_enforced() {
    let files = testsupport::emit_to_memory().unwrap();
    let rows = testsupport::classify_invariants("rm").unwrap();
    // Every RM generation carries its own cores file with the same adjudicated
    // register split: 30 terminology/code-set invariants wired to the binding
    // table, the 4 `VERSIONED_OBJECT` aggregate invariants pending — per
    // generation (#1942: a selectable generation is a complete peer).
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

        // Every runtime-hook-missing invariant is adjudicated as either
        // enforced in `validate::terminology` (terminology/code-set) or a
        // versioned-object aggregate.
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

/// Every `OAS_MONOMORPHIZATIONS` entry is grounded in the vendored bundles: the
/// schema key exists, and the `title` the entry cites is the `title` the bundles
/// really declare for it. The mapping is only legitimate because the OAS names
/// its own spec class there — an entry that no longer matches is a guess.
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

/// The generated ITS-REST contract carries NO DTO struct for a monomorphized
/// schema, and the sites that referenced one now name the spec type.
///
/// The DTO form was doubly wrong: it lost the spec type's strict canonical-JSON
/// reader, and it was `allOf`-truncated (only the schema's OWN properties), so
/// `ObjectRefOfHierObjectId` shipped without the mandatory `namespace`/`type`.
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

/// Every generated transport DTO is either closed or open, never silently lossy.
///
/// OpenAPI 3.0.3 §Schema Object: "Consistent with JSON Schema,
/// `additionalProperties` defaults to `true`". So a schema refuses undeclared
/// members only when it says `additionalProperties: false`, which the DTO
/// realizes as `deny_unknown_fields`; every other schema keeps them in the
/// flattened `additional_properties` map. A DTO with neither would drop
/// members the schema admits (#3526).
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

/// The shared-module fallback document carries the `allOf` BASE closure of the
/// hoisted schemas, not the hoisted names alone.
///
/// `emit_common` flattens an `allOf` composition by resolving the base `$ref`
/// through the very document it is emitting from. A document holding only the
/// hoisted names leaves that ref dangling, and the struct silently ships with
/// just its own `properties` — the truncation that shipped `Clstr` without a
/// single `ITEM`/`LOCATABLE` member.
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

/// Every `default` facet the vendored schemas carry is accounted for: either
/// the emitter renders it into a literal Rust default, or it is a NAMED
/// un-renderable adjudication. Nothing is silently dropped.
///
/// The facet is an undeclared extension to the released persistence model (LANG
/// `docs/UML/classes/org.openehr.lang.bmm_persistence.p_bmm_property.adoc`
/// declares no `default` attribute), which is exactly why every occurrence needs
/// a disposition rather than an assumption.
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
    // and the reverse: no adjudicated exclusion is stale (it must still be an
    // un-rendered vendored facet).
    for (owner, field) in [
        ("RESOURCE_DESCRIPTION", "parent_resource"),
        ("CODE_SET", "status"),
        ("TERMINOLOGY_GROUP", "status"),
        ("CODE", "status"),
        ("TERMINOLOGY_CONCEPT", "status"),
    ] {
        assert!(
            facets
                .iter()
                .any(|f| f.owner == owner && f.field == field && f.rendered.is_none()),
            "{owner}.{field} is adjudicated un-renderable but carries no un-rendered vendored \
             facet — the entry is stale",
        );
    }
}

/// The hand-written `field_default` table is the RESIDUE of the vendored facet,
/// never a duplicate of it: no entry may name a property that already carries a
/// renderable vendored `default`.
///
/// Without this, the loaded input and the hand table can silently disagree —
/// which is precisely what happened while the loader dropped the facet
/// (`Point_interval` declared all four flags WITH defaults and got none, because
/// the table keyed only the inherited `Interval` sites).
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

/// The vendored facet reaches the emitted canonical-JSON reader: the four
/// `Point_interval` flags — the ONLY place the BASE schema states the interval
/// default — now carry it, exactly as the inherited `Proper_interval` sites do.
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

/// Five class names are declared by BOTH the RM 1.2.0 and the BASE 1.3.0 BMM.
/// That is spec-mandated, not accidental duplication (RM
/// `docs/common/master08-resource_package.adoc` keeps the ADL-1.4 resource
/// package "retained only while needed by AOM 1.4 based archetypes and tools",
/// and BASE `docs/foundation_types/master00-amendment_record.adoc` adds
/// `CODE_PHRASE` to Foundation Types as a LEGACY class for AOM 1.4) — so both
/// generations are emitted, each into its owning component's crate.
///
/// This pins the twin set and the member divergences that motivate it, so a
/// re-vendored input that silently unifies or further diverges them fails here
/// with the adjudication in view (`plan::composition`'s module note).
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

    // The ADL-1.4 generation's two member placements that look like defects and
    // are not: the RM twin keeps the pre-SPECPUB-6 spelling, and puts
    // `copyright` on the ITEM (which the vendored ADL-1.4 `Resource.xsd` also
    // does), while the BASE twin carries the corrected/relocated forms.
    let rm_translation = testsupport::declared_attributes("rm", "TRANSLATION_DETAILS")
        .unwrap()
        .unwrap();
    let base_translation = testsupport::declared_attributes("base", "TRANSLATION_DETAILS")
        .unwrap()
        .unwrap();
    assert!(
        rm_translation.contains("accreditaton") && !rm_translation.contains("accreditation"),
        "the RM (ADL-1.4) TRANSLATION_DETAILS no longer carries the retained `accreditaton` \
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

/// The acceptance-boundary ledger (#1943; the REMOVED direction #1961; the
/// RETYPED direction #2382): the EXACT model delta between the stable
/// profile's released generations and the development pins — every class and
/// attribute ADDED, REMOVED, or RETYPED, an attribute's signature covering its
/// type, its existence and its container cardinality — pinned so a re-vendor
/// that changes any direction FAILS here until the application's profile
/// boundary is extended to cover the new surface.
///
/// Verified first-hand 2026-08-05 over the vendored BMMs; the retype direction
/// 2026-08-14. The wire consequences the pins protect: no ADDED delta is
/// client-postable (`EHR.tags` is server-managed — the item-tag API stores
/// rows, the EHR wire never carries the attribute; `CODE_PHRASE`/
/// `RESOURCE_DESCRIPTION` are BASE resource-metadata surface outside the REST
/// ingress). In the REMOVED direction, `PARTY.reverse_relationships` IS
/// client-postable released surface, and the stable demographic ingress
/// boundary accepts it (`ferroehr-rest` `api::demographic::party::rm_party`);
/// the nine 1.1.0-BMM-only classes carry no enforceable wire surface
/// (documented nowhere — the #1927 defect family) and no served root reaches
/// them. In the RETYPED direction, `GENERIC_ENTRY.data` IS client-postable
/// released surface whose two types are disjoint, and the profile gate refuses
/// the development-only form on the stable profile; `RESOURCE_DESCRIPTION`
/// stays outside the REST ingress. A new delta entry invalidates the matching
/// adjudication — extend the boundary in `ferroehr` first, then re-pin here.
///
/// SCOPE (#2390): the pin covers RM and BASE — the two profile-coupled
/// components whose types can reach a stored version body or the REST
/// ingress, which is the surface the acceptance boundary gates. LANG is
/// DELIBERATELY outside it, for two adjudicated reasons: LANG types are
/// code-generation input only (the application binds nothing but the
/// `openehr_lang::Generation` token — `ferroehr::config::profile`), so no
/// LANG delta entry can change what the profile boundary accepts or serves;
/// and the `v1_1` generation emits the v2.x and BMM3 SPECIFICATION UNITS side
/// by side with 18 class names declared in BOTH with materially different
/// shapes, so a name-keyed cross-generation attribute diff over that
/// generation is ill-defined by construction. `docs/VERSIONS.md` §Spec
/// version policy states the same scope, so the policy's citation and this
/// guard agree about what is covered.
#[test]
fn profile_generation_delta_is_pinned() {
    // RM: 1.2.0 adds exactly `EHR.tags` over 1.1.0 (and no classes).
    let rm =
        testsupport::generation_attribute_delta("rm", "v1_1", "v1_2").expect("rm generations load");
    assert_eq!(rm.classes_added, Vec::<String>::new());
    assert_eq!(rm.attributes_added, vec!["EHR.tags".to_owned()]);

    // RM REMOVED direction (#1961): the development generation drops the
    // nine 1.1.0-BMM-only classes (undocumented machine-readable classes of
    // a released artifact — the #1927 defect family; no docs text anywhere
    // defines them, so they carry no enforceable wire surface) and
    // `PARTY.reverse_relationships` (upstream SPECRM-124, RM
    // `demographic/master00-amendment_record.adoc` — real released surface,
    // the stable ingress boundary accepts it).
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
        vec!["PARTY.reverse_relationships".to_owned()]
    );

    // RM RETYPED direction (#2382): the development generation widens
    // `GENERIC_ENTRY.data` from `ITEM_TREE` to the abstract `ITEM` — SPECRM-18,
    // "Change `GENERIC_ENTRY` data attribute type from `ITEM_TREE` to the
    // abstract `ITEM` class" (RM `integration/master00-amendment_record.adoc`).
    // The two types are disjoint, so this IS the construct class the profile
    // acceptance boundary exists for.
    assert_eq!(
        rm.attributes_changed,
        vec!["GENERIC_ENTRY.data: ITEM_TREE -> ITEM".to_owned()]
    );

    // BASE: 1.3.0 adds exactly the legacy CODE_PHRASE class (SPECAM-82) and
    // RESOURCE_DESCRIPTION.title over 1.2.0, and removes nothing.
    let base = testsupport::generation_attribute_delta("base", "v1_2", "v1_3")
        .expect("base generations load");
    assert_eq!(base.classes_added, vec!["CODE_PHRASE".to_owned()]);
    assert_eq!(
        base.attributes_added,
        vec!["RESOURCE_DESCRIPTION.title".to_owned()]
    );
    assert_eq!(base.classes_removed, Vec::<String>::new());
    assert_eq!(base.attributes_removed, Vec::<String>::new());

    // BASE RETYPED direction: `RESOURCE_DESCRIPTION.lifecycle_state` becomes a
    // plain `String` — SPECBASE-39, "Change `RESOURCE_DESCRIPTION`
    // _lifecycle_state_ type to `String`" (BASE
    // `resource/master00-amendment_record.adoc`). Resource metadata, outside
    // the REST ingress, and the development form is the wider one.
    assert_eq!(
        base.attributes_changed,
        vec!["RESOURCE_DESCRIPTION.lifecycle_state: Terminology_code -> String".to_owned()]
    );
}

/// The delta comparison SEES a retype: injecting one into a scratch copy of a
/// loaded generation model makes it surface, and the existence sets stay
/// silent about it.
///
/// The mutation proof for the pin above (#2382). Before the signature
/// comparison landed, the name-set difference reported nothing for a changed
/// type — which is exactly how the `GENERIC_ENTRY.data` retype stayed
/// invisible to a guard whose contract is the EXACT model delta.
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

/// The generations of one crate realize the SAME BMM-declared functions.
///
/// Templates give every generation one hand-written source, so a class both
/// generations declare must expose the same accessors in both. A divergence is a
/// per-generation override that drifted, or a re-vendored rename only one
/// generation followed.
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

/// The unrealized-function projection is TOTAL over emitted classes: a class
/// that declares functions and has no behaviour sibling still gets measured.
///
/// The projection used to skip those classes outright, so it was silent about
/// exactly the ones with the most missing — 576 declared functions unreported
/// against 75 shown (#2247). This asserts the PROPERTY rather than naming a
/// witness: it finds a class that has no behaviour sibling in the tree as it
/// stands and still declares an unrealized function, and requires the
/// projection to report it. Naming one made the test fail the moment that
/// function was implemented, which is a burn-down succeeding, not a regression.
#[test]
fn the_unrealized_projection_measures_classes_without_a_behaviour_sibling() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/openehr-rm/src");
    let reported = testsupport::unrealized_bmm_functions("rm").expect("crate tree readable");

    // A reported entry whose class owns no `*_impl.rs` anywhere under its
    // generation: that class is measured despite having no sibling, which is
    // the property the sibling gate used to break.
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

/// Every BMM-declared function on an emitted class is realized, ratcheted
/// against the committed list of gaps.
///
/// The BMM declares functions by name and result type only, so their bodies are
/// hand-written. This asserts the unrealized set equals
/// `unrealized_bmm_functions.txt` EXACTLY: a new gap fails, and implementing a
/// listed one fails until the line is deleted. The list only shrinks; the
/// burn-down is issue #2030.
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

/// Generation-twin discipline (#1964): a hand-written file that is
/// byte-identical across a crate's generations modulo generation tokens MUST
/// be a template (`tools/openehr-codegen/templates/<crate>/…`) — one source,
/// per-generation copies stamped by `emit` — so twins can never silently
/// diverge. A non-empty list here names the families to convert.
///
/// The `openehr-lang` `v1_0`/`v1_1` sibling pairs (`lexer/mod.rs`,
/// `lexer/token.rs`, `lexer/reclassify.rs`, `odin/parser.rs`) are NOT covered,
/// and that is the adjudicated outcome (#2984), not a gap: measured on
/// 2026-09-01 they diverge by 117–892 diff lines per pair, because `v1_0` is
/// the faithful Release-1.0.0 generation — an ODIN-only lexer over the
/// release's own grammar set, no EL/BEL (`docs/VERSIONS.md` §LANG; the #1946
/// faithful-emission decision) — while `v1_1` carries the development
/// generation's larger token and reclassification surface. Whole-file-verbatim
/// overrides cannot express an eighty-percent-shared file, so conversion would
/// re-house the divergence without removing the duplication. The identity
/// check below stays the machine half: if a pair ever converges to
/// identical-modulo-tokens, it appears in this list and MUST convert then.
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

/// Reading `xsi:type` variants as CONCRETE descendants only discards no
/// document shape.
///
/// A type declared `abstract` is not a legal `xsi:type` value, so it is
/// correctly absent from a slot's dispatch enum. What has to hold alongside
/// that is the other half: every CONCRETE type below such an abstract type is
/// legal at the slot and must be a variant.
///
/// The closures are full of slots typed above an abstract type — `EXPR_ITEM`
/// over `EXPR_OPERATOR`, `C_OBJECT` over `C_DOMAIN_TYPE`, `OBJECT_ID` over
/// `UID_BASED_ID` — so this is a live property, not a vacuous one, and a
/// re-vendoring that broke either half would otherwise surface as a document
/// that silently fails to parse (#2271).
/// An `xs:enumeration` facet set reaches the generated model as a typed enum,
/// never as free text.
///
/// The facet declares a CLOSED value space, so a slot typed to it must refuse
/// anything outside the set — carrying it as `String` makes an out-of-range
/// value indistinguishable from a declared one. Both halves are checked: every
/// faceted simple type of every XSD-driven closure emits its enum, and every
/// element slot declared with one is typed to that enum, so a re-vendoring that
/// adds a faceted type (or retypes a slot) cannot fall back to `String`
/// unnoticed.
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

/// The complex-invariant register's venue claims are real, exactly like the
/// emitted register's: a citation names a vendored file, a realizing venue
/// names an existing file that names the invariant, a non-realizing venue
/// names no site — and the five RM resource-package adjudications the ch.8
/// audit produced are pinned by name and venue.
#[test]
fn complex_register_venues_are_real() {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rows = testsupport::accounted_complex_invariants("rm").unwrap();
    // The bucket is fully drained (#2454): every complex invariant carries a
    // register row, and the accounted set spans the whole merged closure
    // (BASE foundation classes included) — a shrink means rows fell out of
    // the accounting, not out of the model.
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

/// `REST_PATH_IDENTIFIERS` is held to the vendored OAS components: every path
/// parameter of the codegen bundles has exactly one entry, no entry names a
/// component the bundles no longer declare, and the description each entry
/// quotes is the bundles' own text, so a re-vendored ITS-REST release that
/// changes a parameter's meaning fails here instead of drifting.
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

/// Every OAS operation that declares a request body has its media types, the
/// `requestBody.content` keys the route table's `ROUTE_REQUEST_MEDIA` carries,
/// and none is claimed for an operation without a body.
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
