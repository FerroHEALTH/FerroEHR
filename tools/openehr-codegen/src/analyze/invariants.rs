// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 2 — ANALYZE: the **assertion-dialect** analyzer for BMM class
//! invariants.
//!
//! `BMM_CLASS.invariants` records each invariant as an expression string in
//! openEHR's Eiffel/UML assertion surface, not `base_expressions.g4`:
//! `not links.is_empty`, `X /= Void implies not X.is_empty`, `A xor B`,
//! `valid_iso8601_date (value)`, the terminology/repository predicates,
//! quantifiers (`for_all …`) and cross-object navigation.
//!
//! This module tokenizes an expression and classifies it into one of the three
//! [`Bucket`]s by a paren-aware, worst-bucket-wins recursion over the boolean
//! structure; it produces no text. The render stage emits a check for a
//! [`Bucket::Emitted`] verdict and reports the other two.
//!
//! An unrecognised leaf form classifies as [`Bucket::Complex`], never as
//! `Emitted`: the classifier under-claims, so a false "emittable" can never
//! slip a new rejection onto the wire.

/// Which R5 emission bucket a BMM invariant expression falls into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Bucket {
    /// Mechanically emittable: the whole expression reduces to boolean
    /// combinations of leaf forms the emitter can render over the class's own
    /// fields plus the kept validator runtime (`Void`/empty checks, numeric
    /// comparisons, `xor`/`and`/`or`/`implies`, `is_equal ("LITERAL")`, the
    /// ISO-8601 validators, and the runtime-backed leaf predicates).
    Emitted,
    /// Structurally within the dialect, but a leaf predicate needs a runtime
    /// hook the emitter cannot yet call (the openEHR terminology service, a
    /// code-set, the demographic repository, or the versioned-object aggregate
    /// model). Not emitted; listed. The `&str` names the missing hook.
    RuntimeHookMissing(&'static str),
    /// Beyond the assertion-dialect scope: a quantifier (`for_all`/`exists`), a
    /// lambda predicate, cross-object navigation, or arithmetic over related
    /// objects. Stays hand-written; listed. The `&str` names the reason.
    Complex(&'static str),
}

impl Bucket {
    /// Combine two sub-expression verdicts: the most blocking wins
    /// (`Complex` > `RuntimeHookMissing` > `Emitted`). A boolean expression is
    /// only emittable if every operand is.
    fn worse(self, other: Bucket) -> Bucket {
        fn rank(b: &Bucket) -> u8 {
            match b {
                Bucket::Emitted => 0,
                Bucket::RuntimeHookMissing(_) => 1,
                Bucket::Complex(_) => 2,
            }
        }
        if rank(&other) > rank(&self) {
            other
        } else {
            self
        }
    }
}

/// Classify one BMM invariant expression into its R5 bucket.
pub(crate) fn classify(expr: &str) -> Bucket {
    classify_tokens(&tokenize(expr))
}

/// Split an expression into word/paren tokens. A token is a lone `(` or `)`, or
/// a maximal run of non-whitespace, non-paren characters. Char-based (the
/// dialect carries multi-byte curly quotes `“ ”`), so no byte slicing.
fn tokenize(expr: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    for c in expr.chars() {
        if c == '(' || c == ')' {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
            toks.push(c.to_string());
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    toks
}

/// The top-level boolean connectives, lowest-binding first. Splitting at any one
/// and recursing both sides is sound because [`Bucket::worse`] is associative
/// and commutative — the exact split point does not change the verdict.
const CONNECTIVES: [&str; 4] = ["implies", "xor", "or", "and"];

fn classify_tokens(toks: &[String]) -> Bucket {
    let toks = strip_outer_parens(toks);
    if toks.is_empty() {
        return Bucket::Complex("empty expression");
    }
    // Split at the first depth-0 connective, if any.
    for kw in CONNECTIVES {
        if let Some(i) = top_level_index(toks, kw)
            && let Some((left, tail)) = toks.split_at_checked(i)
            && let Some((_connective, mut rest)) = tail.split_first()
        {
            // Drop a trailing `then`/`else` of `and then` / `or else`.
            if let Some((head, tail)) = rest.split_first()
                && matches!(head.as_str(), "then" | "else")
            {
                rest = tail;
            }
            return classify_tokens(left).worse(classify_tokens(rest));
        }
    }
    // A leading `not` does not change the bucket; classify the operand.
    if let Some((head, rest)) = toks.split_first()
        && head == "not"
    {
        return classify_tokens(rest);
    }
    classify_leaf(toks)
}

/// If `toks` is a single fully-enclosing `( … )` pair, return the inner slice;
/// otherwise return `toks` unchanged.
fn strip_outer_parens(toks: &[String]) -> &[String] {
    // `inner` is the token run between a leading `(` and the final token; its
    // absence (fewer than two tokens, or no leading paren) means nothing to strip.
    let Some((first, tail)) = toks.split_first() else {
        return toks;
    };
    if first != "(" {
        return toks;
    }
    let Some((_last, inner)) = tail.split_last() else {
        return toks;
    };
    // The opening paren must close exactly at the final token.
    let mut depth = 0i32;
    for (i, t) in toks.iter().enumerate() {
        match t.as_str() {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth == 0 {
                    return if i + 1 == toks.len() {
                        strip_outer_parens(inner)
                    } else {
                        toks
                    };
                }
            }
            _ => {}
        }
    }
    toks
}

/// The index of the first `kw` token at paren depth 0, if present.
fn top_level_index(toks: &[String], kw: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (i, t) in toks.iter().enumerate() {
        match t.as_str() {
            "(" => depth += 1,
            ")" => depth -= 1,
            w if depth == 0 && w == kw => return Some(i),
            _ => {}
        }
    }
    None
}

/// Named leaf predicate functions the kept validator runtime already realises
/// (`crates/openehr-rm/src/validate.rs` + the ISO-8601 validators + the
/// `push_*` invariant cores): a leaf that is exactly one of these applied to a
/// field is mechanically emittable as a call into that runtime.
const RUNTIME_PREDICATES: [&str; 8] = [
    "valid_iso8601_date",
    "valid_iso8601_time",
    "valid_iso8601_date_time",
    "valid_iso8601_duration",
    "valid_magnitude_status",
    "valid_percentage",
    "valid_proportion_kind",
    "is_valid_match_code",
];

/// The classifier's recognised runtime-backed leaf predicates — the ones a
/// leaf `pred (field)` classifies as [`Bucket::Emitted`] because a named runtime
/// function realizes them. The `plan::overrides::DIALECT_PREDICATES` table maps
/// each to that function; the emitter-invariant suite pins the two in lockstep.
pub(crate) fn runtime_predicates() -> &'static [&'static str] {
    &RUNTIME_PREDICATES
}

/// Classify a leaf (no top-level connective). Order matters: the blocking
/// signals (quantifier/navigation/arithmetic, then terminology/repository/
/// aggregate hooks) are checked before the emittable forms, and an
/// unrecognised leaf is conservatively `Complex`.
fn classify_leaf(toks: &[String]) -> Bucket {
    let joined = toks.join(" ");
    let has = |needle: &str| toks.iter().any(|t| t.contains(needle));

    if let Some(bucket) = structural_depth(toks, &has) {
        return bucket;
    }
    if let Some(bucket) = missing_runtime_hook(&has) {
        return bucket;
    }
    if let Some(bucket) = cross_object_navigation(toks, &has) {
        return bucket;
    }

    // ── Emittable leaf forms ──
    // A runtime-backed predicate applied to a field: `pred (field)` / `pred(field)`.
    if RUNTIME_PREDICATES.iter().any(|p| has(p)) {
        return Bucket::Emitted;
    }
    // `X.is_equal ("LITERAL")` — string-literal equality (curly or straight quotes).
    if has(".is_equal") && (joined.contains('"') || joined.contains('“')) {
        return Bucket::Emitted;
    }
    if is_emittable_atom(toks) {
        return Bucket::Emitted;
    }
    Bucket::Complex("unrecognised leaf form")
}

/// True structural depth: quantifiers, lambdas, boolean method calls and
/// arithmetic over related objects.
///
/// A boolean-returning *method* call is a BMM function, not a stored property,
/// so the emitter has no field to project it from and it is not mechanically
/// evaluable. `is_empty`/`empty` are the recognised emptiness methods handled
/// as emittable atoms; `is_justified` (`ITEM_TAG`) is a real function call.
fn structural_depth(toks: &[String], has: &impl Fn(&str) -> bool) -> Option<Bucket> {
    if has("for_all") || has("forall") || has("exists") || toks.iter().any(|t| t == "for") {
        return Some(Bucket::Complex("quantifier over a collection"));
    }
    if toks.iter().any(|t| t == "|") {
        return Some(Bucket::Complex("lambda predicate"));
    }
    if has(".is_justified") {
        return Some(Bucket::Complex(
            "boolean method call (a BMM function, not a field)",
        ));
    }
    if has(".diff") || has(".to_seconds") || has(".mod") || has(".floor") || has(".item") {
        return Some(Bucket::Complex("cross-object arithmetic / navigation"));
    }
    if toks
        .iter()
        .any(|t| matches!(t.as_str(), "-" | "+" | "*" | "/"))
    {
        return Some(Bucket::Complex("arithmetic over related objects"));
    }
    None
}

/// The runtime hooks the emitter has no access to: terminology, code sets, the
/// demographic repository, and the versioned-object aggregate model.
///
/// Checked before the generic membership/navigation signals, because the
/// terminology/code-set predicates read as `.has_code…` (a `.has` substring)
/// and a code lookup is a missing *hook*, not irreducible structural depth.
fn missing_runtime_hook(has: &impl Fn(&str) -> bool) -> Option<Bucket> {
    if has("terminology") || has("Terminology") || has("has_code_for_group_id") {
        return Some(Bucket::RuntimeHookMissing("openEHR terminology service"));
    }
    if has("code_set") || has("has_code") {
        return Some(Bucket::RuntimeHookMissing("openEHR code-set access"));
    }
    if has("repository") {
        return Some(Bucket::RuntimeHookMissing("demographic repository access"));
    }
    if has("all_versions")
        || has("all_version_ids")
        || has("version_count")
        || has("latest_version")
    {
        return Some(Bucket::RuntimeHookMissing(
            "versioned-object aggregate model",
        ));
    }
    None
}

/// Membership over a related collection, and navigation off `self`.
fn cross_object_navigation(toks: &[String], has: &impl Fn(&str) -> bool) -> Option<Bucket> {
    if has(".has") || has("has_object") || has("has_key") {
        return Some(Bucket::Complex("membership over a related collection"));
    }
    if toks.iter().any(|t| t == "self")
        || has("parent.")
        || has(".source")
        || has(".target")
        || has(".relationships")
        || has(".reverse_relationships")
        || has(".description")
        || has(".data")
        || has(".origin")
    {
        return Some(Bucket::Complex("cross-object navigation"));
    }
    None
}

/// Whether a leaf is one of the simple emittable atoms: an emptiness/`Void`
/// check, a numeric comparison, a boolean field, or a field/field equality —
/// each over the class's own (dot-free) attribute path.
fn is_emittable_atom(toks: &[String]) -> bool {
    // Drop any leading `not`.
    let toks = match toks.split_first() {
        Some((head, rest)) if head == "not" => rest,
        _ => toks,
    };
    match toks {
        // `field.is_empty` / `field.empty` (Eiffel emptiness methods, not file
        // extensions — the pedantic extension lint is a false positive here).
        #[expect(
            clippy::case_sensitive_file_extension_comparisons,
            reason = "`.is_empty`/`.empty` are Eiffel method calls in a BMM assertion, not file extensions"
        )]
        [a] if a.ends_with(".is_empty") || a.ends_with(".empty") => is_field_path(trim_suffix(a)),
        // a lone boolean field: `is_archetype_root`, `is_inline`, `is_masked`, …
        [a] => is_field_path(a),
        // `X /= Void`, `X = Void`, `X /= void`, `X = void`
        [a, op, b]
            if matches!(op.as_str(), "=" | "/=") && matches!(b.as_str(), "Void" | "void") =>
        {
            is_field_path(a)
        }
        // numeric comparison: `size >= 0`, `denominator /= 0.0`, `sequence_nr >= 1`
        [a, op, b]
            if matches!(op.as_str(), "=" | "/=" | ">=" | "<=" | ">" | "<") && is_number(b) =>
        {
            is_field_path(a)
        }
        // field/field equality: `type = name`, `purpose = name`
        [a, op, b] if op == "=" && is_field_path(a) && is_field_path(b) => true,
        _ => false,
    }
}

