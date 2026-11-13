// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The federation agreements: what a federation partner's verified external
//! persons, or the partner organisation itself, may do here, on which legal
//! ground, read from the CDR.
//!
//! An agreement is a grant of [`Operation::FederatedPersonAccess`] or
//! [`Operation::FederatedOrganisationAccess`] held by the partner's `ACTOR`,
//! which carries a `client_id` identifier under a `federation` issuer. Its
//! terms are the elements of the capability's `credentials` that the codes
//! of [`crate::party_binding::grants::operation::AGREEMENT`] name, and it holds while the grant does
//! (`time_validity`, `not_before`, `not_after`). A commit of a `ROLE` whose
//! agreement breaks a rule here is refused, and a request is admitted only
//! under an agreement that keeps them. An organisation runs without a named
//! person, so its agreement is read-only (EHDS Art. 13(4)) and states its
//! legal ground and access class. A `chained` access is met jointly by this
//! node's record carrying the requester's correlation id and the requester's
//! log naming the person, which the agreement obliges the requester to keep
//! at least as long as this node keeps its own (EHDS Annex II 3.2(b); no
//! source settles this reading: our own design). No openEHR spec gives a
//! capability an authorization meaning: our own design.

use std::fmt;
use std::num::NonZeroU32;

use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_server::section::CallerRate;
use serde::Serialize;

use crate::config::retention::RetentionPeriod;
use crate::party_binding::grants::capabilities::Credentials;
use crate::party_binding::grants::operation::Operation;
use crate::party_binding::grants::trust::{Correlation, FederationTrust, PartnerIssuer};
use crate::system_log::config::Retention;

/// Who an agreement admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Accessor {
    /// A verified external person, named in the token.
    Person,
    /// The external organisation, with no named person.
    Organisation,
}

impl Accessor {
    /// The operation an agreement for this accessor grants.
    #[must_use]
    pub const fn operation(self) -> Operation {
        match self {
            Self::Person => Operation::FederatedPersonAccess,
            Self::Organisation => Operation::FederatedOrganisationAccess,
        }
    }

    /// The accessor `operation` admits, `None` for an operation that is no
    /// agreement.
    #[must_use]
    pub const fn of(operation: Operation) -> Option<Self> {
        match operation {
            Operation::FederatedPersonAccess => Some(Self::Person),
            Operation::FederatedOrganisationAccess => Some(Self::Organisation),
            _ => None,
        }
    }
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessClass {
    /// No natural person views the data.
    Automated,
    /// The requester logs the person behind each request, and each request
    /// carries the requester's correlation id as an attested claim.
    Chained,
}

impl AccessClass {
    /// Its code: `automated` or `chained`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Automated => "automated",
            Self::Chained => "chained",
        }
    }

    fn of(word: &str) -> Option<Self> {
        [Self::Automated, Self::Chained]
            .into_iter()
            .find(|class| class.as_str() == word)
    }
}

/// The MyHealth@EU service of a contact point's agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    fn of(word: &str) -> Option<Self> {
        match word {
            "patient_summary" => Some(Self::PatientSummary),
            "eprescription" => Some(Self::Eprescription),
            "orcd" => Some(Self::Orcd),
            _ => None,
        }
    }
}

/// Where the consent an exchange is conditioned on is registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentSource {
    /// An external consent registry, whose question names the responsible
    /// professional.
    Registry,
    /// The holder's own consent registration.
    Local,
}

impl ConsentSource {
    fn of(word: &str) -> Option<Self> {
        match word {
            "registry" => Some(Self::Registry),
            "local" => Some(Self::Local),
            _ => None,
        }
    }
}

/// The legal ground of an agreement: the GDPR Art. 6(1) and Art. 9(2) points
/// and the provision of the law that applies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegalGround {
    /// The point of GDPR Art. 6(1), `a` to `f`.
    pub gdpr_6_1: String,
    /// The point of GDPR Art. 9(2), `a` to `j`.
    pub gdpr_9_2: String,
    /// The provision of the law that applies, as the agreement names it.
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

