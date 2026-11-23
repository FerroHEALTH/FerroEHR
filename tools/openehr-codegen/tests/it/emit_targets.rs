// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every text-producing emit target over the real pipeline: each render is
//! byte-deterministic, produces exactly its file set, and equals the committed
//! generated tree.

use openehr_codegen::testsupport;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The complete file set `emit-json` produces, in emitted-map (sorted) order.
const JSON_FILES: &[&str] = &[
    "openehr-am/src/json_serde.rs",
    "openehr-base/src/json_serde.rs",
    "openehr-its/src/json_codec/generated/mod.rs",
    "openehr-its/src/json_codec/generated/structural.rs",
    "openehr-lang/src/json_serde.rs",
    "openehr-rm/src/json_serde.rs",
    "openehr-term/src/json_serde.rs",
];

/// The complete file set `emit-xml` produces, in emitted-map (sorted) order.
const XML_FILES: &[&str] = &[
    "openehr-its/src/xml/generated/impls.rs",
    "openehr-its/src/xml/generated/mod.rs",
];

/// The file set `emit-rest` produces, in sorted order.
const REST_FILES: &[&str] = &[
    "openehr-its/src/rest/generated/admin.rs",
    "openehr-its/src/rest/generated/common.rs",
    "openehr-its/src/rest/generated/definition.rs",
    "openehr-its/src/rest/generated/demographic.rs",
    "openehr-its/src/rest/generated/ehr.rs",
    "openehr-its/src/rest/generated/mod.rs",
    "openehr-its/src/rest/generated/query.rs",
    "openehr-its/src/rest/generated/system.rs",
];

/// The complete file set `emit-opt` produces, in emitted-map (sorted) order.
const OPT_FILES: &[&str] = &[
    "openehr-its/src/opt14/impls.rs",
    "openehr-its/src/opt14/mod.rs",
    "openehr-its/src/opt14/types.rs",
];

/// The file set `emit-aom2` produces, in sorted order.
const AOM2_FILES: &[&str] = &[
    "openehr-its/src/aom2/impls.rs",
    "openehr-its/src/aom2/mod.rs",
    "openehr-its/src/aom2/types.rs",
    "openehr-its/src/aom2_model/impls.rs",
    "openehr-its/src/aom2_model/mod.rs",
    "openehr-its/src/aom2_model/types.rs",
];

/// The file set `emit-rm-model` produces, one `model/` subtree per RM
/// generation.
const RM_MODEL_FILES: &[&str] = &[
    "openehr-rm/src/v1_1/model/data.rs",
    "openehr-rm/src/v1_1/model/mod.rs",
    "openehr-rm/src/v1_2/model/data.rs",
    "openehr-rm/src/v1_2/model/mod.rs",
];

/// The complete file set `emit-validate` produces, in emitted-map (sorted)
/// order: one invariant-core file per RM generation.
const VALIDATE_FILES: &[&str] = &[
    "openehr-rm/src/v1_1/validate/generated.rs",
    "openehr-rm/src/v1_1/validate/tree_generated.rs",
    "openehr-rm/src/v1_2/access/generated.rs",
    "openehr-rm/src/v1_2/released/generated.rs",
    "openehr-rm/src/v1_2/validate/generated.rs",
    "openehr-rm/src/v1_2/validate/tree_generated.rs",
];

/// The generated spec crates `emit` composes, one per `COMPOSITIONS` row.
const EMIT_CRATES: &[&str] = &[
    "openehr-am",
    "openehr-base",
    "openehr-lang",
    "openehr-rm",
    "openehr-term",
];

/// The first-line marker a generation-twin template stamp carries.
const TEMPLATE_MARKER: &str = "// @generated-from-template";

// ── emit ────────────────────────────────────────────────────────────────────

/// Rendering the whole spec layer twice yields byte-identical output.
#[test]
fn emit_is_byte_deterministic() {
    let a = testsupport::emit_to_memory().unwrap();
    let b = testsupport::emit_to_memory().unwrap();
    assert_deterministic("emit", &a, &b);
}

