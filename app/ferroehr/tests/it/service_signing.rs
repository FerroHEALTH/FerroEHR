// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end tests for version signing (`VERSION.signature`, RM common
//! §"Digital Signature")
//! against a real `PostgreSQL` 18 (shared testkit harness).
//!
//! The strongest assertion: the digest recomputes from the **served**
//! `ORIGINAL_VERSION`'s `canonical_form` — proving commit-time and read-time
//! object identity. Also: `EHR_STATUS` / FOLDER / contribution versions are all
//! signed; client-supplied signatures are stored verbatim; `verify_on_read =
//! strict` turns a tampered row into a 5xx; canonical XML carries the signature.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use ferroehr::service::FerroEhrService;
use ferroehr::service::status::{CallStatusType, SmError};
use ferroehr::versioning::signature::config::{Mode, SigningConfig, VerifyOnRead};
use ferroehr::versioning::signature::signer::Signer;
use ferroehr::versioning::signature::verify::Verdict;

use ferroehr::service::version_update::change_type_coded;
use openehr_base::prelude::ObjectVersionId;
use openehr_its::rest::generated::common::{UpdateAttestation, UpdateAudit, UpdateAuditData};
use openehr_rm::prelude::DvText;
use openehr_rm::v1_2::common::change_control::version_impl::canonical_form_of_json;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};

use crate::fixtures::{change_type, committer_proxy, composition, uid, uv};

/// A wire `UPDATE_ATTESTATION` partial for an attestation committed WITH the
/// version (`UPDATE_VERSION.attestations`; SM
/// `UML/classes/update_version.adoc` §Attributes) — the "Signing content at
/// committal" case of RM common master06 §Attestation.
fn update_attestation(reason: &str) -> UpdateAttestation {
    UpdateAttestation {
        _type: None,
        system_id: None,
        change_type: change_type_coded("666"),
        description: None,
        committer: committer_proxy("witness"),
        attested_view: None,
        proof: None,
        items: None,
        reason: openehr_its::json::from_canonical_value::<DvText>(
            &json!({ "_type": "DV_TEXT", "value": reason }),
        )
        .expect("reason DV_TEXT"),
        is_pending: false,
        additional_properties: std::collections::BTreeMap::new(),
    }
}

/// The contribution-level `UPDATE_AUDIT`.
fn contribution_audit(change_code: &str, committer_name: &str) -> UpdateAudit {
    UpdateAudit::UpdateAudit(UpdateAuditData {
        _type: None,
        system_id: None,
        change_type: change_type_coded(change_code),
        description: None,
        committer: committer_proxy(committer_name),
        additional_properties: std::collections::BTreeMap::new(),
    })
}

/// Split an `OBJECT_VERSION_ID` into `(object_id uuid, trunk version)`.
fn version_components(ovid: &str) -> (ferroehr::ids::VoId, String) {
    let parts: Vec<&str> = ovid.split("::").collect();
    (parts[0].parse().expect("vo uuid"), parts[2].to_owned())
}

/// Assert a served `ORIGINAL_VERSION` carries a server digest signature that
/// recomputes from its own `canonical_form` — the strongest test.
fn assert_digest_recomputes(ov: &Value) {
    assert_eq!(
        ov["_type"], "ORIGINAL_VERSION",
        "the served version must be an ORIGINAL_VERSION"
    );
    let signature = ov["signature"]
        .as_str()
        .expect("ORIGINAL_VERSION.signature");
    assert!(
        signature.starts_with("sha256:"),
        "expected a digest signature, got {signature}"
    );
    // canonical_form_of_json strips the signature key + JCS-canonicalises — the
    // exact bytes that were signed at commit. digest_default().verify recomputes.
    let canonical = canonical_form_of_json(ov).expect("canonical form");
    assert_eq!(
        Signer::digest_default().verify(&canonical, signature),
        Verdict::DigestMatch,
        "digest must recompute from the served version's canonical form"
    );
}

async fn create_ehr(svc: &FerroEhrService) -> String {
    svc.create_ehr(None).await.expect("create_ehr").to_string()
}