/// One federation agreement as its grant's credentials hold it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agreement {
    /// Who it admits, by the operation it grants.
    pub accessor: Accessor,
    /// Its name (`agreement_name`).
    pub name: String,
    /// Its version (`agreement_version`).
    pub version: String,
    /// The jurisdiction it holds for, upper-cased (`jurisdiction`).
    pub jurisdiction: String,
    /// The kind of exchange it covers (`exchange`).
    pub exchange: String,
    /// What the exchange is for (`purpose`).
    pub purpose: Option<String>,
    /// The legal ground of the processing (`gdpr_art_6_1`, `gdpr_art_9_2`,
    /// `national_provision`).
    pub legal_ground: Option<LegalGround>,
    /// The source of the consent the exchange is conditioned on, never its
    /// legal ground (`consent`).
    pub consent: Option<ConsentSource>,
    /// The most an external caller may read, `patient/` scopes
    /// (`read_scope`).
    pub reads: Vec<String>,
    /// The template ids an external person may commit (`template`).
    pub writes: Vec<String>,
    /// The window in whole hours in which an external person may discard a
    /// dispensation of the same point of care; none admits no discard
    /// (`discard_window`; MyHealth@EU Requirements Catalogue 10.0.0,
    /// requirement 07.04, sets `PT24H`).
    pub discard_window_hours: Option<u32>,
    /// The MyHealth@EU service a contact point's person agreement serves,
    /// which names the `*_NOT_FOUND` code of a request outside the TRC
    /// patient (`ehdsi_service`).
    pub ehdsi_service: Option<EhdsiService>,
    /// The admission rate per caller, above which a request is refused before
    /// any patient lookup (`requests_per_second`, `burst`).
    pub caller_rate: Option<CallerRate>,
    /// Whether a professional of the agreement may read an opted-out EHR
    /// where the law provides the EHDS Art. 10(2) vital-interest access
    /// (`opt_out_override`).
    pub opt_out_override: bool,
    /// For an organisation: whether no natural person views the data, or the
    /// requester logs the person (`access_class`).
    pub access_class: Option<AccessClass>,
    /// For a `chained` access: how long the requester keeps its log, a
    /// calendar period such as `P5Y` (`requester_log_retention`).
    pub requester_log_retention: Option<RetentionPeriod>,
}