/// `emit` covers every composed crate, the RM model and cores, and the
/// template stamps, each file well-formed.
#[test]
fn emit_emits_every_composed_crate_including_the_template_stamps() {
    let files = testsupport::emit_to_memory().unwrap();
    assert_well_formed("emit", "@generated", &files);
    for krate in EMIT_CRATES {
        let prefix = format!("{krate}/src/");
        assert!(
            files.keys().any(|p| p.starts_with(&prefix)),
            "emit: {krate} contributed no files",
        );
    }
    // The RM crate's model + invariant cores are part of `emit`'s own set, not
    // a separate run: a plain `emit` keeps the crate self-consistent.
    for path in [
        "openehr-rm/src/v1_2/model/mod.rs",
        "openehr-rm/src/v1_2/access/generated.rs",
        "openehr-rm/src/v1_2/released/generated.rs",
        "openehr-rm/src/v1_2/validate/generated.rs",
    ] {
        assert!(
            files.contains_key(path),
            "emit: {path} is not in the emitted set",
        );
    }
    let stamped: Vec<&String> = files
        .iter()
        .filter(|(_, body)| body.starts_with(TEMPLATE_MARKER))
        .map(|(path, _)| path)
        .collect();
    assert!(
        !stamped.is_empty(),
        "emit: no generation-twin template stamp is in the emitted set",
    );
}

/// The rendered spec layer equals the committed generated tree.
#[test]
fn emit_matches_the_committed_tree() {
    let files = testsupport::emit_to_memory().unwrap();
    assert_tree_matches_committed("emit", &files);
}

// ── emit-json ───────────────────────────────────────────────────────────────

/// Rendering the canonical-JSON impls twice yields byte-identical output.
#[test]
fn emit_json_is_byte_deterministic() {
    let a = testsupport::emit_json_to_memory().unwrap();
    let b = testsupport::emit_json_to_memory().unwrap();
    assert_deterministic("emit-json", &a, &b);
}

/// `emit-json` produces exactly its file set, each file non-empty and carrying
/// the generated banner plus its crate's SPDX header.
#[test]
fn emit_json_emits_its_whole_file_set() {
    let files = testsupport::emit_json_to_memory().unwrap();
    assert_file_set("emit-json", &files, JSON_FILES);
}

/// The rendered canonical-JSON impls equal the committed generated tree.
#[test]
fn emit_json_matches_the_committed_tree() {
    let files = testsupport::emit_json_to_memory().unwrap();
    assert_matches_committed_tree("emit-json", &files);
}

// ── emit-xml ────────────────────────────────────────────────────────────────

/// Rendering the canonical-XML impls twice yields byte-identical output.
#[test]
fn emit_xml_is_byte_deterministic() {
    let a = testsupport::emit_xml_to_memory().unwrap();
    let b = testsupport::emit_xml_to_memory().unwrap();
    assert_deterministic("emit-xml", &a, &b);
}

/// `emit-xml` produces exactly its file set, each file non-empty and carrying
/// the generated banner plus its crate's SPDX header.
#[test]
fn emit_xml_emits_its_whole_file_set() {
    let files = testsupport::emit_xml_to_memory().unwrap();
    assert_file_set("emit-xml", &files, XML_FILES);
}

/// The rendered canonical-XML impls equal the committed generated tree.
#[test]
fn emit_xml_matches_the_committed_tree() {
    let files = testsupport::emit_xml_to_memory().unwrap();
    assert_matches_committed_tree("emit-xml", &files);
}

// ── emit-rest ───────────────────────────────────────────────────────────────

/// Rendering the ITS-REST contract twice yields byte-identical output.
#[test]
fn emit_rest_is_byte_deterministic() {
    let a = testsupport::emit_rest_to_memory().unwrap();
    let b = testsupport::emit_rest_to_memory().unwrap();
    assert_deterministic("emit-rest", &a, &b);
}

/// `emit-rest` produces exactly its file set, each file non-empty and carrying
/// the generated banner plus its crate's SPDX header.
#[test]
fn emit_rest_emits_its_whole_file_set() {
    let files = testsupport::emit_rest_to_memory().unwrap();
    assert_file_set("emit-rest", &files, REST_FILES);
}

/// The rendered ITS-REST contract equals the committed generated tree.
#[test]
fn emit_rest_matches_the_committed_tree() {
    let files = testsupport::emit_rest_to_memory().unwrap();
    assert_matches_committed_tree("emit-rest", &files);
}

