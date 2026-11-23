// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Errors for the Simplified Formats layer, one variant per MUST-level
//! rejection of ITS-REST `simplified_formats/`.

/// An error in Simplified-Format (FLAT / STRUCTURED / Web Template) handling.
#[derive(Debug, thiserror::Error)]
pub enum FlatError {
    /// The operational template lacked something the Web Template builder
    /// requires.
    #[error("invalid operational template: {0}")]
    InvalidTemplate(String),

    /// A `C_STRING` pattern that compiles in no regex engine (ADL 1.4
    /// `master05-cadl.adoc` §Regular Expression).
    #[error("the string pattern {pattern:?} at {path} compiles in no regex engine")]
    UnevaluablePattern {
        /// The archetype path of the constrained node.
        path: String,
        /// The pattern as the template states it.
        pattern: String,
    },

    /// An `ARCHETYPE_SLOT` of the template cannot be evaluated (AOM 1.4
    /// `ARCHETYPE_SLOT`), so no filler could be judged against it.
    #[error("the archetype slot under {path} cannot be evaluated")]
    UnevaluableSlot {
        /// The archetype path of the attribute the slot stands under.
        path: String,
        /// What keeps the slot from being evaluated.
        #[source]
        defect: crate::flat::webtemplate::constraint::SlotDefect,
    },

    /// An OPT 1.4 XML document failed to parse.
    #[error("failed to parse OPT 1.4 XML")]
    OptParse(#[from] openehr_its::xml::runtime::XmlError),

    /// A value could not be serialized to JSON.
    #[error("failed to serialize to JSON")]
    Serialize(#[from] serde_json::Error),

    /// A FLAT key is not syntactically valid (ITS-REST simplified_formats
    /// master04 §Flat format syntax rules).
    #[error("malformed simplified path {path:?}: {reason}")]
    MalformedPath {
        /// The offending key as received.
        path: String,
        /// What rule the key breaks.
        reason: String,
    },

    /// A path that resolves to no Web Template node (master04 §Validation).
    #[error("unknown simplified path: {0}")]
    UnknownPath(String),

    /// A value on a node that takes none, such as a structural node (master04
    /// §Validation).
    #[error(
        "{path} is a {rm_type}, which takes no value of its own{}",
        value_hint(alternatives)
    )]
    ValueWithoutInput {
        /// The simplified path the value was given on.
        path: String,
        /// The RM type of the node the path resolved to.
        rm_type: String,
        /// For an ELEMENT with value alternatives, the ids of the alternatives
        /// the value belongs on; empty for a structural node.
        alternatives: Vec<String>,
    },

    /// An attribute suffix is not defined for the RM type at its path
    /// (master05 per-type tables).
    #[error("unknown attribute suffix |{suffix} for {rm_type} at {path}")]
    UnknownSuffix {
        /// The RM type of the node the path resolved to.
        rm_type: String,
        /// The offending suffix (without the pipe).
        suffix: String,
        /// The path the suffix was attached to.
        path: String,
    },

    /// `|other` beside `|code`/`|value`/`|terminology` (master04 §Open
    /// Value-Sets and the `|other` Suffix).
    #[error("|other is mutually exclusive with |code/|value/|terminology at {0}")]
    OtherSuffixConflict(String),

    /// `|other` on a closed value set (master04 §Open Value-Sets and the `|other`
    /// Suffix).
    #[error("|other not allowed at {0}: the value set is closed")]
    OtherOnClosedValueSet(String),

