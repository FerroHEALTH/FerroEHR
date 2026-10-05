// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! **Fixture twins** for the defective documents the strict canonical-JSON
//! reader refuses — the two in the vendored `openehr_sdk` corpus, plus the
//! repo-authored pair for the repeated-member refusal.
//!
//! Both were latent negatives: they only ever read because the canonical-JSON
//! reader was tolerant. Under the strict reader (an undeclared wire key is a
//! refusal; an identifier is built through the BASE grammar) each one is
//! refused — correctly. The owner's twins rule then applies: a spec-correct
//! refusal keeps BOTH halves, so this module pins
//!
//! * the **invalid twin** — the vendored file, kept byte-verbatim (a vendored
//!   corpus is never hand-edited; it is external ground truth), with an
//!   asserted refusal naming the path and the offending member, so a reader
//!   that silently loosened would fail here; and
//! * the **valid twin** — a repo-authored correction under
//!   `tests/fixtures/twins/`, which must read AND round-trip, so the defect is
//!   proven to be the fixture's rather than the model's.
//!
//! The vendored halves are listed in `common::excluded` (the corpus gates'
//! single documented exclusion list) so the exclusion is auditable rather than
//! silent.

use openehr_its::json::from_canonical_json;
use openehr_rm::prelude::{Composition, DvQuantity};
use std::path::{Path, PathBuf};

/// The vendored corpus file `rel` (relative to `tests/vendor/`).
fn vendored(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vendor")
        .join(rel)
}

/// The repo-authored valid twin `name` (under `tests/fixtures/twins/`).
fn twin(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/twins")
        .join(name)
}

/// A half of the repeated-member pair (under `tests/fixtures/repeated_member/`).
///
/// Deliberately NOT under `tests/fixtures/twins/`: that directory has a
/// documented contract (`common::corpus_files` walks it as corpus — it holds
/// the repo-authored VALID half of an adjudicated defective VENDORED document,
/// and every member must be a canonical RM root). This pair is repo-authored on
/// BOTH halves and is a `DV_QUANTITY` fragment, so it lives beside the corpus
/// rather than inside it, and is asserted only here.
fn repeated(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/repeated_member")
        .join(name)
}

/// Read a fixture, or fail loudly naming it.
///
/// Not inside a `#[test]` fn, so the `clippy.toml` test scoping does not reach
/// it; the escape is scoped here with its reason instead.
#[expect(
    clippy::expect_used,
    reason = "both halves of every twin are committed in-tree beside this test \
              (tests/vendor/ and tests/fixtures/twins/), so a read failure means \
              the fixture was deleted — which must fail the suite loudly, not \
              silently pass"
)]
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("a committed twin fixture is readable")
}

