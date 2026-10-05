// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Name conversions from openEHR spec identifiers to idiomatic Rust names.

/// Convert an openEHR class name to a Rust type name in `PascalCase`.
///
/// Each `_`-separated segment is title-cased (first char upper, rest lower), so
/// `DV_QUANTITY` → `DvQuantity`, `ISO_OID` → `IsoOid`, `Iso8601_date` →
/// `Iso8601Date`, `EHR_STATUS` → `EhrStatus`.
#[must_use]
pub(crate) fn type_name(spec: &str) -> String {
    spec.split('_')
        .filter(|s| !s.is_empty())
        .map(|seg| {
            let mut cs = seg.chars();
            match cs.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &cs.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect()
}

/// The Rust field identifier for an openEHR attribute name (already `snake_case`),
/// escaping Rust keywords. Returns the identifier to emit; pair with
/// [`serde_rename`] to keep the wire name correct.
#[must_use]
pub(crate) fn field_ident(spec: &str) -> String {
    match spec {
        // Keywords that cannot be raw identifiers (`crate`/`self`/`super`/`Self`),
        // and `use` which we deliberately suffix for readability by convention.
        "use" => "use_".to_string(),
        "crate" => "crate_".to_string(),
        "self" => "self_".to_string(),
        "super" => "super_".to_string(),
        "Self" => "Self_".to_string(),
        // Every other Rust keyword works fine as a raw identifier.
        _ if is_raw_escapable_keyword(spec) => format!("r#{spec}"),
        _ => spec.to_string(),
    }
}

/// The Rust module identifier for a BMM package segment.
///
/// Package names come verbatim from the vendored schemas and may carry
/// characters Rust cannot express in a module path — LANG 1.0.0 publishes
/// `org.openehr.lang.obsolete-elom` — so a hyphen maps to an underscore (the
/// standard crate-name mangling) and keywords escape exactly like
/// [`field_ident`].
#[must_use]
pub(crate) fn module_ident(segment: &str) -> String {
    field_ident(&segment.replace('-', "_"))
}

/// The `SCREAMING_SNAKE_CASE` associated-constant identifier for a BMM constant
/// name (`Terminology_id_openehr` → `TERMINOLOGY_ID_OPENEHR`). BMM constant
/// names are `[A-Za-z0-9_]`, so upper-casing yields a valid Rust identifier.
pub(crate) fn const_ident(spec: &str) -> String {
    spec.to_uppercase()
}

/// Whether `s` is a Rust keyword that must be written as a raw identifier
/// (`r#{s}`) when used as a field name. Excludes `crate`/`self`/`super`/`Self`
/// (which cannot be raw and are handled separately in [`field_ident`]).
fn is_raw_escapable_keyword(s: &str) -> bool {
    matches!(
        s,
        // Strict keywords (2015).
        "as" | "break" | "const" | "continue" | "dyn" | "else" | "enum" | "extern"
        | "false" | "fn" | "for" | "if" | "impl" | "in" | "let" | "loop" | "match"
        | "mod" | "move" | "mut" | "pub" | "ref" | "return" | "static" | "struct"
        | "trait" | "true" | "type" | "unsafe" | "where" | "while"
        // Edition 2018/2024 keywords.
        | "async" | "await" | "gen"
        // Reserved keywords.
        | "abstract" | "become" | "box" | "do" | "final" | "macro" | "override"
        | "priv" | "typeof" | "unsized" | "virtual" | "yield" | "try"
    )
}

/// The `#[serde(rename = "..")]` value needed for a field, if its emitted
/// identifier does not already serialize to the spec name.
///
/// Raw identifiers (`r#type`) serialize as the bare keyword, so they need no
/// rename; suffixed ones (`use_`) do.
#[must_use]
pub(crate) fn serde_rename(spec: &str, ident: &str) -> Option<String> {
    let wire = ident.strip_prefix("r#").unwrap_or(ident);
    if wire == spec {
        None
    } else {
        Some(spec.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_names() {
        assert_eq!(type_name("DV_QUANTITY"), "DvQuantity");
        assert_eq!(type_name("ISO_OID"), "IsoOid");
        assert_eq!(type_name("Iso8601_date"), "Iso8601Date");
        assert_eq!(type_name("EHR_STATUS"), "EhrStatus");
        assert_eq!(type_name("PARTY_SELF"), "PartySelf");
        assert_eq!(type_name("Interval"), "Interval");
    }

    #[test]
    fn field_idents_and_renames() {
        assert_eq!(field_ident("magnitude"), "magnitude");
        assert_eq!(field_ident("type"), "r#type");
        assert_eq!(field_ident("use"), "use_");
        assert_eq!(serde_rename("type", "r#type"), None);
        assert_eq!(serde_rename("use", "use_"), Some("use".to_string()));
        assert_eq!(serde_rename("magnitude", "magnitude"), None);
    }
}
