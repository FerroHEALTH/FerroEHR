// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The federation agreements (`[[authz.federation.agreement]]`): what a
//! `federation` issuer's verified external persons, or its organisation, may
//! do here, on which legal ground.
//!
//! An agreement is keyed by its issuer and its `accessor` and admits only that
//! accessor kind. An organisation agreement runs without a named person, so it
//! is read-only (EHDS Art. 13(4)) and declares its legal ground and access
//! class. A `chained` access is met jointly by this node's record carrying the
//! requester's correlation id and the requester's log naming the person, which
//! the agreement obliges the requester to keep at least as long as this node
//! keeps its own (EHDS Annex II 3.2(b); no source settles this reading: our own
//! design). No openEHR spec governs configuration: our own design.

use std::collections::BTreeSet;
use std::fmt;

use ferrohealth_server::section::CallerRate;
use ferrohealth_smart::config::{AuthSection, Resolved};
use ferrohealth_smart::federation::{Field, Source};
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::profile::Preset;
use serde::{Deserialize, Serialize};

use crate::system_log::config::Retention;

/// The jurisdiction a `profile = "nl"` issuer implies.
const NL: &str = "NL";

/// The `[authz.federation]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct FederationConfig {
    /// The agreements, one per issuer and accessor.
    pub agreement: Vec<Agreement>,
}

/// One `[[authz.federation.agreement]]` table.
// TODO(#5008): admit an external caller only within the agreement of its issuer and accessor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct Agreement {
    /// The `federation` issuer of the external callers.
    pub issuer: String,
    /// Who the agreement admits: named persons, or the organisation.
    pub accessor: Option<Accessor>,
    /// The agreement's name.
    pub name: String,
    /// The agreement's version.
    pub version: String,
    /// The ISO 3166-1 alpha-2 jurisdiction; a `profile = "nl"` issuer implies
    /// `NL`.
    pub jurisdiction: String,
    /// The kind of exchange the agreement covers.
    pub exchange: String,
    /// What the exchange is for; required of an NL agreement, which may not
    /// serve a keuring (Wabvpz Art. 15f(2)).
    pub purpose: Option<Purpose>,
    /// The legal ground of the processing.
    pub legal_ground: Option<LegalGround>,
    /// The source of the consent the exchange is conditioned on, never its
    /// legal ground.
    pub consent: Option<ConsentSource>,
    /// The most an external caller may read: `patient/` scopes only.
    pub reads: Vec<String>,
    /// The template ids an external person may commit; none by default.
    pub writes: Vec<String>,
    /// Whether an external person may discard a dispensation of the same
    /// point of care within `discard_window`.
    pub discard: bool,
    /// The window of a discard, a whole number of hours (MyHealth@EU
    /// Requirements Catalogue 10.0.0, requirement 07.04, sets `"24h"`).
    pub discard_window: ferrohealth_config::units::Duration,
    /// The MyHealth@EU service a contact point's person agreement serves,
    /// which names the `*_NOT_FOUND` code of a request outside the TRC
    /// patient; required on a `contact_point` issuer and refused elsewhere.
    pub ehdsi_service: Option<EhdsiService>,
    /// The admission rate per caller, above which a request is refused before
    /// any patient lookup; required.
    pub caller_rate: Option<CallerRate>,
    /// Whether a professional of the agreement may read an opted-out EHR
    /// where national law provides the EHDS Art. 10(2) vital-interest access.
    pub opt_out_override: bool,
    /// For an organisation: whether no natural person views the data, or the
    /// requester logs the person.
    pub access_class: Option<AccessClass>,
    /// For a `chained` access: how long the requester keeps its log, a
    /// calendar period (`"P5Y"`).
    pub chained_log_retention: Option<crate::config::retention::RetentionPeriod>,
    /// Retired in v5.7.0: refused at load, naming `discard_window`.
    #[serde(skip_serializing)]
    pub discard_window_hours: ferrohealth_config::removed::Removed,
    /// Retired in v5.7.0: refused at load, naming `chained_log_retention`.
    #[serde(skip_serializing)]
    pub chained_log_retention_years: ferrohealth_config::removed::Removed,
}

impl Agreement {
    /// The discard window in whole hours, when it is one.
    #[must_use]
    pub fn discard_window_hours(&self) -> Option<u32> {
        let millis = self.discard_window.as_millis();
        if millis.checked_rem(3_600_000) != Some(0) {
            return None;
        }
        u32::try_from(millis.checked_div(3_600_000)?).ok()
    }
}

