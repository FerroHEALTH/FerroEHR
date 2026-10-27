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

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::tone::Tone;
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::dialog::{Explanation, info_dialog};
use ferrohealth_ui::view::secret::{SecretState, secret_state};
use ferrohealth_ui::view::status::badge;
use ferrohealth_ui::view::surface::Header;
use ferrohealth_ui::view::tabs::{Tab, TabRow};
use ferrohealth_ui::view::template::settings::{How, Setting, SettingsPage, Source, settings_page};
use leptos::prelude::*;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;
use crate::locale::{Msg, tr};
use crate::pages::common::{Refused, drawn, page_model};

/// The id of the dialog the file line's How configuration works opens.
const HOW_DIALOG: &str = "how-configuration";

/// How configuration works: where the settings are read from, where the
/// viewer shows them and how they change.
fn how_dialog() -> AnyView {
    info_dialog(
        Explanation {
            id: HOW_DIALOG.into(),
            title: tr(Msg::ConfigHowTitle).into(),
        },
        view! {
            <p>{tr(Msg::ConfigHowReads)}</p>
            <div class="fui-ownership">
                <div>
                    <h3>
                        {ferrohealth_ui::icon::icon(Glyph::FILE_TEXT, "16")}
                        {tr(Msg::ConfigHowWhereTitle)}
                    </h3>
                    <p class="fui-small fui-muted">{tr(Msg::ConfigHowWhere)}</p>
                </div>
                <div>
                    <h3>
                        {ferrohealth_ui::icon::icon(Glyph::REFRESH_CW, "16")}
                        {tr(Msg::ConfigHowChangeTitle)}
                    </h3>
                    <p class="fui-small fui-muted">{tr(Msg::ConfigHowChange)}</p>
                </div>
            </div>
            <p class="fui-small fui-muted">{tr(Msg::ConfigHowNothing)}</p>
        },
    )
    .into_any()
}

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

#[cfg(not(target_arch = "wasm32"))]
/// The section a top-level key belongs to: the one that names it, else
/// Operations.
fn section_of(top: &str) -> &'static str {
    SECTIONS
        .iter()
        .find(|(_, _, keys)| keys.contains(&top))
        .map_or("operations", |(key, _, _)| key)
}

/// A leaf of the CDR's configuration as the page draws it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Leaf {
    /// A flag.
    Flag(bool),
    /// A string.
    Text(String),
    /// A number, as the CDR wrote it.
    Number(String),
    /// A null or an empty list.
    Nothing,
    /// A list, each item as text.
    Items(Vec<String>),
    /// An empty object.
    Blank,
}

#[cfg(not(target_arch = "wasm32"))]
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's configuration JSON tree to draw its leaves"
)]
impl From<&serde_json::Value> for Leaf {
    fn from(value: &serde_json::Value) -> Self {
        use serde_json::Value;
        match value {
            Value::Bool(yes) => Self::Flag(*yes),
            Value::String(text) => Self::Text(text.clone()),
            Value::Number(number) => Self::Number(number.to_string()),
            Value::Null => Self::Nothing,
            Value::Array(items) if items.is_empty() => Self::Nothing,
            Value::Array(items) => Self::Items(
                items
                    .iter()
                    .map(|item| match item {
                        Value::String(text) => text.clone(),
                        other => other.to_string(),
                    })
                    .collect(),
            ),
            Value::Object(_) => Self::Blank,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Every leaf of a configuration tree as its dotted key and value.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's configuration JSON tree to draw its leaves"
)]
fn leaves(prefix: &str, value: &serde_json::Value, out: &mut Vec<(String, Leaf)>) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, child) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                leaves(&path, child, out);
            }
        }
        other => out.push((prefix.to_owned(), Leaf::from(other))),
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

/// The words of a setting without a value.
fn none() -> AnyView {
    view! { <span class="fui-muted">{tr(Msg::RelationshipNone)}</span> }.into_any()
}

