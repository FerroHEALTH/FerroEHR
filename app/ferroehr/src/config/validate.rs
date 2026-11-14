// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The semantic validation of the assembled tree: every cross-field rule,
//! reported at once ([`FerroEhrConfig::validate`]), so an operator fixes the
//! configuration in one iteration. No openEHR spec governs configuration: our
//! own design.

use crate::config::loader::{ConfigError, ConfigErrors};
use crate::config::{FerroEhrConfig, auth, authz, deployment};
use crate::storage::cohort_predicate::PredicateKind;

impl FerroEhrConfig {
    /// Aggregated semantic validation: every cross-field rule reported at
    /// once, so an operator fixes the config in one iteration.
    ///
    /// # Errors
    /// [`ConfigErrors`] carrying every failing rule.
    pub fn validate(&self) -> Result<(), ConfigErrors> {
        let mut errors = Vec::new();
        self.validate_system_id(&mut errors);
        self.validate_base_path(&mut errors);
        self.validate_mechanisms(&mut errors);
        self.validate_signing(&mut errors);
        self.validate_key_sources(&mut errors);
        self.validate_privacy(&mut errors);
        self.validate_audit(&mut errors);
        self.validate_audit_syslog(&mut errors);
        self.validate_outbound_clients(&mut errors);
        if let Some(cors) = &self.api.cors {
            errors.extend(cors.errors().into_iter().map(ConfigError::semantic));
        }
        self.validate_cohort(&mut errors);
        self.validate_ehr_index(&mut errors);
        self.validate_deployment(&mut errors);
        self.validate_ehr_access(&mut errors);
        if self.usage_report.state_dir.is_some() {
            // NOTE: no specification governs this: our own design; FerroEHR keeps the
            // report's state in PostgreSQL, so a state directory would do nothing.
            errors.push(ConfigError::semantic(
                "usage_report.state_dir is not used: FerroEHR keeps the usage report state in \
                 the clinical database"
                    .to_owned(),
            ));
        }
        errors.extend(multimedia_endpoint_errors(&self.record.multimedia));
        errors.extend(
            self.record
                .change_feed
                .errors()
                .into_iter()
                .map(ConfigError::semantic),
        );
        // management.listen must not be the server.listen address.
        if let Some(listen) = self.management.listen
            && self.server.shared.listen.port() == listen.port()
            && (self.server.shared.listen.ip() == listen.ip()
                || self.server.shared.listen.ip().is_unspecified()
                || listen.ip().is_unspecified())
        {
            errors.push(ConfigError::semantic(format!(
                "management.listen ({listen}) must differ from server.listen ({})",
                self.server.shared.listen
            )));
        }
        self.validate_body_tier(&mut errors);
        if self.database.audit.listen_url.is_some() {
            // NOTE: our own design (#5589), every policy listener listens on the clinical or
            // the party database, so an audit listener DSN would connect nothing.
            errors.push(ConfigError::semantic(
                "database.audit.listen_url is not used: no policy listener listens on the audit                  domain; set database.listen_url, database.clinical.listen_url or                  database.party.listen_url"
                    .to_owned(),
            ));
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(ConfigErrors(errors))
        }
    }

    /// The outbound endpoints in use, each by the endpoint rules
    /// ([`crate::outbound::problems`]): the ITI-20 feed and the terminology
    /// server when they are configured. A credential beside one in the URL's
    /// userinfo is refused: the upstream would receive two authorizations, and
    /// which one it honours is not ours to guess. No openEHR spec governs
    /// outbound authentication: our own design.
    fn validate_outbound_clients(&self, errors: &mut Vec<ConfigError>) {
        if let Some(feed) = &self.audit.fhir_feed {
            errors.extend(
                crate::outbound::problems("audit.fhir_feed.endpoint", &feed.endpoint)
                    .into_iter()
                    .map(ConfigError::semantic),
            );
        }
        if let Some(terminology) = &self.terminology {
            errors.extend(
                crate::outbound::problems("terminology.endpoint", &terminology.endpoint)
                    .into_iter()
                    .map(ConfigError::semantic),
            );
        }
    }

