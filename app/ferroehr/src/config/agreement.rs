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
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::profile::Preset;
use serde::{Deserialize, Serialize};

use crate::system_log::config::Retention;

/// The jurisdiction a `profile = "nl"` issuer implies.
const NL: &str = "NL";

/// The `[authz.federation]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FederationConfig {
    /// The agreements, one per issuer and accessor.
    pub agreement: Vec<Agreement>,
}

/// One `[[authz.federation.agreement]]` table.
// TODO(#5008): admit an external caller only within the agreement of its issuer and accessor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// point of care within `discard_window_hours`.
    pub discard: bool,
    /// The window of a discard, in hours (MyHealth@EU Requirements Catalogue
    /// 10.0.0, requirement 07.04, sets 24).
    pub discard_window_hours: u32,
    /// The admission rate per caller, above which a request is refused before
    /// any patient lookup; required.
    pub caller_rate: Option<CallerRate>,
    /// Whether a professional of the agreement may read an opted-out EHR
    /// where national law provides the EHDS Art. 10(2) vital-interest access.
    pub opt_out_override: bool,
    /// For an organisation: whether no natural person views the data, or the
    /// requester logs the person.
    pub access_class: Option<AccessClass>,
    /// For a `chained` access: how many years the requester keeps its log.
    pub chained_log_retention_years: Option<u32>,
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
            legal_ground: None,
            consent: None,
            reads: Vec::new(),
            writes: Vec::new(),
            discard: false,
            discard_window_hours: 24,
            caller_rate: None,
            opt_out_override: false,
            access_class: None,
            chained_log_retention_years: None,
        }
    }
}

/// Who an agreement admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
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
#[serde(rename_all = "snake_case")]
pub enum AccessClass {
    /// No natural person views the data.
    Automated,
    /// The requester logs the person behind each request, and each request
    /// carries the requester's correlation id as an attested claim.
    Chained,
}

/// Where the consent an exchange is conditioned on is registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
            .any(|entry| entry.issuer == self.issuer && entry.profile == Preset::Nl);
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
        if kind == Accessor::Organisation {
            organisation(agreement, resolved, holder, &name, &mut errors);
        }
    }
    errors
}

/// The rules every agreement is held to.
fn common(agreement: &Agreement, auth: &AuthSection, name: &str, errors: &mut Vec<String>) {
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
    let contact_point = resolved.issuers.iter().any(|settings| {
        settings.issuer == agreement.issuer
            && settings
                .federation
                .as_ref()
                .is_some_and(|f| f.contact_point == Some(true))
    });
    if contact_point {
        errors.push(format!(
            "{name} names a contact_point issuer, whose requests name a professional"
        ));
    }
    if agreement.access_class == Some(AccessClass::Chained) {
        chained(agreement, holder, name, errors);
    }
}

/// A `chained` agreement declares a requester log retention at least the
/// holder's.
fn chained(agreement: &Agreement, holder: Retention, name: &str, errors: &mut Vec<String>) {
    let Some(years) = agreement.chained_log_retention_years else {
        errors.push(format!(
            "{name} is chained and needs chained_log_retention_years"
        ));
        return;
    };
    let covers = match holder {
        Retention::Forever => false,
        Retention::Years(kept) => years >= kept,
        Retention::Days(kept) => u64::from(years) * 365 >= u64::from(kept),
    };
    if !covers {
        errors.push(format!(
            "{name} keeps the requester's log {years} years, shorter than this node's \
             audit.store {}",
            holder.describe()
        ));
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
