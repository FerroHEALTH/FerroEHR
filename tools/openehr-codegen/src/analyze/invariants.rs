// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Stage 2, analyze: the assertion-dialect classifier for BMM class
//! invariants, which records each one in openEHR's Eiffel/UML assertion form.
//!
//! An expression is tokenized and classified into a [`Bucket`], the worst
//! bucket of its boolean operands winning; an unrecognised leaf is
//! [`Bucket::Complex`], so the classifier never over-claims.

/// Which R5 emission bucket a BMM invariant expression falls into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Bucket {
    /// Emittable: boolean combinations of leaf forms over the class's own fields
    /// and the validator runtime.
    Emitted,
    /// Within the dialect, but a leaf needs a runtime hook the emitter cannot call;
    /// the `&str` names it.
    RuntimeHookMissing(&'static str),
    /// Beyond the dialect: a quantifier, lambda, navigation or arithmetic over
    /// related objects; the `&str` names the reason.
    Complex(&'static str),
}

impl Bucket {
    /// Combines two verdicts, the most blocking winning.
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

/// Splits an expression into word and paren tokens, by `char` (the dialect
/// carries curly quotes).
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

/// The top-level boolean connectives, lowest-binding first; [`Bucket::worse`]
/// is associative and commutative, so any split point gives the same verdict.
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

/// Leaf predicates the validator runtime realises.
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

/// The recognised runtime-backed leaf predicates;
/// `plan::overrides::DIALECT_PREDICATES` maps each to its function.
pub(crate) fn runtime_predicates() -> &'static [&'static str] {
    &RUNTIME_PREDICATES
}

/// Classifies a leaf: blocking signals first, then the emittable forms, else
/// `Complex`.
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

/// Quantifiers, lambdas, boolean method calls and arithmetic over related
/// objects; `is_empty`/`empty` are emittable atoms.
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

/// The runtime hooks the emitter has no access to, checked before navigation
/// since `.has_code…` reads as membership.
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

/// Whether a leaf is an emittable atom over the class's own attribute path.
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

/// A simple attribute reference, dotted sub-fields allowed, no method calls.
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

    /// A boolean method call (`key.is_justified`) makes the whole `and` complex.
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