#[tokio::test]
async fn composition_version_is_signed_and_digest_recomputes_from_served_version() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    // Create a composition, then commit a second version.
    let ovid_v1 = svc
        .create_composition(ehr_uuid, uv(&composition("v1"), "249", None))
        .await
        .expect("create_composition")
        .version_uid();
    let vo_id = ovid_v1.split("::").next().unwrap().to_owned();
    let vo_uuid = vo_id.parse::<ferroehr::ids::VoId>().expect("vo uuid");
    let ovid_v2 = svc
        .update_composition(
            ehr_uuid,
            vo_uuid,
            uv(&composition("v2"), "251", Some(&ovid_v1)),
        )
        .await
        .expect("update_composition")
        .version_uid();

    for ovid in [&ovid_v1, &ovid_v2] {
        let ov = svc
            .composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
            .await
            .expect("versioned composition version");
        assert_digest_recomputes(&ov);
    }
}

#[tokio::test]
async fn ehr_status_versions_are_signed_and_every_version_carries_a_digest() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    // Update EHR_STATUS → v2, then read the ORIGINAL_VERSION of v1.
    let mut body = svc
        .get_ehr_status_at_time(ehr_uuid, None)
        .await
        .expect("status get");
    let status_ovid_v1 = uid(&body).to_owned();
    body.as_object_mut().expect("status obj").remove("uid");
    // Flip is_queryable (not is_modifiable): the test only needs a second
    // EHR_STATUS version, and with the B2 write guard a deactivated EHR
    // (is_modifiable = false, RM ehr master04 §"EHR Active Status") would
    // refuse the directory commit below.
    body["is_queryable"] = json!(false);
    svc.replace_ehr_status(ehr_uuid, uv(&body, "251", Some(&status_ovid_v1)))
        .await
        .expect("status update");

    let (status_vo, status_ver) = version_components(&status_ovid_v1);
    let ov = svc
        .ehr_status_version_envelope(ehr_uuid, status_vo, &status_ver)
        .await
        .expect("versioned ehr_status version");
    assert_eq!(ov["data"]["_type"], "EHR_STATUS");
    assert_digest_recomputes(&ov);

    // Also commit a directory (FOLDER) — its version is signed even though the
    // FOLDER endpoints serve the bare folder (no ORIGINAL_VERSION wrapper).
    let folder = json!({
        "_type": "FOLDER",
        "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
        "name": { "_type": "DV_TEXT", "value": "root" }
    });
    svc.create_directory(ehr_uuid, uv(&folder, "249", None))
        .await
        .expect("create_directory");

    // Sweep: EHR_STATUS (x2), EHR_ACCESS, FOLDER — every stored version is signed
    // with a digest (signing is on by default).
    let rows = sqlx::query("SELECT kind, signature FROM version ORDER BY kind, sys_version")
        .fetch_all(&pool)
        .await
        .expect("select version");
    assert!(!rows.is_empty());
    for row in &rows {
        let kind: String = row.try_get("kind").unwrap();
        let sig: Option<String> = row.try_get("signature").expect("signature");
        let sig = sig.unwrap_or_else(|| panic!("{kind} version is unsigned"));
        assert!(
            sig.starts_with("sha256:"),
            "{kind} version signature is not a digest: {sig}"
        );
    }
}

#[tokio::test]
async fn contribution_versions_are_signed() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let contribution_uid = svc
        .commit_contribution(
            ehr_uuid,
            vec![uv(&composition("Via contribution"), "249", None)],
            contribution_audit("249", "Dr. Contribution"),
        )
        .await
        .expect("commit_contribution");
    // NOTE: `commit_contribution` returns the contribution_uid;
    // the created version's OBJECT_VERSION_ID is read back from the CONTRIBUTION.
    let contribution = svc
        .get_contribution(ehr_uuid, contribution_uid.parse().expect("contrib uuid"))
        .await
        .expect("get_contribution");
    let ovid = contribution["versions"][0]["id"]["value"]
        .as_str()
        .unwrap()
        .to_owned();

    let ov = svc
        .composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await
        .expect("versioned composition version");
    assert_digest_recomputes(&ov);
}

