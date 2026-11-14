// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The data types and the provision both releases share, read into and
//! written from [`super::model`] once, through one macro instantiated per
//! release over that release's generated types.
//!
//! The data types (`Coding`, `CodeableConcept`, `Identifier`, `Reference`,
//! `Period`, `Extension`) and the provision elements common to R4 and R5
//! have the same names and shapes in both releases (R4 and R5
//! `consent-definitions.html`), so the same text compiles against each. What
//! one release alone has is read and written by the release module through
//! the hooks the macro calls: `rule_extra_in`, `rule_extra_out`, `actor_in`
//! and `actor_out`.

/// Writes the shared readers and writers into the release module that
/// invokes it, over `fhir_types::$v`, naming refusals as `$format`.
macro_rules! release_codec {
    ($v:ident, $format:expr) => {
        use fhir_types::$v::codeable_concept::CodeableConcept;
        use fhir_types::$v::coding::Coding;
        use fhir_types::$v::consent::{ConsentProvision, ConsentProvisionData};
        use fhir_types::$v::extension::{Extension, ExtensionValue};
        use fhir_types::$v::identifier::Identifier;
        use fhir_types::$v::period::Period as FhirPeriod;
        use fhir_types::$v::primitives;
        use fhir_types::$v::reference::Reference;

        use crate::access_exchange::fhir::model::{
            Code, Concept, DataRef, Ext, ExtValue, Ident, Period, Ref, Rule,
        };
        use crate::access_exchange::{Loss, Refused};

        /// The value of a string-valued primitive.
        macro_rules! text {
            ($primitive:expr) => {
                $primitive.as_ref().and_then(|p| p.value.clone())
            };
        }

        /// A string-valued primitive of `value`.
        macro_rules! prim {
            ($kind:ident, $value:expr) => {
                $value.map(|value| primitives::$kind {
                    value: Some(value),
                    ..Default::default()
                })
            };
        }

        /// Refuses `element` when it carries a modifier extension, which a
        /// reader that does not know it must not ignore (FHIR R4 and R5
        /// `extensibility.html`, "modifierExtension").
        fn unmodified(modifiers: &[Extension], element: &str) -> Result<(), Refused> {
            if modifiers.is_empty() {
                Ok(())
            } else {
                Err(Refused::new(
                    $format,
                    format!("{element}.modifierExtension"),
                    Loss::NoDirectiveForm,
                ))
            }
        }

        fn code_in(coding: &Coding) -> Code {
            Code {
                system: text!(coding.system),
                code: text!(coding.code),
                display: text!(coding.display),
            }
        }

        fn code_out(code: &Code) -> Coding {
            Coding {
                system: prim!(Uri, code.system.clone()),
                code: prim!(Code, code.code.clone()),
                display: prim!(String, code.display.clone()),
                ..Default::default()
            }
        }

        fn concept_in(concept: &CodeableConcept) -> Concept {
            Concept {
                codings: concept.coding.iter().map(code_in).collect(),
                text: text!(concept.text),
            }
        }

        fn concept_out(concept: &Concept) -> CodeableConcept {
            CodeableConcept {
                coding: concept.codings.iter().map(code_out).collect(),
                text: prim!(String, concept.text.clone()),
                ..Default::default()
            }
        }

        fn ident_in(identifier: &Identifier) -> Ident {
            Ident {
                system: text!(identifier.system),
                value: text!(identifier.value),
                kind: identifier.r#type.as_ref().and_then(|t| text!(t.text)),
                assigner: identifier
                    .assigner
                    .as_ref()
                    .and_then(|assigner| text!(assigner.display)),
            }
        }

        fn ident_out(ident: &Ident) -> Identifier {
            Identifier {
                system: prim!(Uri, ident.system.clone()),
                value: prim!(String, ident.value.clone()),
                r#type: ident.kind.clone().map(|kind| CodeableConcept {
                    text: prim!(String, Some(kind)),
                    ..Default::default()
                }),
                assigner: ident.assigner.clone().map(|display| {
                    Box::new(Reference {
                        display: prim!(String, Some(display)),
                        ..Default::default()
                    })
                }),
                ..Default::default()
            }
        }

        fn ref_in(reference: &Reference) -> Ref {
            Ref {
                reference: text!(reference.reference),
                kind: text!(reference.r#type),
                identifier: reference.identifier.as_deref().map(ident_in),
                display: text!(reference.display),
            }
        }

        fn ref_out(reference: &Ref) -> Reference {
            Reference {
                reference: prim!(String, reference.reference.clone()),
                r#type: prim!(Uri, reference.kind.clone()),
                identifier: reference.identifier.as_ref().map(|i| Box::new(ident_out(i))),
                display: prim!(String, reference.display.clone()),
                ..Default::default()
            }
        }

        fn period_in(period: &FhirPeriod) -> Period {
            Period {
                start: text!(period.start),
                end: text!(period.end),
            }
        }

        fn period_out(period: &Period) -> FhirPeriod {
            FhirPeriod {
                start: prim!(DateTime, period.start.clone()),
                end: prim!(DateTime, period.end.clone()),
                ..Default::default()
            }
        }

        fn ext_in(extension: &Extension) -> Ext {
            let value = match &extension.value {
                Some(ExtensionValue::Boolean(b)) => b.value.map_or(ExtValue::Other, ExtValue::Boolean),
                Some(ExtensionValue::String(s)) => {
                    s.value.clone().map_or(ExtValue::Other, ExtValue::String)
                }
                Some(ExtensionValue::Coding(c)) => ExtValue::Coding(code_in(c)),
                Some(ExtensionValue::Period(p)) => ExtValue::Period(period_in(p)),
                _ => ExtValue::Other,
            };
            Ext {
                url: extension.url.clone(),
                value,
            }
        }

        fn ext_out(ext: &Ext) -> Extension {
            let value = match &ext.value {
                ExtValue::Boolean(b) => Some(ExtensionValue::Boolean(primitives::Boolean {
                    value: Some(*b),
                    ..Default::default()
                })),
                ExtValue::String(s) => prim!(String, Some(s.clone())).map(ExtensionValue::String),
                ExtValue::Coding(c) => Some(ExtensionValue::Coding(Box::new(code_out(c)))),
                ExtValue::Period(p) => Some(ExtensionValue::Period(Box::new(period_out(p)))),
                ExtValue::Other => None,
            };
            Extension {
                url: ext.url.clone(),
                value,
                ..Default::default()
            }
        }

        /// `attachment` as `details` keeps it; its `data` is refused, since
        /// the evidence of a directive is a composition the EHR holds.
        fn attachment_kept(
            attachment: &fhir_types::$v::attachment::Attachment,
            element: &str,
        ) -> Result<(String, String), Refused> {
            if attachment.data.is_some() {
                return Err(Refused::new(
                    $format,
                    format!("{element}.data"),
                    Loss::NoDirectiveForm,
                ));
            }
            let rendering = crate::access_exchange::fhir::model::line([
                text!(attachment.title),
                text!(attachment.url),
                text!(attachment.content_type),
            ]);
            Ok((element.to_owned(), rendering))
        }

        fn data_in(data: &ConsentProvisionData, element: &str) -> Result<DataRef, Refused> {
            unmodified(&data.modifier_extension, element)?;
            Ok(DataRef {
                meaning: text!(Some(&data.meaning)).unwrap_or_default(),
                reference: ref_in(&data.reference),
                extensions: data.extension.iter().map(ext_in).collect(),
            })
        }

        fn data_out(data: &DataRef) -> ConsentProvisionData {
            ConsentProvisionData {
                meaning: primitives::Code {
                    value: Some(data.meaning.clone()),
                    ..Default::default()
                },
                reference: ref_out(&data.reference),
                extension: data.extensions.iter().map(ext_out).collect(),
                ..Default::default()
            }
        }

        /// Reads `provision`, named `element`, and every provision nested in
        /// it.
        fn rule_in(provision: &ConsentProvision, element: &str) -> Result<Rule, Refused> {
            unmodified(&provision.modifier_extension, element)?;
            let actor_element = format!("{element}.actor");
            let data_element = format!("{element}.data");
            let nested_element = format!("{element}.provision");
            let mut rule = Rule {
                period: provision.period.as_ref().map(period_in),
                actors: provision
                    .actor
                    .iter()
                    .map(|actor| actor_in(actor, &actor_element))
                    .collect::<Result<_, _>>()?,
                actions: provision.action.iter().map(concept_in).collect(),
                labels: provision.security_label.iter().map(code_in).collect(),
                purposes: provision.purpose.iter().map(code_in).collect(),
                codes: provision.code.iter().map(concept_in).collect(),
                data_period: provision.data_period.as_ref().map(period_in),
                data: provision
                    .data
                    .iter()
                    .map(|data| data_in(data, &data_element))
                    .collect::<Result<_, _>>()?,
                rules: provision
                    .provision
                    .iter()
                    .map(|nested| rule_in(nested, &nested_element))
                    .collect::<Result<_, _>>()?,
                extensions: provision.extension.iter().map(ext_in).collect(),
                ..Rule::default()
            };
            rule_extra_in(provision, &mut rule);
            Ok(rule)
        }

        /// Writes `rule` and every rule nested in it.
        fn rule_out(rule: &Rule) -> ConsentProvision {
            let mut provision = ConsentProvision {
                period: rule.period.as_ref().map(period_out),
                actor: rule.actors.iter().map(actor_out).collect(),
                action: rule.actions.iter().map(concept_out).collect(),
                security_label: rule.labels.iter().map(code_out).collect(),
                purpose: rule.purposes.iter().map(code_out).collect(),
                code: rule.codes.iter().map(concept_out).collect(),
                data_period: rule.data_period.as_ref().map(period_out),
                data: rule.data.iter().map(data_out).collect(),
                provision: rule.rules.iter().map(rule_out).collect(),
                extension: rule.extensions.iter().map(ext_out).collect(),
                ..Default::default()
            };
            rule_extra_out(rule, &mut provision);
            provision
        }
    };
}

pub(super) use release_codec;
