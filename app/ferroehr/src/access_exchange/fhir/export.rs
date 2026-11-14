// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Access directives written as `Consent`, one per directive.
//!
//! A directive with no `applies_to`, `operations`, `purposes` or `data` is
//! the base (R4 `policyRule` `OPTIN` or `OPTOUT`, the v3 `ActCode` codes the
//! R4 examples use under the extensible `ConsentPolicyRuleCodes` binding; R5
//! `decision`), and its exceptions are the root provisions. A scoped
//! directive is written as the base of the opposite effect, its scope as the
//! one root provision and its exceptions nested in it, with
//! [`super::EXT_BASE_UNDECIDED`] saying the base is no directive: outside the
//! scope FerroEHR decides by `default_access` and the other directives, which
//! FHIR has no element for. Every directive is `scope` `patient-privacy` in
//! R4 and carries its kind as a `category` of the access control
//! terminology beside LOINC 59284-0 Patient Consent (R4 `Consent.scope` and
//! `Consent.category`, both extensible).
//!
//! Each attribute with no core element goes in a FerroEHR extension:
//! `emergency_override`, an actor scope's `role`, `relationship` and
//! `caller_kind`, a data scope's `archetypes`, `origins` and the
//! `OBJECT_REF` of each object, R4's `time_validity` of an unscoped
//! directive, and, as [`super::EXT_DIRECTIVE`], every attribute no evaluator
//! reads. What neither a core element nor an extension can state is refused
//! with the attribute named: more than one data scope, an excluded bound,
//! and a set of HL7 v3 `Confidentiality` levels that is not every level up to
//! one, which a `securityLabel` stands for.

use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::ObjectRef;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{DirectiveKind, Effect, Term};
use openehr_rm::v1_2::common::generic::party_identified::PartyIdentified;
use openehr_rm::v1_2::common::generic::party_proxy::PartyProxy;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::quantity::dv_interval::DvInterval;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use crate::access_exchange::fhir::import::CONFIDENTIALITY_LEVELS;
use crate::access_exchange::fhir::model::{
    Actor, Code, Concept, DataRef, Document, Ext, ExtValue, Ident, Period, Ref, Rule,
};
use crate::access_exchange::fhir::{
    ACT_CODE, CONFIDENTIALITY, CONSENT_SCOPE, EXT_ARCHETYPE, EXT_BASE_UNDECIDED, EXT_CALLER_KIND,
    EXT_DIRECTIVE, EXT_EMERGENCY_OVERRIDE, EXT_OBJECT_REF, EXT_ORIGIN, EXT_RELATIONSHIP, EXT_ROLE,
    EXT_TIME_VALIDITY, LOINC, PARTICIPATION_TYPE, Release, TEMPLATE_SYSTEM, URI_SYSTEM,
};
use crate::access_exchange::rm::{bounds, members, opposite};
use crate::access_exchange::{Format, Loss, Refused};

/// What an export reads beside the directives.
#[derive(Debug, Clone)]
pub struct ExportContext {
    /// The EHR's subject, R4 `patient` and R5 `subject`.
    pub subject: Ref,
    /// The controller of the `ACCESS_POLICY`, R4 `organization` and R5
    /// `controller`.
    pub controller: Option<PartyRef>,
    /// When the `EHR_ACCESS` version was committed, R4 `dateTime` ("the
    /// time that this statement was made or derived").
    pub stated: String,
}

/// Writes each of `directives` as one `Consent` of `release`.
///
/// # Errors
/// [`Refused`] naming the first attribute a `Consent` cannot state (module
/// docs).
pub fn export(
    release: Release,
    directives: &[AccessDirective],
    context: &ExportContext,
) -> Result<Vec<Document>, Refused> {
    let writer = Writer { release, context };
    directives.iter().map(|d| writer.consent(d)).collect()
}

/// The FHIR reference of the demographic party `party`.
///
/// # Errors
/// [`Refused`] for a party outside the demographic namespace or of an id
/// kind a reference cannot carry.
pub fn party_reference(party: &PartyRef, attribute: &str, format: Format) -> Result<Ref, Refused> {
    let refused = || Refused::new(Format::Directive, attribute, Loss::NoFormatForm(format));
    if party.namespace != "demographic" {
        return Err(refused());
    }
    let kind = match party.r#type.as_str() {
        "PERSON" => Some("Person"),
        "ORGANISATION" => Some("Organization"),
        "GROUP" => Some("Group"),
        "AGENT" => Some("Device"),
        "ROLE" => Some("PractitionerRole"),
        _ => None,
    }
    .map(str::to_owned);
    match &party.id {
        ObjectId::GenericId(id) if id.scheme == "fhir" => Ok(Ref {
            reference: Some(id.value.clone()),
            ..Ref::default()
        }),
        ObjectId::GenericId(id) => Ok(Ref {
            kind,
            identifier: Some(Ident {
                system: Some(id.scheme.clone()),
                value: Some(id.value.clone()),
                ..Ident::default()
            }),
            ..Ref::default()
        }),
        ObjectId::HierObjectId(id) => {
            let value = id.value();
            let (system, value) = if uuid::Uuid::parse_str(value).is_ok() {
                (Some(URI_SYSTEM.to_owned()), format!("urn:uuid:{value}"))
            } else {
                (None, value.to_owned())
            };
            Ok(Ref {
                kind,
                identifier: Some(Ident {
                    system,
                    value: Some(value),
                    ..Ident::default()
                }),
                ..Ref::default()
            })
        }
        _ => Err(refused()),
    }
}

