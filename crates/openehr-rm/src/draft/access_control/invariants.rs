// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: Apache-2.0

//! The class invariants of the draft access control, checked over the
//! generated types of [`crate::draft::access_control`].
//!
//! Each invariant is the BMM's own, named as the BMM names it
//! (`tools/openehr-codegen/draft/bmm/openehr_access_control_0.1.0.bmm.json`).
//! The draft is FerroEHR's change proposal and no openEHR release, so these
//! checks are our own design over our own model (#5506).
//!
//! The `*_valid` invariants that ask an optional list to be absent or
//! non-empty are not checked here: the generated types hold each such list
//! as an `Option<NonEmptyVec<_>>`, so the reader refuses an empty one.
//!
//! The terminology invariants name `Terminology_id_access_control`, which the
//! draft leaves to FerroEHR's operation terminology until openEHR adopts the
//! groups. The draft defines each group by its rubrics and assigns no codes
//! yet, so a coded value holds when its terminology is
//! [`TERMINOLOGY_ID_ACCESS_CONTROL`] and its code is one of the group's rubrics.

use crate::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use crate::draft::access_control::access_control::access_classification::AccessClassification;
use crate::draft::access_control::access_control::access_data_scope::AccessDataScope;
use crate::draft::access_control::access_control::access_directive::AccessDirective;
use crate::draft::access_control::access_control::access_directive_settings::AccessDirectiveSettings;
use crate::draft::access_control::access_control::access_policy::AccessPolicy;
use crate::draft::invariants::{Check, Violations};
use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;

/// The BMM's `Terminology_id_access_control`: FerroEHR's operation terminology,
/// which holds the draft groups until openEHR adopts them.
pub const TERMINOLOGY_ID_ACCESS_CONTROL: &str = "ferroehr-operations";

/// The rubrics of the draft `access default` group.
const ACCESS_DEFAULT: &[&str] = &["open", "restricted", "consent_required"];
/// The rubrics of the draft `access directive kind` group.
const ACCESS_DIRECTIVE_KIND: &[&str] = &["consent", "restriction", "opt_out", "delegation"];
/// The rubrics of the draft `access effect` group.
const ACCESS_EFFECT: &[&str] = &["permit", "deny"];
/// The rubrics of the draft `access relationship` group.
const ACCESS_RELATIONSHIP: &[&str] = &["care", "representative", "proxy"];
/// The rubrics of the draft `access caller kind` group.
const ACCESS_CALLER_KIND: &[&str] = &[
    "professional",
    "access_service",
    "subject",
    "representative",
    "system",
];

/// Whether `coded` is a term of the draft group whose rubrics are `group`.
fn in_group(coded: &DvCodedText, group: &[&str]) -> bool {
    coded.defining_code.terminology_id.value == TERMINOLOGY_ID_ACCESS_CONTROL
        && group.contains(&coded.defining_code.code_string.as_str())
}

/// Whether `coded` is the term `rubric` of the draft terminology.
fn is_code(coded: &DvCodedText, rubric: &str) -> bool {
    in_group(coded, &[rubric])
}

/// `ACCESS_DIRECTIVE.is_deny`: the effect is the `deny` code.
fn is_deny(directive: &AccessDirective) -> bool {
    is_code(&directive.effect, "deny")
}

/// `ACCESS_DIRECTIVE.is_restriction`: the kind is the `restriction` code.
fn is_restriction(directive: &AccessDirective) -> bool {
    is_code(&directive.kind, "restriction")
}

/// `ACCESS_DIRECTIVE.is_opt_out`: the kind is the `opt_out` code.
fn is_opt_out(directive: &AccessDirective) -> bool {
    is_code(&directive.kind, "opt_out")
}

/// Checks every invariant of an `ACCESS_DIRECTIVE_SETTINGS` and of the
/// directives it holds.
///
/// # Errors
/// [`Violations`] naming each invariant the settings break.
pub fn check_settings(settings: &AccessDirectiveSettings) -> Result<(), Violations> {
    let mut check = Check::default();
    check.hold(
        in_group(&settings.default_access, ACCESS_DEFAULT),
        "ACCESS_DIRECTIVE_SETTINGS",
        "Default_access_valid",
    );
    let directives = settings.directives.as_deref().unwrap_or_default();
    check.hold(
        directives.iter().filter(|d| is_opt_out(d)).count() <= 1,
        "ACCESS_DIRECTIVE_SETTINGS",
        "Opt_out_unique",
    );
    for directive in directives {
        directive_rules(&mut check, directive);
    }
    check.finish()
}

