// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Access directives as the IHE APPC Privacy Consent Document, an XACML 2.0
//! policy set (APPC supplement Rev. 1.4, TF-3 5.6, under
//! `docs/specs/ihe-consents/www.ihe.net/`; XACML 2.0 under
//! `docs/specs/xacml/`).
//!
//! The document has one root `PolicySet` whose `PolicySetId` is a URN
//! encoded UUID, which holds a `Description` and matches the patient by
//! `urn:ihe:iti:ser:2016:patient-id` (TF-3 5.6.2.1.1, 5.6.2.1.1.1). The root
//! may reference the policy domain's foundational policies, and "How
//! external references are resolved is out of scope for this profile"
//! (5.6.2.1.1): FerroEHR resolves a reference against
//! `ACCESS_POLICY.catalogue` and refuses one it does not hold, narrowed by
//! the root `Target` as the profile's example (5.6.2.1.1.2) narrows it.
//!
//! Directives combine by deny-overrides, the policy-combining deny-overrides
//! of XACML C.1, so a root combining by `deny-overrides` or
//! `ordered-deny-overrides` maps whole and every other algorithm is refused
//! (XACML C.3 to C.6; APPC fixes none, open issue APPC-7). A `Policy` is one
//! directive: `Effect` sits on `Rule` alone (XACML 5.29, 5.30), so the
//! directive's effect is its base rule's, the one rule with no `Target`,
//! which takes the policy's (5.29). A rule of the opposite effect with a
//! `Target` is an exception, when the `RuleCombiningAlgId` lets it decide
//! where both apply: under deny-overrides (C.1, C.2) a `Permit` base with
//! `Deny` exceptions, under permit-overrides (C.3, C.4) the mirror, under
//! first-applicable (C.5) exceptions before the base. Anything else, and a
//! condition other than the interval over the current date and time
//! FerroEHR writes for `time_validity` (APPC defines no environment
//! attribute, 5.6.2.1.7), is refused with the element named.
//!
//! Export writes one root `PolicySet` under deny-overrides, and one `Policy`
//! per directive: a permit under rule-combining deny-overrides, a deny under
//! permit-overrides, the base and each exception one rule. Each rule's
//! `Description` carries the canonical JSON of the directive's attributes no
//! `Target` states and no scope check reads (TF-3 5.6.2.1.1.1: other
//! elements "may contain additional Description elements"): our own design.

mod target;

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{DirectiveKind, Effect, Term};

use crate::access_exchange::rm::{details, directive, list, members, opposite, plumb, text_of};
use crate::access_exchange::xml::{Node, parse};
use crate::access_exchange::{Format, Loss, Refused, catalogue_entry};
use crate::versioning::audit::dv_text;

/// The policy-combining deny-overrides (XACML B.10, C.1) export writes.
pub const POLICY_DENY_OVERRIDES: &str =
    "urn:oasis:names:tc:xacml:1.0:policy-combining-algorithm:deny-overrides";
/// The policy-combining algorithms a root may name (XACML B.10, C.1, C.2).
const POLICY_DENY_FAMILY: [&str; 2] = [
    POLICY_DENY_OVERRIDES,
    "urn:oasis:names:tc:xacml:1.1:policy-combining-algorithm:ordered-deny-overrides",
];
const RULE_DENY_OVERRIDES: &str =
    "urn:oasis:names:tc:xacml:1.0:rule-combining-algorithm:deny-overrides";
const RULE_PERMIT_OVERRIDES: &str =
    "urn:oasis:names:tc:xacml:1.0:rule-combining-algorithm:permit-overrides";
const RULE_ORDERED_DENY: &str =
    "urn:oasis:names:tc:xacml:1.1:rule-combining-algorithm:ordered-deny-overrides";
const RULE_ORDERED_PERMIT: &str =
    "urn:oasis:names:tc:xacml:1.1:rule-combining-algorithm:ordered-permit-overrides";
