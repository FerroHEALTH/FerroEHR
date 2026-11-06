// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party binding configuration through `ferroehr::config::assemble`: a
//! full configuration of every issuer kind loads, and each boot refusal of
//! `[auth]`, `[authz.party_binding]`, `[[authz.system_client]]` and
//! `[[authz.federation.agreement]]` names its key. No openEHR spec governs
//! configuration: our own design.

use std::collections::HashMap;

use assert_fs::prelude::FileWriteStr as _;
use ferroehr::config::FerroEhrConfig;
use ferroehr::config::binding::{BindingPosture, Providers};

/// An Argon2id PHC string at the OWASP floor.
const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaGhhc2hoYQ";

/// A configuration with one issuer of every kind, a bound persons issuer,
/// a mapped Basic user, a system client and two federation agreements.
fn full() -> String {
    format!(
        r#"
[ehr_index]
provider = "local"

[[auth.issuer]]
issuer = "https://idp.example/realms/staff"
kind = "persons"
audience = ["ferroehr"]
first_party_clients = ["ferroehr-viewer"]

[[auth.issuer]]
issuer = "https://idp.example/realms/patients"
kind = "patients"
audience = ["ferroehr-patients"]

[[auth.issuer]]
issuer = "https://idp.example/realms/machines"
kind = "machines"
audience = ["ferroehr-machines"]

[[auth.issuer]]
issuer = "https://federation.example"
kind = "federation"
audience = ["ferroehr-federation"]

[[auth.federation]]
issuer = "https://federation.example"
mapping = "iua"
contact_point = false
countries = ["DE"]
issuing_authorities = ["BAEK"]
provider_identifier_prefixes = ["urn:example:de:"]

[auth.federation.fields]
"professional.country_code" = [{{ constant = "DE" }}]
"professional.issuing_authority_name" = [{{ constant = "BAEK" }}]

[[auth.party]]
issuer = "https://idp.example/realms/staff"
identifier_issuer = "urn:example:staff-login"
identifier_type = "guid"

[[auth.basic.users]]
user = "alice"
password_hash = "{HASH}"
scopes = ["user/composition-*.cruds"]

[authz.rbac]
role_issuers = ["https://idp.example/realms/staff"]
import_role = "IMPORTER"

[authz.party_binding]
provider = {{ issuer = "urn:oid:2.16.528.1.1007.3.3", id = "12345678" }}

[[authz.party_binding.basic_user]]
user = "alice"
key_issuer = "urn:example:staff-login"
key = "3f2b0c1e-0000-4000-8000-000000000001"

[authz.import]
trusted_systems = ["legacy.example"]

[[authz.system_client]]
issuer = "https://idp.example/realms/machines"
client_id = "ferrobridge"
provider = {{ issuer = "urn:oid:2.16.528.1.1007.3.3", id = "12345678" }}
facilities = [{{ issuer = "urn:example:facility", id_prefix = "ward-" }}]
origins = ["relay"]

[[authz.federation.agreement]]
issuer = "https://federation.example"
accessor = "person"
jurisdiction = "DE"
reads = ["patient/composition-*.r"]
caller_rate = {{ requests_per_second = 5, burst = 10 }}

[[authz.federation.agreement]]
issuer = "https://federation.example"
accessor = "organisation"
name = "Summary exchange"
version = "1.0"
jurisdiction = "DE"
legal_ground = {{ gdpr_6_1 = "e", gdpr_9_2 = "h", national = "a national provision" }}
reads = ["patient/composition-*.r"]
caller_rate = {{ requests_per_second = 5, burst = 10 }}
access_class = "automated"
"#
    )
}

/// The configuration `toml` assembles to, or the assemble refusal.
fn assemble(toml: &str) -> Result<FerroEhrConfig, String> {
    let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
    file.write_str(toml).expect("write");
    ferroehr::config::assemble(Some(file.path()), &HashMap::new(), &[]).map_err(|e| e.to_string())
}

/// The refusal of `toml`, at assemble or at validation.
fn refusal(toml: &str) -> String {
    match assemble(toml) {
        Err(refused) => refused,
        Ok(config) => config
            .validate()
            .expect_err("the configuration is refused")
            .to_string(),
    }
}