/// Checks every invariant of an `ACCESS_POLICY` and of the classifications
/// and catalogue entries it holds.
///
/// # Errors
/// [`Violations`] naming each invariant the policy breaks.
pub fn check_policy(policy: &AccessPolicy) -> Result<(), Violations> {
    let mut check = Check::default();
    // NOTE: RM Common IM, LOCATABLE `Archetyped_valid`: a node is an archetype
    // root exactly when it carries `archetype_details`.
    check.hold(
        policy.archetype_details.is_some(),
        "ACCESS_POLICY",
        "Is_archetype_root",
    );
    check.hold(
        policy
            .relationship_grace
            .as_ref()
            .is_none_or(|grace| grace.magnitude().is_some_and(|seconds| seconds >= 0.0)),
        "ACCESS_POLICY",
        "Grace_valid",
    );
    let catalogue = policy.catalogue.as_deref().unwrap_or_default();
    check.hold(
        catalogue.iter().all(|d| d.identifiers.is_some()),
        "ACCESS_POLICY",
        "Catalogue_identified",
    );
    for classification in policy.classifications.as_deref().unwrap_or_default() {
        classification_rules(&mut check, classification);
    }
    for directive in catalogue {
        directive_rules(&mut check, directive);
    }
    check.finish()
}

fn directive_rules(check: &mut Check, directive: &AccessDirective) {
    check.hold(
        in_group(&directive.kind, ACCESS_DIRECTIVE_KIND),
        "ACCESS_DIRECTIVE",
        "Kind_valid",
    );
    check.hold(
        in_group(&directive.effect, ACCESS_EFFECT),
        "ACCESS_DIRECTIVE",
        "Effect_valid",
    );
    let deny = is_deny(directive);
    check.hold(
        !is_restriction(directive) || deny,
        "ACCESS_DIRECTIVE",
        "Restriction_denies",
    );
    check.hold(
        !is_opt_out(directive) || (deny && directive.data.is_none()),
        "ACCESS_DIRECTIVE",
        "Opt_out_denies_whole_record",
    );
    let exceptions = directive.exceptions.as_deref().unwrap_or_default();
    check.hold(
        exceptions
            .iter()
            .all(|e| is_deny(e) != deny && e.exceptions.is_none()),
        "ACCESS_DIRECTIVE",
        "Exceptions_opposite",
    );
    for scope in directive.applies_to.as_deref().unwrap_or_default() {
        actor_scope_rules(check, scope);
    }
    for scope in directive.data.as_deref().unwrap_or_default() {
        data_scope_rules(check, scope);
    }
    for exception in exceptions {
        directive_rules(check, exception);
    }
}

fn actor_scope_rules(check: &mut Check, scope: &AccessActorScope) {
    check.hold(
        scope.party.is_some()
            || scope.role.is_some()
            || scope.relationship.is_some()
            || scope.caller_kind.is_some(),
        "ACCESS_ACTOR_SCOPE",
        "Not_empty",
    );
    check.hold(
        scope
            .relationship
            .as_ref()
            .is_none_or(|r| in_group(r, ACCESS_RELATIONSHIP)),
        "ACCESS_ACTOR_SCOPE",
        "Relationship_valid",
    );
    check.hold(
        scope
            .caller_kind
            .as_ref()
            .is_none_or(|k| in_group(k, ACCESS_CALLER_KIND)),
        "ACCESS_ACTOR_SCOPE",
        "Caller_kind_valid",
    );
}

fn data_scope_rules(check: &mut Check, scope: &AccessDataScope) {
    check.hold(
        scope.objects.is_some()
            || scope.templates.is_some()
            || scope.archetypes.is_some()
            || scope.categories.is_some()
            || scope.sensitivities.is_some()
            || scope.origins.is_some()
            || scope.recorded.is_some(),
        "ACCESS_DATA_SCOPE",
        "Not_empty",
    );
}

fn classification_rules(check: &mut Check, classification: &AccessClassification) {
    check.hold(
        classification.data.objects.is_none(),
        "ACCESS_CLASSIFICATION",
        "No_objects",
    );
    data_scope_rules(check, &classification.data);
}