    /// The access-log horizon sits between the floors and the ceilings the
    /// jurisdictions in force declare.
    ///
    /// A retention shorter than a floor erases the log while the chain still
    /// verifies (#3242); a retention longer than a ceiling keeps log data past
    /// the point the law says to delete it (#3346), which `0` — keep forever —
    /// always does. The jurisdictions in force are the ones the active
    /// identifier rules name, and the German ceiling additionally waits for the
    /// deployment to declare itself one of the SGB V § 307 controllers. Bounds
    /// are written in calendar years: a retention in years is compared exactly,
    /// a day count conservatively ([`crate::system_log::retention::RetentionBound`]).
    /// Where a floor and a ceiling contradict each other in years, no horizon
    /// satisfies both and the refusal says so; where only the day rounding
    /// leaves no day count between them, the refusal names the year form.
    /// Every emergency purpose code must also be an agreed purpose code.
    fn validate_audit(&self, errors: &mut Vec<ConfigError>) {
        use crate::system_log::config::Retention;
        use crate::system_log::retention::RetentionBound;
        for code in &self.audit.emergency_purpose_codes {
            if !self.audit.purpose_codes.contains(code) {
                errors.push(ConfigError::semantic(format!(
                    "audit.emergency_purpose_codes names {code:?}, which is not on \
                     audit.purpose_codes; an emergency code must be one of the purpose codes \
                     this deployment agrees with its callers, so add it there"
                )));
            }
        }
        let store = &self.audit.store;
        if !(self.audit.enabled && store.enabled) {
            return;
        }
        let jurisdictions: std::collections::BTreeSet<&str> = self
            .privacy
            .identifier_scan
            .rules
            .iter()
            .filter_map(|key| crate::privacy::detect::rule(key))
            .map(|rule| rule.jurisdiction)
            .collect();
        let declared = store.sgb_v_309_controller;
        let mut floors: Vec<(&str, RetentionBound)> = Vec::new();
        let mut ceilings: Vec<(&str, RetentionBound)> = Vec::new();
        for jurisdiction in jurisdictions {
            if let Some(floor) = crate::system_log::retention::retention_floor(jurisdiction) {
                floors.push((jurisdiction, floor));
            }
            if let Some(ceiling) =
                crate::system_log::retention::retention_ceiling(jurisdiction, declared)
            {
                ceilings.push((jurisdiction, ceiling));
            }
        }
        let retention = store.horizon();
        let hint = years_fix(&floors, &ceilings)
            .filter(|_| !matches!(retention, Retention::Years(_)))
            .map(|years| {
                format!(
                    "; no day count satisfies both the floor and the ceiling, so set \
                     retention = \"P{years}Y\" instead"
                )
            })
            .unwrap_or_default();
        let horizon = retention.describe();
        for &(jurisdiction, floor) in &floors {
            let (days, years, source) = (floor.days, floor.years, floor.source);
            for &(capped, ceiling) in &ceilings {
                if ceiling.years < years {
                    errors.push(ConfigError::semantic(format!(
                        "the {capped} access-log retention ceiling of {} days ({} years) is \
                         below the {jurisdiction} floor of {days} days ({years} years, {source}), \
                         so no horizon satisfies both; run the two jurisdictions as separate \
                         deployments, or drop the privacy.identifier_scan rule that does not \
                         apply here",
                        ceiling.days, ceiling.years
                    )));
                }
            }
            if !floor.admits_as_floor(retention) {
                let bound = match retention {
                    Retention::Years(_) => format!("{years} years"),
                    Retention::Forever | Retention::Days(_) => format!("{days} days"),
                };
                errors.push(ConfigError::semantic(format!(
                    "audit.store.{horizon} is below the {jurisdiction} access-log retention floor \
                     of {bound} ({source}); keep records at least that long, or unset \
                     retention to keep them forever{hint}"
                )));
            }
        }
        for &(jurisdiction, ceiling) in &ceilings {
            if !ceiling.admits_as_ceiling(retention) {
                let bound = match retention {
                    Retention::Years(_) => format!("{} years", ceiling.years),
                    Retention::Forever | Retention::Days(_) => format!("{} days", ceiling.days),
                };
                errors.push(ConfigError::semantic(format!(
                    "audit.store.{horizon} is above the {jurisdiction} access-log retention \
                     ceiling of {bound} ({}); set a horizon at or below it, or clear \
                     audit.store.sgb_v_309_controller if this deployment is not one of the \
                     § 307 controllers{hint}",
                    ceiling.source
                )));
            }
        }
    }