impl Default for Agreement {
    fn default() -> Self {
        Self {
            issuer: String::new(),
            accessor: None,
            name: String::new(),
            version: String::new(),
            jurisdiction: String::new(),
            exchange: String::new(),
            purpose: None,
            legal_ground: None,
            consent: None,
            reads: Vec::new(),
            writes: Vec::new(),
            discard: false,
            discard_window: ferrohealth_config::units::Duration::from_hours(24),
            ehdsi_service: None,
            caller_rate: None,
            opt_out_override: false,
            access_class: None,
            chained_log_retention: None,
            discard_window_hours: ferrohealth_config::removed::Removed::default(),
            chained_log_retention_years: ferrohealth_config::removed::Removed::default(),
        }
    }
}

/// Who an agreement admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Accessor {
    /// A verified external person, named in the token.
    Person,
    /// The external organisation, with no named person.
    Organisation,
}

impl fmt::Display for Accessor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Person => "person",
            Self::Organisation => "organisation",
        })
    }
}

/// How an organisation accesses the data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum AccessClass {
    /// No natural person views the data.
    Automated,
    /// The requester logs the person behind each request, and each request
    /// carries the requester's correlation id as an attested claim.
    Chained,
}

/// What an agreement's exchange is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// The care of the patient by the requesting care provider.
    Care,
    /// A keuring: a medical examination for employment, insurance or a
    /// licence, as the Wet op de medische keuringen governs.
    Keuring,
}

/// The MyHealth@EU service of a contact point's agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum EhdsiService {
    /// The Patient Summary.
    PatientSummary,
    /// ePrescription and eDispensation.
    Eprescription,
    /// The original clinical document.
    Orcd,
}

impl EhdsiService {
    /// The code a request outside the TRC patient answers, which NCPeH-A
    /// answers as "No Data" (eHDSI XCA Profile 8.0.1 §2.2.3 and §4.2.2; the
    /// MyHealth@EU Requirements Catalogue 10.0.0 §1.25 code names).
    #[must_use]
    pub const fn not_found(self) -> &'static str {
        match self {
            Self::PatientSummary => "ERROR_PS_NOT_FOUND",
            Self::Eprescription => "ERROR_EP_NOT_FOUND",
            Self::Orcd => "ERROR_ORCD_NOT_FOUND",
        }
    }
}

/// Where the consent an exchange is conditioned on is registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ConsentSource {
    /// Mitz, whose closed question needs a responsible professional.
    Mitz,
    /// The holder's own consent registration.
    Local,
}

/// The legal ground of an agreement: the GDPR Art. 6(1) and Art. 9(2) points
/// and the national provision.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct LegalGround {
    /// The point of GDPR Art. 6(1), `a` to `f`.
    pub gdpr_6_1: String,
    /// The point of GDPR Art. 9(2), `a` to `j`.
    pub gdpr_9_2: String,
    /// The national provision, such as `UAVG Art. 30(3)(a)`.
    pub national: String,
}

impl fmt::Display for LegalGround {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GDPR Art. 6(1)({}), Art. 9(2)({})",
            self.gdpr_6_1, self.gdpr_9_2
        )?;
        if !self.national.is_empty() {
            write!(f, ", {}", self.national)?;
        }
        Ok(())
    }
}

impl LegalGround {
    /// Returns whether both points name a point of their article.
    fn is_coded(&self) -> bool {
        let point = |value: &str, last: char| {
            let mut chars = value.chars();
            matches!((chars.next(), chars.next()), (Some(c), None) if ('a'..=last).contains(&c))
        };
        point(&self.gdpr_6_1, 'f') && point(&self.gdpr_9_2, 'j')
    }
}

impl Agreement {
    /// Returns the jurisdiction the agreement holds for: its own, or `NL`
    /// for an issuer of the `nl` profile.
    #[must_use]
    pub fn jurisdiction_of(&self, auth: &AuthSection) -> String {
        let nl = auth
            .issuers()
            .iter()
            .any(|entry| entry.issuer.expose() == self.issuer && entry.profile == Preset::Nl);
        if self.jurisdiction.is_empty() && nl {
            NL.to_owned()
        } else {
            self.jurisdiction.to_ascii_uppercase()
        }
    }
}