const RULE_FIRST_APPLICABLE: &str =
    "urn:oasis:names:tc:xacml:1.0:rule-combining-algorithm:first-applicable";

/// What an import reads beside the document.
#[derive(Debug, Clone, Copy)]
pub struct ImportContext<'a> {
    /// The `ACCESS_POLICY.catalogue` in force.
    pub catalogue: &'a [AccessDirective],
}

/// What an export reads beside the directives.
#[derive(Debug, Clone, Copy)]
pub struct ExportContext<'a> {
    /// The patient's `II` root (TF-3 5.6.2.1.5.1.5).
    pub patient_root: &'a str,
    /// The patient's `II` extension.
    pub patient_extension: Option<&'a str>,
    /// The plain-text `Description` of the root (TF-3 5.6.2.1.1.1).
    pub description: &'a str,
}

fn malformed(element: &str, why: &str) -> Refused {
    Refused::new(Format::Appc, element, Loss::Malformed(why.to_owned()))
}

/// Reads `text`, a Privacy Consent Document, as directives.
///
/// # Errors
/// [`Refused`] naming the first element with no directive form (module
/// docs).
pub fn import(text: &str, context: &ImportContext<'_>) -> Result<Vec<AccessDirective>, Refused> {
    let root = parse(text).map_err(|error| {
        Refused::new(
            Format::Appc,
            "PolicySet",
            Loss::Malformed(error.to_string()),
        )
    })?;
    if root.name != "PolicySet" {
        return Err(Refused::new(
            Format::Appc,
            root.name.clone(),
            Loss::NoDirectiveForm,
        ));
    }
    if !root
        .get("PolicySetId")
        .is_some_and(|id| id.starts_with("urn:uuid:"))
    {
        return Err(malformed(
            "PolicySet/@PolicySetId",
            "the Root Policy Set ID is a URN-encoded UUID (TF-3 5.6.2.1.1)",
        ));
    }
    if root.first("Description").is_none() {
        return Err(malformed(
            "PolicySet/Description",
            "the root holds a Description (TF-3 5.6.2.1.1.1)",
        ));
    }
    let algorithm = root.get("PolicyCombiningAlgId").unwrap_or_default();
    if !POLICY_DENY_FAMILY.contains(&algorithm) {
        return Err(Refused::new(
            Format::Appc,
            "PolicySet/@PolicyCombiningAlgId",
            Loss::Code(algorithm.to_owned()),
        ));
    }
    let scope = target::read(root.first("Target"), "PolicySet/Target")?;
    if !scope.patient {
        return Err(malformed(
            "PolicySet/Target/Resources",
            "the root matches urn:ihe:iti:ser:2016:patient-id (TF-3 5.6.2.1.1)",
        ));
    }
    let mut out = Vec::new();
    for child in &root.children {
        let mut found = match child.name.as_str() {
            "Description" | "Target" => continue,
            "PolicySetIdReference" | "PolicyIdReference" => {
                let id = child.text.trim();
                catalogue_entry(context.catalogue, None, id)
                    .cloned()
                    .ok_or_else(|| {
                        Refused::new(
                            Format::Appc,
                            format!("PolicySet/{}", child.name),
                            Loss::NotInCatalogue(id.to_owned()),
                        )
                    })?
            }
            "Policy" => policy(child)?,
            other => {
                return Err(Refused::new(
                    Format::Appc,
                    format!("PolicySet/{other}"),
                    Loss::NoDirectiveForm,
                ));
            }
        };
        scope.clone().apply(&mut found, "PolicySet/Target")?;
        out.push(found);
    }
    Ok(out)
}