/// [`full`] with `old` replaced by `new`.
fn with(old: &str, new: &str) -> String {
    let base = full();
    assert!(base.contains(old), "the full configuration holds {old:?}");
    base.replacen(old, new, 1)
}

#[test]
fn a_full_configuration_loads_and_states_its_binding() {
    let config = assemble(&full()).expect("assembles");
    config.validate().expect("validates");
    let posture = BindingPosture::of(&config);
    assert_eq!(posture.issuers.len(), 4);
    let staff = &posture.issuers[0];
    assert_eq!(
        staff.identifier_issuer.as_deref(),
        Some("urn:example:staff-login")
    );
    assert!(matches!(posture.provider, Some(Providers::One(_))));
    assert_eq!(posture.namespace, "demographic");
    assert_eq!(posture.agreements.len(), 2);
    assert_eq!(
        posture.agreements[1].legal_ground.as_deref(),
        Some("GDPR Art. 6(1)(e), Art. 9(2)(h), a national provision")
    );
    let info = serde_json::to_value(ferroehr::telemetry::build_info::BuildInfo::for_config(
        &config,
    ))
    .expect("renders");
    assert_eq!(info["binding"]["issuers"][0]["kind"], "persons", "{info}");
    assert_eq!(info["binding"]["agreements"][1]["accessor"], "organisation");
}

#[test]
fn a_persons_issuer_with_no_party_table_is_refused() {
    let text = refusal(&with(
        "[[auth.party]]\nissuer = \"https://idp.example/realms/staff\"\nidentifier_issuer = \"urn:example:staff-login\"\nidentifier_type = \"guid\"\n",
        "",
    ));
    assert!(
        text.contains(
            "the persons issuer \"https://idp.example/realms/staff\" has no [[auth.party]] table"
        ),
        "{text}"
    );
}

#[test]
fn a_basic_user_with_no_mapping_is_refused_naming_the_user() {
    let text = refusal(&with(
        "[[authz.party_binding.basic_user]]\nuser = \"alice\"",
        "[[authz.party_binding.basic_user]]\nuser = \"bob\"",
    ));
    assert!(
        text.contains("the Basic user \"alice\" maps to no party key"),
        "{text}"
    );
    assert!(text.contains("maps \"bob\", which no"), "{text}");
    let text = refusal(&with(
        "key_issuer = \"urn:example:staff-login\"",
        "key_issuer = \"urn:example:other\"",
    ));
    assert!(
        text.contains(
            "the Basic user \"alice\" is mapped twice or to an identifier issuer no table names"
        ),
        "{text}"
    );
}

#[test]
fn a_table_for_an_issuer_of_the_wrong_kind_or_twice_is_refused() {
    let text = refusal(&with(
        "issuer = \"https://idp.example/realms/machines\"\nclient_id",
        "issuer = \"https://federation.example\"\nclient_id",
    ));
    assert!(
        text.contains(
            "names a federation issuer: a system client's issuer is of kind machines or persons"
        ),
        "{text}"
    );
    let text = refusal(&with(
        "issuer = \"https://federation.example\"\naccessor = \"person\"",
        "issuer = \"https://idp.example/realms/staff\"\naccessor = \"person\"",
    ));
    assert!(
        text.contains("names a persons issuer: an agreement's issuer is of kind federation"),
        "{text}"
    );
    let doubled = with("accessor = \"organisation\"", "accessor = \"person\"");
    assert!(refusal(&doubled).contains("is declared twice"), "{doubled}");
    let party = "[[auth.party]]\nissuer = \"https://idp.example/realms/staff\"\nidentifier_issuer = \"urn:example:staff-login\"\nidentifier_type = \"guid\"\n";
    let text = refusal(&with(
        party,
        &format!(
            "{party}\n[[auth.party]]\nissuer = \"https://idp.example/realms/staff\"\nidentifier_issuer = \"urn:example:b\"\nidentifier_type = \"guid\"\n"
        ),
    ));
    assert!(
        text.contains("two [[auth.party]] tables name the issuer"),
        "{text}"
    );
}

#[test]
fn two_issuers_sharing_an_audience_are_refused() {
    let text = refusal(&with(
        "audience = [\"ferroehr-patients\"]",
        "audience = [\"ferroehr\"]",
    ));
    assert!(text.contains("share the audience \"ferroehr\""), "{text}");
}

