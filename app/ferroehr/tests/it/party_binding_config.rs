// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The party binding configuration through `ferroehr::config::assemble`: a
//! full configuration of every issuer kind loads, and each boot refusal of
//! `[auth]` and `[authz.party_binding]` names its key; the federation
//! agreements and the system clients are CDR data, judged at their commit
//! (`federation_agreements.rs`, `policy_grants.rs`). No openEHR spec governs
//! configuration: our own design.

use std::collections::HashMap;

use assert_fs::prelude::FileWriteStr as _;
use ferroehr::config::FerroEhrConfig;
use ferroehr::config::binding::{BindingPosture, Providers};

/// An Argon2id PHC string at the OWASP floor.
const HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c2FsdHNhbHQ$aGFzaGhhc2hoYXNoaGFzaGhhc2hoYQ";

/// A configuration with one issuer of every kind, a bound persons issuer,
/// and a mapped Basic user.
fn full() -> String {
    format!(
        r#"
[record]
subject_index = "local"

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
    assert!(
        posture.agreements.is_empty(),
        "the configuration states no agreement: they are read from the CDR"
    );
    let info = serde_json::to_value(ferroehr::telemetry::build_info::BuildInfo::for_config(
        &config,
    ))
    .expect("renders");
    assert_eq!(info["binding"]["issuers"][0]["kind"], "persons", "{info}");
    assert_eq!(
        info["binding"]["agreements"],
        serde_json::json!([]),
        "{info}"
    );
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
             under identifier_type \"guid\", which no person template's key carries: the person \
             templates' keys are of the types {\"login\"}"
        ),
        "the default person template's key is of the type login: {text}"
    );
    let text = refusal(&format!("{}\n[authz.party_binding.person]\n", full()));
    assert!(
        text.contains(
            "the [[auth.party]] table of \"https://idp.example/realms/staff\" looks logins up \
             under identifier_type \"guid\", which no person template's key carries: no \
             [[authz.party_binding.person.template]] is declared"
        ),
        "an empty person section maps no person: {text}"
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

/// A section a deployment sets replaces that section's default whole, and
/// every section it leaves out keeps the shipped default.
#[test]
fn an_explicit_section_overrides_its_default_and_leaves_the_others() {
    let config = assemble(&format!(
        "{}\n[authz.party_binding.group]\ntemplate_id = \"own-team\"\nexternal_id = \
         {{ path = \"/details[id2]/items[id3]\", type = \"team\" }}\n",
        bound(&full())
    ))
    .expect("assembles");
    let binding = &config.authz.party_binding;
    let default = ferroehr::config::binding::PartyBindingConfig::default();
    let group = binding.group.as_ref().expect("the group section is set");
    assert_eq!(group.template_id, "own-team");
    assert_eq!(
        group.display_name, None,
        "a key the set section leaves out takes no default"
    );
    assert_eq!(
        binding.person().template[0].template_id,
        "FerroEHR professional"
    );
    assert!(
        binding.person().attribute.is_empty(),
        "the person section is replaced whole, its default attributes with it"
    );
    assert_eq!(
        binding
            .professional_role
            .as_ref()
            .map(|role| role.template_id.as_str()),
        Some("FerroEHR professional role")
    );
    assert_eq!(binding.membership, default.membership);
    assert_eq!(binding.representation, default.representation);
    assert_eq!(binding.patient, default.patient);
    assert_ne!(binding.group, default.group);
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
fn a_removed_key_is_refused_as_unknown() {
    let text = refusal(&with(
        "[authz.party_binding]\n",
        "[authz.party_binding]\nenabled = true\n",
    ));
    assert!(text.contains("unknown field `enabled`"), "{text}");
    let text = refusal("[auth]\nissuer = \"https://idp.example\"\n");
    assert!(text.contains("issuer"), "{text}");
    for (key, name) in [
        ("acting_for_claim = \"act.sub\"", "acting_for_claim"),
        ("client_tokens = \"claim_absent\"", "client_tokens"),
    ] {
        let text = refusal(&with(
            "first_party_clients = [\"ferroehr-viewer\"]",
            &format!("first_party_clients = [\"ferroehr-viewer\"]\n{key}"),
        ));
        assert!(
            text.contains(&format!("unknown field `{name}`")),
            "{key}: {text}"
        );
    }
    let text = refusal(&format!(
        "{}\n[authz.professional]\nclaim = \"sub\"\n",
        full()
    ));
    assert!(text.contains("unknown field `professional`"), "{text}");
    let text = refusal(&format!(
        "{}\n[[authz.federation.agreement]]\nissuer = \"https://federation.example\"\n",
        full()
    ));
    assert!(text.contains("unknown field `federation`"), "{text}");
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
    assert!(text.contains("unknown field `claim`"), "{text}");
    let text = refusal(&with(
        "first_party_clients = [\"ferroehr-viewer\"]",
        "first_party_clients = [\"ferroehr-viewer\"]\ntoken_profile = \"jwt\"",
    ));
    assert!(text.contains("unknown field `token_profile`"), "{text}");
}

/// The system clients are policy parties in the CDR (#5436): the table that
/// declared them is an unknown key, with each of its former keys.
#[test]
fn a_system_client_table_is_refused_as_unknown() {
    for table in [
        "issuer = \"https://idp.example/realms/machines\"\nclient_id = \"ferrobridge\"",
        "provider = { issuer = \"urn:oid:2.16.528.1.1007.3.3\", id = \"12345678\" }",
        "facilities = [{ issuer = \"urn:example:facility\", id_prefix = \"ward-\" }]",
        "writes = [\"Vital signs.v1\"]",
        "origins = [\"relay\", \"device\"]",
        "device_identifier_issuers = [\"urn:oid:2.999.9\"]",
    ] {
        let text = refusal(&format!("{}\n[[authz.system_client]]\n{table}\n", full()));
        assert!(
            text.contains("unknown field `system_client`"),
            "{table}: {text}"
        );
    }
}

#[test]
fn a_namespace_outside_the_object_ref_pattern_is_refused() {
    let text = refusal(&with(
        "[authz.party_binding]\n",
        "[authz.party_binding]\nnamespace = \"9 not a namespace\"\n",
    ));
    assert!(text.contains("OBJECT_REF.namespace pattern"), "{text}");
}

/// The keys #5439 moved into the CDR are unknown keys, refused like any
/// other: trusted import systems, registered origins, change-feed readers and
/// SCIM clients are grants of the policy parties now.
#[test]
fn the_keys_the_cdr_now_holds_are_refused_as_unknown() {
    for (removed, key) in [
        (
            "[authz.import]\ntrusted_systems = [\"legacy.example\"]\n",
            "import",
        ),
        ("[[authz.origin]]\nid = \"lab.example\"\n", "origin"),
        (
            "[record.change_feed]\nreaders = [\"ferrobridge\"]\n",
            "readers",
        ),
        (
            "[auth.scim]\n[[auth.scim.client]]\nclient_id = \"entra\"\n",
            "client",
        ),
    ] {
        let text = refusal(&format!("{}\n{removed}", full()));
        assert!(text.contains(key), "{key}: {text}");
    }
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
        "{}\n[privacy.identifier_protection]\nschemes = [\"guid\"]\nkey = \"{}\"\n",
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

/// [`full`] serving SCIM; who provisions is a `scim_provision` grant in the
/// CDR, so the table holds no client.
#[test]
fn a_scim_table_with_no_client_loads() {
    let config = assemble(&bound(&format!(
        "{}\n[server]\npublic_base_url = \"https://cdr.example.com\"\n\n[auth.scim]\n",
        full()
    )))
    .expect("assembles");
    config.validate().expect("validates");
    assert!(config.auth.scim.is_some());
}

#[test]
fn an_unknown_scim_key_is_named() {
    let unknown = refusal(&format!("{}\n[auth.scim]\nmax_deactivation = 3\n", full()));
    assert!(unknown.contains("max_deactivation"), "{unknown}");
}
