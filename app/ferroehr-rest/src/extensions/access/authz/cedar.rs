// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`CedarEngine`] — the embedded, default ABAC engine, built on `cedar-policy` 4.x.
//!
//! A typed Cedar **schema** (shipped, built from the [`ResourceKind`] ×
//! [`AccessMode`] enums so it can never drift from the op classification) is
//! validated at load; the operator's `*.cedar` **policies** are parsed and
//! schema-validated at boot — an invalid policy set refuses to start
//! (fail-closed at the earliest moment). Optional periodic reload swaps the
//! policy set on validity ([`arc_swap`]). Multi-valued attributes fan out per
//! [`AuthzRequest::combinations`] with the same all-must-permit / short-circuit
//! semantics as the remote PDP, so the two engines are behaviourally identical
//! (a differential test pins the two against the same request corpus).
//!
//! Cedar is deny-by-default with `forbid` overriding `permit`; the shipped
//! example policies document that.
//!
//! ## Attribute model
//! - **principal** `User { organization?, patient?, roles, scopes }` — the
//!   caller, carrying the subject's roles and scopes so a policy can reason
//!   about them. The shipped example policies deliberately use only the
//!   attributes the flat PDP body also carries, which is what keeps the two
//!   engines interchangeable on the same request corpus.
//! - **resource** (one entity type per [`ResourceKind`]) carries the
//!   per-combination candidate `patient?` and `template?`.
//! - **action** `"<kind>.<mode>"` (e.g. `composition.create`, `query.execute`).

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use async_trait::async_trait;
use cedar_policy::{
    Authorizer, Context, Decision as CedarDecision, Entities, Entity, EntityUid, PolicySet,
    Request, RestrictedExpression, Schema, ValidationMode, Validator,
};

use crate::extensions::access::authz::engine::{AuthzError, PolicyEngine};
use crate::extensions::access::authz::request::{
    AccessMode, AuthzRequest, Combination, Decision, ResourceKind,
};

/// Every resource kind, for schema construction.
const KINDS: [ResourceKind; 6] = [
    ResourceKind::Ehr,
    ResourceKind::EhrStatus,
    ResourceKind::Composition,
    ResourceKind::Contribution,
    ResourceKind::Query,
    ResourceKind::Directory,
];

/// Every access mode, for schema construction.
const MODES: [AccessMode; 5] = [
    AccessMode::Create,
    AccessMode::Read,
    AccessMode::Update,
    AccessMode::Delete,
    AccessMode::Execute,
];

/// The Cedar entity-type name for a resource kind (`PascalCase`).
const fn entity_type(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Ehr => "Ehr",
        ResourceKind::EhrStatus => "EhrStatus",
        ResourceKind::Composition => "Composition",
        ResourceKind::Contribution => "Contribution",
        ResourceKind::Query => "Query",
        ResourceKind::Directory => "Directory",
    }
}

/// The Cedar action id for a (kind, mode) pair, e.g. `composition.create`.
fn action_id(kind: ResourceKind, mode: AccessMode) -> String {
    format!("{}.{}", kind.config_key(), mode.as_str())
}

/// Build the shipped Cedar schema source from the kind × mode enums, so the
/// declared entity types and actions always match the runtime mapping.
fn schema_src() -> String {
    let mut s = String::new();
    s.push_str(
        "entity User {\n  organization?: String,\n  patient?: String,\n  \
         roles: Set<String>,\n  scopes: Set<String>,\n};\n",
    );
    for kind in KINDS {
        let _ = writeln!(
            s,
            "entity {} {{\n  patient?: String,\n  template?: String,\n}};",
            entity_type(kind)
        );
    }
    for kind in KINDS {
        for mode in MODES {
            let _ = writeln!(
                s,
                "action \"{}\" appliesTo {{ principal: [User], resource: [{}], \
             context: {{ operation_id: String }} }};",
                action_id(kind, mode),
                entity_type(kind)
            );
        }
    }
    s
}

/// The embedded Cedar engine: a validated schema + a hot-swappable, validated
/// policy set.
#[derive(Debug)]
pub struct CedarEngine {
    schema: Arc<Schema>,
    policies: Arc<ArcSwap<PolicySet>>,
}