/// Every API group gates exactly one server module and one client module.
#[test]
fn every_rest_group_gates_one_server_module_and_one_client_module() {
    let files = testsupport::emit_rest_to_memory().unwrap();
    for group in REST_GROUPS {
        let Some(text) = rest_group(&files, group) else {
            panic!("emit-rest: the {group} group was not rendered");
        };
        assert_eq!(
            text.matches("#[cfg(feature = \"rest-server\")]").count(),
            1,
            "emit-rest: {group} must gate exactly one item on rest-server",
        );
        assert_eq!(
            text.matches(SERVER_GATE).count(),
            1,
            "emit-rest: {group}'s rest-server gate must sit on its `pub mod server`",
        );
        assert_eq!(
            text.matches("#[cfg(feature = \"rest-client\")]").count(),
            1,
            "emit-rest: {group} must gate exactly one item on rest-client",
        );
        assert_eq!(
            text.matches(CLIENT_GATE).count(),
            1,
            "emit-rest: {group}'s rest-client gate must sit on its `pub mod client`",
        );
    }
}

/// Every operation of a group has one outcome enum and one client method.
#[test]
fn every_rest_operation_has_one_outcome_and_one_client_method() {
    let files = testsupport::emit_rest_to_memory().unwrap();
    for group in REST_GROUPS {
        let Some(text) = rest_group(&files, group) else {
            panic!("emit-rest: the {group} group was not rendered");
        };
        let Some(client) = client_module(text) else {
            panic!("emit-rest: {group} renders no client module ahead of its route table");
        };
        let Some(routes) = route_count(text) else {
            panic!("emit-rest: {group} renders no route table");
        };
        let outcomes = client
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("pub enum ") && line.ends_with("Outcome {"))
            .count();
        let methods = client.matches("pub async fn ").count();
        assert!(routes > 0, "emit-rest: {group} declares no operations");
        assert_eq!(
            outcomes, routes,
            "emit-rest: {group} has {outcomes} outcome enums for {routes} operations",
        );
        assert_eq!(
            methods, routes,
            "emit-rest: {group} has {methods} client methods for {routes} operations",
        );
    }
}

/// Every operation of a group has one success enum, trait method, router
/// binding and handler.
#[test]
fn every_rest_operation_has_one_server_response_route_and_handler() {
    let files = testsupport::emit_rest_to_memory().unwrap();
    for group in REST_GROUPS {
        let Some(text) = rest_group(&files, group) else {
            panic!("emit-rest: the {group} group was not rendered");
        };
        let Some(server) = server_module(text) else {
            panic!("emit-rest: {group} renders no server module ahead of its client module");
        };
        let Some(routes) = route_count(text) else {
            panic!("emit-rest: {group} renders no route table");
        };
        let responses = server
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("pub enum ") && line.ends_with("Response {"))
            .count();
        let methods = server
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("async fn ") && !line.starts_with("async fn handle_"))
            .count();
        let bindings = server.matches("axum::routing::on(").count();
        let handlers = server.matches("async fn handle_").count();
        for (what, count) in [
            ("success-answer enums", responses),
            ("trait methods", methods),
            ("router bindings", bindings),
            ("handlers", handlers),
        ] {
            assert_eq!(
                count, routes,
                "emit-rest: {group} has {count} {what} for {routes} operations",
            );
        }
    }
}

/// Every documented response is an outcome variant the client method matches,
/// and no `UnregisteredStatus` reaches the contract.
#[test]
fn every_documented_rest_response_is_a_matched_variant() {
    let files = testsupport::emit_rest_to_memory().unwrap();
    for group in REST_GROUPS {
        let Some(text) = rest_group(&files, group) else {
            panic!("emit-rest: the {group} group was not rendered");
        };
        let Some(client) = client_module(text) else {
            panic!("emit-rest: {group} renders no client module ahead of its route table");
        };
        let answers = client
            .lines()
            .filter(|line| is_answer_doc(line.trim()))
            .count();
        let arms = client.matches("http::StatusCode::").count();
        assert!(answers > 0, "emit-rest: {group} documents no answer");
        assert_eq!(
            answers, arms,
            "emit-rest: {group} documents {answers} answers but matches {arms} statuses",
        );
        assert!(
            !text.contains("UnregisteredStatus") && !text.contains("UNREGISTERED_"),
            "emit-rest: {group} carries a status outside the emitter's status table",
        );
    }
}