    /// `[audit.syslog]` names a `tls://` address, and its
    /// `[audit.syslog.tls]` material reads (#5217).
    fn validate_audit_syslog(&self, errors: &mut Vec<ConfigError>) {
        let Some(syslog) = &self.audit.syslog else {
            return;
        };
        if let Err(e) = syslog.address() {
            errors.push(ConfigError::semantic(e));
        }
        if syslog.spool_max_events == 0 || syslog.spool_max_size.is_zero() {
            errors.push(ConfigError::semantic(
                "audit.syslog.spool_max_events and spool_max_size must be above zero".to_owned(),
            ));
        }
        if let Err(e) =
            ferrohealth_server::tls_client::TlsClient::read("audit.syslog.tls", &syslog.tls)
        {
            errors.push(ConfigError::semantic(e.to_string()));
        }
    }

    /// The subject index is chosen and complete: an IHE patient identity
    /// source (`"pixm"`) with its `[pixm]` endpoint, its clinical domain among
    /// the declared pseudonym namespaces and readable `[pixm.tls]` material, or
    /// an explicit `"local"`. Unset, the refusal names both choices.
    fn validate_ehr_index(&self, errors: &mut Vec<ConfigError>) {
        use crate::service::ehr_index::config::problems;
        let index = self.record.subject_index;
        let pixm = self.pixm.as_ref();
        errors.extend(
            problems(index, pixm, &self.privacy.subject_namespaces)
                .into_iter()
                .map(ConfigError::semantic),
        );
    }

    /// A cohort predicate is only a selection criterion if the deployment can
    /// actually reach the leaf it names: an unknown key, an archetype that is
    /// not a demographic HRID, or a node that is not an at-code binds a
    /// predicate that would match nothing and report an empty cohort rather
    /// than a misconfiguration. No openEHR spec governs configuration — our
    /// own design; the identifier grammars are BASE `base_types`
    /// `master05-identification_package.adoc` §Archetype Identifiers.
    fn validate_cohort(&self, errors: &mut Vec<ConfigError>) {
        use crate::service::linkage::cohort::config as cohort;
        for (key, binding) in &self.privacy.cohort.predicates {
            if !cohort::PREDICATE_KEYS.contains(&key.as_str()) {
                errors.push(ConfigError::semantic(format!(
                    "privacy.cohort.predicates names `{key}`, which is not a cohort predicate; the \
                     bindable keys are {}",
                    cohort::PREDICATE_KEYS.join(", ")
                )));
                continue;
            }
            let is_birth_date = binding.kind == PredicateKind::BirthDate;
            if (key == cohort::BIRTH_DATE_KEY) != is_birth_date {
                errors.push(ConfigError::semantic(format!(
                    "privacy.cohort.predicates.{key}.kind = \"{}\" — `{}` is the age-band predicate \
                     and takes kind \"birth_date\"; no other key does",
                    binding.kind.as_str(),
                    cohort::BIRTH_DATE_KEY
                )));
            }
            if !cohort::is_demographic_archetype_hrid(&binding.archetype) {
                errors.push(ConfigError::semantic(format!(
                    "privacy.cohort.predicates.{key}.archetype = {:?} is not a demographic archetype \
                     HRID (openEHR-DEMOGRAPHIC-<CLASS>.<concept>.v<n>)",
                    binding.archetype
                )));
            }
            if !cohort::is_at_code(&binding.node) {
                errors.push(ConfigError::semantic(format!(
                    "privacy.cohort.predicates.{key}.node = {:?} is not an at-code (at0012)",
                    binding.node
                )));
            }
        }
    }

    /// `deployment.accepts` names each gap once, and only means something
    /// under `production`: a sandbox that lists accepted gaps is describing a
    /// posture it does not enforce, which is the confusion the key exists to
    /// remove.
    fn validate_deployment(&self, errors: &mut Vec<ConfigError>) {
        let mut seen = std::collections::BTreeSet::new();
        for gap in &self.deployment.accepts {
            if !seen.insert(gap.as_str()) {
                errors.push(ConfigError::semantic(format!(
                    "deployment.accepts names `{gap}` twice"
                )));
            }
        }
        if self.deployment.profile == deployment::DeploymentProfile::Sandbox
            && !self.deployment.accepts.is_empty()
        {
            errors.push(ConfigError::semantic(
                "deployment.accepts is set under deployment.profile = \"sandbox\", which accepts \
                 every gap already; the key belongs to a production deployment that runs \
                 without a named separation"
                    .to_owned(),
            ));
        }
    }

