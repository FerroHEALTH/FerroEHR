// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one configuration loader, over the family's `ferrohealth-config`.
//!
//! [`loader`] names what is FerroEHR's own: the `FERROEHR` namespace, the
//! fallbacks below the canonical `FERROEHR__` forms (`DATABASE_URL_UNPOOLED`,
//! `DATABASE_URL` and the libpq set for `db.url`, `RUST_LOG` for
//! `telemetry.filter`, `PORT` for `server.listen`), and the names under the prefix
//! that set no key. [`load_process`] and [`assemble_from`] then resolve every `*_file`
//! secret sibling. No openEHR spec governs configuration — our own design.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use crate::db::DbConfig;
use crate::db::domain::Domain;
use crate::extensions::events::config::EventsConfig;
use ferrohealth_config::libpq;
use ferrohealth_config::loader::{Loaded, Loader};
use ferrohealth_secret::file::{SecretError, read_secret, read_secret_file};
use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_usage_report::section::{CI_VARIABLES, ci_of};

use super::FerroEhrConfig;

/// Names under the `FERROEHR_` prefix that set no configuration key and pass
/// the strict sweep: the build-time variables and
/// the Compose parameters that can reach the container environment.
const RESERVED: &[&str] = &[
    "FERROEHR_BUILD_EPOCH",
    "FERROEHR_RUSTC",
    "FERROEHR_IMAGE",
    "FERROEHR_POSTGRES_IMAGE",
    "FERROEHR_VIEWER_IMAGE",
    "FERROEHR_EHRBASE_IMAGE",
    "FERROEHR_EHRBASE_POSTGRES_IMAGE",
    "FERROEHR_PORT",
    "FERROEHR_DB_PORT",
    "FERROEHR_S3_PORT",
    "FERROEHR_VIEWER_PORT",
    "FERROEHR_TERMINOLOGY_PORT",
    "FERROEHR_PGP_PORT",
    "FERROEHR_PGP_DB_PORT",
    "FERROEHR_EHRBASE_PORT",
    "FERROEHR_CPUS",
    "FERROEHR_MEM",
    "FERROEHR_DB_CPUS",
    "FERROEHR_DB_MEM",
    "FERROEHR_TERMINOLOGY_CPUS",
    "FERROEHR_TERMINOLOGY_MEM",
];