/// The boot refusals of `[[authz.federation.agreement]]`, each naming its
/// table; `holder` is how long this node keeps its access records.
#[must_use]
pub fn errors(
    agreements: &[Agreement],
    auth: &AuthSection,
    resolved: &Resolved,
    holder: Retention,
) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen = BTreeSet::new();
    for agreement in agreements {
        let accessor = agreement
            .accessor
            .map_or_else(|| "-".to_owned(), |a| a.to_string());
        let name = format!(
            "the [[authz.federation.agreement]] of {:?} for accessor {accessor}",
            agreement.issuer
        );
        match resolved.kinds.get(&agreement.issuer) {
            Some(IssuerKind::Federation) => {}
            Some(kind) => errors.push(format!(
                "{name} names a {kind} issuer: an agreement's issuer is of kind federation"
            )),
            None => errors.push(format!("{name} names no [[auth.issuer]]")),
        }
        if !seen.insert((agreement.issuer.as_str(), agreement.accessor)) {
            errors.push(format!("{name} is declared twice"));
        }
        let Some(kind) = agreement.accessor else {
            errors.push(format!(
                "{name} needs accessor = \"person\" or \"organisation\""
            ));
            continue;
        };
        common(agreement, auth, &name, &mut errors);
        let contact_point = is_contact_point(resolved, &agreement.issuer);
        match (contact_point, agreement.ehdsi_service) {
            (true, None) if kind == Accessor::Person => errors.push(format!(
                "{name} is a MyHealth@EU contact point's and needs ehdsi_service"
            )),
            (false, Some(_)) => errors.push(format!(
                "{name} sets ehdsi_service, which only a contact_point issuer's agreement may"
            )),
            _ => {}
        }
        if kind == Accessor::Organisation {
            organisation(agreement, resolved, holder, &name, &mut errors);
            if agreement.access_class == Some(AccessClass::Chained) {
                correlated(agreement, auth, &name, &mut errors);
            }
        }
    }
    errors
}

/// The rules every agreement is held to.
fn common(agreement: &Agreement, auth: &AuthSection, name: &str, errors: &mut Vec<String>) {
    if agreement.discard
        && agreement
            .discard_window_hours()
            .is_none_or(|hours| hours == 0)
    {
        errors.push(format!(
            "{name} sets discard_window = \"{}\", which is no whole number of hours, such as \"24h\"",
            agreement.discard_window
        ));
    }
    if agreement.caller_rate.is_none() {
        errors.push(format!("{name} needs caller_rate"));
    }
    if let Some(scope) = agreement.reads.iter().find(|s| !s.starts_with("patient/")) {
        errors.push(format!(
            "{name} reads {scope:?}: an agreement reads patient/ scopes only"
        ));
    }
    if agreement
        .legal_ground
        .as_ref()
        .is_some_and(|g| !g.is_coded())
    {
        errors.push(format!(
            "{name} has a legal_ground whose gdpr_6_1 is not a point a to f or whose gdpr_9_2 is \
             not a point a to j"
        ));
    }
    let jurisdiction = agreement.jurisdiction_of(auth);
    if jurisdiction == NL && !agreement.reads.is_empty() && agreement.consent.is_none() {
        errors.push(format!(
            "{name} is an NL agreement with reads and needs consent = \"mitz\" or \"local\" \
             (Wabvpz Art. 15a(1))"
        ));
    }
    if jurisdiction == NL {
        match agreement.purpose {
            Some(Purpose::Care) => {}
            Some(Purpose::Keuring) => errors.push(format!(
                "{name} is an NL agreement for a keuring, which has no access to an exchange \
                 system (Wabvpz Art. 15f(2))"
            )),
            None => errors.push(format!(
                "{name} is an NL agreement and needs purpose = \"care\" (Wabvpz Art. 15f(2))"
            )),
        }
    }
}

/// The rules of an agreement that admits an organisation without a named
/// person.
fn organisation(
    agreement: &Agreement,
    resolved: &Resolved,
    holder: Retention,
    name: &str,
    errors: &mut Vec<String>,
) {
    for (key, missing) in [
        ("name", agreement.name.trim().is_empty()),
        ("version", agreement.version.trim().is_empty()),
        ("legal_ground", agreement.legal_ground.is_none()),
        ("access_class", agreement.access_class.is_none()),
    ] {
        if missing {
            errors.push(format!("{name} needs {key}"));
        }
    }
    if !agreement.writes.is_empty() {
        errors.push(format!(
            "{name} lists writes: an organisation access is read-only (EHDS Art. 13(4))"
        ));
    }
    if agreement.opt_out_override {
        errors.push(format!(
            "{name} sets opt_out_override, which only a person agreement may"
        ));
    }
    if agreement.consent == Some(ConsentSource::Mitz) {
        errors.push(format!(
            "{name} takes consent from mitz, whose closed question needs a responsible \
             professional; use consent = \"local\""
        ));
    }
    if is_contact_point(resolved, &agreement.issuer) {
        errors.push(format!(
            "{name} names a contact_point issuer, whose requests name a professional"
        ));
    }
    if agreement.access_class == Some(AccessClass::Chained) {
        chained(agreement, holder, name, errors);
    }
}