#[tokio::test]
async fn client_supplied_signature_is_stored_verbatim() {
    // A CONTRIBUTION creation version carrying an author-generated signature —
    // stored verbatim, never re-signed (RM common master06 §Digital Signature).
    // The read runs under the STRICT default (FerroEhrService::new): a foreign
    // client signature is marked client-supplied at commit, so read-time
    // verification skips it (never recomputes a foreign canonical form) and the
    // strict read still succeeds — proving strict-by-default (#273) does not
    // reject legitimately-stored client signatures.
    const CLIENT_SIG: &str =
        "-----BEGIN PGP SIGNATURE-----\nauthored-elsewhere\n-----END PGP SIGNATURE-----";
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let mut version = uv(&composition("Client signed"), "249", None);
    version.signature = Some(CLIENT_SIG.to_owned());
    let contribution_uid = svc
        .commit_contribution(
            ehr_uuid,
            vec![version],
            contribution_audit("249", "Dr. Author"),
        )
        .await
        .expect("commit_contribution");
    let contribution = svc
        .get_contribution(ehr_uuid, contribution_uid.parse().expect("contrib uuid"))
        .await
        .expect("get_contribution");
    let ovid = contribution["versions"][0]["id"]["value"]
        .as_str()
        .unwrap()
        .to_owned();

    let ov = svc
        .composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await
        .expect("versioned composition version");
    assert_eq!(
        ov["signature"].as_str(),
        Some(CLIENT_SIG),
        "client-supplied signature must be stored verbatim"
    );
}

#[tokio::test]
async fn strict_verify_on_read_rejects_a_tampered_row() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    // A strict-verify service: a signature that does not match the served
    // canonical form is a 5xx integrity failure.
    let config = SigningConfig {
        enabled: true,
        mode: Mode::Digest,
        key_path: None,
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_paths: Vec::new(),
        verify_on_read: Some(VerifyOnRead::Strict),
    };
    let signer = Signer::from_config(&config).expect("strict signer");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_signer(Arc::new(signer));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let ovid = svc
        .create_composition(ehr_uuid, uv(&composition("tamper"), "249", None))
        .await
        .expect("create_composition")
        .version_uid();

    // A clean read verifies fine.
    svc.composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await
        .expect("clean read verifies");

    // Tamper the stored signature via SQL.
    sqlx::query("UPDATE version SET signature = 'sha256:dGFtcGVyZWQ=' WHERE kind = 'COMPOSITION'")
        .execute(&pool)
        .await
        .expect("tamper");

    let tampered = svc
        .composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await;
    // NOTE: a signing/integrity failure surfaces at the SM boundary
    // as `SmError { status: Exception }` (the adapter maps it to the same wire 5xx
    // the old `ApiError::Internal` produced).
    assert!(
        matches!(
            tampered,
            Err(SmError {
                status: CallStatusType::Exception,
                ..
            })
        ),
        "strict verify_on_read must 5xx a tampered signature, got {tampered:?}"
    );
}

#[tokio::test]
async fn default_verify_on_read_is_strict_and_rejects_a_tampered_row() {
    // #273: with signing enabled and verify_on_read UNSET, the effective default
    // is strict — the "sign but silently never check" default is gone. This uses
    // the plain service default (FerroEhrService::new → Signer::digest_default),
    // no explicit signing config, to prove the DEFAULT posture.
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let ovid = svc
        .create_composition(ehr_uuid, uv(&composition("tamper-default"), "249", None))
        .await
        .expect("create_composition")
        .version_uid();

    // A clean read verifies fine under the strict default.
    svc.composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await
        .expect("clean read verifies under the strict default");

    // Tamper the stored (server-generated) signature.
    sqlx::query("UPDATE version SET signature = 'sha256:dGFtcGVyZWQ=' WHERE kind = 'COMPOSITION'")
        .execute(&pool)
        .await
        .expect("tamper");

    let tampered = svc
        .composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await;
    assert!(
        matches!(
            tampered,
            Err(SmError {
                status: CallStatusType::Exception,
                ..
            })
        ),
        "the strict default must 5xx a tampered signature, got {tampered:?}"
    );
}