impl CedarEngine {
    /// Build the engine from a policy directory. The schema is validated, then
    /// every `*.cedar` file in `policy_dir` is parsed and validated against it;
    /// any failure is an [`AuthzError::PolicyLoad`] (refuse to start). When
    /// `reload_secs` is `Some`, a background task periodically reloads and swaps
    /// the policy set on validity (invalid reloads are logged and ignored).
    ///
    /// # Errors
    /// [`AuthzError::PolicyLoad`] if the shipped schema, or the operator policy
    /// set, fails to parse or validate.
    pub fn new(policy_dir: &Path, reload_secs: Option<u64>) -> Result<Self, AuthzError> {
        let schema = build_schema()?;
        let schema = Arc::new(schema);
        let policies = load_and_validate(policy_dir, &schema)?;
        let policies = Arc::new(ArcSwap::from_pointee(policies));
        let engine = Self {
            schema: Arc::clone(&schema),
            policies: Arc::clone(&policies),
        };
        if let Some(secs) = reload_secs.filter(|s| *s > 0) {
            spawn_reload(policy_dir.to_owned(), schema, policies, secs);
        }
        Ok(engine)
    }

    /// Build directly from an in-memory policy-set source (tests).
    ///
    /// # Errors
    /// [`AuthzError::PolicyLoad`] if the schema or the policy source is invalid.
    pub fn from_policy_src(policies_src: &str) -> Result<Self, AuthzError> {
        let schema = Arc::new(build_schema()?);
        let policies = parse_and_validate(policies_src, &schema)?;
        Ok(Self {
            schema,
            policies: Arc::new(ArcSwap::from_pointee(policies)),
        })
    }

    /// Evaluate one combination; `Ok(true)` = permit.
    fn permits(&self, req: &AuthzRequest<'_>, combo: &Combination<'_>) -> Result<bool, AuthzError> {
        let principal = build_principal(combo)?;
        let resource = build_resource(req.kind, combo)?;
        let action: EntityUid = format!("Action::\"{}\"", action_id(req.kind, req.access))
            .parse()
            .map_err(|e| AuthzError::Evaluation(format!("action uid: {e}")))?;
        let principal_uid = principal.uid();
        let resource_uid = resource.uid();
        let entities = Entities::from_entities([principal, resource], Some(&self.schema))
            .map_err(|e| AuthzError::Evaluation(format!("entities: {e}")))?;
        // The operation id travels as request context so a policy can key on the
        // specific operation, not only on the resource family and access mode.
        let context = Context::from_pairs([(
            "operation_id".to_owned(),
            RestrictedExpression::new_string(req.operation_id.to_owned()),
        )])
        .map_err(|e| AuthzError::Evaluation(format!("request context: {e}")))?;
        let request = Request::new(
            principal_uid,
            action,
            resource_uid,
            context,
            Some(&self.schema),
        )
        .map_err(|e| AuthzError::Evaluation(format!("request: {e}")))?;
        let policies = self.policies.load();
        let response = Authorizer::new().is_authorized(&request, &policies, &entities);
        // Cedar SKIPS a policy that errors during evaluation and reports it in
        // the diagnostics rather than failing the request
        // (<https://docs.cedarpolicy.com/auth/authorization.html>). Discarding
        // them means an erroring `forbid` silently stops forbidding — the
        // decision then reflects only the policies that happened to evaluate.
        // A policy set that cannot be evaluated is not a decision, so it is a
        // fail-closed 500 at the PEP.
        let errors: Vec<String> = response
            .diagnostics()
            .errors()
            .map(ToString::to_string)
            .collect();
        if !errors.is_empty() {
            return Err(AuthzError::Evaluation(format!(
                "policy evaluation errors ({}): {}",
                errors.len(),
                errors.join("; ")
            )));
        }
        Ok(response.decision() == CedarDecision::Allow)
    }
}

#[async_trait]
impl PolicyEngine for CedarEngine {
    async fn decide(&self, req: &AuthzRequest<'_>) -> Result<Decision, AuthzError> {
        for combo in req.combinations() {
            if self.permits(req, &combo)? {
                ferroehr::telemetry::metrics::metrics()
                    .authz_cedar_decisions
                    .add(1, &[opentelemetry::KeyValue::new("result", "permit")]);
            } else {
                ferroehr::telemetry::metrics::metrics()
                    .authz_cedar_decisions
                    .add(1, &[opentelemetry::KeyValue::new("result", "deny")]);
                return Ok(Decision::Deny);
            }
        }
        Ok(Decision::Permit)
    }
}