    /// A `restricted` per-EHR default needs authentication to be enforced.
    ///
    /// With `auth.enabled = false` the server wires no authorization at all and
    /// no caller carries a role, so the default could only be ignored, which is
    /// the silent opposite of what the key says. The combination is refused
    /// instead. No openEHR spec governs configuration — our own design.
    fn validate_ehr_access(&self, errors: &mut Vec<ConfigError>) {
        if !self.auth.shared.enabled
            && self.authz.rbac.ehr_access_default == authz::EhrAccessDefault::Restricted
        {
            errors.push(ConfigError::semantic(
                "authz.rbac.ehr_access_default = \"restricted\" cannot be enforced with \
                 auth.enabled = false: no caller is authenticated, so none holds \
                 authz.rbac.admin_role; enable authentication, or set \
                 ehr_access_default = \"open\""
                    .to_owned(),
            ));
        }
    }

    /// The clinical-side privacy policy has to be BUILDABLE, not merely
    /// well-typed: an unknown rule key or an uncompilable pattern would
    /// otherwise leave the scanner silently short a rule while reporting that
    /// the jurisdiction was covered, and a blank pseudonym namespace would
    /// accept an empty `external_ref.namespace` as if it were declared. No
    /// openEHR spec governs configuration — our own design.
    fn validate_privacy(&self, errors: &mut Vec<ConfigError>) {
        if let Err(error) = crate::privacy::PrivacyPolicy::compile(&self.privacy) {
            errors.push(ConfigError::semantic(error.to_string()));
        }
        if self
            .privacy
            .subject_namespaces
            .iter()
            .any(|namespace| namespace.trim().is_empty())
        {
            errors.push(ConfigError::semantic(
                "privacy.subject_namespaces carries a blank entry; every pseudonym namespace is \
                 a name an EHR_STATUS subject reference may claim"
                    .to_owned(),
            ));
        }
    }

    /// The clinical body tier sits inside the listener's limit.
    ///
    /// The listener refuses every body above `server.body_limit` before
    /// a route is matched, so a clinical tier at or above it would never be
    /// the limit that applies. No openEHR spec bounds a request body — our own
    /// design.
    fn validate_body_tier(&self, errors: &mut Vec<ConfigError>) {
        if let Some(tier) = self.api.body_limit
            && usize::try_from(tier.bytes())
                .is_ok_and(|tier| tier >= self.server.shared.body_limit_bytes())
                | usize::try_from(tier.bytes()).is_err()
        {
            errors.push(ConfigError::semantic(format!(
                "api.body_limit (\"{tier}\") must be below server.body_limit (\"{}\"), \
                 the limit of the bulk routes and the ceiling of every route",
                self.server.shared.body_limit
            )));
        }
    }

    /// `api.system_id` is judged by the SAME validating `UID` constructor
    /// the reader uses.
    ///
    /// It is stamped into every `AUDIT_DETAILS` (`System_id_valid`: "not
    /// `system_id.is_empty`", RM
    /// `docs/UML/classes/org.openehr.rm.common.audit_details.adoc` §Invariants)
    /// and occupies the `creating_system_id` position of every
    /// `OBJECT_VERSION_ID` this CDR mints (BASE
    /// `base_types/master05-identification_package.adoc` §Syntaxes).
    fn validate_system_id(&self, errors: &mut Vec<ConfigError>) {
        if let Err(source) =
            openehr_base::v1_3::base_types::identification::uid::Uid::new(&self.api.system_id)
        {
            errors.push(ConfigError::semantic(format!(
                "api.system_id {:?} is not a legal openEHR `uid` \
                 (iso_oid | uuid | internet_id — BASE master05 §Syntaxes): \
                 {source}. It is stamped into EHR.system_id, \
                 AUDIT_DETAILS.system_id, and the creating_system_id of every \
                 OBJECT_VERSION_ID",
                self.api.system_id
            )));
        }
    }