/// Writes directives of one release.
struct Writer<'a> {
    release: Release,
    context: &'a ExportContext,
}

impl Writer<'_> {
    fn refused(&self, attribute: impl Into<String>) -> Refused {
        Refused::new(
            Format::Directive,
            attribute,
            Loss::NoFormatForm(self.release.format()),
        )
    }

    fn consent(&self, directive: &AccessDirective) -> Result<Document, Refused> {
        DirectiveKind::read_coded(&directive.kind).map_err(|error| {
            Refused::new(Format::Directive, "kind", Loss::Code(error.to_string()))
        })?;
        let effect = Effect::read_coded(&directive.effect).map_err(|error| {
            Refused::new(Format::Directive, "effect", Loss::Code(error.to_string()))
        })?;
        let mut document = Document {
            status: "active".to_owned(),
            categories: vec![
                Concept::of(Code::of(LOINC, "59284-0", "Patient Consent")),
                Concept::of(code_of(&directive.kind)),
            ],
            subject: Some(self.context.subject.clone()),
            controllers: self
                .context
                .controller
                .as_ref()
                .map(|c| party_reference(c, "controller", self.release.format()))
                .transpose()?
                .into_iter()
                .collect(),
            instructed_by: self.instructed_by(directive)?,
            identifiers: members(directive.identifiers.as_ref())
                .iter()
                .map(|i| Ident {
                    system: i.issuer.clone(),
                    value: Some(i.id.clone()),
                    kind: i.r#type.clone(),
                    assigner: i.assigner.clone(),
                })
                .collect(),
            evidence: members(directive.evidence.as_ref())
                .iter()
                .map(|uri| Ref {
                    reference: Some(uri.value.clone()),
                    ..Ref::default()
                })
                .collect(),
            ..Document::default()
        };
        match self.release {
            Release::R4 => {
                document.scope = Some(Concept::of(Code::of(
                    CONSENT_SCOPE,
                    "patient-privacy",
                    "Privacy Consent",
                )));
                document.stated = Some(self.context.stated.clone());
            }
            Release::R5 => {
                document.stated = directive
                    .instructed
                    .as_ref()
                    .and_then(|at| at.value.get(..10))
                    .filter(|day| jiff::civil::Date::strptime("%Y-%m-%d", day).is_ok())
                    .map(str::to_owned);
                document.legal_grounds = members(directive.legal_grounds.as_ref())
                    .iter()
                    .map(|g| Concept::of(code_of(g)))
                    .collect();
            }
        }
        let own = residual(directive);
        if scoped(directive) {
            self.base(&mut document, opposite(effect));
            document.extensions.push(boolean(EXT_BASE_UNDECIDED, true));
            let mut rule = self.rule(directive, None)?;
            rule.extensions.push(own);
            rule.rules = members(directive.exceptions.as_ref())
                .iter()
                .map(|x| self.exception(x, Some(opposite(effect))))
                .collect::<Result<_, _>>()?;
            document.rules = vec![rule];
        } else {
            self.base(&mut document, effect);
            if let Some(validity) = &directive.time_validity {
                let period = self.period(validity, "time_validity")?;
                match self.release {
                    Release::R4 => document
                        .extensions
                        .push(ext(EXT_TIME_VALIDITY, ExtValue::Period(period))),
                    Release::R5 => document.period = Some(period),
                }
            }
            if directive.emergency_override {
                document
                    .extensions
                    .push(boolean(EXT_EMERGENCY_OVERRIDE, true));
            }
            document.extensions.push(own);
            document.rules = members(directive.exceptions.as_ref())
                .iter()
                .map(|x| self.exception(x, None))
                .collect::<Result<_, _>>()?;
        }
        if self.release == Release::R5 {
            document.grantees = grantees(&document.rules);
        }
        Ok(document)
    }

    /// Writes `effect` as the base.
    fn base(&self, document: &mut Document, effect: Effect) {
        match self.release {
            Release::R4 => {
                let code = match effect {
                    Effect::Permit => Code::of(ACT_CODE, "OPTIN", "opt-in"),
                    Effect::Deny => Code::of(ACT_CODE, "OPTOUT", "opt-out"),
                };
                document.policy_rule = Some(Concept::of(code));
            }
            Release::R5 => document.decision = Some(effect.code().to_owned()),
        }
    }

    /// R4 `performer`, R5 `grantor`: the party `instructed_by` names.
    fn instructed_by(&self, directive: &AccessDirective) -> Result<Vec<Ref>, Refused> {
        match &directive.instructed_by {
            Some(PartyProxy::PartyIdentified(PartyIdentified::PartyIdentified(data))) => {
                match (&data.external_ref, &data.name) {
                    (Some(party), _) => Ok(vec![party_reference(
                        party,
                        "instructed_by",
                        self.release.format(),
                    )?]),
                    (None, Some(name)) => Ok(vec![Ref {
                        display: Some(name.clone()),
                        ..Ref::default()
                    }]),
                    (None, None) => Ok(Vec::new()),
                }
            }
            Some(PartyProxy::PartySelf(_)) => Ok(vec![self.context.subject.clone()]),
            _ => Ok(Vec::new()),
        }
    }

    /// The exception `x`, nested under a rule of the effect opposite
    /// `effect` when `effect` is given, else a root provision.
    fn exception(&self, x: &AccessDirective, effect: Option<Effect>) -> Result<Rule, Refused> {
        if x.exceptions.is_some() {
            return Err(Refused::new(
                Format::Directive,
                "exceptions.exceptions",
                Loss::TooDeep,
            ));
        }
        let mut rule = self.rule(x, effect)?;
        rule.extensions.push(residual(x));
        Ok(rule)
    }

    /// The provision stating `directive`'s scope, validity and override;
    /// `effect` is written as R4 `type`, which a root provision omits.
    fn rule(&self, directive: &AccessDirective, effect: Option<Effect>) -> Result<Rule, Refused> {
        let mut rule = Rule {
            effect: effect
                .filter(|_| self.release == Release::R4)
                .map(|e| e.code().to_owned()),
            period: directive
                .time_validity
                .as_ref()
                .map(|v| self.period(v, "time_validity"))
                .transpose()?,
            actors: members(directive.applies_to.as_ref())
                .iter()
                .map(|scope| self.actor(scope))
                .collect::<Result<_, _>>()?,
            actions: members(directive.operations.as_ref())
                .iter()
                .map(|op| Concept::of(code_of(op)))
                .collect(),
            purposes: members(directive.purposes.as_ref())
                .iter()
                .map(code_of)
                .collect(),
            ..Rule::default()
        };
        if directive.emergency_override {
            rule.extensions.push(boolean(EXT_EMERGENCY_OVERRIDE, true));
        }
        match members(directive.data.as_ref()) {
            [] => {}
            [scope] => self.data(scope, &mut rule)?,
            _ => return Err(self.refused("data")),
        }
        Ok(rule)
    }

    fn actor(&self, scope: &AccessActorScope) -> Result<Actor, Refused> {
        let mut extensions = Vec::new();
        for (url, coded) in [
            (EXT_ROLE, &scope.role),
            (EXT_RELATIONSHIP, &scope.relationship),
            (EXT_CALLER_KIND, &scope.caller_kind),
        ] {
            if let Some(coded) = coded {
                extensions.push(ext(url, ExtValue::Coding(code_of(coded))));
            }
        }
        let reference = match (&scope.party, &scope.role) {
            (Some(party), _) => party_reference(party, "applies_to.party", self.release.format())?,
            (None, Some(role)) => Ref {
                kind: Some("Group".to_owned()),
                display: Some(role.value.clone()),
                ..Ref::default()
            },
            (None, None) => Ref {
                display: Some(
                    [&scope.caller_kind, &scope.relationship]
                        .into_iter()
                        .flatten()
                        .map(|c| c.value.clone())
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                ..Ref::default()
            },
        };
        Ok(Actor {
            role: Some(Concept::of(Code::of(
                PARTICIPATION_TYPE,
                "PRCP",
                "primary information recipient",
            ))),
            reference: Some(reference),
            extensions,
        })
    }

    /// Writes the one data scope `scope` into `rule`.
    fn data(&self, scope: &AccessDataScope, rule: &mut Rule) -> Result<(), Refused> {
        let mut levels = Vec::new();
        let mut at = None;
        for coded in members(scope.sensitivities.as_ref()) {
            if coded.defining_code.terminology_id.value == CONFIDENTIALITY {
                at.get_or_insert(rule.labels.len());
                levels.push(coded.defining_code.code_string.as_str());
            } else {
                rule.labels.push(code_of(coded));
            }
        }
        if let Some(at) = at {
            let top = CONFIDENTIALITY_LEVELS
                .iter()
                .rposition(|level| levels.contains(level))
                .and_then(|at| CONFIDENTIALITY_LEVELS.get(..=at))
                .filter(|upto| {
                    upto.len() == levels.len() && upto.iter().all(|l| levels.contains(l))
                })
                .and_then(|upto| upto.last())
                .ok_or_else(|| self.refused("data.sensitivities"))?;
            rule.labels.insert(at, Code::of(CONFIDENTIALITY, top, top));
        }
        rule.codes = members(scope.categories.as_ref())
            .iter()
            .map(|c| Concept::of(code_of(c)))
            .collect();
        rule.templates = members(scope.templates.as_ref())
            .iter()
            .map(|t| Code {
                system: Some(TEMPLATE_SYSTEM.to_owned()),
                code: Some(t.clone()),
                display: None,
            })
            .collect();
        for archetype in members(scope.archetypes.as_ref()) {
            rule.extensions.push(ext(
                EXT_ARCHETYPE,
                ExtValue::String(archetype.value.clone()),
            ));
        }
        for origin in members(scope.origins.as_ref()) {
            rule.extensions
                .push(ext(EXT_ORIGIN, ExtValue::String(origin.clone())));
        }
        rule.data_period = scope
            .recorded
            .as_ref()
            .map(|r| self.period(r, "data.recorded"))
            .transpose()?;
        rule.data = members(scope.objects.as_ref()).iter().map(object).collect();
        Ok(())
    }

    fn period(
        &self,
        interval: &DvInterval<DvDateTime>,
        attribute: &str,
    ) -> Result<Period, Refused> {
        let (start, end) = bounds(interval).ok_or_else(|| self.refused(attribute))?;
        Ok(Period { start, end })
    }
}