impl Agreement {
    /// Reads the agreement of `accessor` that `credentials` hold.
    ///
    /// # Errors
    /// Every element that holds no value of its kind, each named by its code.
    pub fn read(accessor: Accessor, credentials: Credentials<'_>) -> Result<Self, Vec<String>> {
        let mut errors = Vec::new();
        let first_text = |code: &str| {
            credentials
                .texts(code)
                .first()
                .map(|text| text.trim().to_owned())
        };
        let first_word = |code: &str| {
            credentials
                .words(code)
                .first()
                .map(|word| word.trim().to_owned())
        };
        let coded = |code: &str, errors: &mut Vec<String>| {
            let word = first_word(code)?;
            let read = match code {
                "consent" => ConsentSource::of(&word).map(Coded::Consent),
                "ehdsi_service" => EhdsiService::of(&word).map(Coded::Service),
                _ => AccessClass::of(&word).map(Coded::Class),
            };
            if read.is_none() {
                errors.push(format!("{code} holds {word:?}, which is none of its codes"));
            }
            read
        };
        let consent = match coded("consent", &mut errors) {
            Some(Coded::Consent(consent)) => Some(consent),
            _ => None,
        };
        let ehdsi_service = match coded("ehdsi_service", &mut errors) {
            Some(Coded::Service(service)) => Some(service),
            _ => None,
        };
        let access_class = match coded("access_class", &mut errors) {
            Some(Coded::Class(class)) => Some(class),
            _ => None,
        };
        let ground = ["gdpr_art_6_1", "gdpr_art_9_2", "national_provision"].map(first_word);
        let legal_ground = ground.iter().any(Option::is_some).then(|| {
            let [six, nine, national] = ground.map(Option::unwrap_or_default);
            LegalGround {
                gdpr_6_1: six,
                gdpr_9_2: nine,
                national,
            }
        });
        let discard_window_hours = first_text("discard_window").and_then(|window| {
            let hours = whole_hours(&window);
            if hours.is_none() {
                errors.push(format!(
                    "discard_window holds {window:?}, which is no whole number of hours above \
                     zero, such as \"PT24H\""
                ));
            }
            hours
        });
        let requester_log_retention = first_text("requester_log_retention").and_then(|period| {
            let parsed = period.parse::<RetentionPeriod>();
            if parsed.is_err() {
                errors.push(format!(
                    "requester_log_retention holds {period:?}, which is no calendar period of \
                     whole years or days, such as \"P5Y\""
                ));
            }
            parsed.ok()
        });
        let caller_rate = rate(credentials, &mut errors);
        let agreement = Self {
            accessor,
            name: first_text("agreement_name").unwrap_or_default(),
            version: first_text("agreement_version").unwrap_or_default(),
            jurisdiction: first_word("jurisdiction")
                .unwrap_or_default()
                .to_ascii_uppercase(),
            exchange: first_text("exchange").unwrap_or_default(),
            purpose: first_word("purpose"),
            legal_ground,
            consent,
            reads: owned(credentials.texts("read_scope")),
            writes: owned(credentials.texts("template")),
            discard_window_hours,
            ehdsi_service,
            caller_rate,
            opt_out_override: credentials.boolean("opt_out_override").unwrap_or(false),
            access_class,
            requester_log_retention,
        };
        if errors.is_empty() {
            Ok(agreement)
        } else {
            Err(errors)
        }
    }

    /// The rules the agreement breaks for a partner whose client identifier
    /// is held under `issuer`, by what `trust` holds of the issuers; empty
    /// when it keeps every one.
    #[must_use]
    pub fn refusals(&self, issuer: &str, trust: &FederationTrust) -> Vec<String> {
        let mut errors = Vec::new();
        self.common(&mut errors);
        let Some(partner) = trust.issuer(issuer) else {
            errors.push(format!(
                "its partner's client identifier names the issuer {}, which is no \
                 [[auth.issuer]] of kind federation",
                SecretUrl::new(issuer).redacted()
            ));
            return errors;
        };
        match (partner.contact_point, self.ehdsi_service) {
            (true, None) if self.accessor == Accessor::Person => errors.push(
                "its partner is a MyHealth@EU contact point and it needs ehdsi_service".to_owned(),
            ),
            (false, Some(_)) => errors.push(
                "it holds ehdsi_service, which only a contact point's agreement may".to_owned(),
            ),
            _ => {}
        }
        if self.accessor == Accessor::Organisation {
            self.organisation(partner, trust.holder(), &mut errors);
        }
        errors
    }

    /// The rules every agreement is held to.
    fn common(&self, errors: &mut Vec<String>) {
        if self.caller_rate.is_none() {
            errors.push(
                "it needs requests_per_second and burst, the admission rate per caller".to_owned(),
            );
        }
        if let Some(scope) = self.reads.iter().find(|s| !s.starts_with("patient/")) {
            errors.push(format!(
                "it reads {scope:?}: an agreement reads patient/ scopes only"
            ));
        }
        if self.legal_ground.as_ref().is_some_and(|g| !g.is_coded()) {
            errors.push(
                "its gdpr_art_6_1 is not a point a to f or its gdpr_art_9_2 is not a point a \
                 to j"
                    .to_owned(),
            );
        }
    }