    /// `api.base_path` names this deployment's API root, and every rule it
    /// breaks is reported at once.
    ///
    /// The last segment is the ITS-REST API version `v1`: the overview locates
    /// the API at a deployment-chosen base followed by that segment
    /// (`ITS-REST/specifications/docs/overview/Resources.md` §Resource
    /// identification — "the request is made to an openEHR API (of version
    /// `v1`), located at `https://openEHRSys.example.com`"), and the REST
    /// policy is single-version. The first segment is `ferroehr`, which is our
    /// own rule and not configurable away. In between, any segment of RFC 3986
    /// unreserved characters is free, with no trailing slash and no empty
    /// segment.
    fn validate_base_path(&self, errors: &mut Vec<ConfigError>) {
        let path = self.api.base_path.as_str();
        let mut refuse = |rule: &str| {
            errors.push(ConfigError::semantic(format!(
                "api.base_path {path:?} {rule}"
            )));
        };

        if !path.starts_with('/') {
            refuse("must start with `/`");
        }
        let trailing_slash = path.len() > 1 && path.ends_with('/');
        if trailing_slash {
            refuse("must not end with `/`");
        }

        let body = path.strip_prefix('/').unwrap_or(path);
        let body = if trailing_slash {
            body.strip_suffix('/').unwrap_or(body)
        } else {
            body
        };
        let segments: Vec<&str> = body.split('/').collect();

        if segments.first() != Some(&"ferroehr") {
            refuse(
                "must begin with the `/ferroehr` segment (the product root is not configurable \
                 away)",
            );
        }
        if segments.last() != Some(&"v1") {
            refuse(
                "must end with the ITS-REST API version segment `v1` (overview Resources.md \
                 §Resource identification)",
            );
        }
        if segments.iter().any(|segment| segment.is_empty()) {
            refuse("must not contain an empty path segment (`//`)");
        }
        for segment in segments
            .iter()
            .filter(|segment| !segment.is_empty() && !segment.chars().all(is_unreserved))
        {
            refuse(&format!(
                "segment {segment:?} must use only RFC 3986 unreserved characters \
                 (A-Z a-z 0-9 - . _ ~)"
            ));
        }
    }

    /// The per-mechanism rules each security section owns.
    ///
    /// Authentication contributes the shape of each configured mechanism (the
    /// `ferrohealth-smart` section rules plus FerroEHR's issuer, key-set and
    /// KDF-floor rules); authorization its roles, policies and the
    /// patient-data assurance blocks; SMART what the discovery document
    /// publishes.
    fn validate_mechanisms(&self, errors: &mut Vec<ConfigError>) {
        if let Err(e) = auth::validate(&self.auth.shared) {
            errors.push(ConfigError::semantic(format!(
                "auth: {}",
                crate::error::ErrorChain::new(&e)
            )));
        }
        if let Err(e) = self.authz.validate() {
            errors.push(ConfigError::semantic(format!("authz: {e}")));
        }
        if let Err(e) = self.authz.assurance.validate() {
            errors.push(ConfigError::semantic(format!("authz: {e}")));
        }
        errors.extend(
            self.identity_errors()
                .into_iter()
                .map(ConfigError::semantic),
        );
        if let Err(e) = self.auth.smart.validate() {
            errors.push(ConfigError::semantic(format!("auth.smart: {e}")));
        }
        errors.extend(
            self.public_url_errors()
                .into_iter()
                .map(ConfigError::semantic),
        );
        let resolved = auth::resolve(&self.auth.shared).ok();
        errors.extend(
            self.auth
                .scim
                .iter()
                .flat_map(crate::config::scim::ScimConfig::errors)
                .map(ConfigError::semantic),
        );
        errors.extend(self.first_admin_error().map(ConfigError::semantic));
        if self.auth.smart.discovery
            && let Some(resolved) = &resolved
            && let Some(named) = &resolved.described_issuer
        {
            errors.extend(described_issuer_error(named, resolved).map(ConfigError::semantic));
        }
        if self.auth.smart.discovery && !auth::has_bearer(&self.auth.shared) {
            errors.push(ConfigError::semantic(
                "auth.smart.discovery = true requires an [auth] issuer: the discovery document is \
                 built from the issuer's metadata, and it directs applications to an \
                 authorization server whose tokens this server must accept"
                    .to_owned(),
            ));
        }
    }

