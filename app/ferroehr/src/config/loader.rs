// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one configuration loader.
//!
//! Performs file discovery plus the pure `config`-crate assembly
//! (defaults < file < env < `--set`) with the strict passes, the two
//! conventional aliases, error enrichment, and `*_file` secret resolution.
//! No openEHR spec governs configuration — our own design.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::config::secret::Secret;
use crate::config::secret::SecretUrl;
use crate::db::DbConfig;
use crate::db::domain::Domain;
use crate::extensions::events::config::EventsConfig;
use crate::extensions::fhir::config::FhirOutboundConfig;
use config::{Config, Environment, File, FileFormat};

use super::FerroEhrConfig;
use super::alias::{CONVENTIONAL, LIST_KEYS};
use super::strict;

/// One configuration error, rendered as a single human line.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ConfigError {
    message: String,
}

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// A semantic (cross-field) validation failure.
    #[must_use]
    pub fn semantic(message: String) -> Self {
        Self::new(message)
    }

    /// An unknown variable in the reserved `FERROEHR_` namespace.
    #[must_use]
    pub fn unknown_env(var: &str, suggestion: Option<String>) -> Self {
        let hint = suggestion.map_or_else(String::new, |s| {
            format!(
                " — did you mean the `{s}` section (FERROEHR__{}__…)?",
                s.to_ascii_uppercase()
            )
        });
        Self::new(format!(
            "unknown configuration environment variable `{var}`{hint}"
        ))
    }

    /// A near-miss in the reserved `FERROEHR_` namespace: the right section but
    /// the old single-`_` prefix. Suggest the exact uniform spelling.
    #[must_use]
    pub fn near_miss_env(var: &str, uniform: &str) -> Self {
        Self::new(format!(
            "unknown configuration environment variable `{var}` — did you mean `{uniform}`?"
        ))
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

impl std::error::Error for ConfigErrors {}

/// File discovery: `--config` → `FERROEHR_CONFIG` → `./ferroehr.toml` →
/// `/etc/ferroehr/ferroehr.toml`. An explicitly-pointed-at file must exist; the
/// search-order files are optional.
///
/// # Errors
/// [`ConfigErrors`] if an explicit path is missing/unreadable.
pub fn discover_file<S: std::hash::BuildHasher>(
    cli_config: Option<&Path>,
    env: &HashMap<String, String, S>,
) -> Result<Option<PathBuf>, ConfigErrors> {
    if let Some(path) = cli_config {
        return require(path);
    }
    if let Some(path) = env.get("FERROEHR_CONFIG") {
        return require(Path::new(path));
    }
    for candidate in ["./ferroehr.toml", "/etc/ferroehr/ferroehr.toml"] {
        let path = Path::new(candidate);
        if path.is_file() {
            return Ok(Some(path.to_path_buf()));
        }
    }
    Ok(None)
}

fn require(path: &Path) -> Result<Option<PathBuf>, ConfigErrors> {
    if path.is_file() {
        Ok(Some(path.to_path_buf()))
    } else {
        Err(ConfigErrors(vec![ConfigError::new(format!(
            "config file not found or unreadable: {}",
            path.display()
        ))]))
    }
}

/// The pure assembly seam: file + env + CLI overrides folded into one
/// validated [`FerroEhrConfig`], strictly (unknown keys rejected).
///
/// # Errors
/// [`ConfigErrors`] collecting every problem found in one pass: an unreadable
/// file, TOML parse errors, unknown/misspelled keys or env vars, type
/// mismatches, and cross-field validation failures.
pub fn assemble<S: std::hash::BuildHasher>(
    file: Option<&Path>,
    env: &HashMap<String, String, S>,
    overrides: &[(String, String)],
) -> Result<FerroEhrConfig, ConfigErrors> {
    let mut errors = strict::strict_env(env);
    let file_content = read_config_file(file, &mut errors);
    let alias_map = conventional_aliases(env);

    // The canonical (uniform-grammar) `FERROEHR__…` variables. Allowlisted
    // infra names never carry the double prefix, so the prefix check alone
    // selects the canonical set.
    let real_map: HashMap<String, String> = env
        .iter()
        .filter(|(key, _)| key.starts_with("FERROEHR__"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();

    let mut builder = Config::builder();
    if let Some(content) = &file_content {
        builder = builder.add_source(File::from_str(content, FileFormat::Toml));
    }
    builder = builder.add_source(env_source(alias_map));
    builder = builder.add_source(env_source(real_map));
    for (key, value) in overrides {
        // `set_override` consumes the builder; clone so a rejected key keeps
        // the builder alive for the remaining overrides (errors aggregate —
        // the strict pass reports them all at once).
        match builder.clone().set_override(key.clone(), value.clone()) {
            Ok(next) => builder = next,
            Err(e) => errors.push(ConfigError::new(format!("--set {key}={value}: {e}"))),
        }
    }

    let config = match builder.build() {
        // `serde_path_to_error` wraps the deserializer so a refusal names the
        // SECTION PATH ("auth.oidc"), which plain serde errors do not carry —
        // the ingredient that lets `enrich` attribute the right file line, or
        // honestly say the key came from the environment.
        Ok(built) => match serde_path_to_error::deserialize::<_, FerroEhrConfig>(built) {
            Ok(config) => Some(config),
            Err(e) => {
                let path = e.path().to_string();
                errors.push(enrich(&path, &e.to_string(), file_content.as_deref()));
                None
            }
        },
        Err(e) => {
            errors.push(ConfigError::new(format!("assembling configuration: {e}")));
            None
        }
    };

    let Some(mut config) = config else {
        return Err(ConfigErrors(errors));
    };

    resolve_secret_files(&mut config, &mut errors);

    if errors.is_empty() {
        Ok(config)
    } else {
        Err(ConfigErrors(errors))
    }
}

/// Reads the configuration file, recording an unreadable path as an error
/// rather than aborting the pass (every problem is reported at once).
fn read_config_file(file: Option<&Path>, errors: &mut Vec<ConfigError>) -> Option<String> {
    let path = file?;
    match std::fs::read_to_string(path) {
        Ok(content) => Some(content),
        Err(e) => {
            errors.push(ConfigError::new(format!(
                "reading config file {}: {e}",
                path.display()
            )));
            None
        }
    }
}

/// The conventional (non-`FERROEHR__`) environment aliases, mapped onto their
/// canonical keys.
///
/// This whole layer sits BELOW the canonical source, so an `FERROEHR__` form
/// always wins. Within the layer, a transaction-pooled endpoint (pgbouncer)
/// hands statements to backend connections without the session `search_path`,
/// which surfaces as intermittent 42P01 "relation does not exist" — so
/// where a managed-Postgres integration offers the DIRECT endpoint beside the
/// pooled one (Neon injects both), the direct one wins.
fn conventional_aliases<S: std::hash::BuildHasher>(
    env: &HashMap<String, String, S>,
) -> HashMap<String, String> {
    let mut alias_map: HashMap<String, String> = HashMap::new();
    if let Some(value) = env.get("DATABASE_URL_UNPOOLED") {
        alias_map.insert("FERROEHR__DB__URL".to_owned(), value.clone());
    }
    for (external, canonical) in CONVENTIONAL {
        if let Some(value) = env.get(*external) {
            alias_map
                .entry((*canonical).to_owned())
                .or_insert_with(|| value.clone());
        }
    }
    // The PaaS port convention: container platforms (Vercel, Cloud Run,
    // Heroku) inject `PORT` and route traffic to it. It carries only the port
    // half, so it maps to an all-interfaces bind rather than joining the 1:1
    // CONVENTIONAL table — still layered BELOW `FERROEHR__SERVER__BIND`.
    if let Some(port) = env.get("PORT") {
        alias_map
            .entry("FERROEHR__SERVER__BIND".to_owned())
            .or_insert_with(|| format!("0.0.0.0:{port}"));
    }
    if let Some(dsn) = libpq_dsn(env) {
        // `or_insert`: a `DATABASE_URL` already claimed this entry in the
        // CONVENTIONAL loop above, and it outranks the assembled form.
        alias_map
            .entry("FERROEHR__DB__URL".to_owned())
            .or_insert(dsn);
    }
    alias_map
}

/// A DSN assembled from the libpq environment convention (PostgreSQL's own
/// variable set, <https://www.postgresql.org/docs/current/libpq-envars.html>),
/// which managed Postgres integrations (Neon on Vercel among them) inject
/// instead of a URL.
///
/// The direct `PGHOST_UNPOOLED` host is preferred over the pooled `PGHOST`
/// (the same transaction-pooling reason as `DATABASE_URL_UNPOOLED`).
fn libpq_dsn<S: std::hash::BuildHasher>(env: &HashMap<String, String, S>) -> Option<String> {
    let host = env.get("PGHOST_UNPOOLED").or_else(|| env.get("PGHOST"))?;
    let mut dsn = String::from("postgres://");
    if let Some(user) = env.get("PGUSER") {
        dsn.push_str(&urlencoding::encode(user));
        if let Some(password) = env.get("PGPASSWORD") {
            dsn.push(':');
            dsn.push_str(&urlencoding::encode(password));
        }
        dsn.push('@');
    }
    dsn.push_str(host);
    if let Some(port) = env.get("PGPORT") {
        dsn.push(':');
        dsn.push_str(port);
    }
    dsn.push('/');
    dsn.push_str(env.get("PGDATABASE").map_or("postgres", String::as_str));
    if let Some(sslmode) = env.get("PGSSLMODE") {
        dsn.push_str("?sslmode=");
        dsn.push_str(sslmode);
    }
    Some(dsn)
}

/// An `FERROEHR_`-prefixed environment source over an injected map (the hermetic
/// seam) with the `__` grammar, typed scalars, and comma-separated lists.
fn env_source(map: HashMap<String, String>) -> Environment {
    let mut env = Environment::with_prefix("FERROEHR")
        .separator("__")
        .try_parsing(true)
        .list_separator(",")
        .source(Some(map.into_iter().collect()));
    for key in LIST_KEYS {
        env = env.with_list_parse_key(key);
    }
    env
}

/// Enrich a `config`/serde deserialize error: surface the offending key under
/// its full section path with a did-you-mean (from serde's "expected one of"
/// list), and attribute a file line only when the key is defined under THAT
/// section — an env-sourced key is named as such instead of pointing the
/// operator at an unrelated valid line.
fn enrich(path: &str, err: &str, file: Option<&str>) -> ConfigError {
    let Some(field) = between(err, "unknown field `", "`") else {
        return ConfigError::new(format!("invalid configuration: {err}"));
    };
    let section = section_of(path);
    let full = if section.is_empty() {
        field.clone()
    } else {
        format!("{section}.{field}")
    };
    let candidates = expected_fields(err);
    let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
    let suggestion = strict::did_you_mean(&field, &refs);
    let line = file.and_then(|c| find_key_line_in_section(c, &section, &field));
    let loc = line.map_or_else(
        || {
            let env_form = env_form_of(&section, &field);
            format!(" (not in the configuration file — check the environment: {env_form})")
        },
        |l| format!(" (line {l})"),
    );
    let hint = suggestion.map_or_else(String::new, |s| format!("; did you mean `{s}`?"));
    ConfigError::new(format!("unknown configuration key `{full}`{loc}{hint}"))
}

/// The section half of a `serde_path_to_error` path: the path up to the failing
/// struct, with array indices dropped (`auth.basic.users[0]` → the TOML header
/// form `auth.basic.users`). `serde_path_to_error` reports the unknown FIELD in
/// the message, so the path IS the section.
fn section_of(path: &str) -> String {
    path.split('.')
        .map(|seg| seg.split_once('[').map_or(seg, |(name, _)| name))
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

/// The `FERROEHR__…` environment spelling of a section + field, so the
/// not-in-file diagnostic tells the operator exactly which variable to check.
fn env_form_of(section: &str, field: &str) -> String {
    let mut parts: Vec<String> = section
        .split('.')
        .filter(|s| !s.is_empty())
        .map(str::to_uppercase)
        .collect();
    parts.push(field.to_uppercase());
    format!("FERROEHR__{}", parts.join("__"))
}

/// The substring between `start` and the next `end`, if present.
fn between(haystack: &str, start: &str, end: &str) -> Option<String> {
    let rest = haystack.split_once(start)?.1;
    rest.split_once(end).map(|(inner, _)| inner.to_owned())
}

/// The "expected one of `a`, `b`, …" candidate list from a serde error.
fn expected_fields(err: &str) -> Vec<String> {
    let Some(after) = err.split_once("expected one of ").map(|(_, r)| r) else {
        return Vec::new();
    };
    after
        .split(',')
        .filter_map(|tok| between(tok, "`", "`"))
        .collect()
}

/// The 1-based line where `key` is assigned UNDER `section` (`[section]` or
/// `[[section]]` headers; the empty section is the top of the file before any
/// header), for `file:line` diagnostics. A bare name match anywhere else would
/// misattribute the diagnostic to an unrelated line.
fn find_key_line_in_section(content: &str, section: &str, key: &str) -> Option<usize> {
    let mut current = String::new();
    for (i, line) in content.lines().enumerate() {
        let trimmed = line.trim_start();
        if let Some(header) = trimmed
            .strip_prefix("[[")
            .and_then(|r| r.split_once("]]"))
            .map(|(h, _)| h)
            .or_else(|| {
                trimmed
                    .strip_prefix('[')
                    .and_then(|r| r.split_once(']'))
                    .map(|(h, _)| h)
            })
        {
            header.trim().clone_into(&mut current);
            continue;
        }
        let is_assign = trimmed
            .strip_prefix(key)
            .is_some_and(|r| r.trim_start().starts_with('='));
        if is_assign && current == section {
            return Some(i + 1);
        }
    }
    None
}

/// Resolve every `*_file` sibling into its `Secret`/string field immediately
/// after extraction: read the file, trim a trailing newline, and reject
/// when both the inline value and its `*_file` are set.
fn resolve_secret_files(config: &mut FerroEhrConfig, errors: &mut Vec<ConfigError>) {
    resolve_secret(
        "signing.key_passphrase",
        &mut config.signing.key_passphrase,
        config.signing.key_passphrase_file.take(),
        errors,
    );
    if let Some(oidc) = config.auth.oidc.as_mut() {
        resolve_secret(
            "auth.oidc.hmac_secret",
            &mut oidc.hmac_secret,
            oidc.hmac_secret_file.take(),
            errors,
        );
        // jwks_json is a plain (non-secret) string blob.
        if let Some(path) = oidc.jwks_json_file.take() {
            if oidc.jwks_json.is_some() {
                errors.push(ConfigError::new(
                    "set only one of auth.oidc.jwks_json / auth.oidc.jwks_json_file".to_owned(),
                ));
            } else {
                match read_trim(&path) {
                    Ok(s) => oidc.jwks_json = Some(s),
                    Err(e) => errors.push(e),
                }
            }
        }
    }
    resolve_secret(
        "demographic.identifier_protection.key",
        &mut config.demographic.identifier_protection.key,
        config.demographic.identifier_protection.key_file.take(),
        errors,
    );
    resolve_secret(
        "multimedia.secret_access_key",
        &mut config.multimedia.secret_access_key,
        config.multimedia.secret_access_key_file.take(),
        errors,
    );
    for (name, client) in &mut config.terminology.external.oauth2_clients {
        let file = client.client_secret_file.take();
        resolve_secret(
            &format!("terminology.external.oauth2_clients.{name}.client_secret"),
            &mut client.client_secret,
            file,
            errors,
        );
    }
    if let Some(basic) = config.auth.basic.as_mut() {
        for user in &mut basic.users {
            let file = user.password_hash_file.take();
            let key = format!("auth.basic.users[{:?}].password_hash", user.username);
            resolve_set_secret(&key, &mut user.password_hash, file, errors);
        }
    }

    // The credential-bearing URLs. Each has a non-empty dev default, so "the
    // operator set it" means "it differs from that default" — a default is not
    // a setting, and comparing against `Default::default()` cannot drift from
    // the `Default` impl the way a duplicated literal would.
    let default_dsn = DbConfig::default().url;
    resolve_secret_url(
        "db.url",
        &mut config.db.url,
        &default_dsn,
        config.db.url_file.take(),
        errors,
    );
    for domain in Domain::ALL {
        let settings = config.storage.domain_mut(domain);
        let file = settings.url_file.take();
        resolve_optional_secret_url(
            &format!("storage.{domain}.url"),
            &mut settings.url,
            file,
            errors,
        );
    }
    resolve_optional_secret_url(
        "db.migrate_url",
        &mut config.db.migrate_url,
        config.db.migrate_url_file.take(),
        errors,
    );
    let default_broker = EventsConfig::default().url;
    resolve_secret_url(
        "events.url",
        &mut config.events.url,
        &default_broker,
        config.events.url_file.take(),
        errors,
    );
    let default_fhir_broker = FhirOutboundConfig::default().url;
    resolve_secret_url(
        "fhir.outbound.url",
        &mut config.fhir.outbound.url,
        &default_fhir_broker,
        config.fhir.outbound.url_file.take(),
        errors,
    );
}

/// Read `path`'s contents (trailing newline trimmed) into `target` as a
/// [`Secret`], unless `target` is already set (both-set is an error).
fn resolve_secret(
    key: &str,
    target: &mut Option<Secret>,
    file: Option<PathBuf>,
    errors: &mut Vec<ConfigError>,
) {
    let Some(path) = file else { return };
    if target.is_some() {
        errors.push(ConfigError::new(format!(
            "set only one of {key} / {key}_file"
        )));
        return;
    }
    match read_trim(&path) {
        Ok(secret) => *target = Some(Secret::new(secret)),
        Err(e) => errors.push(e),
    }
}

/// Read `path`'s contents into a [`Secret`] field that is always present, using
/// emptiness as "unset".
///
/// The `Option<Secret>` fields above can say "the operator set this" with
/// `is_some`; a mandatory field cannot, so an empty value is the unset state.
fn resolve_set_secret(
    key: &str,
    target: &mut Secret,
    file: Option<PathBuf>,
    errors: &mut Vec<ConfigError>,
) {
    let Some(path) = file else { return };
    if !target.expose().is_empty() {
        errors.push(ConfigError::new(format!(
            "set only one of {key} / {key}_file"
        )));
        return;
    }
    match read_trim(&path) {
        Ok(secret) => *target = Secret::new(secret),
        Err(e) => errors.push(e),
    }
}

/// Read `path`'s contents into a [`SecretUrl`] field whose unset state is its
/// own default value.
fn resolve_secret_url(
    key: &str,
    target: &mut SecretUrl,
    default: &SecretUrl,
    file: Option<PathBuf>,
    errors: &mut Vec<ConfigError>,
) {
    let Some(path) = file else { return };
    if target.expose() != default.expose() {
        errors.push(ConfigError::new(format!(
            "set only one of {key} / {key}_file"
        )));
        return;
    }
    match read_trim(&path) {
        Ok(url) => *target = SecretUrl::new(url),
        Err(e) => errors.push(e),
    }
}

/// Read `path`'s contents into `target` as a [`SecretUrl`], unless `target` is
/// already set (both-set is an error).
///
/// The [`Option`] twin of [`resolve_secret_url`], for a URL with no built-in
/// default: "the operator set it" is simply "it is `Some`".
fn resolve_optional_secret_url(
    key: &str,
    target: &mut Option<SecretUrl>,
    file: Option<PathBuf>,
    errors: &mut Vec<ConfigError>,
) {
    let Some(path) = file else { return };
    if target.is_some() {
        errors.push(ConfigError::new(format!(
            "set only one of {key} / {key}_file"
        )));
        return;
    }
    match read_trim(&path) {
        Ok(url) => *target = Some(SecretUrl::new(url)),
        Err(e) => errors.push(e),
    }
}

fn read_trim(path: &Path) -> Result<String, ConfigError> {
    std::fs::read_to_string(path)
        .map(|s| s.trim_end_matches(['\r', '\n']).to_owned())
        .map_err(|e| ConfigError::new(format!("reading secret file {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_env_sourced_unknown_key_names_its_section_and_the_variable() {
        // FERROEHR__AUTH__OIDC__ENABLED builds a nested auth.oidc.enabled that
        // OidcConfig rejects: the diagnostic must name the full path and the
        // environment spelling, never a file line.
        let env: HashMap<String, String> = [(
            "FERROEHR__AUTH__OIDC__ENABLED".to_owned(),
            "false".to_owned(),
        )]
        .into();
        let err = assemble(None, &env, &[]).expect_err("the unknown key is refused");
        let text = err.to_string();
        assert!(
            text.contains("auth.oidc.enabled"),
            "the full section path is named: {text}"
        );
        assert!(
            text.contains("FERROEHR__AUTH__OIDC__ENABLED"),
            "the environment spelling is named: {text}"
        );
        assert!(!text.contains("(line "), "no file line exists: {text}");
    }

    #[test]
    fn a_file_line_is_attributed_only_under_the_owning_section() {
        // The misattribution twin: a bare `enabled =` exists earlier under
        // [admin]; the offender lives under [auth.oidc] and must get ITS line.
        let content =
            "[admin]\nenabled = true\n\n[auth.oidc]\nissuer = \"https://x\"\nbogus_key = 1\n";
        assert_eq!(
            find_key_line_in_section(content, "auth.oidc", "bogus_key"),
            Some(6)
        );
        assert_eq!(
            find_key_line_in_section(content, "admin", "enabled"),
            Some(2)
        );
        // The bare name from the WRONG section resolves to nothing.
        assert_eq!(
            find_key_line_in_section(content, "auth.oidc", "enabled"),
            None
        );
    }

    #[test]
    fn sections_normalize_array_indices_and_env_forms_uppercase() {
        assert_eq!(section_of("auth.basic.users[0]"), "auth.basic.users");
        assert_eq!(section_of(""), "");
        assert_eq!(
            env_form_of("auth.oidc", "enabled"),
            "FERROEHR__AUTH__OIDC__ENABLED"
        );
        assert_eq!(env_form_of("", "bogus"), "FERROEHR__BOGUS");
    }

    #[test]
    fn the_did_you_mean_hint_survives_the_path_rewrite() {
        let err = "unknown field `isser`, expected one of `issuer`, `audience`";
        let text = enrich("auth.oidc", err, None).to_string();
        assert!(text.contains("auth.oidc.isser"), "{text}");
        assert!(text.contains("did you mean `issuer`?"), "{text}");
    }
}