/// Parse + validate the shipped schema.
fn build_schema() -> Result<Schema, AuthzError> {
    let (schema, _warnings) = Schema::from_cedarschema_str(&schema_src())
        .map_err(|e| AuthzError::PolicyLoad(format!("cedar schema: {e}")))?;
    Ok(schema)
}

/// The principal entity for a combination.
///
/// The uid names the authenticated subject rather than a constant, so a policy
/// can be written about a specific caller and a decision log identifies who was
/// asking (NIST SP 800-162 §2.2: subject attributes are half of an ABAC
/// decision).
fn build_principal(combo: &Combination<'_>) -> Result<Entity, AuthzError> {
    // Cedar entity ids are quoted strings; escape the two characters that would
    // otherwise terminate or continue the literal, so a subject carrying them
    // cannot forge a different uid.
    let escaped = combo.subject.replace('\\', "\\\\").replace('"', "\\\"");
    let uid: EntityUid = format!("User::\"{escaped}\"")
        .parse()
        .map_err(|e| AuthzError::Evaluation(format!("principal uid: {e}")))?;
    let mut attrs: HashMap<String, RestrictedExpression> = HashMap::new();
    if let Some(org) = combo.organization {
        attrs.insert(
            "organization".to_owned(),
            RestrictedExpression::new_string(org.to_owned()),
        );
    }
    if let Some(patient) = combo.patient {
        attrs.insert(
            "patient".to_owned(),
            RestrictedExpression::new_string(patient.to_owned()),
        );
    }
    attrs.insert(
        "roles".to_owned(),
        RestrictedExpression::new_set(
            combo
                .roles
                .iter()
                .map(|r| RestrictedExpression::new_string(r.clone())),
        ),
    );
    attrs.insert(
        "scopes".to_owned(),
        RestrictedExpression::new_set(
            combo
                .scopes
                .iter()
                .map(|s| RestrictedExpression::new_string(s.clone())),
        ),
    );
    Entity::new(uid, attrs, HashSet::new())
        .map_err(|e| AuthzError::Evaluation(format!("principal entity: {e}")))
}

/// The resource entity for a combination.
fn build_resource(kind: ResourceKind, combo: &Combination<'_>) -> Result<Entity, AuthzError> {
    let uid: EntityUid = format!("{}::\"resource\"", entity_type(kind))
        .parse()
        .map_err(|e| AuthzError::Evaluation(format!("resource uid: {e}")))?;
    let mut attrs: HashMap<String, RestrictedExpression> = HashMap::new();
    if let Some(patient) = combo.patient {
        attrs.insert(
            "patient".to_owned(),
            RestrictedExpression::new_string(patient.to_owned()),
        );
    }
    if let Some(template) = combo.template {
        attrs.insert(
            "template".to_owned(),
            RestrictedExpression::new_string(template.to_owned()),
        );
    }
    Entity::new(uid, attrs, HashSet::new())
        .map_err(|e| AuthzError::Evaluation(format!("resource entity: {e}")))
}

/// Read + concatenate every `*.cedar` file in `dir`, then parse and validate.
fn load_and_validate(dir: &Path, schema: &Schema) -> Result<PolicySet, AuthzError> {
    let mut sources = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| {
        AuthzError::PolicyLoad(format!("reading policy dir {}: {e}", dir.display()))
    })?;
    for entry in entries {
        let path = entry
            .map_err(|e| AuthzError::PolicyLoad(format!("policy dir entry: {e}")))?
            .path();
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("cedar") {
            let text = std::fs::read_to_string(&path).map_err(|e| {
                AuthzError::PolicyLoad(format!("reading policy {}: {e}", path.display()))
            })?;
            sources.push(text);
        }
    }
    if sources.is_empty() {
        return Err(AuthzError::PolicyLoad(format!(
            "no *.cedar policy files found in {}",
            dir.display()
        )));
    }
    parse_and_validate(&sources.join("\n"), schema)
}

/// Parse a policy-set source and strict-validate it against the schema.
fn parse_and_validate(src: &str, schema: &Schema) -> Result<PolicySet, AuthzError> {
    let policies = PolicySet::from_str(src)
        .map_err(|e| AuthzError::PolicyLoad(format!("parsing policies: {e}")))?;
    let validator = Validator::new(schema.clone());
    let result = validator.validate(&policies, ValidationMode::Strict);
    if result.validation_passed() {
        Ok(policies)
    } else {
        let errors: Vec<String> = result
            .validation_errors()
            .map(ToString::to_string)
            .collect();
        Err(AuthzError::PolicyLoad(format!(
            "policy set failed schema validation: {}",
            errors.join("; ")
        )))
    }
}