    /// `[authz.rbac] first_admin` names a user of `[[auth.basic.users]]`,
    /// since only a Basic user holds the role from this file.
    fn first_admin_error(&self) -> Option<String> {
        let named = self.authz.rbac.first_admin.as_deref()?;
        let listed = self
            .auth
            .shared
            .basic
            .as_ref()
            .is_some_and(|basic| basic.users.iter().any(|user| user.user == named));
        (!listed).then(|| {
            format!(
                "authz.rbac.first_admin names {named:?}, which is no user of \
                 [[auth.basic.users]]: the first administrator is a Basic user"
            )
        })
    }

    /// `[server] public_base_url` holds its rule when set, and is set when a
    /// consumer of the public URL is on: the SMART discovery document (master04
    /// §Services makes every `services.*.baseUrl` an absolute URL), the SCIM
    /// service provider (RFC 7643 §3.1: `location` is "The URI of the
    /// resource being returned") and the `DPoP` proof check of an enabled
    /// `[auth]` (RFC 9449 §4.3, check 9: `htu` is the URI the client
    /// addressed).
    fn public_url_errors(&self) -> Vec<String> {
        if let Some(error) = self.server.public_base_url_error() {
            return vec![error];
        }
        if self.server.public_base_url.is_some() {
            return Vec::new();
        }
        let consumers = [
            (self.auth.smart.discovery, "auth.smart.discovery = true"),
            (self.auth.scim.is_some(), "[auth.scim]"),
            (
                self.auth.shared.enabled && self.auth.shared.dpop.is_some(),
                "[auth.dpop]",
            ),
        ];
        consumers
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, consumer)| {
                format!(
                    "{consumer} requires server.public_base_url, the URL clients reach this \
                     server at, e.g. 'https://cdr.example.com'"
                )
            })
            .collect()
    }

    /// The signing section's own rules: a PGP mode needs a key, and a
    /// passphrase is given inline or by file, never both.
    fn validate_signing(&self, errors: &mut Vec<ConfigError>) {
        if matches!(
            self.record.signing.mode,
            crate::versioning::signature::config::Mode::Pgp
        ) && self.record.signing.key_file.is_none()
        {
            errors.push(ConfigError::semantic(
                "record.signing.mode = \"pgp\" requires record.signing.key_file".to_owned(),
            ));
        }
        if self.record.signing.key_passphrase.is_some()
            && self.record.signing.key_passphrase_file.is_some()
        {
            errors.push(ConfigError::semantic(
                "set only one of record.signing.key_passphrase / key_passphrase_file".to_owned(),
            ));
        }
    }

    /// Every secret that can be given inline or by file is given exactly one
    /// way.
    fn validate_key_sources(&self, errors: &mut Vec<ConfigError>) {
        if self.record.multimedia.secret_access_key.is_some()
            && self.record.multimedia.secret_access_key_file.is_some()
        {
            errors.push(ConfigError::semantic(
                "set only one of record.multimedia.secret_access_key / \
                 secret_access_key_file"
                    .to_owned(),
            ));
        }
    }
}

/// Whether `c` is an RFC 3986 §2.3 unreserved character, the set a path
/// segment may carry without percent-encoding.
const fn is_unreserved(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~')
}

/// The year count that satisfies every floor and ceiling in force when no day
/// count does, or `None` when a day count can, or when no year count can.
///
/// A floor in years binds a day count to the most those years can span and a
/// ceiling to the fewest, so a floor and a ceiling of the same years leave no
/// day count between them while the year count itself satisfies both.
fn years_fix(
    floors: &[(&str, crate::system_log::retention::RetentionBound)],
    ceilings: &[(&str, crate::system_log::retention::RetentionBound)],
) -> Option<u32> {
    let floor = floors
        .iter()
        .map(|(_, bound)| *bound)
        .max_by_key(|b| b.years)?;
    let ceiling = ceilings
        .iter()
        .map(|(_, bound)| *bound)
        .min_by_key(|b| b.years)?;
    let floor_days = floors.iter().map(|(_, bound)| bound.days).max()?;
    let ceiling_days = ceilings.iter().map(|(_, bound)| bound.days).min()?;
    (floor.years <= ceiling.years && floor_days > ceiling_days).then_some(floor.years)
}

