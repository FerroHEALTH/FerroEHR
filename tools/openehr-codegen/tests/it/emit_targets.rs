// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every text-producing emit target — `emit`, `emit-json`, `emit-xml`,
//! `emit-rest`, `emit-opt`, `emit-aom2`, `emit-rm-model`, `emit-validate` — as
//! properties over the **real** pipeline on the **real** vendored inputs.
//!
//! Each test drives `openehr_codegen::testsupport::emit_*_to_memory`, which
//! calls the very render function the matching `cmd_*` handler calls: the
//! handler is a write-files shell over it, so tested text and emitted text
//! cannot drift. Every target is asserted three ways — the render is
//! byte-deterministic, it produces exactly its file set (each non-empty,
//! banner-headed and SPDX-stamped), and its bytes equal the committed
//! generated tree, which is the in-process half of the `codegen-drift` check.
//!
//! `emit` is the one target whose file set is pinned by SHAPE rather than by
//! enumeration: it renders the whole spec layer (four figures of files), so the
//! properties asserted are that every composed crate contributes, that the
//! generation-twin template stamps ride along, and that the RM crate's model and
//! invariant cores are part of the same set.

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

/// The complete file set `emit-rest` produces, in emitted-map (sorted) order:
/// the shared `common` module, one module per API group whose OAS declares
/// operations, and the module declaration file.
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

/// The complete file set `emit-aom2` produces, in emitted-map (sorted) order:
/// the persistent form (`aom2`) and the AOM model form (`aom2_model`), each its
/// own closure into its own module.
const AOM2_FILES: &[&str] = &[
    "openehr-its/src/aom2/impls.rs",
    "openehr-its/src/aom2/mod.rs",
    "openehr-its/src/aom2/types.rs",
    "openehr-its/src/aom2_model/impls.rs",
    "openehr-its/src/aom2_model/mod.rs",
    "openehr-its/src/aom2_model/types.rs",
];

/// The complete file set `emit-rm-model` produces, in emitted-map (sorted)
/// order: one `model/` subtree per RM generation, because a selectable
/// generation is a complete peer and not a types-only shell.
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
    "openehr-rm/src/v1_2/validate/generated.rs",
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

/// `emit` produces one file set covering every composed crate, the RM crate's
/// model and invariant cores, and the generation-twin template stamps — each
/// file non-empty, banner-headed and SPDX-stamped.
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

/// Every API group gates exactly one server module and exactly one client module.
///
/// The two halves ride separate features (`rest-server`, `rest-client`), so a
/// second gate, or a half emitted outside its gate, would compile a half into a
/// consumer that did not ask for it.
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
///
/// The route table lists the group's operations as the OAS declares them, so
/// the outcome enums and the client methods are counted against it: an
/// operation the client half skipped would leave the counts apart.
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

/// Every operation of a group has one success-answer enum, one trait method,
/// one router binding and one handler in the server module.
///
/// Counted against the route table, like the client half: an operation the
/// router skipped would be unreachable over HTTP while its trait method still
/// compiles.
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

/// Every documented response is an outcome variant the client method matches.
///
/// Each variant carries a ``/// The `NNN` answer.`` doc line and each client
/// method one `http::StatusCode::` arm per variant, so the two counts agree
/// exactly when no documented status was dropped from either side. A status the
/// emitter's table does not name renders as `UnregisteredStatus`, which must
/// never reach the committed contract.
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

/// The client module reaches the hoisted shared types one module level up.
///
/// The hoisted schemas live in `generated::common`, a sibling of the group
/// module; the client module sits one level below the group, so every
/// reference it makes is `super::super::common::`, never `super::common::`.
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
///
/// The declaration is the one thing `emit-rm-model` does NOT render — it is a
/// read-modify-write of the generation `mod.rs`, which `emit` instead writes
/// into the rendered body. Both spell it from `cli`'s one `RM_MODEL_DECL`
/// constant; this asserts the outcome the two paths must agree on.
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

/// The rendered invariant cores equal the committed generated tree — which is
/// also what `emit` writes there, since both drive
/// `cli::render_validate_files`.
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

/// Assert every rendered file is non-empty, opens on a line-one banner carrying
/// `banner_needle`, and carries its crate's SPDX licence header.
///
/// `banner_needle` is the target name for every target that stamps it into the
/// banner; `emit`'s banner names only the generator, so it passes `@generated`.
fn assert_well_formed(target: &str, banner_needle: &str, files: &BTreeMap<String, String>) {
    assert!(!files.is_empty(), "{target}: rendered nothing");
    for (path, body) in files {
        assert!(!body.trim().is_empty(), "{target}: {path} rendered empty");
        let banner = body.lines().next().unwrap_or_default();
        assert!(
            banner.contains("@generated") && banner.contains(banner_needle),
            "{target}: {path} does not open with its generated banner: {banner:?}",
        );
        // The needle names the SPDX tag WITHOUT an expression, which the
        // REUSE scanner would otherwise parse and refuse as invalid.
        // REUSE-IgnoreStart
        assert!(
            body.contains("SPDX-License-Identifier: "),
            "{target}: {path} carries no SPDX licence header",
        );
        // REUSE-IgnoreEnd
    }
}

/// Assert a target's rendered bytes equal the committed generated tree.
///
/// The rendered body is put through `rustfmt` first, exactly as the `cmd_*`
/// handler puts the file it just wrote through it — `rustfmt` reading stdin
/// formats byte-identically to `rustfmt` reading that same text from a file, so
/// nothing has to be written to compare.
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

/// Assert `emit`'s whole rendered tree equals the committed generated tree.
///
/// Two things differ from the single-file comparison above. The set runs to four
/// figures, so the bodies are formatted in ONE batched `rustfmt` pass over a
/// scratch tree — the same `rustfmt --edition 2024 --quiet <files>` invocation
/// the `emit` handler makes over the files it has just written. And `emit`
/// finishes each crate by WEAVING module declarations into anchors it rendered
/// (`declare_hand_written_modules`; `emit-json` adds `mod json_serde;` the same
/// way), so an anchor's committed bytes are the rendered bytes plus that weave.
/// The assertion therefore pins three things: the rendered prefix byte for byte,
/// a tail that is nothing but woven declarations, and that only a module ANCHOR
/// (`lib.rs` / `mod.rs`) carries such a tail at all — the weave's own rule, so
/// the tolerance cannot widen into a blanket escape. The woven set is
/// non-empty, which is what keeps that half of the assertion honest.
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

/// Format a whole rendered tree in one `rustfmt` pass, via a scratch directory.
///
/// Per-file `rustfmt` over four figures of files would dominate the run, and
/// `rustfmt` is invoked exactly as the emitter invokes it — over files on disk,
/// batched per crate — so this is also the more faithful reproduction.
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

/// The workspace `crates/` directory every emitted path is relative to.
fn crates_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates")
}

/// A short label for a path, used only in `rustfmt` failure messages.
fn committed_relative_name(path: &str) -> String {
    format!("crates/{path}")
}

/// Format `text` with the same `rustfmt` invocation the emitter shells out to.
///
/// The body is written from a separate thread because a large file (the biggest
/// emitted impl set is megabytes) fills the pipe buffer long before `rustfmt`
/// finishes, and writing it all before reading stdout would deadlock.
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
///
/// The emitted files run to megabytes, so a whole-body `assert_eq!` would print
/// an unreadable pair; the first differing line is what actually identifies the
/// defect.
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