#[test]
fn a_removed_key_is_refused_naming_the_new_shape() {
    let text = refusal(&with(
        "[authz.party_binding]\n",
        "[authz.party_binding]\nenabled = true\n",
    ));
    assert!(
        text.contains("authz.party_binding.enabled is removed"),
        "{text}"
    );
    let text = refusal("[auth]\nissuer = \"https://idp.example\"\n");
    assert!(text.contains("issuer"), "{text}");
    assert!(text.contains("[[auth.issuer]]"), "{text}");
    for key in [
        "acting_for_claim = \"act.sub\"",
        "client_tokens = \"claim_absent\"",
    ] {
        let text = refusal(&with(
            "first_party_clients = [\"ferroehr-viewer\"]",
            &format!("first_party_clients = [\"ferroehr-viewer\"]\n{key}"),
        ));
        assert!(text.contains("is removed"), "{key}: {text}");
    }
    let text = refusal(&format!(
        "{}\n[authz.professional]\nclaim = \"sub\"\n",
        full()
    ));
    assert!(text.contains("authz.professional is removed"), "{text}");
}

#[test]
fn a_role_issuer_of_another_kind_is_refused() {
    for issuer in [
        "https://federation.example",
        "https://idp.example/realms/patients",
        "https://idp.example/realms/machines",
    ] {
        let text = refusal(&with(
            "role_issuers = [\"https://idp.example/realms/staff\"]",
            &format!("role_issuers = [\"{issuer}\"]"),
        ));
        assert!(
            text.contains(&format!(
                "the authz.rbac.role_issuers list names the issuer {issuer:?}, which is not a trusted persons issuer"
            )),
            "{text}"
        );
    }
}

#[test]
fn a_missing_provider_or_both_forms_are_refused() {
    let text = refusal(&with(
        "provider = { issuer = \"urn:oid:2.16.528.1.1007.3.3\", id = \"12345678\" }\n\n[[authz.party_binding.basic_user]]",
        "\n[[authz.party_binding.basic_user]]",
    ));
    assert!(
        text.contains("authz.party_binding.provider is required"),
        "{text}"
    );
    let both = format!(
        "{}\n[[authz.party_binding.provider]]\ntenant = \"t\"\nissuer = \"urn:x\"\nid = \"1\"\n",
        full()
    );
    assert!(assemble(&both).is_err(), "both provider forms load");
    let tenants = with(
        "provider = { issuer = \"urn:oid:2.16.528.1.1007.3.3\", id = \"12345678\" }",
        "provider = [{ tenant = \"t\", issuer = \"urn:x\", id = \"1\" }]",
    );
    assert!(
        refusal(&tenants).contains("names no tenant_claim"),
        "a bound issuer with tenants no table covers"
    );
}

#[test]
fn a_party_claim_and_a_token_profile_are_refused() {
    let text = refusal(&with(
        "identifier_type = \"guid\"",
        "identifier_type = \"guid\"\nclaim = \"oid\"",
    ));
    assert!(text.contains("auth.party.claim"), "{text}");
    assert!(text.contains("is removed"), "{text}");
    let text = refusal(&with(
        "first_party_clients = [\"ferroehr-viewer\"]",
        "first_party_clients = [\"ferroehr-viewer\"]\ntoken_profile = \"jwt\"",
    ));
    assert!(text.contains("token_profile is removed"), "{text}");
}

#[test]
fn a_device_client_without_identifier_issuers_is_refused() {
    let text = refusal(&with(
        "origins = [\"relay\"]",
        "origins = [\"relay\", \"device\"]",
    ));
    assert!(
        text.contains("has origins containing device and needs device_identifier_issuers"),
        "{text}"
    );
}