/// The client module reaches the hoisted shared types as
/// `super::super::common::`.
#[test]
fn rest_client_modules_reach_hoisted_types_through_super_super() {
    let files = testsupport::emit_rest_to_memory().unwrap();
    let mut hoisted = 0_usize;
    for group in REST_GROUPS {
        let Some(text) = rest_group(&files, group) else {
            panic!("emit-rest: the {group} group was not rendered");
        };
        let Some(client) = client_module(text) else {
            panic!("emit-rest: {group} renders no client module ahead of its route table");
        };
        for (at, _) in client.match_indices("super::common::") {
            assert!(
                client
                    .get(..at)
                    .is_some_and(|before| before.ends_with("super::")),
                "emit-rest: {group}'s client module names `super::common::` at byte {at}",
            );
            hoisted += 1;
        }
    }
    assert!(
        hoisted > 0,
        "emit-rest: no client module references a hoisted type, so the property is vacuous",
    );
}

/// The ITS-REST API groups, one generated module each.
const REST_GROUPS: &[&str] = &[
    "admin",
    "definition",
    "demographic",
    "ehr",
    "query",
    "system",
];

/// The feature gate and header of a group's client module, as the emitter
/// writes them.
const CLIENT_GATE: &str = "#[cfg(feature = \"rest-client\")]\npub mod client {";

/// The feature gate and header of a group's server module, as the emitter
/// writes them.
const SERVER_GATE: &str = "#[cfg(feature = \"rest-server\")]\npub mod server {";

/// The server module of a rendered group: from its feature gate up to the
/// client module the emitter writes after it.
fn server_module(text: &str) -> Option<&str> {
    let start = text.find(SERVER_GATE)?;
    let end = text.find(CLIENT_GATE)?;
    text.get(start..end)
}

/// The rendered module of API group `group`.
fn rest_group<'a>(files: &'a BTreeMap<String, String>, group: &str) -> Option<&'a str> {
    files
        .get(&format!("openehr-its/src/rest/generated/{group}.rs"))
        .map(String::as_str)
}

/// The client module of a rendered group: from its feature gate up to the route
/// table the emitter writes after it.
fn client_module(text: &str) -> Option<&str> {
    let start = text.find(CLIENT_GATE)?;
    let end = text.find("pub const ROUTES")?;
    text.get(start..end)
}

/// The number of `(method, path, operation_id)` entries in a group's `ROUTES`.
fn route_count(text: &str) -> Option<usize> {
    let table = text.get(text.find("pub const ROUTES")?..)?;
    let entries = table.get(table.find("= &[")?..table.find("];")?)?;
    Some(entries.matches("(\"").count())
}

/// Whether `line` is the ``/// The `NNN` answer.`` doc line of an outcome
/// variant.
fn is_answer_doc(line: &str) -> bool {
    line.strip_prefix("/// The `")
        .and_then(|rest| rest.strip_suffix("` answer."))
        .is_some_and(|code| code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()))
}

// ── emit-opt ────────────────────────────────────────────────────────────────

/// Rendering the OPT 1.4 model twice yields byte-identical output.
#[test]
fn emit_opt_is_byte_deterministic() {
    let a = testsupport::emit_opt_to_memory().unwrap();
    let b = testsupport::emit_opt_to_memory().unwrap();
    assert_deterministic("emit-opt", &a, &b);
}

/// `emit-opt` produces exactly its file set, each file non-empty and carrying
/// the generated banner plus its crate's SPDX header.
#[test]
fn emit_opt_emits_its_whole_file_set() {
    let files = testsupport::emit_opt_to_memory().unwrap();
    assert_file_set("emit-opt", &files, OPT_FILES);
}

/// The rendered OPT 1.4 model equals the committed generated tree.
#[test]
fn emit_opt_matches_the_committed_tree() {
    let files = testsupport::emit_opt_to_memory().unwrap();
    assert_matches_committed_tree("emit-opt", &files);
}

// ── emit-aom2 ───────────────────────────────────────────────────────────────