fn trim_suffix(a: &str) -> &str {
    a.strip_suffix(".is_empty")
        .or_else(|| a.strip_suffix(".empty"))
        .unwrap_or(a)
}

/// A simple attribute reference: an identifier possibly with dotted sub-fields,
/// but no method calls or navigation the emitter cannot resolve on `self`.
/// (A trailing `.is_empty`/`.empty` is stripped by the caller.)
fn is_field_path(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && !s.starts_with('.')
        && !s.ends_with('.')
}

fn is_number(s: &str) -> bool {
    let s = s.strip_prefix('-').unwrap_or(s);
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_forms_emit() {
        assert_eq!(classify("not links.is_empty"), Bucket::Emitted);
        assert_eq!(
            classify("links /= Void implies not links.is_empty"),
            Bucket::Emitted
        );
        assert_eq!(classify("size >= 0"), Bucket::Emitted);
        assert_eq!(classify("denominator /= 0.0"), Bucket::Emitted);
        assert_eq!(classify("is_periodic xor period = Void"), Bucket::Emitted);
        assert_eq!(classify("is_inline or is_external"), Bucket::Emitted);
        assert_eq!(classify("type = name"), Bucket::Emitted);
        assert_eq!(classify("valid_iso8601_date(value)"), Bucket::Emitted);
        assert_eq!(classify("scheme.is_equal (\"EHR\")"), Bucket::Emitted);
        // Nested parens + `and then`.
        assert_eq!(
            classify("(events /= Void and then not events.is_empty) or summary /= Void"),
            Bucket::Emitted
        );
    }

    #[test]
    fn terminology_and_codeset_need_hooks() {
        assert!(matches!(
            classify("code_set(Code_set_id_languages).has_code(language)"),
            Bucket::RuntimeHookMissing(_)
        ));
        assert!(matches!(
            classify(
                "terminology (Terminology_id_openehr).has_code_for_group_id (Group_id_setting, setting.defining_code)"
            ),
            Bucket::RuntimeHookMissing(_)
        ));
        assert!(matches!(
            classify("all_version_ids.count = version_count"),
            Bucket::RuntimeHookMissing(_)
        ));
    }

    #[test]
    fn quantifiers_and_navigation_are_complex() {
        assert!(matches!(
            classify("for_all c in compositions | c.type.is_equal (\"VERSIONED_COMPOSITION\")"),
            Bucket::Complex(_)
        ));
        assert!(matches!(
            classify("interval_start_time = time - width"),
            Bucket::Complex(_)
        ));
        assert!(matches!(
            classify("source /= Void and then source.relationships.has (self)"),
            Bucket::Complex(_)
        ));
    }

    /// A boolean *method* call (`key.is_justified`, `ITEM_TAG` `Inv_key_valid`)
    /// is Complex — the emitter has no field to project it from — so the whole
    /// `and` is Complex even though `not key.is_empty` alone would emit.
    #[test]
    fn boolean_method_call_is_complex() {
        assert!(matches!(
            classify("not key.is_empty and key.is_justified"),
            Bucket::Complex(_)
        ));
        assert!(matches!(classify("key.is_justified"), Bucket::Complex(_)));
    }

    /// An implies whose consequent needs a hook is hook-missing, not emitted:
    /// worst-bucket-wins across the boolean structure.
    #[test]
    fn worst_bucket_wins() {
        assert!(matches!(
            classify(
                "mode /= Void implies terminology (Terminology_id_openehr).has_code_for_group_id (Group_id_participation_mode, mode.defining_code)"
            ),
            Bucket::RuntimeHookMissing(_)
        ));
    }
}
