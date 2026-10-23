// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Configuration: the CDR's settings as it serves them, redacted, and the
//! viewer's own, read-only, as a settings page (T3) in file mode (FerroUI
//! `design/consoles/ferroehr/`, pages 22.1 to 22.8).
//!
//! The CDR serves its effective configuration as a JSON tree with every
//! secret already redacted by its type (`GET admin/config`, the CDR's own
//! extension behind its admin API, [`crate::bff::system`]). The page sorts
//! the tree's leaves into sections by their top-level key and draws each as
//! a setting row with its key. Both files are read once at start and change
//! only by editing the file and restarting, so nothing here can be changed.
//! No openEHR spec governs configuration: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's configuration JSON tree to draw its leaves"
)]

use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::tone::Tone;
use ferrohealth_ui::view::secret::{SecretState, secret_state};
use ferrohealth_ui::view::status::badge;
use ferrohealth_ui::view::surface::Header;
use ferrohealth_ui::view::tabs::{Tab, TabRow};
use ferrohealth_ui::view::template::settings::{Setting, SettingsPage, Source, settings_page};
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::hooks::use_params_map;
use serde_json::Value;

use crate::locale::{Msg, tr};
use crate::pages::common::{awaited, refused};

/// The sections of Configuration, each with the top-level keys of the CDR's
/// configuration it holds; the viewer's own is last.
const SECTIONS: [(&str, Msg, &[&str]); 7] = [
    (
        "deployment",
        Msg::ConfigDeployment,
        &[
            "deployment_profile",
            "spec_profile",
            "deployment_accepts",
            "deployment",
        ],
    ),
    ("server", Msg::ConfigServer, &["server", "db", "query"]),
    (
        "access",
        Msg::ConfigAccess,
        &["auth", "authz", "admin", "smart"],
    ),
    (
        "records",
        Msg::ConfigRecords,
        &["signing", "audit", "privacy"],
    ),
    (
        "integrations",
        Msg::ConfigIntegrations,
        &["events", "terminology"],
    ),
    (
        "operations",
        Msg::ConfigOperations,
        &["log", "telemetry", "management", "usage_report"],
    ),
    ("viewer", Msg::ConfigViewer, &[]),
];

/// The section a top-level key belongs to: the one that names it, else
/// Operations.
fn section_of(top: &str) -> &'static str {
    SECTIONS
        .iter()
        .find(|(_, _, keys)| keys.contains(&top))
        .map_or("operations", |(key, _, _)| key)
}

/// Every leaf of a configuration tree as its dotted key and value.
fn leaves(prefix: &str, value: &Value, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaves(&path, child, out);
            }
        }
        other => out.push((prefix.to_owned(), other.clone())),
    }
}

/// The words a setting goes by: its last key, its underscores read as
/// spaces and its first letter raised.
fn label_of(key: &str) -> String {
    let last = key.rsplit('.').next().unwrap_or(key).replace('_', " ");
    let mut chars = last.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// A redacted value, as the CDR spells a secret it does not show.
fn is_redacted(text: &str) -> bool {
    text == "***" || text.contains("://***")
}

/// The value of a setting as the row draws it.
fn value_view(value: &Value) -> AnyView {
    match value {
        Value::Bool(true) => {
            badge(Tone::Ok, tr(Msg::ConfigOn)).into_any()
        }
        Value::Bool(false) => {
            badge(Tone::Neutral, tr(Msg::ConfigOff)).into_any()
        }
        Value::String(text) if is_redacted(text) => secret_state(&SecretState::Redacted),
        Value::String(text) if text.is_empty() => {
            view! { <span class="fui-muted">{tr(Msg::RelationshipNone)}</span> }.into_any()
        }
        Value::String(text) => view! { <code>{text.clone()}</code> }.into_any(),
        Value::Number(number) => number.to_string().into_any(),
        Value::Null => {
            view! { <span class="fui-muted">{tr(Msg::RelationshipNone)}</span> }.into_any()
        }
        Value::Array(items) if items.is_empty() => {
            view! { <span class="fui-muted">{tr(Msg::RelationshipNone)}</span> }.into_any()
        }
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::String(text) => view! { <code>{text.clone()}</code> " " }.into_any(),
                other => view! { <code>{other.to_string()}</code> " " }.into_any(),
            })
            .collect_view()
            .into_any(),
        Value::Object(_) => ().into_any(),
    }
}

/// One setting row of a CDR configuration leaf.
fn setting(key: &str, value: &Value) -> Setting {
    Setting {
        label: label_of(key),
        description: String::new(),
        value: value_view(value),
        keys: vec![key.to_owned()],
        next_start: false,
    }
}

/// A setting of the viewer's own.
fn own(label: Msg, description: Msg, key: &str, value: AnyView) -> Setting {
    Setting {
        label: tr(label),
        description: tr(description),
        value,
        keys: vec![key.to_owned()],
        next_start: false,
    }
}