/// Build a digest-mode service with an explicit `verify_on_read` policy.
fn service_with_verify(pool: &PgPool, policy: VerifyOnRead) -> FerroEhrService {
    let config = SigningConfig {
        enabled: true,
        mode: Mode::Digest,
        key_path: None,
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_paths: Vec::new(),
        verify_on_read: Some(policy),
    };
    let signer = Signer::from_config(&config).expect("signer");
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_signer(Arc::new(signer))
}

#[tokio::test]
async fn warn_and_off_verify_on_read_serve_a_tampered_row() {
    // The other two poles of the policy (#273): `warn` logs + meters a mismatch
    // but still serves; explicit `off` never even checks. Neither is a 5xx —
    // together with the strict tests this proves the default is no longer the
    // silent-pass state, and the opt-outs remain reachable.
    for policy in [VerifyOnRead::Warn, VerifyOnRead::Off] {
        let db = testkit::db().await.expect("testkit database");
        let pool = db.pool();
        let svc = service_with_verify(&pool, policy);
        let ehr_id = create_ehr(&svc).await;
        let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

        let ovid = svc
            .create_composition(ehr_uuid, uv(&composition("tamper-nonstrict"), "249", None))
            .await
            .expect("create_composition")
            .version_uid();

        sqlx::query(
            "UPDATE version SET signature = 'sha256:dGFtcGVyZWQ=' WHERE kind = 'COMPOSITION'",
        )
        .execute(&pool)
        .await
        .expect("tamper");

        svc.composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
            .await
            .unwrap_or_else(|e| {
                panic!("verify_on_read = {policy:?} must serve, not reject: {e:?}")
            });
    }
}

#[tokio::test]
async fn canonical_xml_carries_the_signature() {
    // The generated canonical-XML serialization (the same `to_canonical_xml` the
    // REST negotiate path uses) emits the `signature` element.
    use openehr_rm::v1_2::common::change_control::original_version::OriginalVersion;
    use openehr_rm::v1_2::composition::composition::Composition;

    // A full corpus COMPOSITION (all mandatory fields) as the version data —
    // the VALID TWIN of the vendored SDK fixture, whose upstream half carries a
    // placeholder `OBJECT_VERSION_ID` the identifier grammar refuses
    // (`crates/openehr-its/tests/it/fixture_twins.rs` carries the adjudication
    // and pins both halves).
    const CORPUS: &str = include_str!(
        "../../../../crates/openehr-its/tests/fixtures/twins/minimal_persistent.valid.json"
    );
    let data: Composition =
        openehr_its::json::from_canonical_json(CORPUS).expect("typed composition");
    let ov: OriginalVersion<Composition> = openehr_its::json::from_canonical_value(&json!({
        "_type": "ORIGINAL_VERSION",
        "contribution": {
            "_type": "OBJECT_REF", "namespace": "local", "type": "CONTRIBUTION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000abc" }
        },
        "commit_audit": {
            "_type": "AUDIT_DETAILS",
            "system_id": "ferroehr.local",
            "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-07T10:11:12Z" },
            "change_type": change_type("249", "creation"),
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "FerroEHR" }
        },
        "uid": {
            "_type": "OBJECT_VERSION_ID",
            "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000001::ferroehr.local::1"
        },
        "lifecycle_state": change_type("532", "complete"),
        "signature": "sha256:jtWX/CULavvzX0ehjowv2XZPICTQhN1t0+AXHfbEaNc=",
        "data": openehr_its::json::to_canonical_value(&data)
    }))
    .expect("typed ORIGINAL_VERSION");

    let xml = openehr_its::xml::to_canonical_xml_declared(
        &ov,
        "version",
        "VERSION",
        openehr_its::xml::runtime::Namespace::V1,
    )
    .expect("to_canonical_xml_declared");
    assert!(
        xml.contains("sha256:jtWX/CULavvzX0ehjowv2XZPICTQhN1t0+AXHfbEaNc="),
        "canonical XML must carry the signature; got:\n{xml}"
    );
    assert!(xml.contains("signature"), "expected a <signature> element");
}

