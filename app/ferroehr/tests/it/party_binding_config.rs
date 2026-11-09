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

/// A person template whose key is of the `guid` type the staff issuer's
/// `[[auth.party]]` table looks logins up under, with its professional role.
const KEY_TEMPLATE: &str = r#"
[[authz.party_binding.person.template]]
template_id = "FerroEHR professional"
default = true
key = { path = "/details[id2]/items[id3]", type = "guid" }

[authz.party_binding.professional_role]
template_id = "FerroEHR professional role"
"#;

/// `toml` with [`KEY_TEMPLATE`], so its persons issuer's logins can bind.
fn bound(toml: &str) -> String {
    format!("{toml}{KEY_TEMPLATE}")
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
    let config = assemble(&bound(&full())).expect("assembles");
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
fn a_mapped_basic_user_is_named_while_no_assurance_minimum_is_set() {
    let config = assemble(&bound(&full())).expect("assembles");
    assert_eq!(BindingPosture::of(&config).unassured_basic, ["alice"]);
    let mut assured = config;
    assured.authz.assurance.minimum = Some(ferrohealth_smart::assurance::AssuranceLevel::High);
    assert_eq!(
        BindingPosture::of(&assured).unassured_basic,
        Vec::<String>::new()
    );
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

/// An `[[auth.party]]` table looks logins up under an `identifier_type` some
/// person template's key is written under, or no login of its issuer binds.
#[test]
fn a_party_table_whose_identifier_type_no_template_key_carries_is_refused() {
    let text = refusal(&full());
    assert!(
        text.contains(
            "the [[auth.party]] table of \"https://idp.example/realms/staff\" looks logins up \
             under identifier_type \"guid\", which no person template's key carries: no \
             [[authz.party_binding.person.template]] is declared"
        ),
        "{text}"
    );
    let text = refusal(&bound(&full()).replacen("type = \"guid\" }", "type = \"login\" }", 1));
    assert!(
        text.contains(
            "under identifier_type \"guid\", which no person template's key carries: the person \
             templates' keys are of the types {\"login\"}"
        ),
        "{text}"
    );
    assemble(&bound(&full()))
        .expect("assembles")
        .validate()
        .expect("a table whose type a template's key carries loads");
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

/// A contact point's person agreement names the MyHealth@EU service its
/// refusals answer for, and no other issuer's agreement does (#5102).
#[test]
fn a_contact_points_person_agreement_names_its_ehdsi_service() {
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
    let start = contact
        .find("\n[[authz.federation.agreement]]\nissuer = \"https://federation.example\"\naccessor = \"organisation\"")
        .expect("the organisation agreement");
    let persons = contact.split_at(start).0.to_owned();
    let text = refusal(&persons);
    assert!(
        text.contains("contact point's and needs ehdsi_service"),
        "{text}"
    );
    let served = persons.replacen(
        "accessor = \"person\"",
        "accessor = \"person\"\nehdsi_service = \"eprescription\"",
        1,
    );
    assemble(&bound(&served))
        .expect("assembles")
        .validate()
        .expect("a contact point's person agreement naming its service loads");
    let domestic = with(
        "accessor = \"person\"",
        "accessor = \"person\"\nehdsi_service = \"patient_summary\"",
    );
    let text = refusal(&domestic);
    assert!(
        text.contains("sets ehdsi_service, which only a contact_point issuer's agreement may"),
        "{text}"
    );
}

/// A `chained` agreement's issuer reads the correlation id from its token,
/// never from a constant (#5120).
#[test]
fn a_chained_agreement_needs_its_issuer_to_map_the_correlation_id() {
    let chained = with(
        "access_class = \"automated\"",
        "access_class = \"chained\"\nchained_log_retention_years = 30\n\n[audit.store]\nretention_years = 5",
    );
    let text = refusal(&chained);
    assert!(
        text.contains(
            "is chained and its issuer's [[auth.federation]] table maps no correlation_id"
        ),
        "{text}"
    );
    let fields = "\"professional.issuing_authority_name\" = [{ constant = \"BAEK\" }]";
    let constant = chained.replacen(
        fields,
        &format!("{fields}\n\"correlation_id\" = [{{ constant = \"c-1\" }}]"),
        1,
    );
    let text = refusal(&constant);
    assert!(
        text.contains("maps correlation_id from a constant"),
        "{text}"
    );
    let claim = chained.replacen(
        fields,
        &format!("{fields}\n\"correlation_id\" = [{{ claim = \"correlation_id\" }}]"),
        1,
    );
    assemble(&bound(&claim))
        .expect("assembles")
        .validate()
        .expect("a chained agreement whose issuer maps the id from a claim loads");
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

/// An NL agreement declares its purpose, and boot refuses one for a keuring,
/// whose keurend arts has no access to an exchange system (Wabvpz
/// Art. 15f(2), #5107).
#[test]
fn an_nl_agreement_for_a_keuring_or_without_a_purpose_is_refused() {
    let nl = |purpose: &str| {
        with(
            "jurisdiction = \"DE\"",
            &format!("jurisdiction = \"NL\"\nconsent = \"local\"\n{purpose}"),
        )
    };
    let text = refusal(&nl(""));
    assert!(
        text.contains("is an NL agreement and needs purpose"),
        "{text}"
    );
    let text = refusal(&nl("purpose = \"keuring\""));
    assert!(text.contains("is an NL agreement for a keuring"), "{text}");
    assemble(&bound(&nl("purpose = \"care\"")))
        .expect("assembles")
        .validate()
        .expect("an NL agreement for care loads");
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

/// The shared party mapping of #5013: one person template with its three
/// selectors at one path told apart by type, a name attribute, the
/// professional role, a group and the two relationship archetypes.
const MAPPING: &str = r#"
[[authz.party_binding.person.template]]
template_id = "FerroEHR professional"
default = true
key = { path = "/details[id2]/items[id3]", type = "guid" }
user_name = { path = "/details[id2]/items[id3]", type = "scim_user_name" }
external_id = { path = "/details[id2]/items[id3]", type = "scim_external_id" }

[[authz.party_binding.person.attribute]]
scim = "name.formatted"
oidc = "name"
jit = true
name = true
path = { "FerroEHR professional" = "/identities[id4]/details[id5]/items[id6]" }

[authz.party_binding.professional_role]
template_id = "FerroEHR professional role"

[authz.party_binding.group]
template_id = "FerroEHR care team"
external_id = { path = "/details[id2]/items[id3]", type = "scim_external_id" }
display_name = "/identities[id4]/details[id5]/items[id6]"

[authz.party_binding.membership]
archetype_id = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0"

[authz.party_binding.representation]
archetype_id = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0"
relationship_codes = { at0010 = "254", at0011 = "264" }
"#;

/// [`full`] with [`MAPPING`], `old` replaced by `new` in the mapping.
fn mapped(old: &str, new: &str) -> String {
    assert!(MAPPING.contains(old), "the mapping holds {old:?}");
    format!("{}{}", full(), MAPPING.replacen(old, new, 1))
}

#[test]
fn the_shared_party_mapping_loads() {
    let config = assemble(&mapped("", "")).expect("assembles");
    config.validate().expect("validates");
    let sections = config.authz.party_binding.sections();
    assert_eq!(sections.person.template.len(), 1);
    assert!(!sections.person.attribute[0].path.is_empty());
    let mapping = sections
        .binding_mapping(&["urn:example:staff-login".to_owned()], "ADMIN")
        .expect("compiles");
    assert!(mapping.names_template("ferroehr PROFESSIONAL"));
    assert!(mapping.names_template("FerroEHR professional role"));
    assert!(mapping.names_template("FerroEHR care team"));
    assert!(!mapping.names_template("FerroEHR patient"));
}

#[test]
fn an_unknown_key_in_a_mapping_section_is_refused() {
    for (old, new) in [
        ("default = true", "default = true\ncolour = \"blue\""),
        ("jit = true", "jit = true\nrequired = true"),
        (
            "template_id = \"FerroEHR professional role\"",
            "template_id = \"FerroEHR professional role\"\nscope = \"x\"",
        ),
        (
            "relationship_codes",
            "archetype = \"x\"\nrelationship_codes",
        ),
        ("type = \"guid\" }", "type = \"guid\", issuer = \"x\" }"),
    ] {
        let text = refusal(&mapped(old, new));
        assert!(text.contains("unknown field"), "{old}: {text}");
    }
}

/// Asserts the mapping with `old` replaced by `new` is refused with a
/// message holding `expected`.
fn refused_with(old: &str, new: &str, expected: &str) {
    let text = refusal(&mapped(old, new));
    assert!(text.contains(expected), "{old} -> {new}: {text}");
}

#[test]
fn a_mapping_without_exactly_one_default_template_is_refused() {
    refused_with("default = true", "default = false", "exactly one");
}

#[test]
fn a_selector_path_that_does_not_compile_is_refused_naming_it() {
    refused_with(
        "key = { path = \"/details[id2]/items[id3]\"",
        "key = { path = \"details[id2\"",
        "\"FerroEHR professional\" key",
    );
}

#[test]
fn two_equal_selectors_are_refused() {
    refused_with(
        "type = \"scim_user_name\"",
        "type = \"guid\"",
        "key and user_name are one selector",
    );
}

#[test]
fn a_key_of_a_protected_scheme_is_refused() {
    let toml = format!(
        "{}\n[demographic.identifier_protection]\nenabled = true\nschemes = [\"guid\"]\nkey = \"{}\"\n",
        mapped("", ""),
        "00".repeat(32)
    );
    let text = refusal(&toml);
    assert!(text.contains("is protected"), "{text}");
}

#[test]
fn a_bsn_identifier_on_a_professional_template_is_refused() {
    refused_with(
        "type = \"scim_external_id\" }\n\n[[authz",
        "type = \"2.16.840.1.113883.2.4.6.3\" }\n\n[[authz",
        "Dutch BSN namespace",
    );
}

#[test]
fn a_name_mapped_into_details_is_refused() {
    refused_with(
        "\"/identities[id4]/details[id5]/items[id6]\" }",
        "\"/details[id2]/items[id7]\" }",
        "goes in PARTY.identities",
    );
}

#[test]
fn an_identifier_mapped_into_identities_is_refused() {
    refused_with(
        "scim = \"name.formatted\"\noidc = \"name\"\njit = true\nname = true",
        "scim = \"urn:ietf:params:scim:schemas:extension:ferrohealth:2.0:User:loginIdentifier\"",
        "goes in PARTY.details",
    );
}

#[test]
fn an_intrinsic_scim_attribute_is_never_mapped() {
    refused_with(
        "scim = \"name.formatted\"\noidc = \"name\"\njit = true\nname = true",
        "scim = \"userName\"",
        "intrinsic to SCIM",
    );
}

#[test]
fn a_key_fed_by_an_email_address_is_refused() {
    refused_with(
        "scim = \"name.formatted\"\noidc = \"name\"\njit = true\nname = true\npath = { \"FerroEHR professional\" = \"/identities[id4]/details[id5]/items[id6]\" }",
        "oidc = \"email\"\npath = { \"FerroEHR professional\" = \"/details[id2]/items[id3]\" }",
        "never fed by an email address",
    );
}

#[test]
fn a_relationship_code_outside_subject_relationship_is_refused() {
    refused_with(
        "at0011 = \"264\"",
        "at0011 = \"9999\"",
        "subject_relationship group",
    );
}

/// [`full`] serving SCIM to one provisioning client of `issuer` binding
/// `binding_issuer`, followed by `extra`.
fn scim(issuer: &str, binding_issuer: &str, extra: &str) -> String {
    format!(
        "{}\n[scim]\nenabled = true\npublic_base_url = \"https://cdr.example.com\"\n\n\
         [[scim.client]]\nissuer = \"{issuer}\"\nclient_id = \"entra\"\nscope = \"ferroehr/scim\"\n\
         binding_issuer = \"{binding_issuer}\"\n{extra}",
        full()
    )
}

#[test]
fn a_scim_client_of_the_machines_issuer_binding_a_party_issuer_loads() {
    let config = assemble(&bound(&scim(
        "https://idp.example/realms/machines",
        "urn:example:staff-login",
        "",
    )))
    .expect("assembles");
    config.validate().expect("validates");
    assert!(config.scim.enabled);
    assert_eq!(config.scim.client.len(), 1);
}

#[test]
fn every_scim_client_refusal_is_named() {
    let staff = refusal(&scim(
        "https://idp.example/realms/staff",
        "urn:example:staff-login",
        "",
    ));
    assert!(
        staff.contains("names a persons issuer: a SCIM client acts for nobody"),
        "{staff}"
    );
    let unbound = refusal(&scim(
        "https://idp.example/realms/machines",
        "urn:example:other",
        "",
    ));
    assert!(
        unbound.contains("is the identifier_issuer of no [[auth.party]] table"),
        "{unbound}"
    );
    let shared = refusal(&scim(
        "https://idp.example/realms/machines",
        "urn:example:staff-login",
        "\n[[scim.client]]\nissuer = \"https://idp.example/realms/machines\"\n\
         client_id = \"okta\"\nscope = \"ferroehr/scim\"\n\
         binding_issuer = \"urn:example:staff-login\"\n",
    ));
    assert!(shared.contains("shares binding_issuer"), "{shared}");
    let unknown = refusal(&format!(
        "{}\n[scim]\nenabled = true\nmax_deactivation = 3\n",
        full()
    ));
    assert!(unknown.contains("max_deactivation"), "{unknown}");
}
