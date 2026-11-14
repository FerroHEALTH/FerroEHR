// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `Consent` read as access directives.
//!
//! The root provisions are exceptions to the base (R4
//! `Consent.provision`: "An exception to the base policy of this consent";
//! R5 `Consent.provision.provision`: "Provisions which provide exceptions to
//! the base provision or subprovisions"). Our own design reads them so:
//!
//! - A `Consent` FerroEHR wrote from a scoped directive carries
//!   [`super::EXT_BASE_UNDECIDED`]: each root provision is a directive, its
//!   nested provisions the exceptions, and the base no directive.
//! - A base naming an `ACCESS_POLICY.catalogue` entry (an R4 `policyRule`
//!   other than `OPTIN` or `OPTOUT`) with no provisions is a copy of that
//!   entry.
//! - Root provisions that nest nothing are the exceptions of one directive:
//!   the base's effect over the whole EHR.
//! - Root provisions that nest further are one directive each: the
//!   provision's effect over its scope, the nested provisions its
//!   exceptions. A permitting base adds a whole-EHR permit, which
//!   deny-overrides leaves under every scoped deny; a denying base adds
//!   nothing where `default_access` is not `open`, and is refused where it
//!   is, since a whole-EHR deny would override the scoped permits.
//!
//! A provision nested a level deeper, an R4 `type` equal to its parent's
//! effect, `provision.data` of a `meaning` other than `instance`, R5
//! `provision.expression` and `resourceType`, and every element with no
//! directive attribute and no place in `details` refuse the whole
//! `Consent` with the element named.

use openehr_base::v1_3::base_types::identification::archetype_id::ArchetypeId;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::{ObjectRef, ObjectRefData};
use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, DirectiveKind, Effect, OperationClass, Relationship,
    TERMINOLOGY_ID_ACCESS_CONTROL, Term,
};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use crate::access_exchange::fhir::model::{Code, Concept, DataRef, Document, Ext, ExtValue, Rule};
use crate::access_exchange::fhir::{
    ACT_CODE, CONFIDENTIALITY, EXT_ARCHETYPE, EXT_BASE_UNDECIDED, EXT_CALLER_KIND, EXT_DIRECTIVE,
    EXT_EMERGENCY_OVERRIDE, EXT_OBJECT_REF, EXT_ORIGIN, EXT_RELATIONSHIP, EXT_ROLE,
    EXT_TIME_VALIDITY, PARTICIPATION_TYPE, Release, TEMPLATE_SYSTEM, URI_SYSTEM,
};
use crate::access_exchange::rm::{directive, interval, list, opposite, term};
use crate::access_exchange::{Loss, Refused, catalogue_entry};
use crate::access_log::entry::coded;

mod meta;

/// HL7 v3 `Confidentiality`, lowest first: a label "applies to all
/// resources that are labeled [with it] or lower" (R4 and R5
/// `Consent.provision.securityLabel`).
pub const CONFIDENTIALITY_LEVELS: [&str; 6] = ["U", "L", "M", "N", "R", "V"];

/// The actor roles that name who receives the data, the reading of
/// `applies_to`: v3 `ParticipationType` `PRCP` (primary information
/// recipient), the role the R4 examples use, and `IRCP` (information
/// recipient). Our own design: another role says how else the actor is
/// involved (R4 `Consent.provision.actor.role`), which no directive holds.
const RECIPIENT_ROLES: [&str; 2] = ["PRCP", "IRCP"];

/// What an import reads beside the `Consent`.
pub struct ImportContext<'a> {
    /// The `ACCESS_POLICY.catalogue` in force.
    pub catalogue: &'a [AccessDirective],
    /// The `default_access` of the EHR the directives join.
    pub default_access: AccessDefault,
    /// The operation class a terminology server translates a foreign action
    /// coding to.
    pub operations: &'a dyn Fn(&Code) -> Option<OperationClass>,
}

impl std::fmt::Debug for ImportContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImportContext")
            .field("catalogue", &self.catalogue.len())
            .field("default_access", &self.default_access)
            .finish_non_exhaustive()
    }
}

/// The base of a `Consent`.
enum Base<'a> {
    /// None is stated.
    None,
    /// `OPTIN`/`OPTOUT` or `decision`.
    Effect(Effect),
    /// A catalogue entry.
    Catalogue(&'a AccessDirective),
}

