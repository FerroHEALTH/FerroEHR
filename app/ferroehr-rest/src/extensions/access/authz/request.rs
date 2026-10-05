// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The authorization request types the ABAC PDP seam consumes.
//!
//! Covers the resource kind, the access mode (the Cedar action axis), the
//! resolved attributes, and the multi-valued fan-out semantics both
//! engines share.
//!
//! Attributes (`organization`, `patient`, `template`) are resolved by the PEP
//! before the engine is called; `patient`/`template` may be *sets* (query,
//! contribution), which the engine fans out over as a full cartesian product,
//! **all-must-permit**: a request touching several resources is permitted only
//! if every one of them is, and the first deny short-circuits.

/// The resource family a clinical operation acts on. Derived from the
/// operation-id prefix by [`crate::extensions::access::authz::classify::kind_of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    /// An EHR (create / get / get-by-subject).
    Ehr,
    /// An `EHR_STATUS` or a versioned-EHR_STATUS read.
    EhrStatus,
    /// A COMPOSITION or a versioned-composition read.
    Composition,
    /// A CONTRIBUTION.
    Contribution,
    /// An AQL query execution (ad-hoc or stored).
    Query,
    /// A DIRECTORY/FOLDER. Gated only when `abac.check_directory` is set, so a
    /// deployment opts into it explicitly.
    Directory,
}

impl ResourceKind {
    /// The canonical key this kind uses in the `abac.policy` config map and in
    /// the Cedar resource/action names.
    #[must_use]
    pub const fn config_key(self) -> &'static str {
        match self {
            ResourceKind::Ehr => "ehr",
            ResourceKind::EhrStatus => "ehr_status",
            ResourceKind::Composition => "composition",
            ResourceKind::Contribution => "contribution",
            ResourceKind::Query => "query",
            ResourceKind::Directory => "directory",
        }
    }
}

/// The access mode of an operation (the Cedar *action* axis): op ids map
/// onto `ResourceKind × AccessMode` rather than 96 raw op ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessMode {
    /// A create/commit.
    Create,
    /// A read/get.
    Read,
    /// An update/modify.
    Update,
    /// A delete.
    Delete,
    /// A query execution.
    Execute,
}

impl AccessMode {
    /// The lower-case mode name used to compose the Cedar action id
    /// (`composition.create`, `query.execute`, …).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AccessMode::Create => "create",
            AccessMode::Read => "read",
            AccessMode::Update => "update",
            AccessMode::Delete => "delete",
            AccessMode::Execute => "execute",
        }
    }
}

/// A resolved attribute that may be single-valued (non-query) or multi-valued
/// (query result / contribution payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attr {
    /// Exactly one value.
    One(String),
    /// A set of values the engine fans out over.
    ///
    /// An empty set yields no combinations, so nothing is asked and nothing
    /// denies. Both builders map an empty attribute to `None` rather than to
    /// this variant, so a vacuous permit is unreachable by construction — do not
    /// introduce a path that constructs `Set(vec![])`.
    Set(Vec<String>),
}

/// The fully-resolved authorization request handed to a [`PolicyEngine`].
///
/// [`PolicyEngine`]: crate::extensions::access::authz::engine::PolicyEngine
#[derive(Debug, Clone)]
pub struct AuthzRequest<'a> {
    /// The generated operation id (retained for logging/diagnostics).
    pub operation_id: &'a str,
    /// The resource family.
    pub kind: ResourceKind,
    /// The access mode (the action axis).
    pub access: AccessMode,
    /// The authenticated caller's subject identifier (`sub` / Basic username).
    ///
    /// NIST SP 800-162 §2.2 makes subject attributes one half of an ABAC
    /// decision, so a policy must be able to name WHO is asking, not only which
    /// organization and patient are in play. It also gives the decision an
    /// identity to log.
    pub subject: &'a str,
    /// The caller's roles, upper-cased as the RBAC gate sees them.
    ///
    /// A policy engine that cannot see roles can only express attribute rules,
    /// which makes the coarse RBAC tier and the fine ABAC tier unable to reason
    /// about the same caller.
    pub roles: &'a [String],
    /// The caller's OAuth2 scopes, verbatim.
    pub scopes: &'a [String],
    /// The caller's organization (resolved `abac.organization_claim`), if any.
    pub organization: Option<String>,
    /// The patient attribute (single for non-query, a set for query).
    pub patient: Option<Attr>,
    /// The template attribute (single for composition, a set for contribution/query).
    pub template: Option<Attr>,
}