/// One configuration error.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The file or the environment could not be read into the tree.
    #[error(transparent)]
    Load(#[from] ferrohealth_config::error::ConfigError),
    /// A `*_file` secret sibling could not be resolved.
    #[error(transparent)]
    Secret(#[from] SecretError),
    /// A semantic (cross-field) validation failure.
    #[error("{0}")]
    Semantic(String),
}

impl ConfigError {
    /// A semantic (cross-field) validation failure.
    #[must_use]
    pub const fn semantic(message: String) -> Self {
        Self::Semantic(message)
    }
}

/// A non-empty batch of configuration errors, rendered as one block so an
/// operator fixes everything in one iteration.
#[derive(Debug)]
pub struct ConfigErrors(pub Vec<ConfigError>);

impl std::fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{} configuration error(s):", self.0.len())?;
        for e in &self.0 {
            writeln!(f, "  - {e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ConfigErrors {
    // NOTE: RFC 0201 (`Error::source`); a batch of one names its error as the
    // cause, so the chain reaches the root, and a longer batch has no single cause.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self.0.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }
}

impl From<ConfigError> for ConfigErrors {
    fn from(error: ConfigError) -> Self {
        Self(vec![error])
    }
}

/// The loader of the `FERROEHR` namespace.
///
/// FerroEHR's `[server]` defaults (every interface, port 8080, a 64 MiB body
/// ceiling) sit under every source, so a partial `[server]` table keeps the
/// product's value for each key it does not name.
///
/// The fallbacks sit below the canonical `FERROEHR__` form of their key, and
/// a later fallback of the same key wins over an earlier one: for `db.url`
/// the direct `DATABASE_URL_UNPOOLED` wins over `DATABASE_URL`, which wins
/// over the URL composed from the libpq set, because a transaction pooler
/// drops the session `search_path`.
///
/// # Errors
/// [`ConfigError::Load`] if the prefix is refused or the `[server]` defaults
/// do not serialize to a table, neither of which happens.
pub fn loader() -> Result<Loader, ConfigError> {
    let loader = Loader::new("FERROEHR")?
        .defaults("server", &FerroEhrConfig::default().server)?
        .fallback_composed(libpq::VARIABLES, "db.url", libpq::url)
        .fallback("DATABASE_URL", "db.url", None)
        .fallback("DATABASE_URL_UNPOOLED", "db.url", None)
        .fallback("RUST_LOG", "telemetry.filter", None)
        .fallback("PORT", "server.listen", Some(all_interfaces));
    Ok(RESERVED
        .iter()
        .fold(loader, |loader, name| loader.reserve(*name)))
}

/// The all-interfaces bind of the platform's `PORT`.
fn all_interfaces(port: &str) -> String {
    format!("0.0.0.0:{port}")
}

/// Reads the configuration of the running process: the file `cli_config` or
/// `FERROEHR_CONFIG` names, when either does, the process environment, and
/// `overrides` last.
///
/// # Errors
/// [`ConfigErrors`] for any refusal of the loader or of a `*_file` secret.
pub fn load_process(
    cli_config: Option<&Path>,
    overrides: &[(String, String)],
) -> Result<(Loaded<FerroEhrConfig>, FerroEhrConfig), ConfigErrors> {
    let loaded = loader()?
        .load_with::<FerroEhrConfig>(cli_config, overrides)
        .map_err(ConfigError::from)?;
    let config = finish(loaded.settings().clone())?;
    Ok((loaded, config))
}

/// Returns the CI variable of the process environment that switches the usage
/// report off, by `ferrohealth-usage-report`'s rule (`GITHUB_ACTIONS` or `CI`
/// set to `true` or `1`).
#[must_use]
pub fn usage_report_ci() -> Option<&'static str> {
    // NOTE: no specification governs this: our own design; a CI variable whose
    // name or value is not UTF-8 cannot read `true`, so it is legitimately absent.
    let ci: BTreeMap<String, String> = std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| CI_VARIABLES.contains(&name.as_str()))
        .collect();
    ci_of(|name| ci.get(name).cloned())
}

/// Reads the configuration from explicit inputs, the seam every test drives:
/// the file `file` names, the environment `env`, and `overrides` last.
///
/// # Errors
/// [`ConfigErrors`] for an unreadable file and for any refusal of the loader
/// or of a `*_file` secret.
pub fn assemble_from<S: std::hash::BuildHasher>(
    file: Option<&Path>,
    env: &HashMap<String, String, S>,
    overrides: &[(String, String)],
) -> Result<(Loaded<FerroEhrConfig>, FerroEhrConfig), ConfigErrors> {
    let text = file
        .map(std::fs::read_to_string)
        .transpose()
        .map_err(|source| {
            ConfigError::Load(ferrohealth_config::error::ConfigError::Read {
                path: file.map(Path::to_path_buf).unwrap_or_default(),
                source,
            })
        })?;
    let environment: BTreeMap<String, String> = env
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let loaded = loader()?
        .load_sources_with::<FerroEhrConfig>(text.as_deref(), file, &environment, overrides)
        .map_err(ConfigError::from)?;
    let config = finish(loaded.settings().clone())?;
    Ok((loaded, config))
}

/// Resolves every `*_file` secret sibling of `config`.
fn finish(mut config: FerroEhrConfig) -> Result<FerroEhrConfig, ConfigErrors> {
    let mut errors = Vec::new();
    resolve_secret_files(&mut config, &mut errors);
    if errors.is_empty() {
        Ok(config)
    } else {
        Err(ConfigErrors(errors))
    }
}