/// The value of a setting as the row draws it.
fn value_view(value: &Leaf) -> AnyView {
    match value {
        Leaf::Flag(true) => badge(Tone::Ok, tr(Msg::ConfigOn)).into_any(),
        Leaf::Flag(false) => badge(Tone::Neutral, tr(Msg::ConfigOff)).into_any(),
        Leaf::Text(text) if is_redacted(text) => secret_state(&SecretState::Redacted),
        Leaf::Text(text) if text.is_empty() => none(),
        Leaf::Text(text) => view! { <code>{text.clone()}</code> }.into_any(),
        Leaf::Number(number) => number.clone().into_any(),
        Leaf::Nothing => none(),
        Leaf::Items(items) => items
            .iter()
            .map(|item| {
                view! {
                    <code>{item.clone()}</code>
                    " "
                }
                .into_any()
            })
            .collect_view()
            .into_any(),
        Leaf::Blank => ().into_any(),
    }
}

/// One setting row of a CDR configuration leaf.
fn setting(key: &str, value: &Leaf) -> Setting {
    Setting {
        label: label_of(key),
        description: String::new(),
        value: value_view(value),
        keys: vec![key.to_owned()],
        next_start: false,
        source: None,
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
        source: None,
    }
}

/// Where a secret of the viewer's own comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum OwnSecret {
    /// Read from this file.
    File(String),
    /// Set inline.
    Inline,
    /// Unset.
    Unset,
}

impl OwnSecret {
    #[cfg(not(target_arch = "wasm32"))]
    /// The secret read from `file` or set as `inline`.
    fn of(file: &str, inline: &str) -> Self {
        if !file.is_empty() {
            Self::File(file.to_owned())
        } else if !inline.is_empty() {
            Self::Inline
        } else {
            Self::Unset
        }
    }

    /// The secret, drawn.
    fn view(&self) -> AnyView {
        let state = match self {
            Self::File(file) => SecretState::Set(file.clone()),
            Self::Inline => SecretState::Redacted,
            Self::Unset => SecretState::Unset,
        };
        secret_state(&state)
    }
}

/// The viewer's own settings, as the configuration it loaded holds them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Own {
    /// `cdr.base_url`.
    base_url: String,
    /// `cdr.base_path`.
    base_path: String,
    /// `cdr.management_base_url`.
    management_base_url: String,
    /// `auth.basic_enabled`.
    basic: bool,
    /// `auth.oidc.issuer`, when OpenID Connect sign-in is on.
    oidc: Option<String>,
    /// `auth.oidc.client_secret_file` or `client_secret`.
    oidc_secret: OwnSecret,
    /// `session.idle_minutes`.
    idle_minutes: u64,
    /// `session.secret_file` or `secret`.
    session_secret: OwnSecret,
    /// `ui.locale`.
    locale: String,
}

#[cfg(not(target_arch = "wasm32"))]
/// The viewer's own settings, from the configuration it loaded.
fn read_own() -> Option<Own> {
    let state = crate::request::state()?;
    let config = &state.config;
    Some(Own {
        base_url: config.cdr.base_url.clone(),
        base_path: config.cdr.base_path.clone(),
        management_base_url: config.cdr.management_base_url.clone(),
        basic: config.auth.basic_enabled,
        oidc: config
            .auth
            .oidc
            .enabled
            .then(|| config.auth.oidc.issuer.clone()),
        oidc_secret: OwnSecret::of(
            &config.auth.oidc.client_secret_file,
            &config.auth.oidc.client_secret,
        ),
        idle_minutes: config.session.idle_minutes,
        session_secret: OwnSecret::of(&config.session.secret_file, &config.session.secret),
        locale: config.ui.locale.clone(),
    })
}