    /// The rules of an agreement that admits an organisation without a named
    /// person.
    fn organisation(&self, partner: &PartnerIssuer, holder: Retention, errors: &mut Vec<String>) {
        for (code, missing) in [
            ("agreement_name", self.name.is_empty()),
            ("agreement_version", self.version.is_empty()),
            ("gdpr_art_6_1 and gdpr_art_9_2", self.legal_ground.is_none()),
            ("access_class", self.access_class.is_none()),
        ] {
            if missing {
                errors.push(format!("it admits an organisation and needs {code}"));
            }
        }
        if !self.writes.is_empty() {
            errors.push(
                "it lists a template: an organisation access is read-only (EHDS Art. 13(4))"
                    .to_owned(),
            );
        }
        if self.opt_out_override {
            errors.push("it sets opt_out_override, which only a person agreement may".to_owned());
        }
        if self.consent == Some(ConsentSource::Registry) {
            errors.push(
                "it takes consent from a registry, whose question names a responsible \
                 professional; an organisation agreement takes consent = local"
                    .to_owned(),
            );
        }
        if partner.contact_point {
            errors.push(
                "its partner is a MyHealth@EU contact point, whose requests name a professional"
                    .to_owned(),
            );
        }
        if self.access_class == Some(AccessClass::Chained) {
            self.chained(partner, holder, errors);
        }
    }

    /// A `chained` agreement declares a requester log retention at least the
    /// holder's, and its issuer reads the correlation id from the token.
    fn chained(&self, partner: &PartnerIssuer, holder: Retention, errors: &mut Vec<String>) {
        match partner.correlation {
            Correlation::None => errors.push(
                "it is chained and its issuer's [[auth.federation]] table maps no correlation_id"
                    .to_owned(),
            ),
            Correlation::Constant => errors.push(
                "it is chained and its issuer's [[auth.federation]] table maps correlation_id \
                 from a constant: the id is read only from the token"
                    .to_owned(),
            ),
            Correlation::Token => {}
        }
        let Some(period) = self.requester_log_retention else {
            errors.push("it is chained and needs requester_log_retention".to_owned());
            return;
        };
        if !covers(period, holder) {
            errors.push(format!(
                "it keeps the requester's log for \"{period}\", shorter than this node's \
                 audit.store {}",
                holder.describe()
            ));
        }
    }
}

/// One coded value an agreement element holds.
enum Coded {
    Consent(ConsentSource),
    Service(EhdsiService),
    Class(AccessClass),
}

/// The texts `texts`, owned and trimmed.
fn owned(texts: Vec<&str>) -> Vec<String> {
    texts
        .into_iter()
        .map(|text| text.trim().to_owned())
        .collect()
}

/// The whole hours above zero the ISO 8601 duration `text` names, such as
/// `PT24H` (RM `data_types` `DV_DURATION`).
fn whole_hours(text: &str) -> Option<u32> {
    let duration = text.parse::<jiff::SignedDuration>().ok()?;
    let seconds = duration.as_secs();
    if seconds <= 0 || duration.subsec_nanos() != 0 || seconds.checked_rem(3600) != Some(0) {
        return None;
    }
    u32::try_from(seconds.checked_div(3600)?).ok()
}

/// The caller rate the `requests_per_second` and `burst` counts name; an
/// error when only one is held or one is no count above zero.
fn rate(credentials: Credentials<'_>, errors: &mut Vec<String>) -> Option<CallerRate> {
    let count = |code: &str, errors: &mut Vec<String>| {
        let held = !credentials.named(code).is_empty();
        let value = credentials
            .count(code)
            .and_then(|count| u32::try_from(count).ok())
            .and_then(NonZeroU32::new);
        if held && value.is_none() {
            errors.push(format!("{code} holds no DV_COUNT above zero"));
        }
        (held, value)
    };
    let (rate_held, requests_per_second) = count("requests_per_second", errors);
    let (burst_held, burst) = count("burst", errors);
    if let (Some(requests_per_second), Some(burst)) = (requests_per_second, burst) {
        return Some(CallerRate {
            requests_per_second,
            burst,
        });
    }
    if rate_held != burst_held {
        errors.push("requests_per_second and burst are held together".to_owned());
    }
    None
}