/// One point of the cartesian fan-out: a single (organization, patient,
/// template) tuple an engine evaluates in isolation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Combination<'a> {
    /// The authenticated caller's subject identifier.
    pub subject: &'a str,
    /// The caller's roles.
    pub roles: &'a [String],
    /// The caller's OAuth2 scopes.
    pub scopes: &'a [String],
    /// The caller's organization for this evaluation.
    pub organization: Option<&'a str>,
    /// The single candidate patient for this evaluation.
    pub patient: Option<&'a str>,
    /// The single candidate template for this evaluation.
    pub template: Option<&'a str>,
}

impl AuthzRequest<'_> {
    /// The cartesian product of the patient × template candidates. A
    /// single-valued or absent attribute contributes one candidate (`None` when
    /// absent); an **empty** set contributes none, so the product is empty and
    /// the all-must-permit fold is a vacuous permit.
    #[must_use]
    pub fn combinations(&self) -> Vec<Combination<'_>> {
        let patients = candidates(self.patient.as_ref());
        let templates = candidates(self.template.as_ref());
        let org = self.organization.as_deref();
        let mut out = Vec::with_capacity(patients.len() * templates.len());
        for &patient in &patients {
            for &template in &templates {
                out.push(Combination {
                    subject: self.subject,
                    roles: self.roles,
                    scopes: self.scopes,
                    organization: org,
                    patient,
                    template,
                });
            }
        }
        out
    }
}

/// The candidate list for one attribute: `[None]` when absent, one `Some` when
/// single, the set (possibly empty) when multi-valued.
fn candidates(attr: Option<&Attr>) -> Vec<Option<&str>> {
    match attr {
        None => vec![None],
        Some(Attr::One(s)) => vec![Some(s.as_str())],
        Some(Attr::Set(v)) => v.iter().map(|s| Some(s.as_str())).collect(),
    }
}

/// The engine's verdict for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// The caller may proceed.
    Permit,
    /// The caller is forbidden (→ 403 at the PEP).
    Deny,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req<'a>(patient: Option<Attr>, template: Option<Attr>) -> AuthzRequest<'a> {
        AuthzRequest {
            operation_id: "composition_create",
            kind: ResourceKind::Composition,
            access: AccessMode::Create,
            subject: "test-subject",
            roles: &[],
            scopes: &[],
            organization: Some("org1".to_owned()),
            patient,
            template,
        }
    }

    #[test]
    fn single_values_yield_one_combination() {
        let r = req(
            Some(Attr::One("p1".to_owned())),
            Some(Attr::One("t1".to_owned())),
        );
        let combos = r.combinations();
        assert_eq!(combos.len(), 1);
        assert_eq!(combos[0].organization, Some("org1"));
        assert_eq!(combos[0].patient, Some("p1"));
        assert_eq!(combos[0].template, Some("t1"));
    }

    #[test]
    fn absent_attribute_is_a_single_none_candidate() {
        let r = req(None, None);
        let combos = r.combinations();
        assert_eq!(combos.len(), 1);
        assert_eq!(combos[0].patient, None);
        assert_eq!(combos[0].template, None);
    }

    #[test]
    fn cartesian_product_of_sets() {
        let r = req(
            Some(Attr::Set(vec!["p1".to_owned(), "p2".to_owned()])),
            Some(Attr::Set(vec!["t1".to_owned(), "t2".to_owned()])),
        );
        let combos = r.combinations();
        assert_eq!(combos.len(), 4);
        // Row-major order: (p1,t1),(p1,t2),(p2,t1),(p2,t2).
        assert_eq!(combos[0].patient, Some("p1"));
        assert_eq!(combos[0].template, Some("t1"));
        assert_eq!(combos[3].patient, Some("p2"));
        assert_eq!(combos[3].template, Some("t2"));
    }

    #[test]
    fn empty_set_yields_no_combinations() {
        // An empty result set → vacuous permit at the engine.
        let r = req(Some(Attr::Set(vec![])), Some(Attr::One("t1".to_owned())));
        assert!(r.combinations().is_empty());
    }
}