/// The `record.multimedia.url` semantic checks of an enabled integration.
///
/// An enabled integration with a blank or scheme-less endpoint would boot clean
/// and then fail on the first `DV_MULTIMEDIA` commit. A `${VAR:-}` compose
/// pass-through and an empty Helm value both produce exactly that, so it is
/// refused at boot where an operator can still act.
fn multimedia_endpoint_errors(
    config: &crate::extensions::multimedia::config::MultimediaConfig,
) -> Vec<ConfigError> {
    if !config.enabled {
        return Vec::new();
    }
    match config.endpoint_url() {
        Ok(_) => Vec::new(),
        Err(e) => vec![ConfigError::semantic(e.to_string())],
    }
}

/// The refusal of `[auth] described_issuer` when `ferrohealth-smart`'s
/// `describe` refuses it before any issuer is read: it names no `[auth]`
/// issuer, or a `machines` issuer, which signs client credentials tokens only
/// and so serves no user launch (SMART App Launch 2.2.0, Conformance,
/// §Metadata: `authorization_endpoint`).
fn described_issuer_error(
    named: &ferrohealth_secret::secret_url::SecretUrl,
    resolved: &ferrohealth_smart::config::Resolved,
) -> Option<String> {
    use ferrohealth_smart::resource::described::{Candidate, Metadata, describe};
    // NOTE: no issuer's metadata is read at load, so each is offered as available and the start judges it.
    let candidates = resolved
        .kinds
        .iter()
        .map(|(issuer, kind)| Candidate::new(issuer.clone(), *kind, Metadata::Available, ()));
    describe(candidates, Some(named))
        .err()
        .map(|e| format!("auth.described_issuer is refused: {e}"))
}

#[cfg(test)]
mod tests {
    use ferrohealth_secret::secret_url::SecretUrl;
    use ferrohealth_smart::config::AuthSection;

    use super::*;

    /// `[auth] described_issuer` names an `[auth]` issuer that serves a
    /// user launch: an unknown issuer and a `machines` issuer, which signs
    /// client credentials tokens only, are refused before any issuer is read,
    /// and a `persons` issuer passes.
    #[test]
    fn the_described_issuer_names_a_user_facing_auth_issuer() {
        let mut issuer = crate::config::auth_section::machines("https://as.example/machines");
        issuer.extend(crate::config::auth_section::machines(
            "https://as.example/persons",
        ));
        issuer[1].kind = Some(ferrohealth_smart::kind::IssuerKind::Persons);
        let resolved = auth::resolve(&AuthSection {
            issuer,
            ..AuthSection::default()
        })
        .expect("two issuers resolve");
        let unknown =
            described_issuer_error(&SecretUrl::new("https://elsewhere.example"), &resolved)
                .expect("an unknown issuer is refused");
        assert!(unknown.contains("which is no trusted issuer"), "{unknown}");
        let machines =
            described_issuer_error(&SecretUrl::new("https://as.example/machines"), &resolved)
                .expect("a machines issuer is refused");
        assert!(machines.contains("names the machines issuer"), "{machines}");
        assert!(
            described_issuer_error(&SecretUrl::new("https://as.example/persons"), &resolved)
                .is_none()
        );
    }

    /// `[authz.rbac] first_admin` names a user of `[[auth.basic.users]]`:
    /// another name is refused at boot, and an unset key asks nothing.
    #[test]
    fn the_first_admin_names_a_basic_user() {
        let mut config = FerroEhrConfig::default();
        assert_eq!(config.first_admin_error(), None);
        config.auth.shared.basic = Some(ferrohealth_smart::config::BasicSection {
            users: vec![ferrohealth_smart::config::BasicUser {
                user: "root".to_owned(),
                password_hash: None,
                password_hash_file: None,
            }],
            ..ferrohealth_smart::config::BasicSection::default()
        });
        config.authz.rbac.first_admin = Some("root".to_owned());
        assert_eq!(config.first_admin_error(), None);
        config.authz.rbac.first_admin = Some("admin".to_owned());
        let refused = config
            .first_admin_error()
            .expect("an unknown user is refused");
        assert!(refused.contains("\"admin\""), "{refused}");
    }
}