/// **Invalid twin.** Two ENTRY nodes of the vendored feeder-audit document
/// hoist `feeder_system_audit` onto the ENTRY itself. That attribute belongs to
/// `FEEDER_AUDIT`
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.feeder_audit.adoc`
/// §Attributes: `feeder_system_audit: FEEDER_AUDIT_DETAILS`), which reaches a
/// LOCATABLE only through `LOCATABLE.feeder_audit`
/// (`…org.openehr.rm.common.locatable.adoc` §Attributes) — and three sibling
/// nodes in the SAME document nest it correctly, so the document contradicts
/// itself. The strict reader refuses it, naming the path and the member.
#[test]
fn hoisted_feeder_system_audit_is_refused_with_its_path() {
    let src = read(&vendored(
        "openehr_sdk/composition/canonical_json/all_types_systematic_tests_feeder_audit.json",
    ));
    let err = from_canonical_json::<Composition>(&src)
        .expect_err("the hoisted feeder_system_audit must be refused");
    let text = err.to_string();
    assert!(
        text.contains("unknown field `feeder_system_audit`"),
        "the refusal must name the offending member, got: {text}"
    );
    assert!(
        text.contains("INSTRUCTION"),
        "the refusal must name the class that does not declare it, got: {text}"
    );
    assert!(
        text.contains("$.content[2].items[0].items[0].items[0]"),
        "the refusal must name the path to the offending node, got: {text}"
    );
}

/// **Valid twin.** The same document with the two hoisted members wrapped in
/// the `feeder_audit` / `FEEDER_AUDIT` shape its three correct siblings already
/// use — it reads, and the model round-trips it, proving the defect was the
/// fixture's and not the generated model's.
#[test]
fn corrected_feeder_audit_twin_reads_and_round_trips() {
    let src = read(&twin("all_types_systematic_tests_feeder_audit.valid.json"));
    let composition = from_canonical_json::<Composition>(&src)
        .expect("the corrected twin must read under the strict reader");
    let re_encoded = openehr_its::json::to_canonical_json(&composition);
    let again = from_canonical_json::<Composition>(&re_encoded)
        .expect("the re-encoded twin must read back");
    assert_eq!(
        composition, again,
        "the corrected twin must round-trip through the canonical codec"
    );
}

/// **Invalid twin.** The vendored `alternative_types` document carries a
/// PLACEHOLDER version identifier,
/// `__THIS_SHOULD_BE_MODIFIED_BY_THE_TEST_::ehrbase.org::1`, whose `object_id`
/// part matches none of the three `uid` productions of BASE
/// `docs/specs/openehr/BASE/docs/base_types/master05-identification_package.adoc`
/// §Syntaxes (`uid = iso_oid | uuid | internet_id`; an `internet_id` label must
/// begin with a letter). Identifier construction runs that grammar, so the
/// document is refused at parse rather than entering the model.
#[test]
fn placeholder_object_version_id_is_refused() {
    let src = read(&vendored(
        "openehr_sdk/composition/canonical_json/alternative_types.json",
    ));
    let err = from_canonical_json::<Composition>(&src)
        .expect_err("the placeholder OBJECT_VERSION_ID must be refused");
    let text = err.to_string();
    assert!(
        text.contains("OBJECT_VERSION_ID"),
        "the refusal must name the identifier class, got: {text}"
    );
    assert!(
        text.contains("__THIS_SHOULD_BE_MODIFIED_BY_THE_TEST_"),
        "the refusal must echo the rejected component, got: {text}"
    );
}

/// **Valid twin.** The same document with a real UUID `object_id`, which the
/// §Syntaxes `uuid` production admits — it reads and round-trips.
#[test]
fn corrected_alternative_types_twin_reads_and_round_trips() {
    let src = read(&twin("alternative_types.valid.json"));
    let composition = from_canonical_json::<Composition>(&src)
        .expect("the corrected twin must read under the strict reader");
    let re_encoded = openehr_its::json::to_canonical_json(&composition);
    let again = from_canonical_json::<Composition>(&re_encoded)
        .expect("the re-encoded twin must read back");
    assert_eq!(
        composition, again,
        "the corrected twin must round-trip through the canonical codec"
    );
}

/// **Invalid half.** A repeated object member.
///
/// RFC 8259 §4 says the names within an object "SHOULD be unique" and that
/// "when the names within an object are not unique, the behavior of software
/// that receives such an object is unpredictable"
/// (<https://www.rfc-editor.org/rfc/rfc8259#section-4>). No canonical openEHR
/// writer emits one — every attribute is written once, in BMM declaration order
/// (`docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
/// §JSON Format) — so a repeated member is not a shape any conformant producer
/// can have meant. The reader refuses it rather than silently letting one
/// occurrence win, which is the never-lax reading of an undefined behaviour.
///
/// Unlike the two vendored twins above, BOTH halves of this pair are
/// repo-authored (`tests/fixtures/repeated_member/`): the refusal is ours to
/// introduce, so there is no external document to keep verbatim.
#[test]
fn a_repeated_member_is_refused() {
    let src = read(&repeated("duplicate_member.invalid.json"));
    let err = from_canonical_json::<DvQuantity>(&src)
        .expect_err("a repeated object member must be refused");
    let text = err.to_string();
    assert!(
        text.contains("duplicate field `magnitude`"),
        "the refusal must name the repeated member, got: {text}"
    );
}

/// **Invalid half.** A repeated `_type` discriminator — the same refusal, on
/// the one member that is not a modelled attribute (see
/// [`a_repeated_member_is_refused`] for the grounding).
#[test]
fn a_repeated_type_discriminator_is_refused() {
    let src = read(&repeated("duplicate_type_discriminator.invalid.json"));
    let err = from_canonical_json::<DvQuantity>(&src)
        .expect_err("a repeated `_type` discriminator must be refused");
    let text = err.to_string();
    assert!(
        text.contains("duplicate field `_type`"),
        "the refusal must name the repeated discriminator, got: {text}"
    );
}

/// **Valid half.** The same document with the member written once — it reads
/// and round-trips, proving the refusal is about the repetition and nothing
/// else.
#[test]
fn the_single_member_twin_reads_and_round_trips() {
    let src = read(&repeated("duplicate_member.valid.json"));
    let quantity = from_canonical_json::<DvQuantity>(&src).expect("the corrected twin must read");
    let re_encoded = openehr_its::json::to_canonical_json(&quantity);
    let again =
        from_canonical_json::<DvQuantity>(&re_encoded).expect("the re-encoded twin must read back");
    assert_eq!(
        quantity, again,
        "the corrected twin must round-trip through the canonical codec"
    );
}