/// A service with server-side signing DISABLED — the common high-throughput
/// config. With signing off the `audit → sign → version`
/// dependency vanishes, so the commit path folds `audit`, `contribution` and
/// `version` into one statement; this test proves the folded path preserves
/// the RM common master06 versioning semantics byte-for-byte and stores no
/// signature.
fn signing_disabled(pool: &PgPool) -> FerroEhrService {
    let config = SigningConfig {
        enabled: false,
        mode: Mode::Digest,
        key_path: None,
        key_passphrase: None,
        key_passphrase_file: None,
        retired_key_paths: Vec::new(),
        verify_on_read: Some(VerifyOnRead::Off),
    };
    let signer = Signer::from_config(&config).expect("disabled signer");
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_signer(Arc::new(signer))
}

/// The two stored versions of one container after a supersession: exactly one
/// is the trunk head, and neither is signed. The append-only write advances the
/// head past its predecessor rather than closing a row (RM common master06 §The
/// 'Virtual Version Tree').
async fn assert_two_versions_one_head(pool: &PgPool, vo_uuid: ferroehr::ids::VoId) {
    let rows = sqlx::query(
        "SELECT v.sys_version, v.signature, \
                (h.trunk_head_sys_version = v.sys_version) AS open \
         FROM version v JOIN vo_head h ON h.vo_id = v.vo_id \
         WHERE v.vo_id = $1 AND v.kind = 'COMPOSITION' ORDER BY v.sys_version",
    )
    .bind(vo_uuid)
    .fetch_all(pool)
    .await
    .expect("select version");
    assert_eq!(rows.len(), 2, "two composition versions stored");
    let open: Vec<bool> = rows
        .iter()
        .map(|r| r.try_get("open").expect("open flag"))
        .collect();
    assert_eq!(
        open,
        vec![false, true],
        "v1 superseded, v2 current (master06 §The 'Virtual Version Tree')"
    );
    for row in &rows {
        let sig: Option<String> = row.try_get("signature").expect("signature");
        assert!(sig.is_none(), "signing off → version.signature is NULL");
    }
}

#[tokio::test]
async fn signing_disabled_folds_commit_and_preserves_master06_semantics() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = signing_disabled(&pool);
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    // CREATE → the folded path: audit + contribution + version in one CTE.
    let ovid_v1 = svc
        .create_composition(ehr_uuid, uv(&composition("v1"), "249", None))
        .await
        .expect("create_composition")
        .version_uid();
    let (vo_uuid, v1) = version_components(&ovid_v1);
    // master06 §The 'Virtual Version Tree'
    assert_eq!(v1, "1", "first version is trunk 1");

    // The served ORIGINAL_VERSION round-trips: uid stable, no signature, and the
    // server-computed commit instant present (master06 §Committal m3).
    let ov1 = svc
        .composition_version_envelope(ehr_uuid, ovid_v1.parse().expect("ovid"))
        .await
        .expect("v1 original version");
    assert_eq!(ov1["uid"]["value"], ovid_v1);
    assert!(
        ov1.get("signature").and_then(Value::as_str).is_none(),
        "signing off → no VERSION.signature on the served version"
    );
    assert!(
        ov1["commit_audit"]["time_committed"]["value"]
            .as_str()
            .is_some(),
        "commit_audit.time_committed is the server-computed instant"
    );

    // UPDATE → the folded path: the version row and the head upsert in one CTE.
    let ovid_v2 = svc
        .update_composition(
            ehr_uuid,
            vo_uuid,
            uv(&composition("v2"), "251", Some(&ovid_v1)),
        )
        .await
        .expect("update_composition")
        .version_uid();
    let (_, v2) = version_components(&ovid_v2);
    assert_eq!(v2, "2", "second trunk version");
    let latest = svc
        .get_composition_latest(ehr_uuid, vo_uuid)
        .await
        .expect("latest");
    assert_eq!(uid(&latest), ovid_v2, "current version is v2");

    assert_two_versions_one_head(&pool, vo_uuid).await;

    // DELETE → folded path (523|deleted|, no node rows); the current version
    // then resolves to an empty body (204), never 404.
    let ovid_v2_id: ObjectVersionId = ovid_v2.parse().expect("ovid");
    svc.delete_composition(ehr_uuid, &ovid_v2_id, None, None)
        .await
        .expect("delete_composition");
    let deleted = svc
        .get_composition_latest(ehr_uuid, vo_uuid)
        .await
        .expect("deleted get");
    assert_eq!(
        deleted,
        Value::Null,
        "a deleted current version reads empty"
    );

    // A multi-change CONTRIBUTION also folds each change (commit_version_into).
    let contribution_uid = svc
        .commit_contribution(
            ehr_uuid,
            vec![uv(&composition("Via contribution"), "249", None)],
            contribution_audit("249", "Dr. Contribution"),
        )
        .await
        .expect("commit_contribution");
    let contribution = svc
        .get_contribution(ehr_uuid, contribution_uid.parse().expect("contrib uuid"))
        .await
        .expect("get_contribution");
    let c_ovid = contribution["versions"][0]["id"]["value"]
        .as_str()
        .expect("contribution version uid")
        .to_owned();
    let c_ov = svc
        .composition_version_envelope(ehr_uuid, c_ovid.parse().expect("ovid"))
        .await
        .expect("contribution version");
    assert!(
        c_ov.get("signature").and_then(Value::as_str).is_none(),
        "signing off → contribution version is unsigned"
    );
}