/// Resolves every `*_file` sibling into its secret field.
///
/// The key-or-file rule is `ferrohealth-secret`'s: a file's trailing line
/// ending is removed, an empty file is refused, and both set is refused.
fn resolve_secret_files(config: &mut FerroEhrConfig, errors: &mut Vec<ConfigError>) {
    resolve(
        "signing.key_passphrase",
        &mut config.signing.key_passphrase,
        config.signing.key_passphrase_file.take().as_deref(),
        errors,
    );
    resolve(
        "demographic.identifier_protection.key",
        &mut config.demographic.identifier_protection.key,
        config
            .demographic
            .identifier_protection
            .key_file
            .take()
            .as_deref(),
        errors,
    );
    resolve(
        "multimedia.secret_access_key",
        &mut config.multimedia.secret_access_key,
        config.multimedia.secret_access_key_file.take().as_deref(),
        errors,
    );
    if let Some(basic) = config.auth.basic.as_mut() {
        for user in &mut basic.users {
            let key = format!("auth.basic.users[{:?}].password_hash", user.user);
            let file = user.password_hash_file.take();
            resolve(&key, &mut user.password_hash, file.as_deref(), errors);
        }
    }

    // The credential-bearing URLs with a non-empty dev default: "the operator
    // set it" means "it differs from that default".
    let default_dsn = DbConfig::default().url;
    let file = config.db.url_file.take();
    resolve_defaulted(
        "db.url",
        &mut config.db.url,
        &default_dsn,
        file.as_deref(),
        errors,
    );
    for domain in Domain::ALL {
        let settings = config.storage.domain_mut(domain);
        let file = settings.url_file.take();
        resolve(
            &format!("storage.{domain}.url"),
            &mut settings.url,
            file.as_deref(),
            errors,
        );
        let migrator = config.storage.migrate.domain_mut(domain);
        let file = migrator.url_file.take();
        resolve(
            &format!("storage.migrate.{domain}.url"),
            &mut migrator.url,
            file.as_deref(),
            errors,
        );
    }
    resolve(
        "db.migrate_url",
        &mut config.db.migrate_url,
        config.db.migrate_url_file.take().as_deref(),
        errors,
    );
    let default_broker = EventsConfig::default().url;
    let file = config.events.url_file.take();
    resolve_defaulted(
        "events.url",
        &mut config.events.url,
        &default_broker,
        file.as_deref(),
        errors,
    );
}

/// Resolves the optional secret `key` from its inline value or its file.
fn resolve<T: From<secrecy::SecretString>>(
    key: &str,
    target: &mut Option<T>,
    file: Option<&Path>,
    errors: &mut Vec<ConfigError>,
) {
    match read_secret(key, target.take(), file) {
        Ok(value) => *target = value,
        Err(error) => errors.push(error.into()),
    }
}