#[test]
fn every_organisation_agreement_refusal_is_named() {
    let organisation = "accessor = \"organisation\"";
    for (old, new, expected) in [
        ("name = \"Summary exchange\"\n", "", "needs name"),
        ("version = \"1.0\"\n", "", "needs version"),
        (
            "legal_ground = { gdpr_6_1 = \"e\", gdpr_9_2 = \"h\", national = \"a national provision\" }\n",
            "",
            "needs legal_ground",
        ),
        ("access_class = \"automated\"", "", "needs access_class"),
        (
            "access_class = \"automated\"",
            "access_class = \"automated\"\nwrites = [\"Vital signs.v1\"]",
            "an organisation access is read-only",
        ),
        (
            "access_class = \"automated\"",
            "access_class = \"automated\"\nopt_out_override = true",
            "sets opt_out_override",
        ),
        (
            "access_class = \"automated\"",
            "access_class = \"automated\"\nconsent = \"mitz\"",
            "takes consent from mitz",
        ),
        (
            "access_class = \"automated\"",
            "access_class = \"chained\"",
            "is chained and needs chained_log_retention_years",
        ),
    ] {
        let base = full();
        let start = base.find(organisation).expect("the organisation agreement");
        let (head, tail) = base.split_at(start);
        assert!(tail.contains(old), "{old:?}");
        let text = refusal(&format!("{head}{}", tail.replacen(old, new, 1)));
        assert!(text.contains(expected), "{expected}: {text}");
    }
    let chained = with(
        "access_class = \"automated\"",
        "access_class = \"chained\"\nchained_log_retention_years = 2\n\n[audit.store]\nretention_years = 5",
    );
    assert!(
        refusal(&chained).contains("keeps the requester's log 2 years, shorter than"),
        "{chained}"
    );
    let contact = full()
        .replacen(
            "mapping = \"iua\"\ncontact_point = false",
            "mapping = \"ehdsi\"\ncontact_point = true",
            1,
        )
        .replacen(
            "provider_identifier_prefixes = [\"urn:example:de:\"]\n",
            "",
            1,
        )
        .replacen(
            "audience = [\"ferroehr-federation\"]",
            "audience = [\"ferroehr-federation\"]\nsender_constraint = \"required\"",
            1,
        )
        .replacen(
            "[authz.import]",
            "[auth.dpop]\nexternal_url = \"https://ehr.example\"\n\n[authz.import]",
            1,
        );
    let text = refusal(&contact);
    assert!(text.contains("names a contact_point issuer"), "{text}");
}

#[test]
fn every_agreement_needs_a_caller_rate_and_patient_reads() {
    let text = refusal(&with(
        "reads = [\"patient/composition-*.r\"]\ncaller_rate = { requests_per_second = 5, burst = 10 }\n\n[[authz.federation.agreement]]",
        "reads = [\"user/composition-*.r\"]\n\n[[authz.federation.agreement]]",
    ));
    assert!(text.contains("needs caller_rate"), "{text}");
    assert!(
        text.contains("an agreement reads patient/ scopes only"),
        "{text}"
    );
    let nl = with("jurisdiction = \"DE\"", "jurisdiction = \"NL\"");
    assert!(
        refusal(&nl).contains("is an NL agreement with reads and needs consent"),
        "{nl}"
    );
}

#[test]
fn a_namespace_outside_the_object_ref_pattern_is_refused() {
    let text = refusal(&with(
        "[authz.party_binding]\n",
        "[authz.party_binding]\nnamespace = \"9 not a namespace\"\n",
    ));
    assert!(text.contains("OBJECT_REF.namespace pattern"), "{text}");
}

#[test]
fn a_trusted_system_that_is_this_node_is_refused() {
    let text = refusal(&with(
        "trusted_systems = [\"legacy.example\"]",
        "trusted_systems = [\"ferroehr.local\"]",
    ));
    assert!(text.contains("this node's own rest.system_id"), "{text}");
}

#[tokio::test]
async fn an_agreement_naming_a_template_the_store_does_not_hold_is_refused() {
    let (_db, service) = crate::fixtures::service().await;
    let config = assemble(&with(
        "accessor = \"person\"",
        "accessor = \"person\"\nwrites = [\"Not held.v1\"]",
    ))
    .expect("assembles");
    let unheld = service
        .unheld_written_templates(&config.authz)
        .await
        .expect("the store answers")
        .expect_err("refused");
    assert!(
        unheld
            .to_string()
            .contains("writes the template \"Not held.v1\", which the store does not hold"),
        "{unheld}"
    );
    let held = assemble(&full()).expect("assembles");
    service
        .unheld_written_templates(&held.authz)
        .await
        .expect("the store answers")
        .expect("no template is named");
}