#[tokio::test]
async fn creating_system_id_and_signature_survive_a_system_id_change() {
    // M2 (RM common master06 §"Distributed Versioning"): the OBJECT_VERSION_ID's
    // creating_system_id is per-version immutable and reconstructed from
    // storage, never from the live service config. A later `with_system_id`
    // change must not mutate a historical version's uid nor invalidate the
    // signature that was computed over it.
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    // Commit a composition under system id "sys-origin".
    let svc_a = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_system_id("sys-origin");
    let ehr_id = create_ehr(&svc_a).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));
    let ovid = svc_a
        .create_composition(ehr_uuid, uv(&composition("v1"), "249", None))
        .await
        .expect("create_composition")
        .version_uid();
    let vo_id = ovid.split("::").next().unwrap().to_owned();
    let vo_uuid = vo_id.parse::<ferroehr::ids::VoId>().expect("vo uuid");
    // The uid's middle part is the creating system id.
    assert_eq!(
        ovid.split("::").nth(1),
        Some("sys-origin"),
        "uid carries the committing system id"
    );

    // A second service over the SAME pool with a DIFFERENT system id.
    let svc_b = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_system_id("sys-changed");

    // Reading the composition back: the injected uid still carries "sys-origin"
    // (the stored creating_system_id), not the new config value.
    let read = svc_b
        .get_composition_latest(ehr_uuid, vo_uuid)
        .await
        .expect("composition_get");
    assert_eq!(
        uid(&read),
        ovid,
        "uid must be stable across a system-id change"
    );

    // The served ORIGINAL_VERSION still verifies — the signature was computed
    // over the stored creating_system_id, which the read path reconstructs.
    let ov = svc_b
        .composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await
        .expect("versioned composition version");
    assert_eq!(ov["uid"]["value"], ovid);
    assert_digest_recomputes(&ov);
}