/// Resolves the URL `key`, whose unset state is its own default value, from
/// its file.
fn resolve_defaulted(
    key: &str,
    target: &mut SecretUrl,
    default: &SecretUrl,
    file: Option<&Path>,
    errors: &mut Vec<ConfigError>,
) {
    let Some(path) = file else { return };
    if target.expose() != default.expose() {
        errors.push(
            SecretError::Both {
                key: key.to_owned(),
            }
            .into(),
        );
        return;
    }
    match read_secret_file(key, path) {
        Ok(url) => *target = SecretUrl::from(url),
        Err(error) => errors.push(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    /// With no source, `[server]` takes FerroEHR's defaults: every interface
    /// at port 8080 and a 64 MiB body ceiling.
    #[test]
    fn the_server_defaults_are_the_products() {
        let (loaded, config) = assemble_from(None, &env(&[]), &[]).expect("zero-config load");
        assert_eq!(config.server.listen.to_string(), "0.0.0.0:8080");
        assert_eq!(config.server.body_limit_bytes, 64 * 1024 * 1024);
        assert_eq!(
            loaded.provenance().source("server.body_limit_bytes"),
            ferrohealth_config::provenance::Source::Default
        );
    }

    /// A partial `[server]` table keeps the product's value for every key it
    /// does not name, and the value it names wins.
    #[test]
    fn a_partial_server_table_keeps_the_product_defaults() {
        let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp");
        assert_fs::prelude::FileWriteStr::write_str(
            &file,
            "[server]\nlisten = \"10.0.0.5:9090\"\nrequest_timeout_ms = 5000\n",
        )
        .expect("write");
        let (_, config) =
            assemble_from(Some(file.path()), &env(&[]), &[]).expect("a partial table loads");
        assert_eq!(config.server.listen.to_string(), "10.0.0.5:9090");
        assert_eq!(config.server.request_timeout_ms, 5000);
        assert_eq!(config.server.body_limit_bytes, 64 * 1024 * 1024);

        let (_, config) = assemble_from(
            Some(file.path()),
            &env(&[("FERROEHR__SERVER__BODY_LIMIT_BYTES", "1048576")]),
            &[],
        )
        .expect("an override loads");
        assert_eq!(config.server.body_limit_bytes, 1_048_576);
    }

    /// An unknown key set from the environment names its full dotted path and
    /// the variable that set it.
    #[test]
    fn an_env_sourced_unknown_key_names_its_section_and_the_variable() {
        let err = assemble_from(
            None,
            &env(&[("FERROEHR__AUTHZ__RBAC__ENABLD", "false")]),
            &[],
        )
        .expect_err("the unknown key is refused");
        let text = err.to_string();
        assert!(
            text.contains("authz.rbac.enabld"),
            "the full section path is named: {text}"
        );
        assert!(
            text.contains("FERROEHR__AUTHZ__RBAC__ENABLD"),
            "the environment spelling is named: {text}"
        );
    }

    /// A misspelt key keeps its did-you-mean under its full path.
    #[test]
    fn the_did_you_mean_hint_names_the_full_path() {
        let err = assemble_from(None, &env(&[("FERROEHR__AUTH__ISSER", "https://idp")]), &[])
            .expect_err("the unknown key is refused");
        let text = err.to_string();
        assert!(text.contains("auth.isser"), "{text}");
        assert!(text.contains("auth.issuer"), "{text}");
    }

    /// An unknown key in the file is named by its own dotted path, whether it
    /// is a key of a known section, a table inside one, or a section of its
    /// own, never by a doubled path such as `rest.rest`, and the refusal is
    /// the same on every run (#4380).
    #[test]
    fn an_unknown_file_key_is_named_as_itself() {
        for (text, key, doubled) in [
            (
                "[rest]\nno_such_key = true\n",
                "rest.no_such_key",
                "rest.rest",
            ),
            (
                "[server.no_such_table]\nx = 1\n",
                "server.no_such_table",
                "no_such_table.no_such_table",
            ),
            (
                "[no_such_section]\nx = 1\n",
                "no_such_section",
                "no_such_section.no_such_section",
            ),
        ] {
            let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
            assert_fs::prelude::FileWriteStr::write_str(&file, text).expect("write the config");
            let refuse = || {
                assemble_from(Some(file.path()), &env(&[]), &[])
                    .expect_err("the unknown key is refused")
                    .to_string()
            };
            let first = refuse();
            assert!(first.contains(key), "{key} is named: {first}");
            assert!(!first.contains(doubled), "no doubled path: {first}");
            assert_eq!(first, refuse(), "the refusal is deterministic");
        }
    }

    /// Every name the loader passes without setting a key is accepted.
    #[test]
    fn the_reserved_names_pass_the_sweep() {
        let pairs: Vec<(&str, &str)> = RESERVED.iter().map(|name| (*name, "1")).collect();
        assemble_from(None, &env(&pairs), &[]).expect("every reserved name passes");
    }

    // ── The shipped Compose artifacts must pass this very sweep ───────────────
    //
    // A Compose file that spells a reserved-namespace variable any other way
    // produces a container that CANNOT BOOT, and the failure surfaces only at
    // `docker compose up`. This guard walks the committed YAML and runs the real
    // sweep over every variable the CDR service sets, so the drift fails in the
    // test suite instead. No openEHR spec governs configuration or Compose —
    // our own design.

    /// The compose service that runs the CDR binary this crate configures.
    /// Every other service (the database, the viewer with its own
    /// `FERROEHR_VIEWER__…` namespace, an upstream SUT) is unrelated.
    const CDR_SERVICE: &str = "ferroehr";

    /// The repository root — this crate lives at `app/ferroehr`.
    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn is_yaml(path: &Path) -> bool {
        matches!(
            Path::extension(path).and_then(std::ffi::OsStr::to_str),
            Some("yml" | "yaml")
        )
    }

    /// Whether the file is a Compose file at all (as opposed to a Prometheus /
    /// Grafana asset living beside one).
    fn declares_services(path: &Path) -> bool {
        std::fs::read_to_string(path)
            .is_ok_and(|yaml| yaml.lines().any(|line| line.trim_end() == "services:"))
    }

    /// Every committed Compose artifact: the root `docker-compose*.yml` files
    /// plus every `services:`-bearing YAML under `docker/`.
    fn compose_files() -> Vec<PathBuf> {
        let root = repo_root();
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&root).expect("read repo root").flatten() {
            let path = entry.path();
            let is_root_compose = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.starts_with("docker-compose"));
            if is_root_compose && is_yaml(&path) {
                files.push(path);
            }
        }
        let mut dirs = vec![root.join("docker")];
        while let Some(dir) = dirs.pop() {
            let Ok(entries) = std::fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if is_yaml(&path) && declares_services(&path) {
                    files.push(path);
                }
            }
        }
        files.sort();
        files
    }

    /// `(service, 1-based line, variable name)` for every `environment:` entry
    /// in a Compose file. Parsed by indentation, which is all this guard needs:
    /// these files are uniformly two-space indented, so a service name sits at
    /// indent 2, its `environment:` key at 4, and the entries below that — in
    /// either the mapping (`NAME: value`) or the list (`- NAME=value`) form
    /// (docs.docker.com/reference/compose-file/services/#environment).
    fn env_entries(yaml: &str) -> Vec<(String, usize, String)> {
        let mut out = Vec::new();
        let mut in_services = false;
        let mut service = String::new();
        let mut in_env = false;
        for (index, raw) in yaml.lines().enumerate() {
            let line = raw.trim_end();
            let body = line.trim_start();
            if body.is_empty() || body.starts_with('#') {
                continue;
            }
            match line.len() - body.len() {
                0 => {
                    in_services = body == "services:";
                    service.clear();
                    in_env = false;
                }
                2 => {
                    service = body.trim_end_matches(':').to_owned();
                    in_env = false;
                }
                4 => in_env = body == "environment:",
                _ => {
                    if in_services
                        && in_env
                        && !service.is_empty()
                        && let Some(name) = entry_name(body)
                    {
                        out.push((service.clone(), index + 1, name));
                    }
                }
            }
        }
        out
    }

    /// The variable name of one `environment:` entry, in either form.
    fn entry_name(entry: &str) -> Option<String> {
        let name = match entry.strip_prefix("- ") {
            // List form: `- NAME=value`, or a bare `- NAME` host pass-through.
            Some(item) => item.trim().split_once('=').map_or(item.trim(), |(n, _)| n),
            // Mapping form: `NAME: value`.
            None => entry.split_once(':')?.0,
        };
        let name = name.trim();
        if name.is_empty() {
            None
        } else {
            Some(name.to_owned())
        }
    }

    /// The top-level keys of the tree, the sections a `FERROEHR__` variable
    /// may address.
    fn sections() -> Vec<String> {
        match serde_json::to_value(FerroEhrConfig::default()) {
            Ok(serde_json::Value::Object(members)) => members.keys().cloned().collect(),
            other => panic!("the tree serializes as an object: {other:?}"),
        }
    }

    #[test]
    fn shipped_compose_env_passes_the_strict_sweep() {
        let files = compose_files();
        assert!(
            files.len() >= 2,
            "no Compose artifacts discovered — the guard is blind: {files:?}"
        );

        let mut cdr_entries = Vec::new();
        for path in &files {
            let yaml = std::fs::read_to_string(path).expect("read compose file");
            for (service, line, name) in env_entries(&yaml) {
                if service == CDR_SERVICE {
                    cdr_entries.push((path.clone(), line, name));
                }
            }
        }

        assert!(
            cdr_entries
                .iter()
                .any(|(_, _, name)| name == "FERROEHR__DB__URL"),
            "the CDR service's DSN variable was not extracted — the guard is \
             blind (Compose layout or service name drift): {cdr_entries:?}"
        );

        let loader = loader().expect("the loader");
        let config_variable = loader.config_variable();
        let mut known: Vec<&str> = RESERVED.to_vec();
        known.push(config_variable.as_str());
        known.extend(libpq::VARIABLES);
        known.extend(["DATABASE_URL", "DATABASE_URL_UNPOOLED", "RUST_LOG", "PORT"]);
        let sections = sections();
        let mut failures = Vec::new();
        for (path, line, name) in &cdr_entries {
            let one = BTreeMap::from([(name.clone(), String::new())]);
            for error in ferrohealth_config::strict::sweep("FERROEHR", &one, &known) {
                failures.push(format!("{}:{line}: {error}", path.display()));
            }
            if let Some(tail) = name.strip_prefix("FERROEHR__") {
                let section = tail
                    .split("__")
                    .next()
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if !sections.contains(&section) {
                    failures.push(format!(
                        "{}:{line}: {name} names no section of the configuration",
                        path.display()
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "Compose sets variables the boot-time sweep rejects, so the \
             composed server cannot start:\n{}",
            failures.join("\n")
        );
    }

    /// One inline server configuration of the quickstart: the `content` of
    /// the compose config `name` in `docker-compose.yml`, with the compose
    /// `$$` escape read back as `$`
    /// (docs.docker.com/reference/compose-file/configs).
    fn quickstart_config(name: &str) -> FerroEhrConfig {
        let yaml = std::fs::read_to_string(repo_root().join("docker-compose.yml"))
            .expect("read docker-compose.yml");
        let opening = format!("  {name}:");
        let mut lines = yaml
            .lines()
            .skip_while(|line| line.trim_end() != opening)
            .skip(1);
        assert_eq!(
            lines.next().map(str::trim_end),
            Some("    content: |"),
            "{name} is an inline content block"
        );
        let text = lines
            .take_while(|line| line.is_empty() || line.starts_with("      "))
            .map(|line| {
                line.strip_prefix("      ")
                    .unwrap_or(line)
                    .replace("$$", "$")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
        assert_fs::prelude::FileWriteStr::write_str(&file, &text).expect("write the config");
        let (_, config) = assemble_from(Some(file.path()), &env(&[]), &[])
            .unwrap_or_else(|e| panic!("{name} assembles: {e}"));
        config
            .validate()
            .unwrap_or_else(|e| panic!("{name} validates: {e}"));
        config
    }

    /// The quickstart's two configurations, the one `FERROEHR_EHR_INDEX=local`
    /// selects and the default, both validate and differ in the EHR index and
    /// the pseudonym namespaces alone (#4379).
    #[test]
    fn the_quickstart_local_index_differs_in_the_ehr_index_alone() {
        use crate::service::ehr_index::config::{EhrIndexConfig, ProviderKind};

        let mut ihe = quickstart_config("ferroehr-config");
        let mut local = quickstart_config("ferroehr-config-local");
        assert_eq!(ihe.ehr_index.provider, ProviderKind::Ihe);
        assert_eq!(ihe.privacy.subject_namespaces, ["urn:oid:2.999.7.1"]);
        assert_eq!(local.ehr_index.provider, ProviderKind::Local);
        assert!(
            local.privacy.subject_namespaces.is_empty(),
            "no pseudonym namespace is declared on the local index"
        );
        for config in [&mut ihe, &mut local] {
            config.ehr_index = EhrIndexConfig::default();
            config.privacy.subject_namespaces.clear();
        }
        assert_eq!(
            serde_json::to_value(&ihe).expect("render"),
            serde_json::to_value(&local).expect("render"),
            "the two quickstart configurations differ beyond [ehr_index] and \
             [privacy] subject_namespaces"
        );
    }

    /// `FERROEHR__EHR_INDEX__PROVIDER=local` alone switches a configuration
    /// written for an IHE patient identity source to the local index: the IHE
    /// keys are kept and named as unused (#4379).
    #[test]
    fn the_provider_variable_alone_switches_the_ehr_index() {
        let file = assert_fs::NamedTempFile::new("ferroehr.toml").expect("temp file");
        assert_fs::prelude::FileWriteStr::write_str(
            &file,
            "[privacy]\nsubject_namespaces = [\"urn:oid:2.999.7.1\"]\n\n\
             [ehr_index]\nprovider = \"ihe\"\npixm_url = \"http://localhost:8090/fhir\"\n\
             assigning_authority = \"urn:oid:2.999.7.1\"\n",
        )
        .expect("write the config");
        let (_, config) = assemble_from(
            Some(file.path()),
            &env(&[("FERROEHR__EHR_INDEX__PROVIDER", "local")]),
            &[],
        )
        .expect("the switched config assembles");
        config.validate().expect("the switched config validates");
        assert_eq!(
            config.ehr_index.unused_keys(),
            ["ehr_index.pixm_url", "ehr_index.assigning_authority"]
        );
        // The declared namespace stays in force under the local index, so every
        // subject reference is still a pseudonym (no mixed mode, #4379).
        let policy =
            crate::privacy::PrivacyPolicy::compile(&config.privacy).expect("the policy compiles");
        assert!(policy.subject_rule_in_force());
        assert!(!policy.admits_subject("benchmark", "1793131381"));
    }
}