/// Whether `issuer` is a MyHealth@EU contact point (`contact_point = true`).
#[must_use]
pub fn is_contact_point(resolved: &Resolved, issuer: &str) -> bool {
    resolved.issuers.iter().any(|settings| {
        settings.issuer == issuer
            && settings
                .federation
                .as_ref()
                .is_some_and(|f| f.contact_point == Some(true))
    })
}

/// A `chained` agreement declares a requester log retention at least the
/// holder's.
fn chained(agreement: &Agreement, holder: Retention, name: &str, errors: &mut Vec<String>) {
    use crate::config::retention::RetentionPeriod;
    let Some(period) = agreement.chained_log_retention else {
        errors.push(format!("{name} is chained and needs chained_log_retention"));
        return;
    };
    let covers = match (period, holder) {
        (_, Retention::Forever) => false,
        (RetentionPeriod::Years(years), Retention::Years(kept)) => years >= kept,
        (RetentionPeriod::Years(years), Retention::Days(kept)) => {
            u64::from(years) * 365 >= u64::from(kept)
        }
        (RetentionPeriod::Days(days), Retention::Years(kept)) => {
            u64::from(days) >= u64::from(kept) * 366
        }
        (RetentionPeriod::Days(days), Retention::Days(kept)) => days >= kept,
    };
    if !covers {
        errors.push(format!(
            "{name} keeps the requester's log for \"{period}\", shorter than this node's \
             audit.store {}",
            holder.describe()
        ));
    }
}

/// A `chained` agreement's issuer reads the correlation id from its token: its
/// `[[auth.federation]]` table maps `correlation_id`, and from no constant.
fn correlated(agreement: &Agreement, auth: &AuthSection, name: &str, errors: &mut Vec<String>) {
    match correlation_sources(auth, &agreement.issuer) {
        None => errors.push(format!(
            "{name} is chained and its issuer's [[auth.federation]] table maps no correlation_id"
        )),
        Some(sources) if sources.iter().any(|s| matches!(s, Source::Constant(_))) => {
            errors.push(format!(
                "{name} is chained and its issuer's [[auth.federation]] table maps correlation_id \
                 from a constant: the id is read only from the token"
            ));
        }
        Some(_) => {}
    }
}

/// The sources the `[[auth.federation]]` table of `issuer` reads the
/// correlation id of a `chained` request from, when it maps any.
#[must_use]
pub fn correlation_sources<'a>(auth: &'a AuthSection, issuer: &str) -> Option<&'a [Source]> {
    auth.federation
        .iter()
        .find(|table| table.issuer == issuer)
        .and_then(|table| table.fields.get(&Field::CorrelationId))
        .map(Vec::as_slice)
        .filter(|sources| !sources.is_empty())
}

/// How the access record names `source`: its kind and its path.
#[must_use]
pub fn describe_source(source: &Source) -> String {
    match source {
        Source::Claim(path) => format!("claim:{path}"),
        Source::Constant(_) => "constant".to_owned(),
        Source::AuthorizationDetails(path) => format!("authorization_details:{path}"),
        Source::Presentation(path) => format!("presentation:{path}"),
    }
}

/// Returns the agreements' and the system clients' template ids that are
/// not in `held`, each with the table that names it.
#[must_use]
pub fn missing_templates<'a>(
    agreements: &'a [Agreement],
    clients: &'a [crate::config::clients::SystemClient],
    held: &BTreeSet<String>,
) -> Vec<String> {
    let agreed = agreements.iter().flat_map(|a| {
        a.writes.iter().map(move |t| {
            (
                format!("[[authz.federation.agreement]] of {:?}", a.issuer),
                t,
            )
        })
    });
    let declared = clients.iter().flat_map(|c| {
        c.writes.iter().map(move |t| {
            (
                format!(
                    "[[authz.system_client]] {:?} of {:?}",
                    c.client_id, c.issuer
                ),
                t,
            )
        })
    });
    agreed
        .chain(declared)
        .filter(|(_, template)| !held.contains(template.as_str()))
        .map(|(table, template)| {
            format!("the {table} writes the template {template:?}, which the store does not hold")
        })
        .collect()
}