/// The directive one `Policy` states.
fn policy(node: &Node) -> Result<AccessDirective, Refused> {
    if let Some(other) = node
        .children
        .iter()
        .find(|c| !matches!(c.name.as_str(), "Description" | "Target" | "Rule"))
    {
        return Err(Refused::new(
            Format::Appc,
            format!("Policy/{}", other.name),
            Loss::NoDirectiveForm,
        ));
    }
    let rules: Vec<&Node> = node.all("Rule").collect();
    let bases: Vec<usize> = rules
        .iter()
        .enumerate()
        .filter(|(_, r)| r.first("Target").is_none())
        .map(|(at, _)| at)
        .collect();
    let (&[base_at], Some(base)) = (
        bases.as_slice(),
        bases.first().and_then(|at| rules.get(*at)),
    ) else {
        return Err(malformed(
            "Policy/Rule",
            "a policy maps when exactly one rule has no Target, its base (XACML 5.29)",
        ));
    };
    let effect = rule_effect(base)?;
    let algorithm = node.get("RuleCombiningAlgId").unwrap_or_default();
    if rules.len() > 1 {
        let decides = match algorithm {
            RULE_DENY_OVERRIDES | RULE_ORDERED_DENY => effect == Effect::Permit,
            RULE_PERMIT_OVERRIDES | RULE_ORDERED_PERMIT => effect == Effect::Deny,
            RULE_FIRST_APPLICABLE => base_at == rules.len().saturating_sub(1),
            _ => false,
        };
        if !decides {
            return Err(Refused::new(
                Format::Appc,
                "Policy/@RuleCombiningAlgId",
                Loss::Code(algorithm.to_owned()),
            ));
        }
    }
    let own = residual(base);
    let kind = own
        .as_ref()
        .ok()
        .and_then(|own| DirectiveKind::read_coded(&own.kind).ok())
        .unwrap_or(DirectiveKind::Consent);
    let mut out = directive(kind, effect);
    if let Some(description) = node.first("Description") {
        out.name = dv_text(description.text.trim());
    }
    target::read(node.first("Target"), "Policy/Target")?.apply(&mut out, "Policy/Target")?;
    out.time_validity = target::validity(base.first("Condition"), "Policy/Rule/Condition")?;
    let mut exceptions = Vec::new();
    for (at, rule) in rules.iter().enumerate() {
        if at == base_at {
            continue;
        }
        if rule_effect(rule)? != opposite(effect) {
            return Err(Refused::new(
                Format::Appc,
                "Policy/Rule/@Effect",
                Loss::NotException,
            ));
        }
        let own = residual(rule);
        let x_kind = own
            .as_ref()
            .ok()
            .and_then(|own| DirectiveKind::read_coded(&own.kind).ok())
            .unwrap_or(kind);
        let mut x = directive(x_kind, opposite(effect));
        target::read(rule.first("Target"), "Policy/Rule/Target")?
            .apply(&mut x, "Policy/Rule/Target")?;
        x.time_validity = target::validity(rule.first("Condition"), "Policy/Rule/Condition")?;
        stamp(&mut x, own);
        exceptions.push(x);
    }
    out.exceptions = list(exceptions);
    stamp(&mut out, own);
    Ok(out)
}

/// The effect of `rule` (XACML 5.30).
fn rule_effect(rule: &Node) -> Result<Effect, Refused> {
    match rule.get("Effect") {
        Some("Permit") => Ok(Effect::Permit),
        Some("Deny") => Ok(Effect::Deny),
        other => Err(Refused::new(
            Format::Appc,
            "Policy/Rule/@Effect",
            Loss::Code(other.unwrap_or_default().to_owned()),
        )),
    }
}

/// The directive a rule's `Description` carries, or else its plain text.
fn residual(rule: &Node) -> Result<AccessDirective, String> {
    let text = rule
        .first("Description")
        .map(|d| d.text.trim())
        .unwrap_or_default();
    // NOTE: our own design, a Description that is no FerroEHR directive is a person's
    // note, which `details` keeps.
    openehr_its::json::from_canonical_json::<AccessDirective>(text)
        .ok()
        .ok_or_else(|| text.to_owned())
}