    /// A mandatory context field is absent (master04 §Context: "Mandatory:
    /// language, territory").
    #[error("mandatory ctx field {0} is required (master04 §Context: language and territory)")]
    MissingContext(&'static str),

    /// A `|code` outside a closed value set with no `|value` (master04
    /// §Validation).
    #[error("coded value is not a member of the bound value set: '{code}' at {path}")]
    CodeNotInValueSet {
        /// The simplified path of the coded leaf.
        path: String,
        /// The code the document supplied.
        code: String,
    },

    /// A `ctx/` key outside the vocabulary of master06-context_information.
    #[error("unknown context key ctx/{0}")]
    UnknownContext(String),

    /// An absent key the RM types 1..1: a master05 `Required: yes` suffix (§LINK)
    /// or a `ctx/` key for a mandatory attribute (`ctx/participation_function:<i>`).
    #[error("{key} is required")]
    MissingRequiredSuffix {
        /// The full FLAT key the client omitted, e.g. `…/_link:0|meaning` or
        /// `ctx/participation_function:0`.
        key: String,
    },

    /// A value of an RM type master05 gives no mapping table, such as `DV_STATE`;
    /// only `|raw` carries one.
    #[error(
        "{path} is a {rm_type}, for which the simplified formats define no form \
         (simplified_formats master05-rm_mapping has no {rm_type} table); \
         send it as canonical JSON with |raw (master04 §Raw canonical JSON)"
    )]
    NoSimplifiedForm {
        /// The RM type of the node the path resolved to.
        rm_type: String,
        /// The simplified path of the node.
        path: String,
    },

    /// A value of the wrong JSON type or encoding for its slot.
    #[error("invalid value at {path}: {reason}")]
    InvalidValue {
        /// The simplified path of the offending value.
        path: String,
        /// What is wrong with it.
        reason: String,
    },

    /// A `|raw` value without `_type` (master04 §Raw canonical JSON).
    #[error("invalid |raw payload at {path}: the value must be a canonical-JSON object with _type")]
    RawWithoutType {
        /// The simplified path of the `|raw` key.
        path: String,
    },

    /// A `|raw` payload is an RM object the attribute does not admit, or
    /// stands beside other content at its node.
    #[error("invalid |raw payload at {path}: {reason}")]
    InvalidRaw {
        /// The simplified path of the `|raw` key.
        path: String,
        /// What is wrong with the payload.
        reason: String,
    },

    /// The built RM refuses an attribute or a value, or a mandatory attribute
    /// is left unset (#6003): the template-mediated 422 class.
    #[error("the simplified document does not build a valid RM instance")]
    Rm(#[from] openehr_rm::v1_2::access::AccessError),

    /// A `|raw` canonical-JSON fragment the strict canonical reader refuses
    /// (master04 §Raw canonical JSON).
    #[error("invalid |raw payload at {path}")]
    RawDecode {
        /// The simplified path of the fragment.
        path: String,
        /// The reader's refusal.
        #[source]
        source: openehr_its::json::JsonParseError,
    },

    /// The input or produced structure is not the shape the converter expects.
    #[error("simplified-format conversion error: {0}")]
    Conversion(String),

    /// An example COMPOSITION asked of a template not rooted at COMPOSITION.
    #[error(
        "the template root {root} is not a COMPOSITION, so no example COMPOSITION exists for it"
    )]
    NoExampleForRoot {
        /// The template root's RM type.
        root: String,
    },

    /// An example party asked of a template constraining no `identities` member,
    /// which RM `Identities_valid` requires.
    #[error(
        "the {class} template constrains no PARTY_IDENTITY, so no example {class} carries the \
         identity the RM requires"
    )]
    NoExampleIdentity {
        /// The template root's party class.
        class: String,
    },

    /// A party build was asked of a template whose root is not a party class
    /// (`PERSON`, `ORGANISATION`, `GROUP`, `AGENT` or `ROLE`).
    #[error("the template root {root} is not a PARTY class, so no party builds from it")]
    NotAPartyTemplate {
        /// The template root's RM type.
        root: String,
    },

    /// A `ctx/` key on a non-COMPOSITION document (master06).
    #[error(
        "only a COMPOSITION document takes ctx/ keys: ctx/ is the COMPOSITION context vocabulary"
    )]
    ContextOnParty,

    /// A built party that is not an instance of its root class.
    #[error("the built party does not decode as {class}")]
    PartyDecode {
        /// The party class the build targeted.
        class: String,
        /// The canonical-JSON reader's refusal.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// The hint a [`FlatError::ValueWithoutInput`] gives: where the value goes.
fn value_hint(alternatives: &[String]) -> String {
    if alternatives.is_empty() {
        String::new()
    } else {
        format!(
            "; give it on one of its value alternatives: {}",
            alternatives.join(", ")
        )
    }
}