/// The viewer's own settings as rows.
fn viewer_settings(loaded: Option<Own>) -> Vec<Setting> {
    let Some(config) = loaded else {
        return Vec::new();
    };
    let code = |text: &str| view! { <code>{text.to_owned()}</code> }.into_any();
    let on = |yes: bool| value_view(&Leaf::Flag(yes));
    vec![
        own(
            Msg::ConfigCdrAddress,
            Msg::ConfigCdrAddressLine,
            "cdr.base_url",
            code(&config.base_url),
        ),
        own(
            Msg::ConfigCdrBasePath,
            Msg::ConfigCdrBasePathLine,
            "cdr.base_path",
            code(&config.base_path),
        ),
        own(
            Msg::ConfigManagementAddress,
            Msg::ConfigManagementAddressLine,
            "cdr.management_base_url",
            code(&config.management_base_url),
        ),
        own(
            Msg::ConfigBasicSignIn,
            Msg::ConfigBasicSignInLine,
            "auth.basic_enabled",
            on(config.basic),
        ),
        own(
            Msg::ConfigOidcSignIn,
            Msg::ConfigOidcSignInLine,
            "auth.oidc.issuer",
            config.oidc.as_deref().map_or_else(|| on(false), code),
        ),
        own(
            Msg::ConfigOidcSecret,
            Msg::ConfigOidcSecretLine,
            "auth.oidc.client_secret_file",
            config.oidc_secret.view(),
        ),
        own(
            Msg::ConfigIdle,
            Msg::ConfigIdleLine,
            "session.idle_minutes",
            crate::locale::tr_with(Msg::ConfigMinutes, &[("count", config.idle_minutes.into())])
                .into_any(),
        ),
        own(
            Msg::ConfigSessionKey,
            Msg::ConfigSessionKeyLine,
            "session.secret_file",
            config.session_secret.view(),
        ),
        own(
            Msg::ConfigLanguage,
            Msg::ConfigLanguageLine,
            "ui.locale",
            code(&config.locale),
        ),
    ]
}

/// The settings a section shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Shown {
    /// The CDR's leaves of the section, by their dotted key.
    Cdr(Vec<(String, Leaf)>),
    /// The viewer's own, when the render was given its configuration.
    Viewer(Option<Own>),
    /// The CDR's refusal of its configuration.
    Refused(Refused),
}

/// Loads the settings of `section`: the viewer's own, or the CDR's leaves
/// that belong to it.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
#[server(client = crate::session_client::SessionAwareClient)]
async fn load(
    /// The path and query of the page.
    href: String,
    /// The section the route names.
    section: String,
) -> Result<Shown, ViewerError> {
    crate::request::visiting(&href).await?;
    Ok(read_page(section).await)
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`load`] reads, on the server.
#[expect(
    clippy::disallowed_types,
    reason = "the viewer reads the CDR's configuration JSON tree to draw its leaves"
)]
async fn read_page(section: String) -> Shown {
    if section == "viewer" {
        return Shown::Viewer(read_own());
    }
    match crate::bff::system::fetch_admin_config().await {
        Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
            Ok(tree) => {
                let mut all = Vec::new();
                leaves("", &tree, &mut all);
                Shown::Cdr(
                    all.into_iter()
                        .filter(|(key, _)| {
                            section_of(key.split('.').next().unwrap_or(key)) == section
                        })
                        .collect(),
                )
            }
            Err(error) => Shown::Refused(Refused::new(
                tr(Msg::ConfigRefused),
                ViewerError::Internal(error.to_string()),
            )),
        },
        Err(error) => Shown::Refused(Refused::new(tr(Msg::ConfigRefused), error)),
    }
}

/// Configuration.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn ConfigurationPage() -> impl IntoView {
    let section = crate::routes::params()
        .with(|params| params.get("section"))
        .filter(|section| SECTIONS.iter().any(|(key, _, _)| key == section))
        .unwrap_or_else(|| "deployment".to_owned());
    let model = page_model!(load(section.clone()));
    let page = drawn(model, move |shown| {
        let viewer = section == "viewer";
        let (settings, trouble) = match shown {
            Shown::Cdr(rows) => (
                rows.iter()
                    .map(|(key, value)| setting(key, value))
                    .collect(),
                None,
            ),
            Shown::Viewer(own) => (viewer_settings(own), None),
            Shown::Refused(refusal) => (Vec::new(), Some(refusal.view())),
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
                subtitle_view: None,
            },
            actions: ActionGroup::none(),
            source: Source::File {
                file: if viewer {
                    "ferroehr-viewer.toml".to_owned()
                } else {
                    "ferroehr.toml".to_owned()
                },
                how: How::Dialog(HOW_DIALOG.into()),
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
            {how_dialog()}
        }
        .into_any()
    });
    view! {
        <Title text=tr(Msg::NavConfiguration) />
        {page}
    }
}