/// Sets on `target` what its rule's `Description` carries.
fn stamp(target: &mut AccessDirective, own: Result<AccessDirective, String>) {
    match own {
        Ok(own) => {
            target.emergency_override = own.emergency_override;
            plumb(target, Some(own));
        }
        Err(note) if !note.is_empty() => {
            target.details = details(&[("Policy/Rule/Description".to_owned(), note)]);
        }
        Err(_) => {}
    }
}

/// Writes `directives` as one Privacy Consent Document.
///
/// # Errors
/// [`Refused`] naming the first attribute APPC cannot state: a scope
/// attribute with no APPC attribute, an exception with no scope, or an
/// excluded bound.
pub fn export(
    directives: &[AccessDirective],
    context: &ExportContext<'_>,
) -> Result<String, Refused> {
    let mut root = Node::new("PolicySet")
        .attr("xmlns", "urn:oasis:names:tc:xacml:2.0:policy:schema:os")
        .attr("xmlns:hl7", "urn:hl7-org:v3")
        .attr("PolicySetId", &format!("urn:uuid:{}", uuid::Uuid::now_v7()))
        .attr("PolicyCombiningAlgId", POLICY_DENY_OVERRIDES)
        .child(Node::new("Description").with_text(context.description))
        .child(target::patient(
            context.patient_root,
            context.patient_extension,
        ));
    for d in directives {
        root = root.child(policy_of(d)?);
    }
    Ok(root.document())
}

/// The `Policy` of `d`.
fn policy_of(d: &AccessDirective) -> Result<Node, Refused> {
    let effect = Effect::read_coded(&d.effect).map_err(|error| {
        Refused::new(Format::Directive, "effect", Loss::Code(error.to_string()))
    })?;
    DirectiveKind::read_coded(&d.kind)
        .map_err(|error| Refused::new(Format::Directive, "kind", Loss::Code(error.to_string())))?;
    let algorithm = match effect {
        Effect::Permit => RULE_DENY_OVERRIDES,
        Effect::Deny => RULE_PERMIT_OVERRIDES,
    };
    let mut policy = Node::new("Policy")
        .attr("PolicyId", &format!("urn:uuid:{}", uuid::Uuid::now_v7()))
        .attr("RuleCombiningAlgId", algorithm)
        .child(Node::new("Description").with_text(text_of(&d.name)));
    if let Some(scope) = target::write(d)? {
        policy = policy.child(scope);
    }
    policy = policy.child(rule(d, effect, None)?);
    for x in members(d.exceptions.as_ref()) {
        if x.exceptions.is_some() {
            return Err(Refused::new(
                Format::Directive,
                "exceptions.exceptions",
                Loss::TooDeep,
            ));
        }
        let scope = target::write(x)?.ok_or_else(|| target::not_stated("exceptions"))?;
        policy = policy.child(rule(x, opposite(effect), Some(scope))?);
    }
    Ok(policy)
}

/// One rule of `effect` stating `d`'s validity, with `scope` when it is an
/// exception.
fn rule(d: &AccessDirective, effect: Effect, scope: Option<Node>) -> Result<Node, Refused> {
    let mut own = d.clone();
    own.applies_to = None;
    own.operations = None;
    own.purposes = None;
    own.data = None;
    own.time_validity = None;
    own.exceptions = None;
    let name = match effect {
        Effect::Permit => "Permit",
        Effect::Deny => "Deny",
    };
    let mut node = Node::new("Rule")
        .attr("RuleId", &format!("urn:uuid:{}", uuid::Uuid::now_v7()))
        .attr("Effect", name)
        .child(Node::new("Description").with_text(&openehr_its::json::to_canonical_json(&own)));
    if let Some(scope) = scope {
        node = node.child(scope);
    }
    if let Some(validity) = &d.time_validity {
        node = node.child(target::condition(validity)?);
    }
    Ok(node)
}

#[cfg(test)]
mod tests;