/// Spawn the periodic policy-reload task (swap-on-valid).
fn spawn_reload(dir: PathBuf, schema: Arc<Schema>, policies: Arc<ArcSwap<PolicySet>>, secs: u64) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(secs));
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            match load_and_validate(&dir, &schema) {
                Ok(next) => {
                    policies.store(Arc::new(next));
                    tracing::debug!("cedar policy set reloaded");
                }
                Err(e) => tracing::warn!("cedar policy reload skipped (invalid): {e}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::access::authz::request::Attr;

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

    /// A request built from a principal carrying roles, for the role-aware tests.
    fn req_with<'a>(
        subject: &'a str,
        roles: &'a [String],
        scopes: &'a [String],
    ) -> AuthzRequest<'a> {
        AuthzRequest {
            operation_id: "composition_create",
            kind: ResourceKind::Composition,
            access: AccessMode::Create,
            subject,
            roles,
            scopes,
            organization: Some("org1".to_owned()),
            patient: Some(Attr::One("p-1".to_owned())),
            template: None,
        }
    }

    /// A policy keyed on a ROLE must be able to decide, which it cannot when the
    /// principal ships with an empty role set (NIST SP 800-162 §2.2: subject
    /// attributes are half of an ABAC decision).
    #[tokio::test]
    async fn role_aware_policy_permits() {
        let engine = CedarEngine::from_policy_src(
            r#"permit(principal, action, resource) when { principal.roles.contains("CLINICIAN") };"#,
        )
        .expect("policy loads");
        let roles = vec!["CLINICIAN".to_owned()];
        assert_eq!(
            engine
                .decide(&req_with("dr-a", &roles, &[]))
                .await
                .expect("decides"),
            Decision::Permit,
        );
        // The same policy denies a caller without the role.
        assert_eq!(
            engine
                .decide(&req_with("dr-b", &[], &[]))
                .await
                .expect("decides"),
            Decision::Deny,
        );
    }

    /// A policy keyed on a SCOPE likewise sees the caller's scopes.
    #[tokio::test]
    async fn scope_aware_policy_denies() {
        let engine = CedarEngine::from_policy_src(
            r#"permit(principal, action, resource) when { principal.scopes.contains("openehr/*.w") };"#,
        )
        .expect("policy loads");
        let scopes = vec!["openehr/*.r".to_owned()];
        assert_eq!(
            engine
                .decide(&req_with("app-1", &[], &scopes))
                .await
                .expect("decides"),
            Decision::Deny,
        );
    }

    /// The subject names the principal, so a policy can be written about one
    /// caller and a decision log identifies who asked.
    #[tokio::test]
    async fn principal_uid_names_the_subject() {
        let engine =
            CedarEngine::from_policy_src(r#"permit(principal == User::"dr-a", action, resource);"#)
                .expect("policy loads");
        assert_eq!(
            engine
                .decide(&req_with("dr-a", &[], &[]))
                .await
                .expect("decides"),
            Decision::Permit,
        );
        assert_eq!(
            engine
                .decide(&req_with("dr-b", &[], &[]))
                .await
                .expect("decides"),
            Decision::Deny,
        );
    }

    /// A subject carrying Cedar's own quoting characters must not be able to
    /// forge a different principal uid.
    #[tokio::test]
    async fn a_quoting_subject_cannot_forge_another_principal() {
        let engine = CedarEngine::from_policy_src(
            r#"permit(principal == User::"admin", action, resource);"#,
        )
        .expect("policy loads");
        // A subject that would close the literal and name `admin` instead.
        let forged = r#"x", action, resource); permit(principal == User::"admin"#;
        assert_eq!(
            engine
                .decide(&req_with(forged, &[], &[]))
                .await
                .expect("decides"),
            Decision::Deny,
        );
    }

    /// Cedar SKIPS a policy that errors and reports it in the diagnostics
    /// (<https://docs.cedarpolicy.com/auth/authorization.html>), so discarding
    /// them lets an erroring `forbid` silently stop forbidding. A policy set
    /// that cannot be evaluated is not a decision — it is a fail-closed error.
    ///
    /// Two layers protect this, and the test exercises the second. Schema
    /// validation at LOAD refuses the common shapes outright — an unsafe read of
    /// an optional attribute (`principal.patient` on a principal that may carry
    /// none) never reaches evaluation, it fails `from_policy_src` with
    /// `PolicyLoad`. Arithmetic overflow is not caught there, so it is the honest
    /// probe for the runtime backstop.
    #[tokio::test]
    async fn erroring_forbid_policy_is_a_fail_closed_error() {
        let engine = CedarEngine::from_policy_src(
            r"permit(principal, action, resource);
               forbid(principal, action, resource) when { 9223372036854775807 + 1 > 0 };",
        )
        .expect("policy loads");
        let no_attrs = AuthzRequest {
            operation_id: "composition_create",
            kind: ResourceKind::Composition,
            access: AccessMode::Create,
            subject: "dr-a",
            roles: &[],
            scopes: &[],
            organization: None,
            patient: None,
            template: None,
        };
        let outcome = engine.decide(&no_attrs).await;
        assert!(
            matches!(outcome, Err(AuthzError::Evaluation(_))),
            "an erroring forbid must surface, not be skipped: {outcome:?}",
        );
    }

    #[test]
    fn shipped_schema_is_valid() {
        assert!(build_schema().is_ok());
    }

    #[test]
    fn empty_policy_source_is_rejected() {
        // No `*.cedar` content → deny-by-default engine would still build from a
        // non-empty parse, but a syntactically invalid policy must be rejected.
        let bad = CedarEngine::from_policy_src("permit(this is not cedar);");
        assert!(matches!(bad, Err(AuthzError::PolicyLoad(_))));
    }

    #[test]
    fn policy_referencing_unknown_attribute_fails_validation() {
        // `resource.nonsense` is not in the schema → validation error at load.
        let src = r#"permit(principal, action, resource) when { resource.nonsense == "x" };"#;
        let e = CedarEngine::from_policy_src(src).expect_err("must reject");
        assert!(matches!(e, AuthzError::PolicyLoad(_)), "got {e:?}");
    }

    #[tokio::test]
    async fn deny_by_default_when_no_policy_permits() {
        // A forbid-only / unrelated policy set → deny.
        let src = r#"forbid(principal, action, resource) when { resource has patient && resource.patient == "blocked" };"#;
        let engine = CedarEngine::from_policy_src(src).expect("build");
        let r = req(Some(Attr::One("p1".to_owned())), None);
        assert_eq!(engine.decide(&r).await.unwrap(), Decision::Deny);
    }

    #[tokio::test]
    async fn permit_then_forbid_overrides() {
        let src = r#"
permit(principal, action, resource);
forbid(principal, action, resource) when { resource has patient && resource.patient == "blocked" };
"#;
        let engine = CedarEngine::from_policy_src(src).expect("build");
        // Not blocked → permit.
        assert_eq!(
            engine
                .decide(&req(Some(Attr::One("ok".to_owned())), None))
                .await
                .unwrap(),
            Decision::Permit
        );
        // Blocked → forbid overrides permit.
        assert_eq!(
            engine
                .decide(&req(Some(Attr::One("blocked".to_owned())), None))
                .await
                .unwrap(),
            Decision::Deny
        );
    }

    #[tokio::test]
    async fn fan_out_all_must_permit_and_empty_permits() {
        // Permit iff patient == "ok"; a set with one non-"ok" element denies.
        let src = r#"permit(principal, action, resource) when { resource has patient && resource.patient == "ok" };"#;
        let engine = CedarEngine::from_policy_src(src).expect("build");
        assert_eq!(
            engine
                .decide(&req(
                    Some(Attr::Set(vec!["ok".to_owned(), "ok".to_owned()])),
                    None
                ))
                .await
                .unwrap(),
            Decision::Permit
        );
        assert_eq!(
            engine
                .decide(&req(
                    Some(Attr::Set(vec!["ok".to_owned(), "bad".to_owned()])),
                    None
                ))
                .await
                .unwrap(),
            Decision::Deny
        );
        // Empty set → no combinations → vacuous permit.
        assert_eq!(
            engine
                .decide(&req(Some(Attr::Set(vec![])), None))
                .await
                .unwrap(),
            Decision::Permit
        );
    }
}