/// A secret of the viewer's own: read from its file, set inline, or unset.
fn own_secret(file: &str, inline: &str) -> AnyView {
    let state = if !file.is_empty() {
        SecretState::Set(file.to_owned())
    } else if !inline.is_empty() {
        SecretState::Redacted
    } else {
        SecretState::Unset
    };
    secret_state(&state)
}

/// The viewer's own settings, from the configuration it loaded.
fn viewer_settings() -> Vec<Setting> {
    let Some(state) = crate::request::state() else {
        return Vec::new();
    };
    let config = &state.config;
    let code = |text: &str| view! { <code>{text.to_owned()}</code> }.into_any();
    let on = |yes: bool| value_view(&Value::Bool(yes));
    vec![
        own(
            Msg::ConfigCdrAddress,
            Msg::ConfigCdrAddressLine,
            "cdr.base_url",
            code(&config.cdr.base_url),
        ),
        own(
            Msg::ConfigCdrBasePath,
            Msg::ConfigCdrBasePathLine,
            "cdr.base_path",
            code(&config.cdr.base_path),
        ),
        own(
            Msg::ConfigManagementAddress,
            Msg::ConfigManagementAddressLine,
            "cdr.management_base_url",
            code(&config.cdr.management_base_url),
        ),
        own(
            Msg::ConfigBasicSignIn,
            Msg::ConfigBasicSignInLine,
            "auth.basic_enabled",
            on(config.auth.basic_enabled),
        ),
        own(
            Msg::ConfigOidcSignIn,
            Msg::ConfigOidcSignInLine,
            "auth.oidc.issuer",
            if config.auth.oidc.enabled {
                code(&config.auth.oidc.issuer)
            } else {
                on(false)
            },
        ),
        own(
            Msg::ConfigOidcSecret,
            Msg::ConfigOidcSecretLine,
            "auth.oidc.client_secret_file",
            own_secret(
                &config.auth.oidc.client_secret_file,
                &config.auth.oidc.client_secret,
            ),
        ),
        own(
            Msg::ConfigIdle,
            Msg::ConfigIdleLine,
            "session.idle_minutes",
            crate::locale::tr_with(
                Msg::ConfigMinutes,
                &[("count", config.session.idle_minutes.into())],
            )
            .into_any(),
        ),
        own(
            Msg::ConfigSessionKey,
            Msg::ConfigSessionKeyLine,
            "session.secret_file",
            own_secret(&config.session.secret_file, &config.session.secret),
        ),
        own(
            Msg::ConfigLanguage,
            Msg::ConfigLanguageLine,
            "ui.locale",
            code(&config.ui.locale),
        ),
    ]
}

/// Configuration.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn ConfigurationPage() -> impl IntoView {
    let section = use_params_map()
        .with_untracked(|params| params.get("section"))
        .filter(|section| SECTIONS.iter().any(|(key, _, _)| key == section))
        .unwrap_or_else(|| "deployment".to_owned());
    let page = awaited(async move {
        let viewer = section == "viewer";
        let (settings, trouble) = if viewer {
            (viewer_settings(), None)
        } else {
            match crate::bff::system::fetch_admin_config().await {
                Ok(body) => match serde_json::from_str::<Value>(&body) {
                    Ok(tree) => {
                        let mut all = Vec::new();
                        leaves("", &tree, &mut all);
                        let rows = all
                            .iter()
                            .filter(|(key, _)| {
                                section_of(key.split('.').next().unwrap_or(key)) == section
                            })
                            .map(|(key, value)| setting(key, value))
                            .collect();
                        (rows, None)
                    }
                    Err(error) => (
                        Vec::new(),
                        Some(refused(
                            tr(Msg::ConfigRefused),
                            &crate::error::ViewerError::Internal(error.to_string()),
                            None,
                        )),
                    ),
                },
                Err(error) => (
                    Vec::new(),
                    Some(refused(tr(Msg::ConfigRefused), &error, None)),
                ),
            }
        };
        let tabs = SECTIONS
            .iter()
            .map(|(key, words, _)| {
                let href = if *key == "deployment" {
                    "/configuration".to_owned()
                } else {
                    format!("/configuration/{key}")
                };
                Tab::new(*key, href, tr(*words))
            })
            .collect();
        let page = settings_page(SettingsPage {
            header: Header {
                title: tr(Msg::NavConfiguration).into(),
                subtitle: Some(tr(Msg::ConfigSubtitle).into()),
                crumbs: Vec::new(),
                mono: false,
                badge: None,
            },
            actions: ActionGroup::none(),
            source: Source::File {
                file: if viewer {
                    "ferroehr-viewer.toml".to_owned()
                } else {
                    "ferroehr.toml".to_owned()
                },
                how: tr(Msg::ConfigHow),
            },
            tabs: Some(TabRow {
                label: tr(Msg::ConfigSections),
                tabs,
                current: section.clone(),
            }),
            settings,
            replica: None,
        });
        view! {
            {page}
            {trouble}
        }
        .into_any()
    });
    view! {
        <Title text=tr(Msg::NavConfiguration) />
        {page}
    }
}