/// Rendering both AOM2 archetype codecs twice yields byte-identical output.
#[test]
fn emit_aom2_is_byte_deterministic() {
    let a = testsupport::emit_aom2_to_memory().unwrap();
    let b = testsupport::emit_aom2_to_memory().unwrap();
    assert_deterministic("emit-aom2", &a, &b);
}

/// `emit-aom2` produces exactly its file set — both closures, each into its own
/// module — with every file non-empty, banner-headed and SPDX-stamped.
#[test]
fn emit_aom2_emits_its_whole_file_set() {
    let files = testsupport::emit_aom2_to_memory().unwrap();
    assert_file_set("emit-aom2", &files, AOM2_FILES);
}

/// The rendered AOM2 codecs equal the committed generated tree.
#[test]
fn emit_aom2_matches_the_committed_tree() {
    let files = testsupport::emit_aom2_to_memory().unwrap();
    assert_matches_committed_tree("emit-aom2", &files);
}

// ── emit-rm-model ───────────────────────────────────────────────────────────

/// Rendering the static RM attribute/type model twice yields byte-identical
/// output.
#[test]
fn emit_rm_model_is_byte_deterministic() {
    let a = testsupport::emit_rm_model_to_memory().unwrap();
    let b = testsupport::emit_rm_model_to_memory().unwrap();
    assert_deterministic("emit-rm-model", &a, &b);
}

/// `emit-rm-model` produces exactly its file set — one `model/` subtree per RM
/// generation — with every file non-empty, banner-headed and SPDX-stamped.
#[test]
fn emit_rm_model_emits_its_whole_file_set() {
    let files = testsupport::emit_rm_model_to_memory().unwrap();
    assert_file_set("emit-rm-model", &files, RM_MODEL_FILES);
}

/// The rendered RM model equals the committed generated tree — which is also
/// what `emit` writes there, since both drive `cli::render_rm_model_files`.
#[test]
fn emit_rm_model_matches_the_committed_tree() {
    let files = testsupport::emit_rm_model_to_memory().unwrap();
    assert_matches_committed_tree("emit-rm-model", &files);
}

/// Every RM generation's `mod.rs` declares the emitted model module.
#[test]
fn every_rm_generation_declares_its_model_module() {
    let root = crates_root();
    for path in ["openehr-rm/src/v1_1/mod.rs", "openehr-rm/src/v1_2/mod.rs"] {
        let body = std::fs::read_to_string(root.join(path)).unwrap();
        assert!(
            body.contains("pub mod model;"),
            "{path} does not declare the emitted RM model module",
        );
    }
}

// ── emit-validate ───────────────────────────────────────────────────────────

/// Rendering the RM invariant cores twice yields byte-identical output.
#[test]
fn emit_validate_is_byte_deterministic() {
    let a = testsupport::emit_validate_to_memory().unwrap();
    let b = testsupport::emit_validate_to_memory().unwrap();
    assert_deterministic("emit-validate", &a, &b);
}

/// `emit-validate` produces exactly its file set — one invariant-core file per
/// RM generation — with every file non-empty, banner-headed and SPDX-stamped.
#[test]
fn emit_validate_emits_its_whole_file_set() {
    let files = testsupport::emit_validate_to_memory().unwrap();
    assert_file_set("emit-validate", &files, VALIDATE_FILES);
}

/// The rendered invariant cores equal the committed generated tree.
#[test]
fn emit_validate_matches_the_committed_tree() {
    let files = testsupport::emit_validate_to_memory().unwrap();
    assert_matches_committed_tree("emit-validate", &files);
}

// ── shared assertions ───────────────────────────────────────────────────────

/// Assert two renders of the same target agree on every path and every byte.
fn assert_deterministic(target: &str, a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) {
    assert!(!a.is_empty(), "{target}: rendered nothing");
    let paths_a: Vec<&String> = a.keys().collect();
    let paths_b: Vec<&String> = b.keys().collect();
    assert_eq!(
        paths_a, paths_b,
        "{target}: the two renders emitted different files",
    );
    for ((path, body_a), body_b) in a.iter().zip(b.values()) {
        assert!(
            body_a == body_b,
            "{target}: {path} differs between two renders{}",
            difference_hint(body_a, body_b),
        );
    }
}