/// The action codings of `document` none of whose codings is of the access
/// control terminology: the ones a terminology server translates.
#[must_use]
pub fn foreign_actions(document: &Document) -> Vec<Code> {
    fn walk(rules: &[Rule], out: &mut Vec<Code>) {
        for rule in rules {
            for action in &rule.actions {
                if !action
                    .codings
                    .iter()
                    .any(|c| c.system.as_deref() == Some(TERMINOLOGY_ID_ACCESS_CONTROL))
                {
                    out.extend(action.codings.iter().cloned());
                }
            }
            walk(&rule.rules, out);
        }
    }
    let mut out = Vec::new();
    walk(&document.rules, &mut out);
    out
}

/// Reads `document`, a `Consent` of `release`, as directives.
///
/// # Errors
/// [`Refused`] naming the first element with no directive form (module
/// docs).
pub fn import(
    release: Release,
    document: &Document,
    context: &ImportContext<'_>,
) -> Result<Vec<AccessDirective>, Refused> {
    let reader = Reader {
        release,
        context,
        document,
    };
    reader.directives()
}

/// Reads one `Consent`.
struct Reader<'a> {
    release: Release,
    context: &'a ImportContext<'a>,
    document: &'a Document,
}

impl Reader<'_> {
    fn refused(&self, element: impl Into<String>, loss: Loss) -> Refused {
        Refused::new(self.release.format(), element, loss)
    }

    fn base_element(&self) -> &'static str {
        match self.release {
            Release::R4 => "Consent.policyRule",
            Release::R5 => "Consent.decision",
        }
    }

    fn directives(&self) -> Result<Vec<AccessDirective>, Refused> {
        let shared = self.shared()?;
        let kind = shared.kind;
        let rules = &self.document.rules;
        let effect = match self.base()? {
            Base::Catalogue(entry) => {
                if !rules.is_empty() {
                    return Err(self.refused("Consent.provision", Loss::NoDirectiveForm));
                }
                let mut copy = entry.clone();
                shared.stamp_copy(&mut copy);
                return Ok(vec![copy]);
            }
            Base::None => return Err(self.refused(self.base_element(), Loss::NoBase)),
            Base::Effect(effect) => effect,
        };
        let scoped = opposite(effect);
        if flag(&self.document.extensions, EXT_BASE_UNDECIDED) {
            return rules
                .iter()
                .map(|rule| self.scoped(rule, scoped, kind, &shared))
                .collect();
        }
        if rules.iter().all(|rule| rule.rules.is_empty()) {
            let mut whole = directive(kind, effect);
            whole.time_validity = self.consent_validity();
            whole.emergency_override = flag(&self.document.extensions, EXT_EMERGENCY_OVERRIDE);
            whole.exceptions = list(
                rules
                    .iter()
                    .map(|rule| self.exception(rule, scoped, kind, "Consent.provision"))
                    .collect::<Result<_, _>>()?,
            );
            shared.stamp(&mut whole, residual(&self.document.extensions, self)?);
            return Ok(vec![whole]);
        }
        let mut out = rules
            .iter()
            .map(|rule| self.scoped(rule, scoped, kind, &shared))
            .collect::<Result<Vec<_>, _>>()?;
        match effect {
            Effect::Permit => {
                let mut whole = directive(kind, Effect::Permit);
                whole.time_validity = self.consent_validity();
                shared.stamp(&mut whole, None);
                out.push(whole);
            }
            Effect::Deny if self.context.default_access == AccessDefault::Open => {
                return Err(self.refused(self.base_element(), Loss::DenyOverOpen));
            }
            Effect::Deny => {}
        }
        Ok(out)
    }

    /// The base the `Consent` states.
    fn base(&self) -> Result<Base<'_>, Refused> {
        match self.release {
            Release::R4 => {
                let Some(rule) = &self.document.policy_rule else {
                    return Ok(Base::None);
                };
                if rule.codings.iter().any(|c| c.is(ACT_CODE, "OPTIN")) {
                    return Ok(Base::Effect(Effect::Permit));
                }
                if rule.codings.iter().any(|c| c.is(ACT_CODE, "OPTOUT")) {
                    return Ok(Base::Effect(Effect::Deny));
                }
                rule.codings
                    .iter()
                    .find_map(|c| {
                        catalogue_entry(
                            self.context.catalogue,
                            c.system.as_deref(),
                            c.code.as_deref().unwrap_or_default(),
                        )
                    })
                    .map(Base::Catalogue)
                    .ok_or_else(|| {
                        let named = rule.codings.iter().map(Code::token).collect::<Vec<_>>();
                        self.refused("Consent.policyRule", Loss::NotInCatalogue(named.join(", ")))
                    })
            }
            Release::R5 => match self.document.decision.as_deref() {
                None => Ok(Base::None),
                Some(decision) => self.effect(decision, "Consent.decision").map(Base::Effect),
            },
        }
    }

    /// `code` as an effect.
    fn effect(&self, code: &str, element: &str) -> Result<Effect, Refused> {
        match code {
            "permit" => Ok(Effect::Permit),
            "deny" => Ok(Effect::Deny),
            other => Err(self.refused(element, Loss::Code(other.to_owned()))),
        }
    }

    /// The effect of `rule`, an exception to a rule of the effect opposite
    /// `expected`: R4 `type` when present, which must be `expected`.
    fn rule_effect(&self, rule: &Rule, expected: Effect, element: &str) -> Result<(), Refused> {
        match rule.effect.as_deref() {
            None => Ok(()),
            Some(code) if self.effect(code, &format!("{element}.type"))? == expected => Ok(()),
            Some(_) => Err(self.refused(format!("{element}.type"), Loss::NotException)),
        }
    }

    /// `ACCESS_DIRECTIVE.time_validity` of a directive the `Consent` itself
    /// states: R5 `Consent.period`, or the R4 extension.
    fn consent_validity(
        &self,
    ) -> Option<openehr_rm::v1_2::data_types::quantity::dv_interval::DvInterval<
        openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime,
    >> {
        let period = match self.release {
            Release::R5 => self.document.period.clone(),
            Release::R4 => self
                .document
                .extensions
                .iter()
                .find(|e| e.url == EXT_TIME_VALIDITY)
                .and_then(|e| match &e.value {
                    ExtValue::Period(p) => Some(p.clone()),
                    _ => None,
                }),
        };
        period.map(|p| interval(p.start, p.end))
    }

    /// The directive `rule`, a root provision of `effect` with its nested
    /// provisions as exceptions.
    fn scoped(
        &self,
        rule: &Rule,
        effect: Effect,
        kind: DirectiveKind,
        shared: &meta::Shared,
    ) -> Result<AccessDirective, Refused> {
        let element = "Consent.provision";
        self.rule_effect(rule, effect, element)?;
        let mut scoped = directive(kind, effect);
        self.criteria(rule, element, &mut scoped)?;
        if scoped.time_validity.is_none() {
            scoped.time_validity = self.consent_validity();
        }
        let nested = format!("{element}.provision");
        scoped.exceptions = list(
            rule.rules
                .iter()
                .map(|x| self.exception(x, opposite(effect), kind, &nested))
                .collect::<Result<_, _>>()?,
        );
        shared.stamp(&mut scoped, residual(&rule.extensions, self)?);
        Ok(scoped)
    }

    /// The exception `rule` of `effect`, which nests nothing.
    fn exception(
        &self,
        rule: &Rule,
        effect: Effect,
        kind: DirectiveKind,
        element: &str,
    ) -> Result<AccessDirective, Refused> {
        if !rule.rules.is_empty() {
            return Err(self.refused(format!("{element}.provision"), Loss::TooDeep));
        }
        self.rule_effect(rule, effect, element)?;
        let own = residual(&rule.extensions, self)?;
        let kind = own
            .as_ref()
            .and_then(|own| DirectiveKind::read_coded(&own.kind).ok())
            .unwrap_or(kind);
        let mut exception = directive(kind, effect);
        self.criteria(rule, element, &mut exception)?;
        meta::plumb(&mut exception, own);
        Ok(exception)
    }

    /// The scope, validity and override `rule` states, into `target`.
    fn criteria(
        &self,
        rule: &Rule,
        element: &str,
        target: &mut AccessDirective,
    ) -> Result<(), Refused> {
        self.known(&rule.extensions, element)?;
        let actors = format!("{element}.actor");
        target.applies_to = list(
            rule.actors
                .iter()
                .map(|actor| self.actor(actor, &actors))
                .collect::<Result<_, _>>()?,
        );
        target.operations = list(
            rule.actions
                .iter()
                .map(|action| self.operation(action, &format!("{element}.action")))
                .collect::<Result<_, _>>()?,
        );
        target.purposes = list(
            rule.purposes
                .iter()
                .map(|code| self.verbatim(code, &format!("{element}.purpose")))
                .collect::<Result<_, _>>()?,
        );
        target.data = self.data(rule, element)?.map(|scope| vec![scope]).and_then(list);
        target.time_validity = rule.period.clone().map(|p| interval(p.start, p.end));
        target.emergency_override = flag(&rule.extensions, EXT_EMERGENCY_OVERRIDE);
        Ok(())
    }

    fn actor(
        &self,
        actor: &crate::access_exchange::fhir::model::Actor,
        element: &str,
    ) -> Result<AccessActorScope, Refused> {
        self.known(&actor.extensions, element)?;
        if let Some(role) = &actor.role
            && !role
                .codings
                .iter()
                .any(|c| RECIPIENT_ROLES.iter().any(|r| c.is(PARTICIPATION_TYPE, r)))
        {
            let named = role.codings.iter().map(Code::token).collect::<Vec<_>>();
            return Err(self.refused(format!("{element}.role"), Loss::Code(named.join(", "))));
        }
        let coding = |url: &str| {
            actor.extensions.iter().find(|e| e.url == url).map(|e| match &e.value {
                ExtValue::Coding(code) => self.verbatim(code, &format!("{element}.extension")),
                _ => Err(self.refused(
                    format!("{element}.extension"),
                    Loss::Malformed(format!("{url} takes valueCoding")),
                )),
            })
        };
        let role = coding(EXT_ROLE).transpose()?;
        let relationship = coding(EXT_RELATIONSHIP).transpose()?;
        let caller_kind = coding(EXT_CALLER_KIND).transpose()?;
        for (coded, holds) in [
            (&relationship, relationship.as_ref().map(|c| Relationship::read_coded(c).is_ok())),
            (&caller_kind, caller_kind.as_ref().map(|c| CallerKind::read_coded(c).is_ok())),
        ] {
            if let (Some(coded), Some(false)) = (coded, holds) {
                return Err(self.refused(
                    format!("{element}.extension"),
                    Loss::Code(coded.defining_code.code_string.clone()),
                ));
            }
        }
        let party = actor.reference.as_ref().and_then(meta::party_of);
        if party.is_none() && role.is_none() && relationship.is_none() && caller_kind.is_none() {
            return Err(self.refused(format!("{element}.reference"), Loss::Unresolved));
        }
        Ok(AccessActorScope {
            party,
            role,
            relationship,
            caller_kind,
        })
    }

    /// The operation class `action` names, read from the access control
    /// terminology or translated by the terminology server.
    fn operation(&self, action: &Concept, element: &str) -> Result<DvCodedText, Refused> {
        if let Some(own) = action
            .codings
            .iter()
            .find(|c| c.system.as_deref() == Some(TERMINOLOGY_ID_ACCESS_CONTROL))
        {
            let code = own.code.as_deref().unwrap_or_default();
            return OperationClass::read(TERMINOLOGY_ID_ACCESS_CONTROL, code)
                .map(term)
                .map_err(|error| self.refused(element, Loss::Code(error.to_string())));
        }
        action
            .codings
            .iter()
            .find_map(|c| (self.context.operations)(c))
            .map(term)
            .ok_or_else(|| {
                let named = action.codings.iter().map(Code::token).collect::<Vec<_>>();
                self.refused(element, Loss::Untranslated(named.join(", ")))
            })
    }

    /// Refuses an extension of `element` the mapping does not read.
    fn known(&self, extensions: &[Ext], element: &str) -> Result<(), Refused> {
        match extensions
            .iter()
            .find(|e| !meta::KNOWN.contains(&e.url.as_str()))
        {
            Some(unknown) => Err(self.refused(
                format!("{element}.extension"),
                Loss::Code(unknown.url.clone()),
            )),
            None => Ok(()),
        }
    }

    /// `code` as a `DV_CODED_TEXT` of its own system, verbatim.
    fn verbatim(&self, code: &Code, element: &str) -> Result<DvCodedText, Refused> {
        match (&code.system, &code.code) {
            (Some(system), Some(value)) => Ok(coded(
                system,
                value,
                code.display.as_deref().unwrap_or(value),
            )),
            _ => Err(self.refused(
                element,
                Loss::Malformed("a coding needs a system and a code".to_owned()),
            )),
        }
    }

    /// The one data scope `rule` states; `None` when it states none.
    fn data(&self, rule: &Rule, element: &str) -> Result<Option<AccessDataScope>, Refused> {
        if !rule.resource_types.is_empty() {
            return Err(self.refused(format!("{element}.resourceType"), Loss::NoDirectiveForm));
        }
        if rule.expression {
            return Err(self.refused(format!("{element}.expression"), Loss::NoDirectiveForm));
        }
        let mut sensitivities = Vec::new();
        for label in &rule.labels {
            let label_element = format!("{element}.securityLabel");
            if label.system.as_deref() == Some(CONFIDENTIALITY) {
                let code = label.code.as_deref().unwrap_or_default();
                let Some(at) = CONFIDENTIALITY_LEVELS.iter().position(|l| *l == code) else {
                    return Err(self.refused(label_element, Loss::Code(label.token())));
                };
                for level in CONFIDENTIALITY_LEVELS.iter().take(at.saturating_add(1)) {
                    let level = coded(CONFIDENTIALITY, level, level);
                    if !sensitivities.contains(&level) {
                        sensitivities.push(level);
                    }
                }
            } else {
                sensitivities.push(self.verbatim(label, &label_element)?);
            }
        }
        let categories = rule
            .codes
            .iter()
            .map(|concept| match concept.codings.as_slice() {
                [one] => self.verbatim(one, &format!("{element}.code")),
                _ => Err(self.refused(format!("{element}.code"), Loss::Several)),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let template_element = match self.release {
            Release::R4 => format!("{element}.class"),
            Release::R5 => format!("{element}.documentType"),
        };
        let templates = rule
            .templates
            .iter()
            .map(|code| match (code.system.as_deref(), &code.code) {
                (Some(TEMPLATE_SYSTEM), Some(id)) => Ok(id.clone()),
                _ => Err(self.refused(template_element.as_str(), Loss::Code(code.token()))),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let objects = rule
            .data
            .iter()
            .map(|data| self.object(data, &format!("{element}.data")))
            .collect::<Result<Vec<_>, _>>()?;
        let scope = AccessDataScope {
            objects: list(objects),
            templates: list(templates),
            archetypes: list(
                strings(&rule.extensions, EXT_ARCHETYPE)
                    .into_iter()
                    .map(|value| ArchetypeId { value })
                    .collect(),
            ),
            categories: list(categories),
            sensitivities: list(sensitivities),
            origins: list(strings(&rule.extensions, EXT_ORIGIN)),
            recorded: rule.data_period.clone().map(|p| interval(p.start, p.end)),
        };
        let empty = scope.objects.is_none()
            && scope.templates.is_none()
            && scope.archetypes.is_none()
            && scope.categories.is_none()
            && scope.sensitivities.is_none()
            && scope.origins.is_none()
            && scope.recorded.is_none();
        Ok((!empty).then_some(scope))
    }

    /// The object `data` names: the `OBJECT_REF` the FerroEHR extension
    /// carries, or a versioned object by its uuid.
    fn object(&self, data: &DataRef, element: &str) -> Result<ObjectRef, Refused> {
        self.known(&data.extensions, element)?;
        if data.meaning != "instance" {
            return Err(self.refused(
                format!("{element}.meaning"),
                Loss::Code(data.meaning.clone()),
            ));
        }
        if let Some(json) = strings(&data.extensions, EXT_OBJECT_REF).first() {
            return openehr_its::json::from_canonical_json(json).map_err(|error| {
                self.refused(
                    format!("{element}.extension"),
                    Loss::Malformed(error.message().to_owned()),
                )
            });
        }
        data.reference
            .identifier
            .as_ref()
            .filter(|i| i.system.as_deref() == Some(URI_SYSTEM))
            .and_then(|i| i.value.as_deref()?.strip_prefix("urn:uuid:"))
            .and_then(|uuid| HierObjectId::new(uuid).ok())
            .map(|id| {
                ObjectRef::ObjectRef(ObjectRefData {
                    namespace: "local".to_owned(),
                    r#type: "VERSIONED_OBJECT".to_owned(),
                    id: ObjectId::HierObjectId(id),
                })
            })
            .ok_or_else(|| self.refused(format!("{element}.reference"), Loss::Unresolved))
    }
}

/// Whether `extensions` hold `url` with `valueBoolean` true.
fn flag(extensions: &[Ext], url: &str) -> bool {
    extensions
        .iter()
        .any(|e| e.url == url && e.value == ExtValue::Boolean(true))
}

/// The `valueString`s of `url` in `extensions`.
fn strings(extensions: &[Ext], url: &str) -> Vec<String> {
    extensions
        .iter()
        .filter(|e| e.url == url)
        .filter_map(|e| match &e.value {
            ExtValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// The directive the [`EXT_DIRECTIVE`] extension of `extensions` carries.
fn residual(extensions: &[Ext], reader: &Reader<'_>) -> Result<Option<AccessDirective>, Refused> {
    strings(extensions, EXT_DIRECTIVE)
        .first()
        .map(|json| {
            openehr_its::json::from_canonical_json::<AccessDirective>(json).map_err(|error| {
                reader.refused("extension", Loss::Malformed(error.message().to_owned()))
            })
        })
        .transpose()
}
