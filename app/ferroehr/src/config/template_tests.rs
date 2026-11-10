// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shipped template against the configuration types: it parses to the
//! defaults, declares every key and validates once the operator chooses.

use super::tests::{assemble_ok, env, json, toml_file};
use super::*;

#[test]
fn template_parses_to_default() {
    let file = toml_file(DEFAULT_TEMPLATE);
    let c = assemble_ok(Some(file.path()), &env(&[]), &[]);
    assert_eq!(json(&c), json(&FerroEhrConfig::default()));
}

/// The shipped template passes what `ferroehr config check` validates once
/// the operator makes the two choices no default makes for them: the
/// patient identity source and the authentication mechanism.
#[test]
fn the_default_template_passes_validation_once_the_operator_chooses() {
    let file = toml_file(DEFAULT_TEMPLATE);
    let unchosen = assemble_ok(Some(file.path()), &env(&[]), &[]);
    let refused = unchosen
        .validate()
        .expect_err("no patient identity source is chosen");
    assert!(
        refused.to_string().contains("record.subject_index"),
        "{refused}"
    );
    let chosen = assemble_ok(
        Some(file.path()),
        &env(&[]),
        &[("record.subject_index".to_owned(), "local".to_owned())],
    );
    chosen.validate().expect("the shipped template validates");
    assert!(matches!(
        auth::require_mechanism(&chosen.auth.shared),
        Err(auth::AuthConfigError::NoMechanism)
    ));
}

#[test]
fn template_mentions_every_section() {
    let sections = match serde_json::to_value(FerroEhrConfig::default()) {
        Ok(serde_json::Value::Object(members)) => members.keys().cloned().collect::<Vec<_>>(),
        other => panic!("the tree serializes as an object: {other:?}"),
    };
    for section in sections {
        let header = format!("[{section}]");
        let dotted = format!("[{section}."); // sub-tables count too
        let scalar = format!("\n{section} = "); // top-level scalar keys
        assert!(
            DEFAULT_TEMPLATE.contains(&header)
                || DEFAULT_TEMPLATE.contains(&dotted)
                || DEFAULT_TEMPLATE.contains(&scalar)
                || DEFAULT_TEMPLATE.contains(&format!("\n#? {section} = ")),
            "template missing section {section}"
        );
    }
}

/// Whether the template declares `key` as a key line — live (`key = …`) or
/// as a `#?` reference line for an optional/secret/derived one. A mention
/// inside another key's trailing comment does not count: a key mentioned
/// only that way stays invisible to an operator reading the template.
fn template_declares(key: &str) -> bool {
    DEFAULT_TEMPLATE.lines().any(|line| {
        let line = line.trim_start();
        let line = line.strip_prefix("#?").map_or(line, str::trim_start);
        line.strip_prefix(key)
            .is_some_and(|rest| rest.trim_start().starts_with('='))
    })
}

/// Every field the default config SERIALIZES must be a key in the template.
///
/// Total for concrete fields and needs no hand-maintained list: it walks the
/// serialized default, so a newly added field is covered the moment it
/// exists. `Option::None` fields serialize away and are covered by
/// [`template_declares_every_optional_oidc_field`] instead.
#[test]
fn template_declares_every_serialized_field() {
    fn walk(value: &serde_json::Value, missing: &mut Vec<String>) {
        let serde_json::Value::Object(map) = value else {
            return;
        };
        for (key, child) in map {
            match child {
                serde_json::Value::Object(_) => walk(child, missing),
                // An absent optional serializes to null, so this walk cannot
                // see it at all — that class is the other test's job. An
                // absent optional SUB-TABLE lands here too and is declared
                // as a `[section]` header rather than a key line.
                serde_json::Value::Null => {}
                _ if template_declares(key) => {}
                _ if DEFAULT_TEMPLATE.contains(&format!(".{key}]")) => {}
                _ => missing.push(key.clone()),
            }
        }
    }
    let mut missing = Vec::new();
    walk(&json(&FerroEhrConfig::default()), &mut missing);
    assert!(
        missing.is_empty(),
        "the shipped ferroehr.toml template declares no key line for: {}. \
         Every field belongs in it — the file's own header calls itself the \
         complete configuration, and the shipped template is how an \
         operator discovers a key.",
        missing.join(", ")
    );
}

/// The field names a `deny_unknown_fields` struct accepts, taken from serde
/// itself: an unknown key is refused with `expected one of …`, which is the
/// authoritative set and needs no hand-maintained list.
fn accepted_fields<T: serde::de::DeserializeOwned>(section: &str) -> Vec<String> {
    let Err(err) = toml::from_str::<T>("__probe_unknown_key__ = 1") else {
        panic!("[{section}] must carry deny_unknown_fields")
    };
    let err = err.to_string();
    let Some((_, list)) = err.split_once("expected one of ") else {
        panic!("serde no longer reports an accepted-field list for [{section}]: {err}")
    };
    let fields: Vec<String> = list
        .split([',', '`'])
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect();
    // Without this the test passes vacuously if serde ever reformats that
    // message — a guard that silently checks nothing is worse than none.
    assert!(
        !fields.is_empty(),
        "extracted no fields for [{section}]; serde's message format changed \
         and this test is no longer checking anything: {err}"
    );
    fields
}

/// Optional fields serialize away when `None`, so the walk above cannot see
/// them, which is the class an operator can least afford to have
/// undeclared.
///
/// Covered here are the sections carrying secret or file-indirection keys,
/// where an undiscoverable key costs an operator most. Adding another
/// section is one line, because the field list comes from serde.
#[test]
fn template_declares_every_optional_field_of_the_secret_bearing_sections() {
    let sections = [
        (
            "auth",
            accepted_fields::<ferrohealth_smart::config::AuthSection>("auth"),
        ),
        (
            "authz.assurance",
            accepted_fields::<ferrohealth_smart::assurance::AssuranceSection>("authz.assurance"),
        ),
        (
            "authz.party_binding",
            retired::live(
                accepted_fields::<binding::PartyBindingConfig>("authz.party_binding"),
                &["enabled", "provisional_report_days"],
            ),
        ),
        (
            "database",
            accepted_fields::<crate::db::DbConfig>("database"),
        ),
        (
            "pixm",
            accepted_fields::<crate::service::ehr_index::config::PixmConfig>("pixm"),
        ),
        (
            "record.signing",
            accepted_fields::<crate::versioning::signature::config::SigningConfig>(
                "record.signing",
            ),
        ),
        (
            "record.multimedia",
            retired::live(
                accepted_fields::<crate::extensions::multimedia::config::MultimediaConfig>(
                    "record.multimedia",
                ),
                &["allow_http"],
            ),
        ),
    ];
    let missing: Vec<String> = sections
        .iter()
        .flat_map(|(section, fields)| {
            // A sub-table field is declared by its own `[section.field]`
            // header, exactly as the serialized-field walk accepts one, or
            // by the `[[section.field.…]]` header of the array it holds.
            fields
                .iter()
                .filter(move |f| {
                    !template_declares(f)
                        && !DEFAULT_TEMPLATE.contains(&format!("[{section}.{f}]"))
                        && !DEFAULT_TEMPLATE.contains(&format!("[[{section}.{f}."))
                })
                .map(move |f| format!("{section}.{f}"))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "the template declares no key line for: {}",
        missing.join(", ")
    );
}