/// Assert a target emitted exactly `expected`, with every file well-formed
/// ([`assert_well_formed`]) and its banner naming the target.
fn assert_file_set(target: &str, files: &BTreeMap<String, String>, expected: &[&str]) {
    let actual: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(actual, expected, "{target}: emitted a different file set");
    assert_well_formed(target, target, files);
}

/// Asserts every rendered file is non-empty, opens on a banner carrying
/// `banner_needle`, and carries its crate's SPDX header.
fn assert_well_formed(target: &str, banner_needle: &str, files: &BTreeMap<String, String>) {
    assert!(!files.is_empty(), "{target}: rendered nothing");
    for (path, body) in files {
        assert!(!body.trim().is_empty(), "{target}: {path} rendered empty");
        let banner = body.lines().next().unwrap_or_default();
        assert!(
            banner.contains("@generated") && banner.contains(banner_needle),
            "{target}: {path} does not open with its generated banner: {banner:?}",
        );
        // The needle omits the SPDX expression, which REUSE would refuse.
        // REUSE-IgnoreStart
        assert!(
            body.contains("SPDX-License-Identifier: "),
            "{target}: {path} carries no SPDX licence header",
        );
        // REUSE-IgnoreEnd
    }
}

/// Asserts a target's rendered bytes, through `rustfmt`, equal the committed
/// generated tree.
#[expect(
    clippy::panic,
    reason = "test diagnostics: a missing committed file names itself instead of \
              failing as a bare unwrap"
)]
fn assert_matches_committed_tree(target: &str, files: &BTreeMap<String, String>) {
    let root = crates_root();
    for (path, body) in files {
        let full = root.join(path);
        let committed = std::fs::read_to_string(&full)
            .unwrap_or_else(|e| panic!("{target}: cannot read committed {path}: {e}"));
        let rendered = rustfmt(&committed_relative_name(path), body);
        assert!(
            rendered == committed,
            "{target}: {path} does not match the committed generated tree{}",
            difference_hint(&rendered, &committed),
        );
    }
}

/// Asserts `emit`'s whole rendered tree equals the committed tree, allowing
/// only woven module declarations after a `lib.rs` or `mod.rs` anchor.
#[expect(
    clippy::panic,
    reason = "test diagnostics: a missing committed file names itself instead of \
              failing as a bare unwrap"
)]
fn assert_tree_matches_committed(target: &str, files: &BTreeMap<String, String>) {
    let root = crates_root();
    let mut woven = 0_usize;
    for (path, rendered) in rustfmt_tree(target, files) {
        let full = root.join(&path);
        let committed = std::fs::read_to_string(&full)
            .unwrap_or_else(|e| panic!("{target}: cannot read committed {path}: {e}"));
        if committed == rendered {
            continue;
        }
        let Some(tail) = committed.strip_prefix(&rendered) else {
            panic!(
                "{target}: {path} does not match the committed generated tree{}",
                difference_hint(&rendered, &committed),
            );
        };
        assert!(
            path.ends_with("/lib.rs") || path.ends_with("/mod.rs"),
            "{target}: {path} is not a module anchor, so nothing may be woven into it:\n{tail}",
        );
        assert!(
            is_woven_declarations(tail),
            "{target}: {path} carries content beyond the rendered body that is not a woven \
             module declaration:\n{tail}",
        );
        woven += 1;
    }
    assert!(
        woven > 0,
        "{target}: no anchor carried a woven declaration — the weave is not being exercised",
    );
}

/// Whether a committed file's tail beyond the rendered body is only the module
/// declarations the CLI weaves in after writing it.
fn is_woven_declarations(tail: &str) -> bool {
    tail.lines().all(|line| {
        let t = line.trim();
        t.is_empty()
            || t.starts_with("// ") && t.ends_with("auto-declared:")
            || (t.starts_with("mod ") || t.starts_with("pub mod ")) && t.ends_with(';')
    })
}

