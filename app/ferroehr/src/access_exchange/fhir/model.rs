// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! One `Consent` as both releases read: the elements the mapping reads,
//! named by release-neutral fields, so the mapping is written once and each
//! release only reads and writes its own resource.
//!
//! A reader fills every field from the resource and refuses an element with
//! no field here; a writer writes every field. The fields that only one
//! release has are named so.

/// A `Coding`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_field_names,
    reason = "the fields are the elements of FHIR `Coding`, `code` among them"
)]
pub struct Code {
    /// `system`.
    pub system: Option<String>,
    /// `code`.
    pub code: Option<String>,
    /// `display`.
    pub display: Option<String>,
}

impl Code {
    /// The coding `code` of `system`, displayed as `display`.
    #[must_use]
    pub fn of(system: &str, code: &str, display: &str) -> Self {
        Self {
            system: Some(system.to_owned()),
            code: Some(code.to_owned()),
            display: Some(display.to_owned()),
        }
    }

    /// Whether the coding is `code` of `system`.
    #[must_use]
    pub fn is(&self, system: &str, code: &str) -> bool {
        self.system.as_deref() == Some(system) && self.code.as_deref() == Some(code)
    }

    /// The coding as `system|code`, how an element is named in a refusal.
    #[must_use]
    pub fn token(&self) -> String {
        format!(
            "{}|{}",
            self.system.as_deref().unwrap_or_default(),
            self.code.as_deref().unwrap_or_default()
        )
    }
}

/// A `CodeableConcept`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Concept {
    /// `coding`.
    pub codings: Vec<Code>,
    /// `text`.
    pub text: Option<String>,
}

impl Concept {
    /// The concept of the one coding `code`.
    #[must_use]
    pub fn of(code: Code) -> Self {
        Self {
            codings: vec![code],
            text: None,
        }
    }
}

/// An `Identifier`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ident {
    /// `system`.
    pub system: Option<String>,
    /// `value`.
    pub value: Option<String>,
    /// `type.text`.
    pub kind: Option<String>,
    /// `assigner.display`.
    pub assigner: Option<String>,
}

/// A `Reference`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ref {
    /// `reference`, a literal reference.
    pub reference: Option<String>,
    /// `type`.
    pub kind: Option<String>,
    /// `identifier`.
    pub identifier: Option<Ident>,
    /// `display`.
    pub display: Option<String>,
}

impl Ref {
    /// The reference as a line of text, how `details` keeps it.
    #[must_use]
    pub fn render(&self) -> String {
        let identifier = self.identifier.as_ref().map(|i| {
            format!(
                "{}|{}",
                i.system.as_deref().unwrap_or_default(),
                i.value.as_deref().unwrap_or_default()
            )
        });
        line([
            self.kind.clone(),
            self.reference.clone(),
            identifier,
            self.display.clone(),
        ])
    }
}

/// The parts present, joined by a space: one line of `details`.
#[must_use]
pub fn line<const N: usize>(parts: [Option<String>; N]) -> String {
    parts.into_iter().flatten().collect::<Vec<_>>().join(" ")
}

/// A `Period`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Period {
    /// `start`.
    pub start: Option<String>,
    /// `end`.
    pub end: Option<String>,
}

/// The value of an `Extension`, in the forms the FerroEHR extensions take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtValue {
    /// `valueBoolean`.
    Boolean(bool),
    /// `valueString`.
    String(String),
    /// `valueCoding`.
    Coding(Code),
    /// `valuePeriod`.
    Period(Period),
    /// Any other form, which no FerroEHR extension takes.
    Other,
}

/// An `Extension`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ext {
    /// `url`.
    pub url: String,
    /// `value[x]`.
    pub value: ExtValue,
}

/// A `Consent.provision.actor`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Actor {
    /// `role`, required in R4 and optional in R5.
    pub role: Option<Concept>,
    /// `reference`, required in R4 and optional in R5.
    pub reference: Option<Ref>,
    /// `extension`.
    pub extensions: Vec<Ext>,
}

/// A `Consent.provision.data`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DataRef {
    /// `meaning`.
    pub meaning: String,
    /// `reference`.
    pub reference: Ref,
    /// `extension`.
    pub extensions: Vec<Ext>,
}

/// A `Consent.provision`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rule {
    /// `type`, R4 only.
    pub effect: Option<String>,
    /// `period`.
    pub period: Option<Period>,
    /// `actor`.
    pub actors: Vec<Actor>,
    /// `action`.
    pub actions: Vec<Concept>,
    /// `securityLabel`.
    pub labels: Vec<Code>,
    /// `purpose`.
    pub purposes: Vec<Code>,
    /// R4 `class`, R5 `documentType`.
    pub templates: Vec<Code>,
    /// `resourceType`, R5 only.
    pub resource_types: Vec<Code>,
    /// `code`.
    pub codes: Vec<Concept>,
    /// `dataPeriod`.
    pub data_period: Option<Period>,
    /// `data`.
    pub data: Vec<DataRef>,
    /// Whether `expression` is present, R5 only.
    pub expression: bool,
    /// `provision`.
    pub rules: Vec<Rule>,
    /// `extension`.
    pub extensions: Vec<Ext>,
}

/// A `Consent`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Document {
    /// `status`.
    pub status: String,
    /// `scope`, R4 only.
    pub scope: Option<Concept>,
    /// `category`.
    pub categories: Vec<Concept>,
    /// R4 `patient`, R5 `subject`.
    pub subject: Option<Ref>,
    /// R4 `dateTime`, R5 `date`.
    pub stated: Option<String>,
    /// `period`, R5 only.
    pub period: Option<Period>,
    /// R4 `performer`, R5 `grantor`.
    pub instructed_by: Vec<Ref>,
    /// `grantee`, R5 only.
    pub grantees: Vec<Ref>,
    /// R4 `organization`, R5 `controller`.
    pub controllers: Vec<Ref>,
    /// R4 `sourceReference`, R5 `sourceReference`.
    pub evidence: Vec<Ref>,
    /// `identifier`.
    pub identifiers: Vec<Ident>,
    /// `regulatoryBasis`, R5 only.
    pub legal_grounds: Vec<Concept>,
    /// R4 `policyRule`.
    pub policy_rule: Option<Concept>,
    /// `decision`, R5 only.
    pub decision: Option<String>,
    /// R5 `policyBasis.url`.
    pub policy_basis: Option<String>,
    /// R4 `provision`, at most one, or R5 `provision`.
    pub rules: Vec<Rule>,
    /// `extension`.
    pub extensions: Vec<Ext>,
    /// What the document says that no field above and no directive attribute
    /// holds, as `(element, rendering)`, kept in the directive's `details`.
    pub kept: Vec<(String, String)>,
}