/// Whether a requester keeping its log for `period` covers the holder's
/// horizon `holder`.
fn covers(period: RetentionPeriod, holder: Retention) -> bool {
    match (period, holder) {
        (_, Retention::Forever) => false,
        (RetentionPeriod::Years(years), Retention::Years(kept)) => years >= kept,
        (RetentionPeriod::Years(years), Retention::Days(kept)) => {
            u64::from(years) * 365 >= u64::from(kept)
        }
        (RetentionPeriod::Days(days), Retention::Years(kept)) => {
            u64::from(days) >= u64::from(kept) * 366
        }
        (RetentionPeriod::Days(days), Retention::Days(kept)) => days >= kept,
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_types,
        reason = "owner-approved 2026-08-03 (#1694 family 1): a capability's credentials tree"
    )]

    use std::collections::BTreeMap;

    use serde_json::{Value, json};

    use super::*;
    use crate::party_binding::grants::capabilities::Grant;
    use crate::party_binding::policy_mark::TERMINOLOGY;

    fn element(code: &str, value: &Value) -> Value {
        json!({"_type": "ELEMENT", "archetype_node_id": "at1",
               "name": {"_type": "DV_CODED_TEXT", "value": code,
                        "defining_code": {"terminology_id": {"value": TERMINOLOGY},
                                          "code_string": code}},
               "value": value})
    }

    fn text(value: &str) -> Value {
        json!({"_type": "DV_TEXT", "value": value})
    }

    fn count(value: i64) -> Value {
        json!({"_type": "DV_COUNT", "magnitude": value})
    }

    fn read(accessor: Accessor, items: &[Value]) -> Result<Agreement, Vec<String>> {
        let grant = Grant::new(
            accessor.operation(),
            uuid::Uuid::nil(),
            Some(std::sync::Arc::new(
                json!({"_type": "ITEM_TREE", "items": items}),
            )),
        );
        Agreement::read(accessor, grant.credentials())
    }

    fn rated() -> Vec<Value> {
        vec![
            element("requests_per_second", &count(5)),
            element("burst", &count(10)),
            element("read_scope", &text("patient/composition-*.r")),
        ]
    }

    fn trust(contact_point: bool, correlation: Correlation, holder: Retention) -> FederationTrust {
        FederationTrust::new(
            BTreeMap::from([(
                "https://fed".to_owned(),
                PartnerIssuer {
                    contact_point,
                    correlation,
                    correlation_source: None,
                },
            )]),
            holder,
        )
    }

    #[test]
    fn a_person_agreement_reads_its_terms_and_keeps_the_rules() {
        let mut items = rated();
        items.extend([
            element("agreement_name", &text("Regional exchange")),
            element("jurisdiction", &text("de")),
            element("gdpr_art_6_1", &text("e")),
            element("gdpr_art_9_2", &text("h")),
            element(
                "discard_window",
                &json!({"_type": "DV_DURATION", "value": "PT24H"}),
            ),
            element("template", &text("T.v1")),
        ]);
        let agreement = read(Accessor::Person, &items).expect("it reads");
        assert_eq!(agreement.name, "Regional exchange");
        assert_eq!(agreement.jurisdiction, "DE");
        assert_eq!(agreement.discard_window_hours, Some(24));
        assert_eq!(agreement.writes, ["T.v1"]);
        assert_eq!(
            agreement.legal_ground.as_ref().map(ToString::to_string),
            Some("GDPR Art. 6(1)(e), Art. 9(2)(h)".to_owned())
        );
        let none = trust(false, Correlation::None, Retention::Forever);
        assert_eq!(
            agreement.refusals("https://fed", &none),
            Vec::<String>::new()
        );
        let refused = agreement.refusals("https://persons", &none);
        assert!(
            refused[0].contains("no [[auth.issuer]] of kind federation"),
            "{refused:?}"
        );
    }

    #[test]
    fn an_element_holding_no_value_of_its_kind_is_named() {
        let errors = read(
            Accessor::Organisation,
            &[
                element("access_class", &text("sometimes")),
                element(
                    "discard_window",
                    &json!({"_type": "DV_DURATION", "value": "PT90M"}),
                ),
                element("requests_per_second", &count(0)),
                element("requester_log_retention", &text("forever")),
            ],
        )
        .expect_err("refused");
        let text = errors.join("; ");
        for code in [
            "access_class",
            "discard_window",
            "requests_per_second",
            "requester_log_retention",
        ] {
            assert!(text.contains(code), "{code}: {text}");
        }
    }

    #[test]
    fn every_organisation_rule_is_named() {
        let mut items = rated();
        items.extend([
            element("template", &text("T.v1")),
            element(
                "opt_out_override",
                &json!({"_type": "DV_BOOLEAN", "value": true}),
            ),
            element("consent", &text("registry")),
            element("access_class", &text("chained")),
            element("gdpr_art_6_1", &text("z")),
        ]);
        let agreement = read(Accessor::Organisation, &items).expect("it reads");
        let errors = agreement
            .refusals(
                "https://fed",
                &trust(true, Correlation::Constant, Retention::Years(5)),
            )
            .join("; ");
        for expected in [
            "needs agreement_name",
            "needs agreement_version",
            "gdpr_art_6_1 is not a point",
            "read-only (EHDS Art. 13(4))",
            "only a person agreement may",
            "takes consent from a registry",
            "contact point, whose requests name a professional",
            "from a constant",
            "needs requester_log_retention",
        ] {
            assert!(errors.contains(expected), "{expected}: {errors}");
        }
    }

    #[test]
    fn a_chained_requester_keeps_its_log_at_least_as_long_as_the_holder() {
        assert!(covers(RetentionPeriod::Years(5), Retention::Years(5)));
        assert!(!covers(RetentionPeriod::Years(1), Retention::Days(400)));
        assert!(!covers(RetentionPeriod::Years(99), Retention::Forever));
        assert!(covers(RetentionPeriod::Days(1830), Retention::Years(5)));
    }

    #[test]
    fn a_contact_points_person_agreement_names_its_service_and_no_other_does() {
        let person = read(Accessor::Person, &rated()).expect("it reads");
        let contact = trust(true, Correlation::None, Retention::Forever);
        assert!(person.refusals("https://fed", &contact)[0].contains("needs ehdsi_service"));
        let mut items = rated();
        items.push(element("ehdsi_service", &text("eprescription")));
        let served = read(Accessor::Person, &items).expect("it reads");
        assert_eq!(
            served.ehdsi_service.map(EhdsiService::not_found),
            Some("ERROR_EP_NOT_FOUND")
        );
        assert_eq!(
            served.refusals("https://fed", &contact),
            Vec::<String>::new()
        );
        let plain = trust(false, Correlation::None, Retention::Forever);
        assert!(served.refusals("https://fed", &plain)[0].contains("only a contact point's"));
    }

    #[test]
    fn every_agreement_needs_a_caller_rate_and_patient_reads() {
        let agreement = read(
            Accessor::Person,
            &[element("read_scope", &text("user/composition-*.r"))],
        )
        .expect("it reads");
        let errors = agreement
            .refusals(
                "https://fed",
                &trust(false, Correlation::None, Retention::Forever),
            )
            .join("; ");
        assert!(errors.contains("requests_per_second and burst"), "{errors}");
        assert!(errors.contains("patient/ scopes only"), "{errors}");
    }
}