/// Formats a whole rendered tree in one `rustfmt` pass, as the emitter does.
#[expect(
    clippy::panic,
    reason = "test diagnostics: each scratch-tree failure mode names itself, so a red \
              run points at the toolchain rather than at the emitter"
)]
fn rustfmt_tree(target: &str, files: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let scratch =
        std::env::temp_dir().join(format!("openehr-codegen-{target}-{}", std::process::id()));
    drop(std::fs::remove_dir_all(&scratch));
    let mut by_crate: BTreeMap<&str, Vec<PathBuf>> = BTreeMap::new();
    for (path, body) in files {
        let full = scratch.join(path);
        let parent = full
            .parent()
            .unwrap_or_else(|| panic!("{target}: {path} has no parent directory"));
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("{target}: cannot create {}: {e}", parent.display()));
        std::fs::write(&full, body)
            .unwrap_or_else(|e| panic!("{target}: cannot write scratch {path}: {e}"));
        let krate = path.split('/').next().unwrap_or_default();
        by_crate.entry(krate).or_default().push(full);
    }
    // `emit` formats on disk, where a crate's `draft` subtree (`emit-drafts`
    // output, outside this target) answers the `mod draft;` rustfmt follows.
    for krate in by_crate.keys() {
        copy_tree(
            &crates_root().join(krate).join("src/draft"),
            &scratch.join(krate).join("src/draft"),
        );
    }
    for (krate, paths) in &by_crate {
        let status = Command::new("rustfmt")
            .args(["--edition", "2024", "--quiet"])
            .args(paths)
            .status()
            .unwrap_or_else(|e| panic!("{target}: rustfmt is not runnable: {e}"));
        assert!(status.success(), "{target}: rustfmt failed on {krate}");
    }
    let out = files
        .keys()
        .map(|path| {
            let body = std::fs::read_to_string(scratch.join(path))
                .unwrap_or_else(|e| panic!("{target}: cannot read scratch {path}: {e}"));
            (path.clone(), body)
        })
        .collect();
    drop(std::fs::remove_dir_all(&scratch));
    out
}

// ── helpers ─────────────────────────────────────────────────────────────────

/// Copy the tree at `from` to `to`, if `from` exists.
#[expect(
    clippy::panic,
    reason = "test diagnostics: a scratch copy that fails names the path"
)]
fn copy_tree(from: &Path, to: &Path) {
    let Ok(entries) = std::fs::read_dir(from) else {
        return;
    };
    std::fs::create_dir_all(to).unwrap_or_else(|e| panic!("cannot create {}: {e}", to.display()));
    for entry in entries.flatten() {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &target);
        } else {
            std::fs::copy(&path, &target)
                .unwrap_or_else(|e| panic!("cannot copy {}: {e}", path.display()));
        }
    }
}

/// The workspace `crates/` directory every emitted path is relative to.
fn crates_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates")
}

/// A short label for a path, used only in `rustfmt` failure messages.
fn committed_relative_name(path: &str) -> String {
    format!("crates/{path}")
}

/// Formats `text` as the emitter's `rustfmt` call does, writing from a
/// separate thread so a large body cannot deadlock the pipe.
#[expect(
    clippy::panic,
    reason = "test diagnostics: each subprocess failure mode names itself, so a red \
              run points at the toolchain rather than at the emitter"
)]
fn rustfmt(label: &str, text: &str) -> String {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--quiet", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("{label}: rustfmt is not runnable: {e}"));
    let mut stdin = child
        .stdin
        .take()
        .unwrap_or_else(|| panic!("{label}: rustfmt stdin was not piped"));
    let body = text.to_owned();
    let writer = std::thread::spawn(move || stdin.write_all(body.as_bytes()));
    let out = child
        .wait_with_output()
        .unwrap_or_else(|e| panic!("{label}: rustfmt did not complete: {e}"));
    writer
        .join()
        .unwrap_or_else(|_| panic!("{label}: the rustfmt writer thread panicked"))
        .unwrap_or_else(|e| panic!("{label}: writing to rustfmt failed: {e}"));
    assert!(
        out.status.success(),
        "{label}: rustfmt failed ({}): {}",
        out.status,
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8(out.stdout)
        .unwrap_or_else(|e| panic!("{label}: rustfmt emitted non-UTF-8: {e}"))
}

/// A one-line locator for the first place two texts diverge.
fn difference_hint(a: &str, b: &str) -> String {
    for (i, (la, lb)) in a.lines().zip(b.lines()).enumerate() {
        if la != lb {
            return format!(
                "\n  first difference at line {}:\n  - {la}\n  + {lb}",
                i + 1
            );
        }
    }
    format!(
        "\n  line counts differ: {} vs {}",
        a.lines().count(),
        b.lines().count(),
    )
}