/// Whether `directive` has a scope: actors, operations, purposes or data.
fn scoped(directive: &AccessDirective) -> bool {
    directive.applies_to.is_some()
        || directive.operations.is_some()
        || directive.purposes.is_some()
        || directive.data.is_some()
}

/// The coding of `coded`, its terminology id as the system.
fn code_of(coded: &DvCodedText) -> Code {
    Code::of(
        &coded.defining_code.terminology_id.value,
        &coded.defining_code.code_string,
        &coded.value,
    )
}

fn ext(url: &str, value: ExtValue) -> Ext {
    Ext {
        url: url.to_owned(),
        value,
    }
}

fn boolean(url: &str, value: bool) -> Ext {
    ext(url, ExtValue::Boolean(value))
}

/// The `provision.data` of one object: its uuid as the reference and the
/// whole `OBJECT_REF` in the FerroEHR extension.
fn object(object: &ObjectRef) -> DataRef {
    let id = match object {
        ObjectRef::ObjectRef(data) => Some(&data.id),
        ObjectRef::PartyRef(party) => Some(&party.id),
        ObjectRef::LocatableRef(_) | ObjectRef::AccessGroupRef(_) => None,
    };
    let identifier = id.and_then(|id| match id {
        ObjectId::HierObjectId(hier) => Some(Ident {
            system: Some(URI_SYSTEM.to_owned()),
            value: Some(format!("urn:uuid:{}", hier.value())),
            ..Ident::default()
        }),
        _ => None,
    });
    DataRef {
        meaning: "instance".to_owned(),
        reference: Ref {
            identifier,
            ..Ref::default()
        },
        extensions: vec![ext(
            EXT_OBJECT_REF,
            ExtValue::String(openehr_its::json::to_canonical_json(object)),
        )],
    }
}

/// The [`EXT_DIRECTIVE`] extension of `directive`: the directive with every
/// attribute the evaluator reads cleared, which the core elements state.
fn residual(directive: &AccessDirective) -> Ext {
    let mut own = directive.clone();
    own.applies_to = None;
    own.operations = None;
    own.purposes = None;
    own.data = None;
    own.time_validity = None;
    own.emergency_override = false;
    own.exceptions = None;
    ext(
        EXT_DIRECTIVE,
        ExtValue::String(openehr_its::json::to_canonical_json(&own)),
    )
}

/// Every actor reference of `rules`, R5 `grantee` "for ease of search
/// only".
fn grantees(rules: &[Rule]) -> Vec<Ref> {
    let mut out: Vec<Ref> = Vec::new();
    for rule in rules {
        for reference in rule.actors.iter().filter_map(|a| a.reference.clone()) {
            if (reference.identifier.is_some() || reference.reference.is_some())
                && !out.contains(&reference)
            {
                out.push(reference);
            }
        }
        for nested in grantees(&rule.rules) {
            if !out.contains(&nested) {
                out.push(nested);
            }
        }
    }
    out
}