/// RM common master06 §Digital Signature: "The serialisation process works by
/// the simple rule of serialising the entire Version object (note that the
/// signature attribute will be Void at this point)". `signature` is the ONLY
/// excluded attribute — so the `ORIGINAL_VERSION.attestations` a version
/// carried at committal (`UPDATE_VERSION.attestations`, SM
/// `UML/classes/update_version.adoc` §Attributes; master06 §Attestation,
/// "Signing content at committal") are INSIDE the signed canonical form.
///
/// Asserted both ways: the served version still recomputes its own digest with
/// the attestation present (so commit-time and read-time bytes agree), and
/// altering that attestation in the database is caught by a strict
/// `verify_on_read`.
#[tokio::test]
async fn at_committal_attestation_is_signed_and_a_tamper_is_caught() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = service_with_verify(&pool, VerifyOnRead::Strict);
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let mut version = uv(&composition("attested"), "249", None);
    version.attestations = Some(vec![update_attestation("witnessed")]);
    let ovid = svc
        .create_composition(ehr_uuid, version)
        .await
        .expect("create_composition with an accompanying attestation")
        .version_uid();

    // The served version carries the completed ATTESTATION and still verifies
    // against its own canonical form — the attestation is part of the signed
    // bytes, not an addendum the digest ignores.
    let ov = svc
        .composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await
        .expect("clean read verifies");
    assert_eq!(
        ov["attestations"].as_array().map(Vec::len),
        Some(1),
        "the at-committal attestation is served on the version"
    );
    assert_digest_recomputes(&ov);

    // Tamper the stored attestation — not the version row, not the signature.
    let tampered_rows = sqlx::query(
        "UPDATE vo_attestation SET data = jsonb_set(data, '{reason,value}', '\"forged\"') \
         WHERE at_committal RETURNING id",
    )
    .fetch_all(&pool)
    .await
    .expect("tamper the at-committal attestation");
    assert_eq!(
        tampered_rows.len(),
        1,
        "exactly one at-committal attestation row exists to tamper"
    );

    let tampered = svc
        .composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await;
    assert!(
        matches!(
            tampered,
            Err(SmError {
                status: CallStatusType::Exception,
                ..
            })
        ),
        "an altered at-committal attestation must fail strict verify_on_read, got {tampered:?}"
    );
}

/// The other side of the same rule. RM common master06 §Attestation:
/// "Attestations can be added at any time after committal of the content being
/// attested", and §Contributions makes the `666|attestation|` change "a new
/// `ATTESTATION` … added to the attestations list of an EXISTING
/// `ORIGINAL_VERSION`". Such an attestation necessarily post-dates the
/// signature computed at committal, so it is served OUTSIDE the signed
/// canonical form: adding one leaves verification passing, and altering one
/// cannot make verification fail.
#[tokio::test]
async fn after_committal_attestation_is_outside_the_signed_form() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = service_with_verify(&pool, VerifyOnRead::Strict);
    let ehr_id = create_ehr(&svc).await;
    let ehr_uuid = ferroehr::ids::EhrId(ehr_id.parse::<uuid::Uuid>().expect("ehr uuid"));

    let ovid = svc
        .create_composition(ehr_uuid, uv(&composition("plain"), "249", None))
        .await
        .expect("create_composition")
        .version_uid();

    // A later 666|attestation|-only CONTRIBUTION attaches an ATTESTATION to the
    // existing version — no new version is committed.
    svc.create_ehr_contribution(
        ehr_uuid,
        json!({
            "audit": { "change_type": { "_type": "DV_CODED_TEXT", "value": "modification", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" }, "code_string": "251" } }, "committer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" } }, "versions": [{
                "preceding_version_uid": { "value": ovid.clone() },
                "commit_audit": {
                    "change_type": change_type("666", "attestation"),
                    "committer": { "_type": "PARTY_IDENTIFIED", "name": "senior reviewer" },
                    "reason": { "_type": "DV_TEXT", "value": "authorised" },
                    "is_pending": false
                }
            }]
        }),
    )
    .await
    .expect("666-only contribution");

    let ov = svc
        .composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await
        .expect("a post-committal attestation does not invalidate the signature");
    assert_eq!(
        ov["attestations"].as_array().map(Vec::len),
        Some(1),
        "the after-committal attestation is served on the version"
    );

    // Altering it is likewise outside the signed form: the read still succeeds.
    let tampered_rows = sqlx::query(
        "UPDATE vo_attestation SET data = jsonb_set(data, '{reason,value}', '\"forged\"') \
         WHERE NOT at_committal RETURNING id",
    )
    .fetch_all(&pool)
    .await
    .expect("tamper the after-committal attestation");
    assert_eq!(tampered_rows.len(), 1, "one after-committal row to tamper");

    svc.composition_version_envelope(ehr_uuid, ovid.parse().expect("ovid"))
        .await
        .expect("an after-committal attestation is not covered by the signature");
}
